"""Type stubs for the pygjson native extension module."""

from typing import Dict, Iterator, List, Tuple, TypeVar, Union, overload

T = TypeVar("T")

class Kind:
    """Mirror of ``gjson::Kind``.

    Compare values with the class attributes ``Kind.Null``, ``Kind.False_``,
    ``Kind.True_``, ``Kind.Number``, ``Kind.String``, ``Kind.Array`` and
    ``Kind.Object``.
    """

    Null: "Kind"
    False_: "Kind"
    True_: "Kind"
    Number: "Kind"
    String: "Kind"
    Array: "Kind"
    Object: "Kind"

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...

class Value:
    """A JSON value returned by :func:`get` or :func:`parse`."""

    def kind(self) -> Kind:
        """Return the :class:`Kind` of this value."""

    def exists(self) -> bool:
        """Whether the value was actually present in the source JSON."""

    def to_str(self) -> str:
        """String representation of the value. Same as ``str(value)``."""

    def to_int(self) -> int:
        """Signed integer value (``i64``)."""

    def to_uint(self) -> int:
        """Unsigned integer value (``u64``)."""

    def to_float(self) -> float:
        """Floating point value. Same as ``float(value)``."""

    def to_bool(self) -> bool:
        """Boolean value. Returns ``True`` only for the JSON literal ``true``."""

    def json(self) -> str:
        """Raw JSON text for this value."""

    @overload
    def get(self, path: str) -> "Value": ...
    @overload
    def get(self, path: str, default: T) -> Union["Value", T]: ...
    def get(self, path: str, default: object = ...) -> object:
        """Get a child value at the given gjson path.

        If ``default`` is given and the path is not found, returns ``default``
        instead of a ``Value`` with ``exists=False``.
        """

    def to_list(self) -> List["Value"]:
        """Return the value as a list of :class:`Value` objects."""

    def to_dict(self) -> Dict[str, "Value"]:
        """Return the value as a ``dict[str, Value]``."""

    def keys(self) -> List[str]:
        """Return the object's keys. Raises ``TypeError`` for non-Object values."""

    def values(self) -> List["Value"]:
        """Return the object's values. Raises ``TypeError`` for non-Object values."""

    def items(self) -> List[Tuple[str, "Value"]]:
        """Return ``(key, value)`` pairs. Raises ``TypeError`` for non-Object values."""

    def __contains__(self, item: str) -> bool: ...
    def __len__(self) -> int: ...
    def __iter__(self) -> Iterator[Union[str, "Value"]]: ...
    def __getitem__(self, key: str) -> "Value": ...
    def __int__(self) -> int: ...
    def __float__(self) -> float: ...
    def __bool__(self) -> bool: ...
    def __repr__(self) -> str: ...
    def __str__(self) -> str: ...

@overload
def get(json: str, path: str) -> Value: ...
@overload
def get(json: str, path: str, default: T) -> Union[Value, T]: ...
def get(json: str, path: str, default: object = ...) -> object:
    """Get the value at ``path`` from the given JSON document.

    If ``default`` is given and the path is not found, returns ``default``
    instead of a ``Value`` with ``exists=False``.
    """

def parse(json: str) -> Value:
    """Parse the entire JSON document into a :class:`Value`."""

def valid(json: str) -> bool:
    """Validate whether ``json`` is a syntactically valid JSON document."""
