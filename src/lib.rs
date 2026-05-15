use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError, PyUnicodeDecodeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyNone, PySlice, PyString, PyTuple};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    static TRIE_CACHE: RefCell<HashMap<Vec<String>, Arc<gjson::CompiledPaths>>> =
        RefCell::new(HashMap::new());
}

fn get_or_build_compiled(key: Vec<String>) -> Arc<gjson::CompiledPaths> {
    TRIE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(c) = cache.get(&key) {
            return Arc::clone(c);
        }
        let path_refs: Vec<&str> = key.iter().map(String::as_str).collect();
        let c = Arc::new(gjson::compile_paths(&path_refs));
        cache.insert(key, Arc::clone(&c));
        c
    })
}

fn key_from_path_list(list: &Bound<'_, PyList>) -> Vec<String> {
    list.iter()
        .map(|item| item.cast::<Path>().unwrap().borrow().path.clone())
        .collect()
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

/// A JSON value returned by `get` / `parse`.
///
/// The wrapper holds a reference-counted handle to the raw JSON text together
/// with the byte range that this particular value occupies inside it. Child
/// values produced by `get`, iteration, etc. share the same `Arc`
/// instead of cloning the underlying text, which avoids a fresh heap
/// allocation per child element.
#[pyclass(module = "pygjson._pygjson", name = "Result")]
pub struct JsonResult {
    raw: Arc<str>,
    start: usize,
    end: usize,
    kind: gjson::Kind,
    exists: bool,
}

impl JsonResult {
    fn raw_slice(&self) -> &str {
        &self.raw[self.start..self.end]
    }

    fn parsed(&self) -> gjson::Value<'_> {
        gjson::parse(self.raw_slice())
    }

    fn from_owned_text(text: &str, kind: gjson::Kind, exists: bool) -> Self {
        let raw: Arc<str> = Arc::from(text);
        let end = raw.len();
        Self {
            raw,
            start: 0,
            end,
            kind,
            exists,
        }
    }

    fn child(parent: &Arc<str>, child: gjson::Value<'_>) -> Self {
        let kind = child.kind();
        let exists = child.exists();
        let child_text = child.json();
        if !child_text.is_empty() {
            let parent_bytes = parent.as_bytes();
            let parent_start_addr = parent_bytes.as_ptr() as usize;
            let parent_end_addr = parent_start_addr + parent_bytes.len();
            let child_start_addr = child_text.as_ptr() as usize;
            if child_start_addr >= parent_start_addr
                && child_start_addr + child_text.len() <= parent_end_addr
            {
                let start = child_start_addr - parent_start_addr;
                return Self {
                    raw: Arc::clone(parent),
                    start,
                    end: start + child_text.len(),
                    kind,
                    exists,
                };
            }
        }
        Self::from_owned_text(child_text, kind, exists)
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
            gjson::Kind::String => Ok(self.parsed().str().into_pyobject(py)?.into_any().unbind()),
            gjson::Kind::Array => {
                let list = PyList::empty(py);
                let parsed = self.parsed();
                let mut err: Option<PyErr> = None;
                parsed.each(|_k, v| {
                    let child = JsonResult::child(&self.raw, v);
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
                    let child = JsonResult::child(&self.raw, v);
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
        self.parsed().str().to_string()
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
    fn get(&self, path: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
        if let Ok(cp) = path.cast::<Path>() {
            let borrow = cp.borrow();
            // SAFETY: raw_slice() is always valid UTF-8 (stored as Arc<str>)
            let v = unsafe { gjson::get_bytes(self.raw_slice().as_bytes(), &borrow.path) };
            return Ok(JsonResult::child(&self.raw, v));
        }
        let s = path.extract::<&str>()?;
        // SAFETY: raw_slice() is always valid UTF-8 (stored as Arc<str>)
        let v = unsafe { gjson::get_bytes(self.raw_slice().as_bytes(), s) };
        Ok(JsonResult::child(&self.raw, v))
    }

    /// Get child values at each of the given gjson paths.
    /// Accepts either a `list[str]` or a `list[Path]`.
    /// When `list[Path]` is passed, the internal trie is cached and
    /// reused across calls with the same compiled path objects.
    fn get_many(&self, paths: &Bound<'_, PyAny>) -> PyResult<Vec<JsonResult>> {
        let list = paths.cast::<PyList>()?;
        let key = if !list.is_empty() && list.get_item(0)?.cast::<Path>().is_ok() {
            key_from_path_list(&list)
        } else {
            list.extract::<Vec<String>>()?
        };
        let compiled = get_or_build_compiled(key);
        // SAFETY: raw_slice() is always valid UTF-8 (stored as Arc<str>)
        let vs = unsafe {
            gjson::get_many_compiled_bytes(self.raw_slice().as_bytes(), &compiled)
        };
        Ok(vs.into_iter().map(|v| JsonResult::child(&self.raw, v)).collect())
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
            gjson::Kind::String => Ok(self.parsed().str().chars().count()),
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
            gjson::Kind::Array => ValueIterator::for_array_values(self),
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
                    let chars: Vec<char> = self.parsed().str().chars().collect();
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
                    let chars: Vec<char> = self.parsed().str().chars().collect();
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
                        children.push(JsonResult::child(&self.raw, v));
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
                        children.push(JsonResult::child(&self.raw, v));
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
                    let result = JsonResult::child(&self.raw, self.parsed().get(&s));
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
            gjson::Kind::String => !self.parsed().str().is_empty(),
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

    fn __repr__(&self) -> String {
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
                    let child = JsonResult::child(&self.raw, v);
                    reprs.push(child.__repr__());
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
            gjson::Kind::String => self.parsed().str().to_string(),
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
    fn for_array_values(value: &JsonResult) -> Self {
        let mut children: Vec<JsonResult> = Vec::new();
        let parsed = gjson::parse(value.raw_slice());
        parsed.each(|_k, v| {
            children.push(JsonResult::child(&value.raw, v));
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
        let parsed = gjson::parse(value.raw_slice());
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

    fn for_object_values(value: &JsonResult) -> Self {
        Self::for_array_values(value)
    }

    fn for_object_items(value: &JsonResult) -> Self {
        let mut children: Vec<JsonResult> = Vec::new();
        let mut strings: Vec<Box<str>> = Vec::new();
        let parsed = gjson::parse(value.raw_slice());
        parsed.each(|k, v| {
            strings.push(k.str().to_string().into_boxed_str());
            children.push(JsonResult::child(&value.raw, v));
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
        let s = value.parsed().str().to_string();
        let strings: Vec<Box<str>> = s
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
                    raw: Arc::clone(&v.raw),
                    start: v.start,
                    end: v.end,
                    kind: v.kind,
                    exists: v.exists,
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
                    raw: Arc::clone(&v.raw),
                    start: v.start,
                    end: v.end,
                    kind: v.kind,
                    exists: v.exists,
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
        Py::new(py, ValueIterator::for_object_values(&v))
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
        Py::new(py, ValueIterator::for_object_items(&v))
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
fn get(json: &str, path: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
    let raw: Arc<str> = Arc::from(json);
    if let Ok(cp) = path.cast::<Path>() {
        let borrow = cp.borrow();
        let parsed = gjson::get(&raw, &borrow.path);
        return Ok(JsonResult::child(&raw, parsed));
    }
    let s = path.extract::<&str>()?;
    let parsed = gjson::get(&raw, s);
    Ok(JsonResult::child(&raw, parsed))
}

/// Parse the entire JSON document into a `Result`.
#[pyfunction]
fn parse(py: Python<'_>, json: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
    if let Ok(s) = json.extract::<&str>() {
        let raw: Arc<str> = Arc::from(s);
        let parsed = gjson::parse(&raw);
        Ok(JsonResult::child(&raw, parsed))
    } else if let Ok(b) = json.extract::<&[u8]>() {
        let s = std::str::from_utf8(b).map_err(|e| -> PyErr {
            match PyUnicodeDecodeError::new_utf8(py, b, e) {
                Ok(bound) => bound.into(),
                Err(e) => e,
            }
        })?;
        let raw: Arc<str> = Arc::from(s);
        // SAFETY: raw was just validated as valid UTF-8
        let parsed = unsafe { gjson::parse_bytes(raw.as_bytes()) };
        Ok(JsonResult::child(&raw, parsed))
    } else {
        Err(PyTypeError::new_err("json must be str or bytes"))
    }
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
fn get_many(json: &str, paths: &Bound<'_, PyAny>) -> PyResult<Vec<JsonResult>> {
    let raw: Arc<str> = Arc::from(json);
    let list = paths.cast::<PyList>()?;
    let key = if !list.is_empty() && list.get_item(0)?.cast::<Path>().is_ok() {
        key_from_path_list(&list)
    } else {
        list.extract::<Vec<String>>()?
    };
    let compiled = get_or_build_compiled(key);
    let vs = gjson::get_many_compiled(&raw, &compiled);
    Ok(vs.into_iter().map(|v| JsonResult::child(&raw, v)).collect())
}

/// Get the value at `path` from the given JSON bytes.
/// `path` accepts either a `str` or a `Path`.
#[pyfunction]
fn get_bytes(py: Python<'_>, json: &[u8], path: &Bound<'_, PyAny>) -> PyResult<JsonResult> {
    let s = std::str::from_utf8(json).map_err(|e| -> PyErr {
        match PyUnicodeDecodeError::new_utf8(py, json, e) {
            Ok(bound) => bound.into(),
            Err(e) => e,
        }
    })?;
    let raw: Arc<str> = Arc::from(s);
    if let Ok(cp) = path.cast::<Path>() {
        let borrow = cp.borrow();
        // SAFETY: raw was just validated as valid UTF-8
        let v = unsafe { gjson::get_bytes(raw.as_bytes(), &borrow.path) };
        return Ok(JsonResult::child(&raw, v));
    }
    let p = path.extract::<&str>()?;
    // SAFETY: raw was just validated as valid UTF-8
    let v = unsafe { gjson::get_bytes(raw.as_bytes(), p) };
    Ok(JsonResult::child(&raw, v))
}

/// Get the values at each path in `paths` from the given JSON bytes.
/// `paths` accepts either a `list[str]` or a `list[Path]`.
#[pyfunction]
fn get_many_bytes(py: Python<'_>, json: &[u8], paths: &Bound<'_, PyAny>) -> PyResult<Vec<JsonResult>> {
    let s = std::str::from_utf8(json).map_err(|e| -> PyErr {
        match PyUnicodeDecodeError::new_utf8(py, json, e) {
            Ok(bound) => bound.into(),
            Err(e) => e,
        }
    })?;
    let raw: Arc<str> = Arc::from(s);
    let list = paths.cast::<PyList>()?;
    let key = if !list.is_empty() && list.get_item(0)?.cast::<Path>().is_ok() {
        key_from_path_list(&list)
    } else {
        list.extract::<Vec<String>>()?
    };
    let compiled = get_or_build_compiled(key);
    // SAFETY: raw was just validated as valid UTF-8
    let vs = unsafe { gjson::get_many_compiled_bytes(raw.as_bytes(), &compiled) };
    Ok(vs.into_iter().map(|v| JsonResult::child(&raw, v)).collect())
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
