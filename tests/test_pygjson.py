import pytest
import pygjson
from pygjson import Result, get, get_bytes, get_many, get_many_bytes, parse, validate

JSON = """{
  "name": {"first": "Tom", "last": "Anderson"},
  "age": 37,
  "children": ["Sara", "Alex", "Jack"],
  "fav.movie": "Deer Hunter",
  "friends": [
    {"first": "Dale", "last": "Murphy", "age": 44, "nets": ["ig", "fb", "tw"]},
    {"first": "Roger", "last": "Craig", "age": 68, "nets": ["fb", "tw"]},
    {"first": "Jane", "last": "Murphy", "age": 47, "nets": ["ig", "tw"]}
  ]
}"""


def test_basic_get():
    assert str(get(JSON, "name.last")) == "Anderson"
    assert str(get(JSON, "name.first")) == "Tom"
    assert int(get(JSON, "age")) == 37
    assert float(get(JSON, "age")) == 37.0
    assert int(get(JSON, "children.#")) == 3
    assert str(get(JSON, "children.1")) == "Alex"


def test_escaped_key():
    assert str(get(JSON, "fav\\.movie")) == "Deer Hunter"


def test_query_filter():
    assert str(get(JSON, 'friends.#(last="Murphy").first')) == "Dale"
    all_murphys = get(JSON, 'friends.#(last="Murphy")#.first')
    assert all_murphys.type_ == list
    assert [str(v) for v in all_murphys] == ["Dale", "Jane"]


def test_type_and_exists():
    v = get(JSON, "friends")
    assert v.type_ == list
    assert v.exists()
    missing = get(JSON, "no.such.path")
    assert not missing.exists()
    assert not bool(missing)


def test_array_iteration():
    nets = get(JSON, "friends.0.nets")
    assert nets.type_ == list
    assert [str(n) for n in nets] == ["ig", "fb", "tw"]


def test_to_dict_object():
    name = get(JSON, "name")
    m = dict(name.items())
    assert set(m.keys()) == {"first", "last"}
    assert isinstance(m["first"], Result)
    assert str(m["first"]) == "Tom"


def test_list_string():
    first = get(JSON, "name.first")
    assert list(first) == list("Tom")
    assert len(first) == 3


def test_list_array():
    children = get(JSON, "children")
    result = list(children)
    assert [str(v) for v in result] == ["Sara", "Alex", "Jack"]
    assert len(children) == 3


def test_list_object():
    name = get(JSON, "name")
    assert list(name) == ["first", "last"]
    assert len(name) == 2


def test_dict_object():
    name = get(JSON, "name")
    d = dict(name)
    assert set(d.keys()) == {"first", "last"}
    assert str(d["first"]) == "Tom"
    assert str(d["last"]) == "Anderson"


def test_dict_non_object_raises():
    import pytest
    with pytest.raises(TypeError):
        dict(get(JSON, "children"))   # Array
    with pytest.raises(TypeError):
        dict(get(JSON, "name.first")) # String
    with pytest.raises(TypeError):
        dict(get(JSON, "age"))        # Number


def test_keys_values_items():
    name = get(JSON, "name")
    assert list(name.keys()) == ["first", "last"]
    assert [str(v) for v in name.values()] == ["Tom", "Anderson"]
    assert [(k, str(v)) for k, v in name.items()] == [("first", "Tom"), ("last", "Anderson")]


def test_views_are_lazy_and_support_protocol():
    name = get(JSON, "name")

    kv = name.keys()
    assert len(kv) == 2
    assert "first" in kv
    assert "missing" not in kv
    assert list(kv) == ["first", "last"]
    assert list(kv) == ["first", "last"]

    vv = name.values()
    assert len(vv) == 2
    assert [str(v) for v in vv] == ["Tom", "Anderson"]

    iv = name.items()
    assert len(iv) == 2
    assert [(k, str(v)) for k, v in iv] == [("first", "Tom"), ("last", "Anderson")]


