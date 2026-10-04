"""Literal parameter sets for static test collection.

``@parametrize("value, expected", [(1, 2), (2, 3)])`` creates one test case per
row. Stack decorators for a Cartesian product. Lists, dictionaries with string
keys, strings, finite numbers, booleans, and None can be argument values. Tuple
containers and tuple rows are supported; tuple-valued arguments are rejected
by collection because the worker protocol preserves JSON value types.

Collection reports dynamic expressions, empty sets, and malformed rows as errors.
This decorator preserves the original callable and does not import pytest.
"""
from __future__ import annotations

from collections.abc import Callable, Sequence


def parametrize[F: Callable](
    argnames: str | Sequence[str],
    argvalues: Sequence[object],
    *,
    ids: Sequence[str | int | float | bool | None] | None = None,
    indirect: bool = False,
) -> Callable[[F], F]:
    """Declare independent literal cases; the native collector expands them."""
    if indirect:
        raise ValueError("indirect parametrization is unsupported")

    def decorate(function: F) -> F:
        existing = getattr(function, "_taut_parametrize", ())
        function._taut_parametrize = (  # type: ignore[attr-defined]
            *existing,
            {"argnames": argnames, "argvalues": argvalues, "ids": ids},
        )
        return function

    return decorate
