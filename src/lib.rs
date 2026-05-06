use pyo3::exceptions::{PyTypeError, PyUnicodeDecodeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyNone, PyString, PyTuple};
use std::sync::Arc;

#[pyclass(module = "pygjson._pygjson", eq, eq_int, skip_from_py_object)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Null,
    #[pyo3(name = "False_")]
    False,
    Number,
    String,
    #[pyo3(name = "True_")]
    True,
    Array,
    Object,
}

#[pymethods]
impl Kind {
    fn __repr__(&self) -> String {
        format!("Kind.{:?}", self)
    }
}

fn map_kind(k: gjson::Kind) -> Kind {
    match k {
        gjson::Kind::Null => Kind::Null,
        gjson::Kind::False => Kind::False,
        gjson::Kind::Number => Kind::Number,
        gjson::Kind::String => Kind::String,
        gjson::Kind::True => Kind::True,
        gjson::Kind::Array => Kind::Array,
        gjson::Kind::Object => Kind::Object,
    }
}

/// A JSON value returned by `get` / `parse`.
///
/// The wrapper holds a reference-counted handle to the raw JSON text together
/// with the byte range that this particular value occupies inside it. Child
/// values produced by `get`, iteration, `to_list`, etc. share the same `Arc`
/// instead of cloning the underlying text, which avoids a fresh heap
/// allocation per child element.
#[pyclass(module = "pygjson._pygjson", name = "Result")]
pub struct JsonResult {
    raw: Arc<str>,
    start: usize,
    end: usize,
    kind: Kind,
    exists: bool,
}

impl JsonResult {
    fn raw_slice(&self) -> &str {
        &self.raw[self.start..self.end]
    }

