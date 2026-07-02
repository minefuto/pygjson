use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError, PyUnicodeDecodeError};
use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyNone, PySlice, PyString, PyTuple,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

thread_local! {
    static TRIE_CACHE: RefCell<HashMap<u64, Vec<(Box<[Box<str>]>, Arc<gjson::CompiledPaths>)>>> =
        RefCell::new(HashMap::new());
}

const TRIE_CACHE_MAX: usize = 256;

fn hash_paths(paths: &[&str]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in paths {
        p.hash(&mut h);
    }
    h.finish()
}

fn get_or_build_compiled(paths: &[&str]) -> Arc<gjson::CompiledPaths> {
    let key = hash_paths(paths);
    TRIE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(bucket) = cache.get(&key) {
            for (stored, compiled) in bucket {
                if stored.len() == paths.len()
                    && stored.iter().zip(paths).all(|(a, b)| a.as_ref() == *b)
                {
                    return Arc::clone(compiled);
                }
            }
        }
        if cache.len() >= TRIE_CACHE_MAX {
            cache.clear();
        }
        let compiled = Arc::new(gjson::compile_paths(paths));
        let stored: Box<[Box<str>]> = paths.iter().map(|s| Box::from(*s)).collect();
        cache.entry(key).or_default().push((stored, Arc::clone(&compiled)));
        compiled
    })
}

/// Keeps the Python objects backing a `list[str]` / `list[Path]` argument
/// alive so their path strings can be borrowed without copying.
enum PathListGuard<'py> {
    Strs(Vec<Bound<'py, PyString>>),
    Paths(Vec<PyRef<'py, Path>>),
}

impl PathListGuard<'_> {
    fn path_refs(&self) -> PyResult<Vec<&str>> {
        match self {
            Self::Strs(v) => v.iter().map(|s| s.to_str()).collect(),
            Self::Paths(v) => Ok(v.iter().map(|p| p.path.as_str()).collect()),
        }
    }
}

fn extract_path_list<'py>(list: &Bound<'py, PyList>) -> PyResult<PathListGuard<'py>> {
    if !list.is_empty() && list.get_item(0)?.cast::<Path>().is_ok() {
        let mut v = Vec::with_capacity(list.len());
        for item in list.iter() {
            let cp = item.cast_into::<Path>().map_err(|_| {
                PyTypeError::new_err("paths must be a list[str] or a list[Path], not a mix")
            })?;
            v.push(cp.borrow());
        }
        Ok(PathListGuard::Paths(v))
    } else {
        let mut v = Vec::with_capacity(list.len());
        for item in list.iter() {
            let s = item
                .cast_into::<PyString>()
                .map_err(|e| PyTypeError::new_err(e.to_string()))?;
            v.push(s);
        }
        Ok(PathListGuard::Strs(v))
    }
}

fn compiled_from_path_arg(paths: &Bound<'_, PyAny>) -> PyResult<Arc<gjson::CompiledPaths>> {
    let list = paths.cast::<PyList>()?;
    let guard = extract_path_list(&list)?;
    Ok(get_or_build_compiled(&guard.path_refs()?))
}

/// A pre-compiled gjson path, ready to be passed to `get`, `get_bytes`, `get_many`, or `get_many_bytes`.
#[pyclass(module = "pygjson._pygjson", name = "Path")]
pub struct Path {
    path: String,
}

#[pymethods]
impl Path {
    fn __repr__(&self) -> String {
        format!("Path({:?})", self.path)
    }
}

/// Owner of (and raw view into) immutable UTF-8 JSON text.
enum RawJson {
    /// Borrows the UTF-8 buffer of a Python `str` (its cached UTF-8 form) or
    /// `bytes` (validated as UTF-8 at construction). `owner` keeps the
    /// buffer alive; nothing ever writes through `ptr`.
    Py {
        owner: Py<PyAny>,
        ptr: *const u8,
        len: usize,
    },
    /// Text owned by pygjson itself (gjson-recomposed values such as
    /// `#.field` arrays, count results, or `[...]` slices).
    Owned(Arc<str>),
}

// SAFETY: `ptr`/`len` point into a buffer owned by `owner`:
// - for `str`, the UTF-8 cache returned by PyUnicode_AsUTF8AndSize, which
//   CPython keeps valid and unmodified for the lifetime of the object;
// - for `bytes`, the immutable payload of the bytes object.
// The strong reference in `owner` keeps the object alive for as long as this
// value exists, the buffer is never written through the pointer, and pyo3
// defers refcount decrements when a `Py` is dropped without the interpreter
// attached, so sharing across threads is sound.
unsafe impl Send for RawJson {}
unsafe impl Sync for RawJson {}

