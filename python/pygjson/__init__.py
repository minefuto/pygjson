"""Python bindings for gjson.rs - fast JSON path queries."""

from ._pygjson import Kind, Value
from ._pygjson import get as _get, parse, valid
from ._pygjson import get_many as _get_many

__all__ = ["Kind", "Value", "get", "get_many", "parse", "valid"]

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


def get_many(json: str, paths, default=_MISSING):
    """Get the values at each path in ``paths`` from the given JSON document.

    If ``default`` is given, any path that is not found returns ``default``
    instead of a ``Value`` with ``exists=False``.
    """
    results = _get_many(json, list(paths))
    if default is _MISSING:
        return results
    return [r if r.exists() else default for r in results]
