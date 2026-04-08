"""Python bindings for gjson.rs - fast JSON path queries."""

from ._pygjson import Kind, Value
from ._pygjson import get as _get, parse, valid

__all__ = ["Kind", "Value", "get", "parse", "valid"]

_MISSING = object()


def get(json: str, path: str, default=_MISSING):
    """Get the value at ``path`` from the given JSON document.

    If ``default`` is given and the path is not found, returns ``default``
    instead of a ``Value`` with ``exists=False``.
    """
    result = _get(json, path)
    if not result.exists() and default is not _MISSING:
        return default
    return result