impl RawJson {
    fn as_str(&self) -> &str {
        match self {
            RawJson::Py { ptr, len, .. } => unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(*ptr, *len))
            },
            RawJson::Owned(s) => s,
        }
    }

    fn clone_ref(&self, py: Python<'_>) -> Self {
        match self {
            RawJson::Py { owner, ptr, len } => RawJson::Py {
                owner: owner.clone_ref(py),
                ptr: *ptr,
                len: *len,
            },
            RawJson::Owned(s) => RawJson::Owned(Arc::clone(s)),
        }
    }

    fn from_pystring(s: &Bound<'_, PyString>) -> PyResult<Self> {
        let text = s.to_str()?;
        Ok(RawJson::Py {
            owner: s.as_any().clone().unbind(),
            ptr: text.as_ptr(),
            len: text.len(),
        })
    }

    fn from_pybytes(py: Python<'_>, b: &Bound<'_, PyBytes>) -> PyResult<Self> {
        let bytes = b.as_bytes();
        let checked = if bytes.len() >= DETACH_THRESHOLD {
            py.detach(|| std::str::from_utf8(bytes))
        } else {
            std::str::from_utf8(bytes)
        };
        checked.map_err(|e| utf8_decode_err(py, bytes, e))?;
        Ok(RawJson::Py {
            owner: b.as_any().clone().unbind(),
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        })
    }
}

fn utf8_decode_err(py: Python<'_>, bytes: &[u8], e: std::str::Utf8Error) -> PyErr {
    match PyUnicodeDecodeError::new_utf8(py, bytes, e) {
        Ok(bound) => bound.into(),
        Err(err) => err,
    }
}

/// Documents at or above this size release the interpreter (GIL) while the
/// pure-Rust query runs, letting other Python threads make progress. Smaller
/// documents skip the thread-state save/restore, which would otherwise cost
/// more than the query itself.
const DETACH_THRESHOLD: usize = 32 * 1024;

fn get_detached<'a>(py: Python<'_>, json: &'a str, path: &'a str) -> gjson::Value<'a> {
    if json.len() >= DETACH_THRESHOLD {
        py.detach(|| gjson::get(json, path))
    } else {
        gjson::get(json, path)
    }
}

/// A JSON value returned by `get` / `parse`.
///
/// The wrapper borrows the raw JSON text of the source Python object (kept
/// alive via a strong reference) together with the byte range that this
/// particular value occupies inside it. Child values produced by `get`,
/// iteration, etc. share the same underlying buffer instead of cloning the
/// text, which avoids a fresh heap allocation per child element.
#[pyclass(module = "pygjson._pygjson", name = "Result")]
pub struct JsonResult {
    raw: RawJson,
    start: usize,
    end: usize,
    kind: gjson::Kind,
    exists: bool,
    info: u32,
    str_cache: OnceLock<Box<str>>,
}

impl JsonResult {
    fn raw_slice(&self) -> &str {
        &self.raw.as_str()[self.start..self.end]
    }

    fn parsed(&self) -> gjson::Value<'_> {
        gjson::Value::from_raw_json(self.raw_slice(), self.info)
    }

    /// String content per gjson `Value::str` semantics, without re-scanning:
    /// unescaped strings are sliced straight out of the raw text; escaped
    /// strings are unescaped once and cached.
    fn str_value(&self) -> &str {
        match self.kind {
            gjson::Kind::True => "true",
            gjson::Kind::False => "false",
            gjson::Kind::Object | gjson::Kind::Array | gjson::Kind::Number => self.raw_slice(),
            gjson::Kind::String => {
                if gjson::Value::info_has_escapes(self.info) {
                    self.str_cache
                        .get_or_init(|| self.parsed().str().to_string().into_boxed_str())
                } else {
                    let raw = self.raw_slice();
                    &raw[1..raw.len() - 1]
                }
            }
            gjson::Kind::Null => "",
        }
    }

    fn from_owned_parts(text: &str, info: u32, kind: gjson::Kind, exists: bool) -> Self {
        let raw: Arc<str> = Arc::from(text);
        let end = raw.len();
        Self {
            raw: RawJson::Owned(raw),
            start: 0,
            end,
            kind,
            exists,
            info,
            str_cache: OnceLock::new(),
        }
    }

    fn from_owned_text(text: &str, kind: gjson::Kind, exists: bool) -> Self {
        let info = gjson::parse(text).info_bits();
        Self::from_owned_parts(text, info, kind, exists)
    }

    fn child(py: Python<'_>, parent: &RawJson, child: gjson::Value<'_>) -> Self {
        let kind = child.kind();
        let exists = child.exists();
        let info = child.info_bits();
        let child_text = child.json();
        if !child_text.is_empty() {
            let parent_bytes = parent.as_str().as_bytes();
            let parent_start_addr = parent_bytes.as_ptr() as usize;
            let parent_end_addr = parent_start_addr + parent_bytes.len();
            let child_start_addr = child_text.as_ptr() as usize;
            if child_start_addr >= parent_start_addr
                && child_start_addr + child_text.len() <= parent_end_addr
            {
                let start = child_start_addr - parent_start_addr;
                return Self {
                    raw: parent.clone_ref(py),
                    start,
                    end: start + child_text.len(),
                    kind,
                    exists,
                    info,
                    str_cache: OnceLock::new(),
                };
            }
        }
        Self::from_owned_parts(child_text, info, kind, exists)
    }
}

