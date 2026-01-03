"""Taut - Tests, without the overhead.

A fast Python test runner written in Rust.

This module provides decorators for marking tests:
- @skip - Skip a test, optionally with a reason
- @mark - Add metadata to a test (slow=True, group="auth", etc.)
- @parallel - Mark a test as parallel-safe
"""

from typing import Any, Callable, List, Optional, TypeVar, Union

F = TypeVar("F", bound=Callable[..., Any])


# =============================================================================
# @skip decorator
# =============================================================================


def skip(arg: Union[F, str, None] = None, *, reason: Optional[str] = None) -> Any:
    """
    Decorator to skip a test.

    Usage:
        @skip
        def test_not_implemented():
            pass

        @skip("API is down")
        def test_api():
            pass

        @skip(reason="Flaky test")
        def test_flaky():
            pass
    """

    def decorator(func: F) -> F:
        func._taut_skip = True  # type: ignore[attr-defined]
        func._taut_skip_reason = skip_reason or ""  # type: ignore[attr-defined]
        return func

    # @skip (no parens, no reason)
    if callable(arg):
        skip_reason = ""
        return decorator(arg)

    # @skip("reason") or @skip(reason="reason")
    skip_reason = arg or reason or ""
    return decorator


# =============================================================================
# @mark decorator
# =============================================================================


def mark(**kwargs: Any) -> Callable[[F], F]:
    """
    Decorator to add metadata markers to a test.

    Usage:
        @mark(slow=True)
        def test_expensive():
            pass

        @mark(group="auth")
        def test_login():
            pass

        @mark(group=["auth", "integration"])
        def test_api_auth():
            pass

        @mark(slow=True, group="integration")
        def test_slow_integration():
            pass
    """

    def decorator(func: F) -> F:
        if not hasattr(func, "_taut_markers"):
            func._taut_markers = {}  # type: ignore[attr-defined]
        func._taut_markers.update(kwargs)  # type: ignore[attr-defined]
        return func

    return decorator


# =============================================================================
# @parallel decorator
# =============================================================================


def parallel(func: Optional[F] = None) -> Any:
    """
    Decorator to mark a test as safe to run in parallel with other parallel tests.

    By default, tests run sequentially. Use @parallel to opt-in to parallel execution.

    Usage:
        @parallel
        def test_fast():
            pass

        @parallel()
        def test_also_fast():
            pass

        @parallel()
        class TestFastOperations:
            def test_a(self):
                pass

            def test_b(self):
                pass
    """

    def decorator(func_or_class: F) -> F:
        func_or_class._taut_parallel = True  # type: ignore[attr-defined]
        return func_or_class

    # @parallel (no parens)
    if func is not None:
        return decorator(func)

    # @parallel()
    return decorator


# =============================================================================
# Exports
# =============================================================================

__all__ = ["skip", "mark", "parallel"]
__version__ = "0.1.0"