    fn parsed(&self) -> gjson::Value<'_> {
        gjson::parse(self.raw_slice())
    }

    fn from_owned_text(text: &str, kind: Kind, exists: bool) -> Self {
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
        let kind = map_kind(child.kind());
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
            Kind::Null => PyNone::get(py).as_any().clone().unbind(),
            Kind::False | Kind::True => py.get_type::<PyBool>().into_any().unbind(),
            Kind::Number => {
                let s = self.raw_slice();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    py.get_type::<PyFloat>().into_any().unbind()
                } else {
                    py.get_type::<PyInt>().into_any().unbind()
                }
            }
            Kind::String => py.get_type::<PyString>().into_any().unbind(),
            Kind::Array => py.get_type::<PyList>().into_any().unbind(),
            Kind::Object => py.get_type::<PyDict>().into_any().unbind(),
        }
    }

    /// Return the inner value as the Python type indicated by `type_`.
    ///
    /// Null → None; bool kinds → bool; Number → int or float;
    /// String → str; Array → list[Result]; Object → dict[str, Result].
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match self.kind {
            Kind::Null => Ok(PyNone::get(py).as_any().clone().unbind()),
            Kind::False | Kind::True => {
                Ok(self.parsed().bool().into_pyobject(py)?.as_any().clone().unbind())
            }
            Kind::Number => {
                let s = self.raw_slice();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    Ok(self.parsed().f64().into_pyobject(py)?.into_any().unbind())
                } else if s.starts_with('-') {
                    Ok(self.parsed().i64().into_pyobject(py)?.into_any().unbind())
                } else {
                    Ok(self.parsed().u64().into_pyobject(py)?.into_any().unbind())
                }
            }
            Kind::String => Ok(self.parsed().str().into_pyobject(py)?.into_any().unbind()),
            Kind::Array => {
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
            Kind::Object => {
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

    /// Boolean value. Returns `true` only for the JSON literal `true`.
    fn to_bool(&self) -> bool {
        self.parsed().bool()
    }

    /// Raw JSON text for this value.
    fn json(&self) -> String {
        self.raw_slice().to_string()
    }

    /// Get a child value at the given gjson path.
    fn get(&self, path: &str) -> JsonResult {
        JsonResult::child(&self.raw, self.parsed().get(path))
    }

    /// Get child values at each of the given gjson paths.
    fn get_many(&self, paths: Vec<String>) -> Vec<JsonResult> {
        let path_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        gjson::get_many(self.raw_slice(), &path_refs)
            .into_iter()
            .map(|v| JsonResult::child(&self.raw, v))
            .collect()
    }

    /// Get a child value at the given gjson path from the byte-slice representation.
    fn get_bytes(&self, path: &str) -> JsonResult {
        // SAFETY: raw_slice() is always valid UTF-8 (stored as Arc<str>)
        let v = unsafe { gjson::get_bytes(self.raw_slice().as_bytes(), path) };
        JsonResult::child(&self.raw, v)
    }

    /// Get child values at each of the given gjson paths from the byte-slice representation.
    fn get_many_bytes(&self, paths: Vec<String>) -> Vec<JsonResult> {
        let path_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        // SAFETY: raw_slice() is always valid UTF-8 (stored as Arc<str>)
        let vs = unsafe { gjson::get_many_bytes(self.raw_slice().as_bytes(), &path_refs) };
        vs.into_iter().map(|v| JsonResult::child(&self.raw, v)).collect()
    }

    /// Return the value as a list of `Result` objects (empty for non-arrays).
    fn to_list(&self) -> Vec<JsonResult> {
        let mut out = Vec::new();
        let parsed = self.parsed();
        parsed.each(|_k, v| {
            out.push(JsonResult::child(&self.raw, v));
            true
        });
        out
    }

    /// Return the value as a `dict[str, Result]` (empty for non-objects).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        if matches!(self.kind, Kind::Object) {
            let parsed = self.parsed();
            let mut err: Option<PyErr> = None;
            parsed.each(|k, v| {
                let key = k.str().to_string();
                let child = JsonResult::child(&self.raw, v);
                match dict.set_item(key, child) {
                    Ok(()) => true,
                    Err(e) => {
                        err = Some(e);
                        false
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
        }
        Ok(dict)
    }

    /// Membership test: `item in value`.
    ///
    /// For Object: returns `True` if `item` is a key in the object.
    /// For Array: returns `True` if any element's string representation equals `item`.
    /// Other kinds raise `TypeError`.
    fn __contains__(&self, item: &str) -> PyResult<bool> {
        match self.kind {
            Kind::Object => {
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
            Kind::Array => {
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
            Kind::String => Ok(self.parsed().str().chars().count()),
            Kind::Array | Kind::Object => {
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
            Kind::String => ValueIterator::for_string_chars(self),
            Kind::Array => ValueIterator::for_array_values(self),
            Kind::Object => ValueIterator::for_object_keys(self),
            _ => {
                return Err(PyTypeError::new_err(
                    "Result is not iterable (only String, Array, and Object are iterable)",
                ));
            }
        };
        Py::new(py, it)
    }

    /// Subscript access for Object values (enables the `dict()` mapping protocol).
    fn __getitem__(&self, key: &str) -> PyResult<JsonResult> {
        if !matches!(self.kind, Kind::Object) {
            return Err(PyTypeError::new_err(
                "subscript access is only supported for Object values",
            ));
        }
        Ok(JsonResult::child(&self.raw, self.parsed().get(key)))
    }

    /// Return a lazy view of the object's keys (similar to `dict.keys()`).
    /// Raises `TypeError` for non-Object values.
    fn keys(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<KeysView>> {
        if !matches!(slf.kind, Kind::Object) {
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
        if !matches!(slf.kind, Kind::Object) {
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
        if !matches!(slf.kind, Kind::Object) {
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
        self.exists
    }

    fn __repr__(&self) -> String {
        format!("Result({})", self.raw_slice())
    }

    fn __str__(&self) -> String {
        self.parsed().str().to_string()
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
#[pyfunction]
fn get(json: &str, path: &str) -> JsonResult {
    let raw: Arc<str> = Arc::from(json);
    let parsed = gjson::get(&raw, path);
    JsonResult::child(&raw, parsed)
}

/// Parse the entire JSON document into a `Result`.
#[pyfunction]
fn parse(json: &str) -> JsonResult {
    let raw: Arc<str> = Arc::from(json);
    let parsed = gjson::parse(&raw);
    JsonResult::child(&raw, parsed)
}

/// Validate whether `json` is a syntactically valid JSON document.
#[pyfunction]
fn validate(json: &str) -> bool {
    gjson::valid(json)
}

/// Get the values at each path in `paths` from the given JSON document.
#[pyfunction]
fn get_many(json: &str, paths: Vec<String>) -> Vec<JsonResult> {
    let raw: Arc<str> = Arc::from(json);
    let path_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    gjson::get_many(&raw, &path_refs)
        .into_iter()
        .map(|v| JsonResult::child(&raw, v))
        .collect()
}

/// Get the value at `path` from the given JSON bytes.
#[pyfunction]
fn get_bytes(py: Python<'_>, json: &[u8], path: &str) -> PyResult<JsonResult> {
    let s = std::str::from_utf8(json).map_err(|e| -> PyErr {
        match PyUnicodeDecodeError::new_utf8(py, json, e) {
            Ok(bound) => bound.into(),
            Err(e) => e,
        }
    })?;
    let raw: Arc<str> = Arc::from(s);
    // SAFETY: raw was just validated as valid UTF-8
    let v = unsafe { gjson::get_bytes(raw.as_bytes(), path) };
    Ok(JsonResult::child(&raw, v))
}

/// Get the values at each path in `paths` from the given JSON bytes.
#[pyfunction]
fn get_many_bytes(py: Python<'_>, json: &[u8], paths: Vec<String>) -> PyResult<Vec<JsonResult>> {
    let s = std::str::from_utf8(json).map_err(|e| -> PyErr {
        match PyUnicodeDecodeError::new_utf8(py, json, e) {
            Ok(bound) => bound.into(),
            Err(e) => e,
        }
    })?;
    let raw: Arc<str> = Arc::from(s);
    let path_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    // SAFETY: raw was just validated as valid UTF-8
    let vs = unsafe { gjson::get_many_bytes(raw.as_bytes(), &path_refs) };
    Ok(vs.into_iter().map(|v| JsonResult::child(&raw, v)).collect())
}

#[pymodule]
fn _pygjson(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Kind>()?;
    m.add_class::<JsonResult>()?;
    m.add_class::<ValueIterator>()?;
    m.add_class::<KeysView>()?;
    m.add_class::<ValuesView>()?;
    m.add_class::<ItemsView>()?;
    m.add_function(wrap_pyfunction!(get, m)?)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    m.add_function(wrap_pyfunction!(get_many, m)?)?;
    m.add_function(wrap_pyfunction!(get_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(get_many_bytes, m)?)?;
    Ok(())
}
