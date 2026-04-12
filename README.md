# pygjson

![PyPI - Version](https://img.shields.io/pypi/v/pygjson)
![GitHub License](https://img.shields.io/github/license/minefuto/pygjson)

PYGJSON is a Python binding for [tidwall/gjson.rs](https://github.com/tidwall/gjson.rs) - fast JSON path queries.

The original GJSON: [tidwall/gjson](https://github.com/tidwall/gjson)

## Installation

```bash
pip install pygjson
```

## Quick example

```python
import pygjson

JSON = """{
  "name": {"first": "Tom", "last": "Anderson"},
  "age": 37,
  "children": ["Sara", "Alex", "Jack"],
  "friends": [
    {"first": "Dale",  "last": "Murphy", "age": 44},
    {"first": "Roger", "last": "Craig",  "age": 68},
    {"first": "Jane",  "last": "Murphy", "age": 47}
  ]
}"""

str(pygjson.get(JSON, "name.last"))                           # 'Anderson'
int(pygjson.get(JSON, "age"))                                 # 37
int(pygjson.get(JSON, "children.#"))                         # 3
str(pygjson.get(JSON, "children.1"))                          # 'Alex'
str(pygjson.get(JSON, 'friends.#(last="Murphy").first'))      # 'Dale'

[str(v) for v in pygjson.get(JSON, "children|@reverse")]
# ['Jack', 'Alex', 'Sara']

pygjson.valid(JSON)  # True
```

## API

### Module-level functions

| Function                       | Description                                                  |
|-------------------------------|--------------------------------------------------------------|
| `get(json, path)`             | Query `json` at `path`; returns `Value` (gjson-native)       |
| `get(json, path, default)`    | Returns `default` if path is not found (Pythonic)            |
| `parse(json)`                 | Parse the entire JSON document into a `Value`                |
| `valid(json)`                 | `True` if `json` is syntactically valid                      |

### Value

`get` and `parse` return a `Value`. The API is split into two layers:

**gjson-native methods** — mirror the Rust `gjson::Value` API directly:

| Method             | Description                                              |
|--------------------|----------------------------------------------------------|
| `v.kind()`         | Returns a `Kind` enum value                              |
| `v.exists()`       | `True` if the value was actually found in the JSON       |
| `v.to_str()`       | String representation (gjson `str` behaviour)            |
| `v.to_int()`       | Signed 64-bit integer (`i64`)                            |
| `v.to_uint()`      | Unsigned 64-bit integer (`u64`)                          |
| `v.to_float()`     | 64-bit float                                             |
| `v.to_bool()`      | `True` only for the JSON literal `true`                  |
| `v.json()`         | Raw JSON text for this value                             |
| `v.get(path)`      | Sub-query relative to this value (gjson-native)          |
| `v.get(path, default)` | Sub-query; returns `default` if not found (Pythonic) |
| `v.to_list()`      | `list[Value]` for arrays                                 |
| `v.to_dict()`      | `dict[str, Value]` for objects                           |

**Pythonic methods** — follow standard Python protocols:

| Syntax              | Description                                                  |
|---------------------|--------------------------------------------------------------|
| `str(v)`            | String representation                                        |
| `int(v)`            | Integer (negative → `i64`, non-negative → `u64`)             |
| `float(v)`          | 64-bit float                                                 |
| `bool(v)`           | `True` if `v.exists()`                                       |
| `len(v)`            | Chars for String; element count for Array/Object             |
| `v[key]`            | Subscript access for Object values                           |
| `key in v`          | Key membership for Object; str match for Array elements      |
| `iter(v)`           | Lazy iterator: chars for String; `Value`s for Array; keys for Object |
| `v.keys()`          | Lazy `KeysView` of object keys (raises `TypeError` for non-Object)   |
| `v.values()`        | Lazy `ValuesView` of object values (raises `TypeError` for non-Object) |
| `v.items()`         | Lazy `ItemsView` of `(key, Value)` pairs (raises `TypeError` for non-Object) |

#### Lazy iteration

`iter(v)`, `v.keys()`, `v.values()` and `v.items()` all return lightweight
lazy objects rather than fully-materialised lists, mirroring Python's built-in
`dict_keys` / `dict_values` / `dict_items`. Only one child wrapper is alive at
any time, so iterating a large array or object never pays the cost of
allocating one Python object per element up front.

```python
v = parse('{"a": 1, "b": 2, "c": 3}')

ks = v.keys()           # KeysView(["a", "b", "c"]) — no materialisation yet
len(ks)                 # 3
"a" in ks               # True
list(ks)                # ['a', 'b', 'c']

for k, child in v.items():   # ItemsView, lazily yields one pair at a time
    ...
```

If you need a fully materialised collection (for example to keep references
to every child), call `v.to_list()` / `v.to_dict()` or wrap the view with
`list(...)` / `dict(...)` explicitly.

### Kind

```python
from pygjson import Kind
Kind.Null   Kind.False_   Kind.True_   Kind.Number
Kind.String Kind.Array    Kind.Object
```

(`False` and `True` are Python keywords, so the variants are named with a
trailing underscore.)

## Usage examples

```python
from pygjson import get, parse, valid, Kind

# gjson-native: missing value returns Value(exists=False)
v = get(JSON, "no.such.path")
v.exists()   # False
bool(v)      # False

# Pythonic: missing value returns None (or a custom default)
get(JSON, "no.such.path", None)   # None
get(JSON, "no.such.path", 42)     # 42

# Type conversion
age = get(JSON, "age")
age.to_int()    # gjson i64 behaviour
int(age)        # Python int protocol

# Boolean distinction
get('{"flag": true}', "flag").to_bool()    # True  (JSON true literal)
bool(get(JSON, "age"))                     # True  (value exists)

# Array iteration
children = get(JSON, "children")
list(children)                  # [Value("Sara"), Value("Alex"), Value("Jack")]
[str(v) for v in children]      # ['Sara', 'Alex', 'Jack']
"Sara" in children              # True

# Object (dict-like) access
name = get(JSON, "name")
name["first"]                   # Value("Tom")
"first" in name                 # True
list(name)                      # ['first', 'last']  — keys
dict(name)                      # {'first': Value("Tom"), 'last': Value("Anderson")}
for k, v in name.items():
    print(k, str(v))

# Chained queries
parse(JSON).get("name").get("first")   # Value("Tom")
```

## Path syntax

For the full path / query / modifier syntax see the upstream
[gjson.rs](https://github.com/tidwall/gjson.rs) and the original
[GJSON path syntax](https://github.com/tidwall/gjson/blob/master/SYNTAX.md).

## License

MIT