def test_iter_yields_one_at_a_time():
    children = get(JSON, "children")
    it = iter(children)
    assert str(next(it)) == "Sara"
    assert str(next(it)) == "Alex"
    assert str(next(it)) == "Jack"
    import pytest
    with pytest.raises(StopIteration):
        next(it)


def test_keys_values_items_non_object_raises():
    import pytest
    for path in ("children", "name.first", "age"):
        v = get(JSON, path)
        with pytest.raises(TypeError):
            v.keys()
        with pytest.raises(TypeError):
            v.values()
        with pytest.raises(TypeError):
            v.items()


def test_object_iteration():
    name = get(JSON, "name")
    assert list(name) == ["first", "last"]
    assert ("first", "Tom") in [(k, str(v)) for k, v in name.items()]


def test_parse_and_get_chained():
    root = parse(JSON)
    assert root.exists()
    assert str(root.get("name").get("first")) == "Tom"


def test_validate():
    assert validate(JSON)
    assert validate("[1,2,3]")
    assert not validate("{not json")


def test_modifier_reverse():
    rev = get(JSON, "children|@reverse")
    assert [str(v) for v in rev] == ["Jack", "Alex", "Sara"]


def test_repr():
    # Object: show up to 2 keys
    name = get(JSON, "name")
    assert repr(name) == '<Result type=dict, keys=["first", "last"]>'

    # Object with 3+ keys: truncate with ...
    friends0 = get(JSON, "friends.0")
    r = repr(friends0)
    assert r.startswith("<Result type=dict")
    assert "..." in r

    # Array: show up to 2 raw values
    children = get(JSON, "children")
    r = repr(children)
    assert r.startswith("<Result type=list")
    assert '"Sara"' in r
    assert "..." in r

    # Number
    assert repr(get(JSON, "age")) == "37"

    # String
    assert repr(get(JSON, "name.first")) == "Tom"

    # Bool
    assert repr(get('{"flag": true}', "flag")) == "True"
    assert repr(get('{"flag": false}', "flag")) == "False"

    # Null
    assert repr(get('{"key": null}', "key")) == "None"


def test_module_exports():
    assert pygjson.get is get
    assert pygjson.parse is parse
    assert pygjson.validate is validate


def test_to_str():
    assert get(JSON, "name.last").to_str() == "Anderson"
    assert get(JSON, "age").to_str() == "37"


def test_to_int():
    assert get(JSON, "age").to_int() == 37
    assert get(JSON, "children.#").to_int() == 3
    assert get('{"n": -1}', "n").to_int() == -1
    # Large u64 value (> i64::MAX)
    assert get('{"n": 18446744073709551615}', "n").to_int() == 18446744073709551615


def test_to_float():
    assert get(JSON, "age").to_float() == 37.0


def test_to_bool():
    assert get('{"flag": true}', "flag").to_bool() is True
    assert get('{"flag": false}', "flag").to_bool() is False
    assert get(JSON, "age").to_bool() is True
    assert get('{"n": 0}', "n").to_bool() is False
    assert get(JSON, "name.last").to_bool() is False


def test_dunder_int():
    assert int(get(JSON, "age")) == 37
    assert int(get('{"n": 18446744073709551615}', "n")) == 18446744073709551615
    assert int(get('{"n": -1}', "n")) == -1


def test_dunder_float():
    assert float(get(JSON, "age")) == 37.0
    assert float(get('{"x": 3.14}', "x")) == 3.14


def test_contains_object():
    name = get(JSON, "name")
    assert "first" in name
    assert "last" in name
    assert "missing" not in name


def test_contains_array():
    children = get(JSON, "children")
    assert "Sara" in children
    assert "Alex" in children
    assert "nobody" not in children


def test_contains_non_collection_raises():
    import pytest
    with pytest.raises(TypeError):
        _ = "x" in get(JSON, "age")
    with pytest.raises(TypeError):
        _ = "x" in get(JSON, "name.first")


def test_get_many_module_level_basic():
    results = get_many(JSON, ["name.first", "age", "children.1"])
    assert len(results) == 3
    assert str(results[0]) == "Tom"
    assert int(results[1]) == 37
    assert str(results[2]) == "Alex"


