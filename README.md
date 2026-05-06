# pygjson

![PyPI - Python Version](https://img.shields.io/pypi/pyversions/pygjson)
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

pygjson.validate(JSON)  # True
```

## API

### Module-level functions

| Function                        | Description                                              |
|---------------------------------|----------------------------------------------------------|
| `get(json, path)`               | Query `json` (str) at `path`; returns `Result`           |
| `get_bytes(json, path)`         | Query `json` (bytes) at `path`; returns `Result`         |
| `get_many(json, paths)`         | Query `json` (str) at each path; returns `list[Result]`  |
| `get_many_bytes(json, paths)`   | Query `json` (bytes) at each path; returns `list[Result]`|
| `parse(json)`                   | Parse the entire JSON document into a `Result`           |
| `validate(json)`                | `True` if `json` is syntactically valid                  |

`get_bytes` and `get_many_bytes` raise `UnicodeDecodeError` if `json` is not valid UTF-8.

### Result

`get` and `parse` return a `Result`. The API is split into two layers:

**Properties**

| Property    | Description                                                                      |
|-------------|----------------------------------------------------------------------------------|
| `v.type_`   | Python type for this value: `None`, `bool`, `int`, `float`, `str`, `list`, `dict` |
| `v.value`   | Value converted to the corresponding Python type                                 |

**gjson-native methods** — mirror the Rust `gjson::Value` API:

| Method                   | Description                                               |
|--------------------------|-----------------------------------------------------------|
| `v.exists()`             | `True` if the value was found in the JSON                 |
| `v.to_str()`             | String representation (gjson `str` behaviour)             |
| `v.to_int()`             | Integer (`i64` for negative, `u64` for non-negative)      |
| `v.to_float()`           | 64-bit float                                              |
| `v.to_bool()`            | `True` only for the JSON literal `true`                   |
| `v.get(path)`            | Sub-query relative to this value                          |
| `v.get_bytes(path)`      | Same as `get` but uses byte-slice internally              |
| `v.get_many(paths)`      | Sub-query at multiple paths; returns `list[Result]`       |
| `v.get_many_bytes(paths)`| Same as `get_many` but uses byte-slice internally         |

**Pythonic methods** — follow standard Python protocols:

| Syntax              | Description                                                                   |
|---------------------|-------------------------------------------------------------------------------|
| `str(v)`,`repr(v)`  | dict: `<Result type=dict, keys=[...]>`; list: `<Result type=list, value=[...]>`; others: `str(v.value)` |
| `int(v)`            | Integer (negative → `i64`, non-negative → `u64`)                              |
| `float(v)`          | 64-bit float                                                                  |
| `bool(v)`           | Equivalent to `bool(v.value)` — `False` for null/false/0/""/[]/{}            |
| `len(v)`            | Chars for String; element count for Array/Object                              |
| `v[key]`            | Subscript access — see table below                                            |
| `key in v`          | Key membership for Object; string match for Array elements                    |
| `iter(v)`           | Lazy iterator: chars for String; `Result`s for Array; keys for Object         |
| `v.keys()`          | Lazy `KeysView` of object keys (raises `TypeError` for non-Object)            |
| `v.values()`        | Lazy `ValuesView` of object values (raises `TypeError` for non-Object)        |
| `v.items()`         | Lazy `ItemsView` of `(key, Result)` pairs (raises `TypeError` for non-Object) |

#### Subscript access `v[key]`

| Value type | `key` type | Result                                     |
|------------|------------|--------------------------------------------|
| String     | `int`      | Nth code point as `str` (negative indexing supported) |
| String     | `slice`    | Substring as `str`                         |
| String     | `str`      | `TypeError`                                |
| Array      | `int`      | `Result` at that index (negative indexing supported) |
| Array      | `slice`    | New `Result` of type Array with selected elements |
| Array      | `str`      | `TypeError`                                |
| Object     | `str`      | `Result` at that key                       |
| Object     | `int`/`slice` | `KeyError`                              |
| Null       | `int`      | `IndexError`                               |
| Null       | `slice`    | Empty `Result` (`exists()` is `False`)     |
| Null       | `str`      | `TypeError`                                |

#### Lazy iteration

`iter(v)`, `v.keys()`, `v.values()` and `v.items()` all return lightweight
lazy objects rather than fully-materialised lists, mirroring Python's built-in
`dict_keys` / `dict_values` / `dict_items`.

```python
v = parse('{"a": 1, "b": 2, "c": 3}')

ks = v.keys()           # KeysView — no materialisation yet
len(ks)                 # 3
"a" in ks               # True
list(ks)                # ['a', 'b', 'c']

for k, child in v.items():   # ItemsView, lazily yields one pair at a time
    ...
```

If you need a fully materialised collection, wrap the view with `list(...)` or
`dict(...)` explicitly, or use `v.value` to get a native Python object.

## Usage examples

```python
from pygjson import get, get_bytes, get_many, get_many_bytes, parse, validate

# Missing value returns Result(exists=False)
v = get(JSON, "no.such.path")
v.exists()   # False
bool(v)      # False (null → bool(None) = False)

# Type conversion
age = get(JSON, "age")
age.to_int()    # gjson i64/u64 behaviour
int(age)        # Python int protocol
age.type_       # <class 'int'>
age.value       # 37

# Boolean distinction
get('{"flag": true}', "flag").to_bool()   # True  (JSON true literal)
bool(get(JSON, "age"))                    # True  (37 is truthy)
bool(get('{"n": 0}', "n"))               # False (0 is falsy)

# Bytes input
get_bytes(JSON.encode(), "name.first")    # Result("Tom")

# Array iteration and subscript
children = get(JSON, "children")
list(children)                  # [Result("Sara"), Result("Alex"), Result("Jack")]
[str(v) for v in children]      # ['Sara', 'Alex', 'Jack']
"Sara" in children              # True
str(children[0])                # 'Sara'
str(children[-1])               # 'Jack'
[str(v) for v in children[1:]] # ['Alex', 'Jack']

# String subscript
first = get(JSON, "name.first")   # "Tom"
first[0]                          # 'T'
first[-1]                         # 'm'
first[1:]                         # 'om'
first[::-1]                       # 'moT'

# Object (dict-like) access
name = get(JSON, "name")
str(name["first"])              # 'Tom'
"first" in name                 # True
list(name)                      # ['first', 'last']  — keys
dict(name)                      # {'first': Result("Tom"), 'last': Result("Anderson")}
for k, v in name.items():
    print(k, str(v))

# repr
repr(name)                      # '<Result type=dict, keys=["first", "last"]>'
repr(children)                  # '<Result type=list, value=["Sara", "Alex", ...]>'
repr(age)                       # '37'
repr(first)                     # 'Tom'

# Chained queries
parse(JSON).get("name").get("first")   # Result("Tom")

# Fetch multiple paths in one call
get_many(JSON, ["name.first", "age", "children.1"])
# [Result(Tom), Result(37), Result(Alex)]

# Missing paths return Result(exists=False)
get_many(JSON, ["name.first", "no.such.path"])
# [Result(Tom), Result()]

# get_many with bytes input
get_many_bytes(JSON.encode(), ["name.first", "age"])
# [Result(Tom), Result(37)]

# Result.get_many for sub-queries relative to a parsed document
parse(JSON).get_many(["name.first", "name.last"])
# [Result(Tom), Result(Anderson)]
```

## Path syntax

For the full path / query / modifier syntax see the upstream
[gjson.rs](https://github.com/tidwall/gjson.rs) and the original
[GJSON path syntax](https://github.com/tidwall/gjson/blob/master/SYNTAX.md).

## License

MIT
