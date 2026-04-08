use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

/// Mirror of `gjson::Kind`, exposed to Python as a class with constant attributes.
#[pyclass(module = "pygjson._pygjson", eq, eq_int)]
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
/// The wrapper owns the raw JSON text representing this value, so it can be
/// passed around freely from Python without lifetime concerns. Subsequent
/// operations re-parse the (typically tiny) raw text on demand.
#[pyclass(module = "pygjson._pygjson", name = "Value")]
pub struct Value {
    raw: String,
    kind: Kind,
    exists: bool,
}

impl Value {
    fn from_gjson(v: gjson::Value) -> Self {
        Value {
            kind: map_kind(v.kind()),
            exists: v.exists(),
            raw: v.json().to_string(),
        }
    }

    fn parsed(&self) -> gjson::Value<'_> {
        gjson::parse(&self.raw)
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
    fn to_int(&self, py: Python<'_>) -> PyObject {
        self.parsed().i64().into_py(py)
    }

    /// Unsigned integer value (`u64`).
    fn to_uint(&self, py: Python<'_>) -> PyObject {
        self.parsed().u64().into_py(py)
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
        self.raw.clone()
    }

    /// Get a child value at the given gjson path.
    ///
    /// If `default` is given and the path is not found, returns `default` instead.
    #[pyo3(signature = (path, *args))]
    fn get(&self, py: Python<'_>, path: &str, args: &Bound<'_, pyo3::types::PyTuple>) -> PyResult<PyObject> {
        if args.len() > 1 {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "get() takes at most 2 positional arguments",
            ));
        }
        let val = Value::from_gjson(self.parsed().get(path));
        if !val.exists && !args.is_empty() {
            return Ok(args.get_item(0)?.into_py(py));
        }
        Ok(val.into_py(py))
    }

    /// Return the value as a list of `Value` objects (empty for non-arrays).
    fn to_list(&self) -> Vec<Value> {
        let mut out = Vec::new();
        self.parsed().each(|_k, v| {
            out.push(Value::from_gjson(v));
            true
        });
        out
    }

    /// Return the value as a `dict[str, Value]` (empty for non-objects).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new_bound(py);
        let parsed = self.parsed();
        // Only iterate as a map for objects; arrays would yield empty keys.
        if matches!(self.kind, Kind::Object) {
            let mut err: Option<PyErr> = None;
            parsed.each(|k, v| {
                let key = k.str().to_string();
                match dict.set_item(key, Value::from_gjson(v).into_py(py)) {
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
    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let list = PyList::empty_bound(py);
        match self.kind {
            Kind::String => {
                for ch in self.parsed().str().chars() {
                    let _ = list.append(ch.to_string().into_py(py));
                }
            }
            Kind::Array => {
                self.parsed().each(|_k, v| {
                    let _ = list.append(Value::from_gjson(v).into_py(py));
                    true
                });
            }
            Kind::Object => {
                self.parsed().each(|k, _v| {
                    let _ = list.append(k.str().to_string().into_py(py));
                    true
                });
            }
            _ => {
                return Err(PyTypeError::new_err(
                    "Value is not iterable (only String, Array, and Object are iterable)",
                ));
            }
        }
        list.call_method0("__iter__")
    }

    /// Subscript access for Object values (enables the `dict()` mapping protocol).
    fn __getitem__(&self, key: &str) -> PyResult<Value> {
        if !matches!(self.kind, Kind::Object) {
            return Err(PyTypeError::new_err(
                "subscript access is only supported for Object values",
            ));
        }
        Ok(Value::from_gjson(self.parsed().get(key)))
    }

    /// Return the object's keys as a list (enables `dict()` mapping protocol).
    /// Raises `TypeError` for non-Object values.
    fn keys(&self) -> PyResult<Vec<String>> {
        if !matches!(self.kind, Kind::Object) {
            return Err(PyTypeError::new_err(
                "keys() is only available for Object values",
            ));
        }
        let mut out = Vec::new();
        self.parsed().each(|k, _v| {
            out.push(k.str().to_string());
            true
        });
        Ok(out)
    }

    /// Return the object's values as a list.
    /// Raises `TypeError` for non-Object values.
    fn values(&self) -> PyResult<Vec<Value>> {
        if !matches!(self.kind, Kind::Object) {
            return Err(PyTypeError::new_err(
                "values() is only available for Object values",
            ));
        }
        let mut out = Vec::new();
        self.parsed().each(|_k, v| {
            out.push(Value::from_gjson(v));
            true
        });
        Ok(out)
    }

    /// Return the object's (key, value) pairs as a list of tuples.
    /// Raises `TypeError` for non-Object values.
    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        if !matches!(self.kind, Kind::Object) {
            return Err(PyTypeError::new_err(
                "items() is only available for Object values",
            ));
        }
        let list = PyList::empty_bound(py);
        self.parsed().each(|k, v| {
            let tup = PyTuple::new_bound(
                py,
                &[k.str().to_string().into_py(py), Value::from_gjson(v).into_py(py)],
            );
            let _ = list.append(tup);
            true
        });
        Ok(list)
    }

    fn __int__(&self, py: Python<'_>) -> PyObject {
        if self.raw.starts_with('-') {
            self.parsed().i64().into_py(py)
        } else {
            self.parsed().u64().into_py(py)
        }
    }

    fn __float__(&self) -> f64 {
        self.parsed().f64()
    }

    fn __bool__(&self) -> bool {
        self.exists
    }

    fn __repr__(&self) -> String {
        format!("Value({})", self.raw)
    }

    fn __str__(&self) -> String {
        self.parsed().str().to_string()
    }
}

/// Get the value at `path` from the given JSON document.
#[pyfunction]
fn get(json: &str, path: &str) -> Value {
    Value::from_gjson(gjson::get(json, path))
}

/// Parse the entire JSON document into a `Value`.
#[pyfunction]
fn parse(json: &str) -> Value {
    Value::from_gjson(gjson::parse(json))
}

/// Validate whether `json` is a syntactically valid JSON document.
#[pyfunction]
fn valid(json: &str) -> bool {
    gjson::valid(json)
}

#[pymodule]
fn _pygjson(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Kind>()?;
    m.add_class::<Value>()?;
    m.add_function(wrap_pyfunction!(get, m)?)?;
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_function(wrap_pyfunction!(valid, m)?)?;
    Ok(())
}