def test_get_many_missing_no_default():
    results = get_many(JSON, ["name.first", "no.such.path"])
    assert len(results) == 2
    assert str(results[0]) == "Tom"
    assert isinstance(results[1], Result)
    assert not results[1].exists()


def test_get_many_empty_paths():
    results = get_many(JSON, [])
    assert results == []


def test_get_many_value_method():
    root = parse(JSON)
    results = root.get_many(["name.first", "name.last"])
    assert len(results) == 2
    assert str(results[0]) == "Tom"
    assert str(results[1]) == "Anderson"


def test_get_many_value_method_missing_no_default():
    root = parse(JSON)
    results = root.get_many(["name.first", "no.such.path"])
    assert len(results) == 2
    assert str(results[0]) == "Tom"
    assert isinstance(results[1], Result)
    assert not results[1].exists()


def test_get_many_module_export():
    assert pygjson.get_many is get_many


def test_type_null():
    result = get('{"key": null}', "key")
    assert result.type_ is None


def test_type_bool():
    assert get('{"flag": true}', "flag").type_ is bool
    assert get('{"flag": false}', "flag").type_ is bool


def test_type_number():
    assert get('{"n": 42}', "n").type_ is int
    assert get('{"n": -1}', "n").type_ is int
    assert get('{"n": 3.14}', "n").type_ is float
    assert get('{"n": 1e10}', "n").type_ is float


def test_type_string():
    assert get(JSON, "name.first").type_ is str


def test_type_array():
    assert get(JSON, "children").type_ is list


def test_type_object():
    assert get(JSON, "name").type_ is dict


def test_value_null():
    result = get('{"key": null}', "key")
    assert result.value is None


def test_value_bool():
    assert get('{"flag": true}', "flag").value is True
    assert get('{"flag": false}', "flag").value is False


def test_value_number():
    assert get('{"n": 42}', "n").value == 42
    assert isinstance(get('{"n": 42}', "n").value, int)
    assert get('{"n": 3.14}', "n").value == 3.14
    assert isinstance(get('{"n": 3.14}', "n").value, float)


def test_value_string():
    assert get(JSON, "name.first").value == "Tom"


def test_value_array():
    children = get(JSON, "children")
    v = children.value
    assert isinstance(v, list)
    assert [str(item) for item in v] == ["Sara", "Alex", "Jack"]


def test_value_object():
    name = get(JSON, "name")
    v = name.value
    assert isinstance(v, dict)
    assert set(v.keys()) == {"first", "last"}
    assert str(v["first"]) == "Tom"


# ---------------------------------------------------------------------------
# get_bytes / get_many_bytes
# ---------------------------------------------------------------------------

def test_get_bytes_module_level():
    assert str(get_bytes(JSON.encode(), "name.first")) == "Tom"
    assert int(get_bytes(JSON.encode(), "age")) == 37


def test_get_bytes_invalid_utf8():
    with pytest.raises(UnicodeDecodeError):
        get_bytes(b"\xff\xfe", "name")


def test_get_many_bytes_module_level():
    results = get_many_bytes(JSON.encode(), ["name.first", "age"])
    assert str(results[0]) == "Tom"
    assert int(results[1]) == 37


def test_get_bytes_exports():
    assert pygjson.get_bytes is get_bytes
    assert pygjson.get_many_bytes is get_many_bytes


def test_kind_not_exported():
    import pygjson._pygjson as m
    assert not hasattr(m, "Kind")


# ---------------------------------------------------------------------------
# __bool__ (bool(result.value) semantics)
# ---------------------------------------------------------------------------

def test_dunder_bool_null():
    assert not bool(get('{"x": null}', "x"))
    assert not bool(get(JSON, "no.such.path"))  # missing → Null → false


def test_dunder_bool_bool_values():
    assert not bool(get('{"x": false}', "x"))
    assert bool(get('{"x": true}', "x"))


