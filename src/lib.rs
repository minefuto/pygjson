use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use std::sync::Arc;

/// Mirror of `gjson::Kind`, exposed to Python as a class with constant attributes.
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
#[pyclass(module = "pygjson._pygjson", name = "Value")]
pub struct Value {
    raw: Arc<str>,
    start: usize,
    end: usize,
    kind: Kind,
    exists: bool,
}

impl Value {
    fn raw_slice(&self) -> &str {
        &self.raw[self.start..self.end]
    }

    fn parsed(&self) -> gjson::Value<'_> {
        gjson::parse(self.raw_slice())
    }

    /// Build a `Value` that owns a fresh `Arc<str>` containing `text`.
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

    /// Build a child `Value` that shares the parent's `Arc<str>` whenever the
    /// child's text is a borrowed slice of it. Falls back to a fresh
    /// allocation for owned children (e.g. modifier output).
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
impl Value {
    /// Return the gjson `Kind` of this value.
    fn kind(&self) -> Kind {
        self.kind
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

    /// Signed integer value (`i64`).
    fn to_int(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.parsed().i64().into_pyobject(py)?.into_any().unbind())
    }

    /// Unsigned integer value (`u64`).
    fn to_uint(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(self.parsed().u64().into_pyobject(py)?.into_any().unbind())
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
    ///
    /// If `default` is given and the path is not found, returns `default` instead.
    #[pyo3(signature = (path, *args))]
    fn get(
        &self,
        py: Python<'_>,
        path: &str,
        args: &Bound<'_, pyo3::types::PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() > 1 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "get() takes at most 2 positional arguments",
            ));
        }
        let parsed = self.parsed();
        let val = Value::child(&self.raw, parsed.get(path));
        if !val.exists && !args.is_empty() {
            return Ok(args.get_item(0)?.unbind());
        }
        Ok(Py::new(py, val)?.into_any())
    }

    /// Get child values at each of the given gjson paths.
    ///
    /// If `default` is given and a path is not found, returns `default` in
    /// that position instead of a `Value` with `exists=False`.
    #[pyo3(signature = (paths, *args))]
    fn get_many(
        &self,
        py: Python<'_>,
        paths: Vec<String>,
        args: &Bound<'_, pyo3::types::PyTuple>,
    ) -> PyResult<Py<PyAny>> {
        if args.len() > 1 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "get_many() takes at most 2 positional arguments",
            ));
        }
        let path_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let values: Vec<Value> = gjson::get_many(self.raw_slice(), &path_refs)
            .into_iter()
            .map(|v| Value::child(&self.raw, v))
            .collect();
        let has_default = !args.is_empty();
        let list = pyo3::types::PyList::empty(py);
        for v in values {
            if has_default && !v.exists {
                list.append(args.get_item(0)?)?;
            } else {
                list.append(Py::new(py, v)?)?;
            }
        }
        Ok(list.into_any().unbind())
    }

    /// Return the value as a list of `Value` objects (empty for non-arrays).
    fn to_list(&self) -> Vec<Value> {
        let mut out = Vec::new();
        let parsed = self.parsed();
        parsed.each(|_k, v| {
            out.push(Value::child(&self.raw, v));
            true
        });
        out
    }

    /// Return the value as a `dict[str, Value]` (empty for non-objects).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        // Only iterate as a map for objects; arrays would yield empty keys.
        if matches!(self.kind, Kind::Object) {
            let parsed = self.parsed();
            let mut err: Option<PyErr> = None;
            parsed.each(|k, v| {
                let key = k.str().to_string();
                let child = Value::child(&self.raw, v);
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
            _ => Err(PyTypeError::new_err("Value has no len()")),
        }
    }

    /// Iterate: String → chars, Array → Values, Object → keys.
    ///
    /// Returns a lazy `ValueIterator` so the elements are produced one at a
    /// time and only one Python wrapper is alive at any moment.
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<ValueIterator>> {
        let it = match self.kind {
            Kind::String => ValueIterator::for_string_chars(self),
            Kind::Array => ValueIterator::for_array_values(self),
            Kind::Object => ValueIterator::for_object_keys(self),
            _ => {
                return Err(PyTypeError::new_err(
                    "Value is not iterable (only String, Array, and Object are iterable)",
                ));
            }
        };
        Py::new(py, it)
    }

    /// Subscript access for Object values (enables the `dict()` mapping protocol).
    fn __getitem__(&self, key: &str) -> PyResult<Value> {
        if !matches!(self.kind, Kind::Object) {
            return Err(PyTypeError::new_err(
                "subscript access is only supported for Object values",
            ));
        }
        Ok(Value::child(&self.raw, self.parsed().get(key)))
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
        format!("Value({})", self.raw_slice())
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
    /// Yield successive `str` items from `strings`.
    Strings,
    /// Yield successive `Value` items from `children`.
    Values,
    /// Yield `(key, value)` tuples from `strings` and `children` in lockstep.
    Items,
}

/// Lazy iterator over an Array, Object or String value.
///
/// The constructor walks the underlying gjson value once and records either
/// child `Value` handles (which share the parent's `Arc<str>`) or pre-computed
/// strings (for keys / chars). `__next__` then yields one Python object at a
/// time, so the peak number of simultaneously-live Python wrappers is one
/// regardless of the collection size.
#[pyclass(module = "pygjson._pygjson")]
pub struct ValueIterator {
    children: Vec<Value>,
    strings: Vec<Box<str>>,
    cursor: usize,
    mode: IterMode,
}

impl ValueIterator {
    fn for_array_values(value: &Value) -> Self {
        let mut children: Vec<Value> = Vec::new();
        let parsed = gjson::parse(value.raw_slice());
        parsed.each(|_k, v| {
            children.push(Value::child(&value.raw, v));
            true
        });
        Self {
            children,
            strings: Vec::new(),
            cursor: 0,
            mode: IterMode::Values,
        }
    }

    fn for_object_keys(value: &Value) -> Self {
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

    fn for_object_values(value: &Value) -> Self {
        // Same shape as `for_array_values`, kept as a separate constructor so
        // that the call sites read clearly.
        Self::for_array_values(value)
    }

    fn for_object_items(value: &Value) -> Self {
        let mut children: Vec<Value> = Vec::new();
        let mut strings: Vec<Box<str>> = Vec::new();
        let parsed = gjson::parse(value.raw_slice());
        parsed.each(|k, v| {
            strings.push(k.str().to_string().into_boxed_str());
            children.push(Value::child(&value.raw, v));
            true
        });
        Self {
            children,
            strings,
            cursor: 0,
            mode: IterMode::Items,
        }
    }

    fn for_string_chars(value: &Value) -> Self {
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
                let cloned = Value {
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
                let cloned = Value {
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

/// Lightweight view returned by `Value.keys()`. Iteration constructs a
/// `ValueIterator` lazily so the keys are only collected when actually used.
#[pyclass(module = "pygjson._pygjson")]
pub struct KeysView {
    value: Py<Value>,
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

/// Lightweight view returned by `Value.values()`.
#[pyclass(module = "pygjson._pygjson")]
pub struct ValuesView {
    value: Py<Value>,
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
            parts.push(format!("Value({})", vv.json()));
            true
        });
        format!("ValuesView([{}])", parts.join(", "))
    }
}

/// Lightweight view returned by `Value.items()`.
#[pyclass(module = "pygjson._pygjson")]
pub struct ItemsView {
    value: Py<Value>,
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
            parts.push(format!("({:?}, Value({}))", k.str(), vv.json()));
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
fn get(json: &str, path: &str) -> Value {
    let raw: Arc<str> = Arc::from(json);
    let parsed = gjson::get(&raw, path);
    Value::child(&raw, parsed)
}

/// Parse the entire JSON document into a `Value`.
#[pyfunction]
fn parse(json: &str) -> Value {
    let raw: Arc<str> = Arc::from(json);
    let parsed = gjson::parse(&raw);
    Value::child(&raw, parsed)
}

/// Validate whether `json` is a syntactically valid JSON document.
#[pyfunction]
fn valid(json: &str) -> bool {
    gjson::valid(json)
}

/// Get the values at each path in `paths` from the given JSON document.
#[pyfunction]
fn get_many(json: &str, paths: Vec<String>) -> Vec<Value> {
    let raw: Arc<str> = Arc::from(json);
    let path_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    gjson::get_many(&raw, &path_refs)
        .into_iter()
        .map(|v| Value::child(&raw, v))
        .collect()
}

#[pymodule]
fn _pygjson(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Kind>()?;
    m.add_class::<Value>()?;
    m.add_class::<ValueIterator>()?;
    m.add_class::<KeysView>()?;
    m.add_class::<ValuesView>()?;
    m.add_class::<ItemsView>()?;
    m.add_function(wrap_pyfunction!(get, m)?)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(valid, m)?)?;
    m.add_function(wrap_pyfunction!(get_many, m)?)?;
    Ok(())
}
