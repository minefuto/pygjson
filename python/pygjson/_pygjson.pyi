"""Type stubs for the pygjson native extension module."""

from typing import Any, Dict, Iterator, List, Optional, Sequence, Tuple, Union

class ValueIterator:
    """Lazy iterator over a :class:`Result`'s children."""

    def __iter__(self) -> "ValueIterator": ...
    def __next__(self) -> Union[str, "Result", Tuple[str, "Result"]]: ...
    def __length_hint__(self) -> int: ...

class KeysView:
    """Lazy view of an Object value's keys (similar to ``dict.keys()``)."""

    def __iter__(self) -> Iterator[str]: ...
    def __len__(self) -> int: ...
    def __contains__(self, item: str) -> bool: ...
    def __repr__(self) -> str: ...

class ValuesView:
    """Lazy view of an Object value's values (similar to ``dict.values()``)."""

    def __iter__(self) -> Iterator["Result"]: ...
    def __len__(self) -> int: ...
    def __repr__(self) -> str: ...

class ItemsView:
    """Lazy view of an Object value's ``(key, value)`` pairs."""

    def __iter__(self) -> Iterator[Tuple[str, "Result"]]: ...
    def __len__(self) -> int: ...
    def __repr__(self) -> str: ...

class Result:
    """A JSON value returned by :func:`get` or :func:`parse`."""

    @property
    def type_(self) -> Optional[type]:
        """Return the Python type for this value's JSON kind.

        Null → None; True/False → bool; Number → int or float;
        String → str; Array → list; Object → dict.
        """

    @property
    def value(self) -> Any:
        """Return the inner value as the Python type indicated by ``type_``.

        Null → None; bool kinds → bool; Number → int or float;
        String → str; Array → list[Result]; Object → dict[str, Result].
        """

    def exists(self) -> bool:
        """Whether the value was actually present in the source JSON."""

    def to_str(self) -> str:
        """String representation of the value. Same as ``str(value)``."""

    def to_int(self) -> int:
        """Integer value. Uses ``u64`` for non-negative, ``i64`` for negative."""

    def to_float(self) -> float:
        """Floating point value. Same as ``float(value)``."""

    def to_bool(self) -> bool:
        """Boolean value. Returns ``True`` only for the JSON literal ``true``."""

    def json(self) -> str:
        """Raw JSON text for this value."""

    def get(self, path: str) -> "Result":
        """Get a child value at the given gjson path."""

    def get_bytes(self, path: str) -> "Result":
        """Get a child value at the given gjson path using byte-slice internally."""

    def get_many(self, paths: Sequence[str]) -> List["Result"]:
        """Get child values at each of the given gjson paths."""

    def get_many_bytes(self, paths: Sequence[str]) -> List["Result"]:
        """Get child values at each of the given gjson paths using byte-slice internally."""

    def to_list(self) -> List["Result"]:
        """Return the value as a list of :class:`Result` objects."""

    def to_dict(self) -> Dict[str, "Result"]:
        """Return the value as a ``dict[str, Result]``."""

    def keys(self) -> KeysView:
        """Return a lazy view of the object's keys.

        Raises ``TypeError`` for non-Object values.
        """

    def values(self) -> ValuesView:
        """Return a lazy view of the object's values.

        Raises ``TypeError`` for non-Object values.
        """

    def items(self) -> ItemsView:
        """Return a lazy view of the object's ``(key, value)`` pairs.

        Raises ``TypeError`` for non-Object values.
        """

    def __contains__(self, item: str) -> bool: ...
    def __len__(self) -> int: ...
    def __iter__(self) -> Iterator[Union[str, "Result"]]: ...
    def __getitem__(self, key: Union[str, int, slice]) -> Union["Result", str]: ...
    def __int__(self) -> int: ...
    def __float__(self) -> float: ...
    def __bool__(self) -> bool: ...
    def __repr__(self) -> str: ...
    def __str__(self) -> str: ...

def get(json: str, path: str) -> Result:
    """Get the value at ``path`` from the given JSON document."""

def get_bytes(json: bytes, path: str) -> Result:
    """Get the value at ``path`` from the given JSON bytes."""

def get_many(json: str, paths: Sequence[str]) -> List[Result]:
    """Get the values at each path in ``paths`` from the given JSON document."""

def get_many_bytes(json: bytes, paths: Sequence[str]) -> List[Result]:
    """Get the values at each path in ``paths`` from the given JSON bytes."""

def parse(json: str) -> Result:
    """Parse the entire JSON document into a :class:`Result`."""

def validate(json: str) -> bool:
    """Validate whether ``json`` is a syntactically valid JSON document."""