def test_dunder_bool_number():
    assert bool(get('{"x": 1}', "x"))
    assert bool(get('{"x": -1}', "x"))
    assert not bool(get('{"x": 0}', "x"))
    assert not bool(get('{"x": 0.0}', "x"))
    assert bool(get('{"x": 3.14}', "x"))


def test_dunder_bool_string():
    assert bool(get('{"x": "hello"}', "x"))
    assert not bool(get('{"x": ""}', "x"))


def test_dunder_bool_array():
    assert bool(get('{"x": [1]}', "x"))
    assert not bool(get('{"x": []}', "x"))


def test_dunder_bool_object():
    assert bool(get('{"x": {"a": 1}}', "x"))
    assert not bool(get('{"x": {}}', "x"))


# ---------------------------------------------------------------------------
# __getitem__ extension
# ---------------------------------------------------------------------------

def test_getitem_string_int():
    s = get(JSON, "name.first")  # "Tom"
    assert s[0] == "T"
    assert s[1] == "o"
    assert s[-1] == "m"


def test_getitem_string_int_oob():
    s = get(JSON, "name.first")
    with pytest.raises(IndexError):
        _ = s[100]
    with pytest.raises(IndexError):
        _ = s[-100]


def test_getitem_string_slice():
    s = get(JSON, "name.first")  # "Tom"
    assert s[0:2] == "To"
    assert s[1:] == "om"
    assert s[::-1] == "moT"
    assert s[:] == "Tom"


def test_getitem_string_str_raises():
    s = get(JSON, "name.first")
    with pytest.raises(TypeError):
        _ = s["x"]


def test_getitem_string_bool_as_int():
    s = get(JSON, "name.first")  # "Tom"
    assert s[False] == "T"  # False == 0
    assert s[True] == "o"   # True == 1


def test_getitem_array_int():
    children = get(JSON, "children")
    assert str(children[0]) == "Sara"
    assert str(children[2]) == "Jack"
    assert str(children[-1]) == "Jack"
    assert str(children[-3]) == "Sara"


def test_getitem_array_int_oob():
    children = get(JSON, "children")
    with pytest.raises(IndexError):
        _ = children[100]
    with pytest.raises(IndexError):
        _ = children[-100]


def test_getitem_array_slice():
    children = get(JSON, "children")
    sliced = children[0:2]
    assert isinstance(sliced, Result)
    assert sliced.type_ == list
    assert [str(v) for v in sliced] == ["Sara", "Alex"]
    assert [str(v) for v in children[::-1]] == ["Jack", "Alex", "Sara"]
    assert [str(v) for v in children[:]] == ["Sara", "Alex", "Jack"]


def test_getitem_array_str_raises():
    children = get(JSON, "children")
    with pytest.raises(TypeError):
        _ = children["x"]


def test_getitem_array_bool_as_int():
    children = get(JSON, "children")
    assert str(children[False]) == "Sara"   # False == 0
    assert str(children[True]) == "Alex"    # True == 1


def test_getitem_object_str():
    name = get(JSON, "name")
    assert str(name["first"]) == "Tom"
    assert str(name["last"]) == "Anderson"


def test_getitem_object_int_raises():
    name = get(JSON, "name")
    with pytest.raises(KeyError):
        _ = name[0]


def test_getitem_object_slice_raises():
    name = get(JSON, "name")
    with pytest.raises(KeyError):
        _ = name[0:1]


def test_getitem_null_int_raises():
    null_val = get('{"x": null}', "x")
    with pytest.raises(IndexError):
        _ = null_val[0]


def test_getitem_null_slice_returns_empty():
    null_val = get('{"x": null}', "x")
    r = null_val[0:1]
    assert isinstance(r, Result)
    assert not r.exists()


def test_getitem_null_str_raises():
    null_val = get('{"x": null}', "x")
    with pytest.raises(TypeError):
        _ = null_val["key"]


def test_getitem_number_raises():
    age = get(JSON, "age")
    with pytest.raises(TypeError):
        _ = age[0]