#[pymethods]
impl JsonResult {
    /// Return the Python type corresponding to this value's JSON kind.
    ///
    /// Null   → None
    /// True/False → bool
    /// Number → int (integer) or float (floating-point)
    /// String → str
    /// Array  → list
    /// Object → dict
    #[getter]
    fn type_(&self, py: Python<'_>) -> Py<PyAny> {
        match self.kind {
            gjson::Kind::Null => PyNone::get(py).as_any().clone().unbind(),
            gjson::Kind::False | gjson::Kind::True => py.get_type::<PyBool>().into_any().unbind(),
            gjson::Kind::Number => {
                let s = self.raw_slice();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    py.get_type::<PyFloat>().into_any().unbind()
                } else {
                    py.get_type::<PyInt>().into_any().unbind()
                }
            }
            gjson::Kind::String => py.get_type::<PyString>().into_any().unbind(),
            gjson::Kind::Array => py.get_type::<PyList>().into_any().unbind(),
            gjson::Kind::Object => py.get_type::<PyDict>().into_any().unbind(),
        }
    }

    /// Return the inner value as the Python type indicated by `type_`.
    ///
    /// Null → None; bool kinds → bool; Number → int or float;
    /// String → str; Array → list[Result]; Object → dict[str, Result].
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.kind {
            gjson::Kind::Null => Ok(PyNone::get(py).as_any().clone().unbind()),
            gjson::Kind::False | gjson::Kind::True => {
                Ok(self.parsed().bool().into_pyobject(py)?.as_any().clone().unbind())
            }
            gjson::Kind::Number => {
                let s = self.raw_slice();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    Ok(self.parsed().f64().into_pyobject(py)?.into_any().unbind())
                } else if s.starts_with('-') {
                    Ok(self.parsed().i64().into_pyobject(py)?.into_any().unbind())
                } else {
                    Ok(self.parsed().u64().into_pyobject(py)?.into_any().unbind())
                }
            }
            gjson::Kind::String => Ok(self.str_value().into_pyobject(py)?.into_any().unbind()),
            gjson::Kind::Array => {
                let list = PyList::empty(py);
                let parsed = self.parsed();
                let mut err: Option<PyErr> = None;
                parsed.each(|_k, v| {
                    let child = JsonResult::child(py, &self.raw, v);
                    match Py::new(py, child) {
                        Ok(obj) => match list.append(obj) {
                            Ok(()) => true,
                            Err(e) => {
                                err = Some(e);
                                false
                            }
                        },
                        Err(e) => {
                            err = Some(e);
                            false
                        }
                    }
                });
                if let Some(e) = err {
                    return Err(e);
                }
                Ok(list.into_any().unbind())
            }
            gjson::Kind::Object => {
                let dict = PyDict::new(py);
                let parsed = self.parsed();
                let mut err: Option<PyErr> = None;
                parsed.each(|k, v| {
                    let key = k.str().to_string();
                    let child = JsonResult::child(py, &self.raw, v);
                    match Py::new(py, child) {
                        Ok(obj) => match dict.set_item(key, obj) {
                            Ok(()) => true,
                            Err(e) => {
                                err = Some(e);
                                false
                            }
                        },
                        Err(e) => {
                            err = Some(e);
                            false
                        }
                    }
                });
                if let Some(e) = err {
                    return Err(e);
                }
                Ok(dict.into_any().unbind())
            }
        }
    }

    /// Whether the value was actually present in the source JSON.
    fn exists(&self) -> bool {
        self.exists
    }

    /// String representation of the value (matches `gjson::Value::str`).
    /// Same as `str(value)` in Python.
    fn to_str(&self) -> String {
        self.str_value().to_string()
    }

    /// Integer value. Uses `u64` for non-negative values, `i64` for negative.
    fn to_int(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.raw_slice().starts_with('-') {
            Ok(self.parsed().i64().into_pyobject(py)?.into_any().unbind())
        } else {
            Ok(self.parsed().u64().into_pyobject(py)?.into_any().unbind())
        }
    }

    /// Floating point value.
    /// Same as `float(value)` in Python.
    fn to_float(&self) -> f64 {
        self.parsed().f64()
    }

    /// Boolean value via gjson's interpretation: `true` for the JSON boolean `true`,
    /// non-zero for numbers; `false` for all other types (string, null, array, object).
    fn to_bool(&self) -> bool {
        self.parsed().bool()
    }

    /// Get a child value at the given gjson path.
    /// Accepts either a `str` or a `Path`.
    fn get(&self, py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
        if let Ok(cp) = path.cast::<Path>() {
            let borrow = cp.borrow();
            // SAFETY: raw_slice() is always valid UTF-8
            let v = unsafe { gjson::get_bytes(self.raw_slice().as_bytes(), &borrow.path) };
            return Ok(JsonResult::child(py, &self.raw, v));
        }
        let s = path.extract::<&str>()?;
        // SAFETY: raw_slice() is always valid UTF-8
        let v = unsafe { gjson::get_bytes(self.raw_slice().as_bytes(), s) };
        Ok(JsonResult::child(py, &self.raw, v))
    }

    /// Get child values at each of the given gjson paths.
    /// Accepts either a `list[str]` or a `list[Path]`.
    /// When `list[Path]` is passed, the internal trie is cached and
    /// reused across calls with the same compiled path objects.
    fn get_many(&self, py: Python<'_>, paths: &Bound<'_, PyAny>) -> PyResult<Vec<JsonResult>> {
        let compiled = compiled_from_path_arg(paths)?;
        // SAFETY: raw_slice() is always valid UTF-8
        let vs = unsafe {
            gjson::get_many_compiled_bytes(self.raw_slice().as_bytes(), &compiled)
        };
        Ok(vs.into_iter().map(|v| JsonResult::child(py, &self.raw, v)).collect())
    }

    /// Membership test: `item in value`.
    ///
    /// For Object: returns `True` if `item` is a key in the object.
    /// For Array: returns `True` if any element's string representation equals `item`.
    /// Other kinds raise `TypeError`.
    fn __contains__(&self, item: &str) -> PyResult<bool> {
        match self.kind {
            gjson::Kind::Object => {
                let mut found = false;
                self.parsed().each(|k, _v| {
                    if k.str() == item {
                        found = true;
                        false
                    } else {
                        true
                    }
                });
                Ok(found)
            }
            gjson::Kind::Array => {
                let mut found = false;
                self.parsed().each(|_k, v| {
                    if v.str() == item {
                        found = true;
                        false
                    } else {
                        true
                    }
                });
                Ok(found)
            }
            _ => Err(PyTypeError::new_err(
                "__contains__ is only supported for Array and Object values",
            )),
        }
    }

    /// Number of elements: chars for String, elements for Array/Object.
    fn __len__(&self) -> PyResult<usize> {
        match self.kind {
            gjson::Kind::String => Ok(self.str_value().chars().count()),
            gjson::Kind::Array | gjson::Kind::Object => {
                let mut count = 0usize;
                self.parsed().each(|_k, _v| {
                    count += 1;
                    true
                });
                Ok(count)
            }
            _ => Err(PyTypeError::new_err("Result has no len()")),
        }
    }

    /// Iterate: String → chars, Array → Results, Object → keys.
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<ValueIterator>> {
        let it = match self.kind {
            gjson::Kind::String => ValueIterator::for_string_chars(self),
            gjson::Kind::Array => ValueIterator::for_array_values(py, self),
            gjson::Kind::Object => ValueIterator::for_object_keys(self),
            _ => {
                return Err(PyTypeError::new_err(
                    "Result is not iterable (only String, Array, and Object are iterable)",
                ));
            }
        };
        Py::new(py, it)
    }

    /// Subscript access.
    ///
    /// String: int → Nth code point; slice → substring; str → TypeError
    /// Array:  int → Result; slice → Array Result of selected elements; str → TypeError
    /// Object: str → Result; int/slice → KeyError
    /// Null:   int → IndexError; slice → empty Result; str → TypeError
    fn __getitem__(&self, key: &Bound<'_, PyAny>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.kind {
            gjson::Kind::String => {
                if let Ok(slice) = key.cast::<PySlice>() {
                    let chars: Vec<char> = self.str_value().chars().collect();
                    let idx = slice.indices(chars.len() as isize)?;
                    let mut s = String::new();
                    let mut i = idx.start;
                    while (idx.step > 0 && i < idx.stop) || (idx.step < 0 && i > idx.stop) {
                        s.push(chars[i as usize]);
                        i += idx.step;
                    }
                    return Ok(s.into_pyobject(py)?.into_any().unbind());
                }
                if let Ok(n) = key.extract::<isize>() {
                    let chars: Vec<char> = self.str_value().chars().collect();
                    let len = chars.len() as isize;
                    let actual = if n < 0 { n + len } else { n };
                    if actual < 0 || actual >= len {
                        return Err(PyIndexError::new_err("string index out of range"));
                    }
                    let c = chars[actual as usize].to_string();
                    return Ok(c.into_pyobject(py)?.into_any().unbind());
                }
                Err(PyTypeError::new_err(
                    "string indices must be integers or slices, not str",
                ))
            }
            gjson::Kind::Array => {
                if let Ok(slice) = key.cast::<PySlice>() {
                    let mut children: Vec<JsonResult> = Vec::new();
                    self.parsed().each(|_k, v| {
                        children.push(JsonResult::child(py, &self.raw, v));
                        true
                    });
                    let len = children.len() as isize;
                    let idx = slice.indices(len)?;
                    let mut parts: Vec<String> = Vec::new();
                    let mut i = idx.start;
                    while (idx.step > 0 && i < idx.stop) || (idx.step < 0 && i > idx.stop) {
                        parts.push(children[i as usize].raw_slice().to_string());
                        i += idx.step;
                    }
                    let json_array = format!("[{}]", parts.join(","));
                    let result = JsonResult::from_owned_text(&json_array, gjson::Kind::Array, true);
                    return Ok(Py::new(py, result)?.into_any());
                }
                if let Ok(n) = key.extract::<isize>() {
                    let mut children: Vec<JsonResult> = Vec::new();
                    self.parsed().each(|_k, v| {
                        children.push(JsonResult::child(py, &self.raw, v));
                        true
                    });
                    let len = children.len() as isize;
                    let actual = if n < 0 { n + len } else { n };
                    if actual < 0 || actual >= len {
                        return Err(PyIndexError::new_err("list index out of range"));
                    }
                    let child = children.remove(actual as usize);
                    return Ok(Py::new(py, child)?.into_any());
                }
                Err(PyTypeError::new_err(
                    "list indices must be integers or slices, not str",
                ))
            }
            gjson::Kind::Object => {
                if let Ok(s) = key.extract::<String>() {
                    let result = JsonResult::child(py, &self.raw, self.parsed().get(&s));
                    return Ok(Py::new(py, result)?.into_any());
                }
                Err(PyKeyError::new_err(key.repr()?.to_string()))
            }
            gjson::Kind::Null => {
                if key.cast::<PySlice>().is_ok() {
                    let result = JsonResult::from_owned_text("", gjson::Kind::Null, false);
                    return Ok(Py::new(py, result)?.into_any());
                }
                if key.extract::<isize>().is_ok() {
                    return Err(PyIndexError::new_err("null value has no indices"));
                }
                Err(PyTypeError::new_err("null value is not subscriptable"))
            }
            _ => Err(PyTypeError::new_err("value does not support subscript access")),
        }
    }

    /// Return a lazy view of the object's keys (similar to `dict.keys()`).
    /// Raises `TypeError` for non-Object values.
    fn keys(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<KeysView>> {
        if !matches!(slf.kind, gjson::Kind::Object) {
            return Err(PyTypeError::new_err(
                "keys() is only available for Object values",
            ));
        }
        Py::new(
            py,
            KeysView {
                value: slf.into(),
            },
        )
    }

    /// Return a lazy view of the object's values (similar to `dict.values()`).
    /// Raises `TypeError` for non-Object values.
    fn values(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<ValuesView>> {
        if !matches!(slf.kind, gjson::Kind::Object) {
            return Err(PyTypeError::new_err(
                "values() is only available for Object values",
            ));
        }
        Py::new(
            py,
            ValuesView {
                value: slf.into(),
            },
        )
    }

    /// Return a lazy view of the object's `(key, value)` pairs.
    /// Raises `TypeError` for non-Object values.
    fn items(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<ItemsView>> {
        if !matches!(slf.kind, gjson::Kind::Object) {
            return Err(PyTypeError::new_err(
                "items() is only available for Object values",
            ));
        }
        Py::new(
            py,
            ItemsView {
                value: slf.into(),
            },
        )
    }

    fn __int__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.raw_slice().starts_with('-') {
            Ok(self.parsed().i64().into_pyobject(py)?.into_any().unbind())
        } else {
            Ok(self.parsed().u64().into_pyobject(py)?.into_any().unbind())
        }
    }

    fn __float__(&self) -> f64 {
        self.parsed().f64()
    }

    fn __bool__(&self) -> bool {
        match self.kind {
            gjson::Kind::Null | gjson::Kind::False => false,
            gjson::Kind::True => true,
            gjson::Kind::Number => self.parsed().f64() != 0.0,
            gjson::Kind::String => !self.str_value().is_empty(),
            gjson::Kind::Array | gjson::Kind::Object => {
                let mut has = false;
                self.parsed().each(|_k, _v| {
                    has = true;
                    false
                });
                has
            }
        }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        match self.kind {
            gjson::Kind::Object => {
                let mut keys: Vec<String> = Vec::new();
                self.parsed().each(|k, _v| {
                    keys.push(k.str().to_string());
                    true
                });
                let display = if keys.len() >= 3 {
                    format!(
                        "[{}, {}, ...]",
                        format!("{:?}", keys[0]),
                        format!("{:?}", keys[1])
                    )
                } else {
                    let parts: Vec<String> = keys.iter().map(|k| format!("{:?}", k)).collect();
                    format!("[{}]", parts.join(", "))
                };
                format!("<Result type=dict, keys={}>", display)
            }
            gjson::Kind::Array => {
                let mut reprs: Vec<String> = Vec::new();
                self.parsed().each(|_k, v| {
                    let child = JsonResult::child(py, &self.raw, v);
                    reprs.push(child.__repr__(py));
                    true
                });
                let display = if reprs.len() >= 3 {
                    format!("[{}, {}, ...]", reprs[0], reprs[1])
                } else {
                    format!("[{}]", reprs.join(", "))
                };
                format!("<Result type=list, value={}>", display)
            }
            gjson::Kind::Null => "None".to_string(),
            gjson::Kind::False => "False".to_string(),
            gjson::Kind::True => "True".to_string(),
            gjson::Kind::Number => self.raw_slice().to_string(),
            gjson::Kind::String => self.str_value().to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Lazy iterator and view types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum IterMode {
    Strings,
    Values,
    Items,
}

/// Lazy iterator over an Array, Object or String value.
#[pyclass(module = "pygjson._pygjson")]
pub struct ValueIterator {
    children: Vec<JsonResult>,
    strings: Vec<Box<str>>,
    cursor: usize,
    mode: IterMode,
}

impl ValueIterator {
    fn for_array_values(py: Python<'_>, value: &JsonResult) -> Self {
        let mut children: Vec<JsonResult> = Vec::new();
        let parsed = value.parsed();
        parsed.each(|_k, v| {
            children.push(JsonResult::child(py, &value.raw, v));
            true
        });
        Self {
            children,
            strings: Vec::new(),
            cursor: 0,
            mode: IterMode::Values,
        }
    }

    fn for_object_keys(value: &JsonResult) -> Self {
        let mut strings: Vec<Box<str>> = Vec::new();
        let parsed = value.parsed();
        parsed.each(|k, _v| {
            strings.push(k.str().to_string().into_boxed_str());
            true
        });
        Self {
            children: Vec::new(),
            strings,
            cursor: 0,
            mode: IterMode::Strings,
        }
    }

    fn for_object_values(py: Python<'_>, value: &JsonResult) -> Self {
        Self::for_array_values(py, value)
    }

    fn for_object_items(py: Python<'_>, value: &JsonResult) -> Self {
        let mut children: Vec<JsonResult> = Vec::new();
        let mut strings: Vec<Box<str>> = Vec::new();
        let parsed = value.parsed();
        parsed.each(|k, v| {
            strings.push(k.str().to_string().into_boxed_str());
            children.push(JsonResult::child(py, &value.raw, v));
            true
        });
        Self {
            children,
            strings,
            cursor: 0,
            mode: IterMode::Items,
        }
    }

    fn for_string_chars(value: &JsonResult) -> Self {
        let strings: Vec<Box<str>> = value
            .str_value()
            .chars()
            .map(|c| c.to_string().into_boxed_str())
            .collect();
        Self {
            children: Vec::new(),
            strings,
            cursor: 0,
            mode: IterMode::Strings,
        }
    }
}

#[pymethods]
impl ValueIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let i = self.cursor;
        match self.mode {
            IterMode::Strings => {
                if i >= self.strings.len() {
                    return Ok(None);
                }
                self.cursor += 1;
                Ok(Some(self.strings[i].as_ref().into_pyobject(py)?.into_any().unbind()))
            }
            IterMode::Values => {
                if i >= self.children.len() {
                    return Ok(None);
                }
                self.cursor += 1;
                let v = &self.children[i];
                let cloned = JsonResult {
                    raw: v.raw.clone_ref(py),
                    start: v.start,
                    end: v.end,
                    kind: v.kind,
                    exists: v.exists,
                    info: v.info,
                    str_cache: OnceLock::new(),
                };
                Ok(Some(Py::new(py, cloned)?.into_any()))
            }
            IterMode::Items => {
                if i >= self.children.len() {
                    return Ok(None);
                }
                self.cursor += 1;
                let v = &self.children[i];
                let cloned = JsonResult {
                    raw: v.raw.clone_ref(py),
                    start: v.start,
                    end: v.end,
                    kind: v.kind,
                    exists: v.exists,
                    info: v.info,
                    str_cache: OnceLock::new(),
                };
                let key_obj = self.strings[i].as_ref().into_pyobject(py)?;
                let val_obj = Py::new(py, cloned)?.into_bound(py).into_any();
                let tup = PyTuple::new(py, [key_obj.into_any(), val_obj])?;
                Ok(Some(tup.into_any().unbind()))
            }
        }
    }

    fn __length_hint__(&self) -> usize {
        match self.mode {
            IterMode::Strings => self.strings.len().saturating_sub(self.cursor),
            IterMode::Values | IterMode::Items => {
                self.children.len().saturating_sub(self.cursor)
            }
        }
    }
}

#[pyclass(module = "pygjson._pygjson")]
pub struct KeysView {
    value: Py<JsonResult>,
}

#[pymethods]
impl KeysView {
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<ValueIterator>> {
        let v = self.value.borrow(py);
        Py::new(py, ValueIterator::for_object_keys(&v))
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.value.borrow(py).__len__()
    }

    fn __contains__(&self, py: Python<'_>, item: &str) -> bool {
        let v = self.value.borrow(py);
        let mut found = false;
        v.parsed().each(|k, _vv| {
            if k.str() == item {
                found = true;
                false
            } else {
                true
            }
        });
        found
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let v = self.value.borrow(py);
        let mut parts: Vec<String> = Vec::new();
        v.parsed().each(|k, _vv| {
            parts.push(format!("{:?}", k.str()));
            true
        });
        format!("KeysView([{}])", parts.join(", "))
    }
}

#[pyclass(module = "pygjson._pygjson")]
pub struct ValuesView {
    value: Py<JsonResult>,
}

#[pymethods]
impl ValuesView {
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<ValueIterator>> {
        let v = self.value.borrow(py);
        Py::new(py, ValueIterator::for_object_values(py, &v))
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.value.borrow(py).__len__()
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let v = self.value.borrow(py);
        let mut parts: Vec<String> = Vec::new();
        v.parsed().each(|_k, vv| {
            parts.push(format!("Result({})", vv.json()));
            true
        });
        format!("ValuesView([{}])", parts.join(", "))
    }
}

#[pyclass(module = "pygjson._pygjson")]
pub struct ItemsView {
    value: Py<JsonResult>,
}

#[pymethods]
impl ItemsView {
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<ValueIterator>> {
        let v = self.value.borrow(py);
        Py::new(py, ValueIterator::for_object_items(py, &v))
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        self.value.borrow(py).__len__()
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let v = self.value.borrow(py);
        let mut parts: Vec<String> = Vec::new();
        v.parsed().each(|k, vv| {
            parts.push(format!("({:?}, Result({}))", k.str(), vv.json()));
            true
        });
        format!("ItemsView([{}])", parts.join(", "))
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Get the value at `path` from the given JSON document.
/// `path` accepts either a `str` or a `Path`.
#[pyfunction]
fn get(py: Python<'_>, json: &Bound<'_, PyString>, path: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
    let raw = RawJson::from_pystring(json)?;
    if let Ok(cp) = path.cast::<Path>() {
        let borrow = cp.borrow();
        let v = get_detached(py, raw.as_str(), &borrow.path);
        return Ok(JsonResult::child(py, &raw, v));
    }
    let v = get_detached(py, raw.as_str(), path.extract::<&str>()?);
    Ok(JsonResult::child(py, &raw, v))
}

/// Parse the entire JSON document into a `Result`.
#[pyfunction]
fn parse(py: Python<'_>, json: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
    let raw = if let Ok(s) = json.cast::<PyString>() {
        RawJson::from_pystring(&s)?
    } else if let Ok(b) = json.cast::<PyBytes>() {
        RawJson::from_pybytes(py, &b)?
    } else {
        return Err(PyTypeError::new_err("json must be str or bytes"));
    };
    let v = gjson::parse(raw.as_str());
    Ok(JsonResult::child(py, &raw, v))
}

/// Validate whether `json` is a syntactically valid JSON document.
#[pyfunction]
fn validate(json: &Bound<'_, PyAny>) -> PyResult<bool> {
    if let Ok(s) = json.extract::<&str>() {
        Ok(gjson::valid(s))
    } else if let Ok(b) = json.extract::<&[u8]>() {
        Ok(gjson::valid_bytes(b))
    } else {
        Err(PyTypeError::new_err("json must be str or bytes"))
    }
}

/// Get the values at each path in `paths` from the given JSON document.
/// `paths` accepts either a `list[str]` or a `list[Path]`.
#[pyfunction]
fn get_many(py: Python<'_>, json: &Bound<'_, PyString>, paths: &Bound<'_, PyAny>) -> PyResult<Vec<JsonResult>> {
    let raw = RawJson::from_pystring(json)?;
    let compiled = compiled_from_path_arg(paths)?;
    let text = raw.as_str();
    let vs = if text.len() >= DETACH_THRESHOLD {
        py.detach(|| gjson::get_many_compiled(text, &compiled))
    } else {
        gjson::get_many_compiled(text, &compiled)
    };
    Ok(vs.into_iter().map(|v| JsonResult::child(py, &raw, v)).collect())
}

/// Get the value at `path` from the given JSON bytes.
/// `path` accepts either a `str` or a `Path`.
#[pyfunction]
fn get_bytes(py: Python<'_>, json: &Bound<'_, PyBytes>, path: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
    let raw = RawJson::from_pybytes(py, json)?;
    if let Ok(cp) = path.cast::<Path>() {
        let borrow = cp.borrow();
        let v = get_detached(py, raw.as_str(), &borrow.path);
        return Ok(JsonResult::child(py, &raw, v));
    }
    let v = get_detached(py, raw.as_str(), path.extract::<&str>()?);
    Ok(JsonResult::child(py, &raw, v))
}

/// Get the values at each path in `paths` from the given JSON bytes.
/// `paths` accepts either a `list[str]` or a `list[Path]`.
#[pyfunction]
fn get_many_bytes(py: Python<'_>, json: &Bound<'_, PyBytes>, paths: &Bound<'_, PyAny>) -> PyResult<Vec<JsonResult>> {
    let raw = RawJson::from_pybytes(py, json)?;
    let compiled = compiled_from_path_arg(paths)?;
    let text = raw.as_str();
    let vs = if text.len() >= DETACH_THRESHOLD {
        py.detach(|| gjson::get_many_compiled(text, &compiled))
    } else {
        gjson::get_many_compiled(text, &compiled)
    };
    Ok(vs.into_iter().map(|v| JsonResult::child(py, &raw, v)).collect())
}

/// Pre-compile a gjson path string for repeated use.
/// Pass the returned `Path` to `get`, `get_bytes`, `get_many`, or
/// `get_many_bytes` instead of a plain string to avoid per-call path overhead.
#[pyfunction]
fn compile(path: &str) -> Path {
    Path { path: path.to_owned() }
}

#[pymodule]
fn _pygjson(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<JsonResult>()?;
    m.add_class::<Path>()?;
    m.add_class::<ValueIterator>()?;
    m.add_class::<KeysView>()?;
    m.add_class::<ValuesView>()?;
    m.add_class::<ItemsView>()?;
    m.add_function(wrap_pyfunction!(compile, m)?)?;
    m.add_function(wrap_pyfunction!(get, m)?)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    m.add_function(wrap_pyfunction!(get_many, m)?)?;
    m.add_function(wrap_pyfunction!(get_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(get_many_bytes, m)?)?;
    Ok(())
}
