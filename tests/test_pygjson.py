import pygjson
from pygjson import Kind, Value, get, parse, valid

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
    assert all_murphys.kind() == Kind.Array
    assert [str(v) for v in all_murphys.to_list()] == ["Dale", "Jane"]


def test_kind_and_exists():
    v = get(JSON, "friends")
    assert v.kind() == Kind.Array
    assert v.exists()
    missing = get(JSON, "no.such.path")
    assert not missing.exists()
    assert not bool(missing)


def test_array_iteration():
    nets = get(JSON, "friends.0.nets")
    assert nets.kind() == Kind.Array
    assert [str(n) for n in nets.to_list()] == ["ig", "fb", "tw"]
    assert [str(n) for n in nets] == ["ig", "fb", "tw"]


def test_to_dict_object():
    name = get(JSON, "name")
    m = name.to_dict()
    assert set(m.keys()) == {"first", "last"}
    assert isinstance(m["first"], Value)
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
    # keys() / values() / items() should now return view objects (similar to
    # dict.keys()) rather than fully-materialised lists. They support len(),
    # iteration, and (for keys) membership tests.
    name = get(JSON, "name")

    kv = name.keys()
    assert len(kv) == 2
    assert "first" in kv
    assert "missing" not in kv
    assert list(kv) == ["first", "last"]
    # The view is re-iterable.
    assert list(kv) == ["first", "last"]

    vv = name.values()
    assert len(vv) == 2
    assert [str(v) for v in vv] == ["Tom", "Anderson"]

    iv = name.items()
    assert len(iv) == 2
    assert [(k, str(v)) for k, v in iv] == [("first", "Tom"), ("last", "Anderson")]


def test_iter_yields_one_at_a_time():
    # ``iter()`` returns a custom iterator object so that callers can pull
    # one element at a time without paying for materialising the rest.
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
    # __iter__ yields keys for Object
    assert list(name) == ["first", "last"]
    # items() yields (key, Value) pairs
    assert ("first", "Tom") in [(k, str(v)) for k, v in name.items()]


def test_parse_and_get_chained():
    root = parse(JSON)
    assert root.exists()
    assert str(root.get("name").get("first")) == "Tom"


def test_valid():
    assert valid(JSON)
    assert valid("[1,2,3]")
    assert not valid("{not json")


def test_modifier_reverse():
    rev = get(JSON, "children|@reverse")
    assert [str(v) for v in rev.to_list()] == ["Jack", "Alex", "Sara"]


def test_repr_and_json():
    v = get(JSON, "name")
    assert v.json().startswith("{")
    assert "Value(" in repr(v)


def test_kind_values_distinct():
    kinds = [Kind.Null, Kind.False_, Kind.True_, Kind.Number,
             Kind.String, Kind.Array, Kind.Object]
    for i, a in enumerate(kinds):
        for j, b in enumerate(kinds):
            assert (a == b) is (i == j)


def test_module_exports():
    assert pygjson.get is get
    assert pygjson.parse is parse
    assert pygjson.valid is valid


def test_get_default_module_level():
    # default なし: gjson 本来の動作 (exists=False の Value が返る)
    missing = get(JSON, "no.such.path")
    assert isinstance(missing, Value)
    assert not missing.exists()

    # default=None: None が返る
    assert get(JSON, "no.such.path", None) is None

    # 任意の値をデフォルトにできる
    assert get(JSON, "no.such.path", 42) == 42
    assert get(JSON, "no.such.path", "fallback") == "fallback"

    # 見つかった場合は default を無視して Value が返る
    result = get(JSON, "age", None)
    assert isinstance(result, Value)
    assert int(result) == 37

    # JSON null は exists=True なので default は使われない
    null_result = get('{"key": null}', "key", "fallback")
    assert isinstance(null_result, Value)
    assert null_result.exists()


def test_get_default_value_method():
    root = parse(JSON)

    # default なし: gjson 本来の動作
    missing = root.get("no.such.path")
    assert isinstance(missing, Value)
    assert not missing.exists()

    # default=None
    assert root.get("no.such.path", None) is None

    # 任意の値をデフォルトにできる
    assert root.get("no.such.path", 99) == 99

    # 見つかった場合は default を無視
    result = root.get("age", None)
    assert isinstance(result, Value)
    assert int(result) == 37

    # JSON null は exists=True なので default は使われない
    null_root = parse('{"key": null}')
    null_result = null_root.get("key", "fallback")
    assert isinstance(null_result, Value)
    assert null_result.exists()


def test_to_str():
    assert get(JSON, "name.last").to_str() == "Anderson"
    assert get(JSON, "age").to_str() == "37"


def test_to_int():
    assert get(JSON, "age").to_int() == 37
    assert get(JSON, "children.#").to_int() == 3
    assert get('{"n": -1}', "n").to_int() == -1


def test_to_uint():
    assert get(JSON, "age").to_uint() == 37
    assert get('{"n": 18446744073709551615}', "n").to_uint() == 18446744073709551615


def test_to_float():
    assert get(JSON, "age").to_float() == 37.0


def test_to_bool():
    # JSON true/false literals
    assert get('{"flag": true}', "flag").to_bool() is True
    assert get('{"flag": false}', "flag").to_bool() is False
    # Numbers: non-zero → True, zero → False (gjson behaviour)
    assert get(JSON, "age").to_bool() is True   # 37 is non-zero
    assert get('{"n": 0}', "n").to_bool() is False
    # Strings and null → False
    assert get(JSON, "name.last").to_bool() is False


def test_dunder_int():
    assert int(get(JSON, "age")) == 37
    # Large u64 value (> i64::MAX)
    assert int(get('{"n": 18446744073709551615}', "n")) == 18446744073709551615
    # Negative value uses i64
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
