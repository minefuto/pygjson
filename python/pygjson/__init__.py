"""Python bindings for gjson.rs - fast JSON path queries."""

from ._pygjson import Result
from ._pygjson import get, get_many, parse, validate

__all__ = ["Result", "get", "get_many", "parse", "validate"]
