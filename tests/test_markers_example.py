"""Test file demonstrating taut markers."""

import sys

sys.path.insert(0, "python")

from taut import skip, mark, parallel


@skip
def test_skipped_no_reason():
    """This test should be skipped."""
    assert False, "This should never run"


@skip("API is temporarily unavailable")
def test_skipped_with_reason():
    """This test should be skipped with a reason."""
    assert False, "This should never run"


@skip(reason="Known bug, fix later")
def test_skipped_keyword_reason():
    """This test should be skipped with keyword reason."""
    assert False, "This should never run"


@mark(slow=True)
def test_marked_slow():
    """This test is marked as slow."""
    assert True


@mark(group="auth")
def test_marked_group():
    """This test is in the auth group."""
    assert True


@mark(group=["auth", "integration"])
def test_marked_multiple_groups():
    """This test is in multiple groups."""
    assert True


@mark(slow=True, group="integration")
def test_marked_slow_and_group():
    """This test is slow and in the integration group."""
    assert True


@parallel()
def test_parallel_safe():
    """This test can run in parallel."""
    assert True


@parallel
def test_parallel_no_parens():
    """This test can run in parallel (no parens)."""
    assert True


def test_normal():
    """A normal test without markers."""
    assert True


@parallel()
class TestParallelClass:
    """All tests in this class can run in parallel."""

    def test_method_a(self):
        assert True

    def test_method_b(self):
        assert True


class TestMixedClass:
    """Class with mixed parallel/sequential tests."""

    @parallel()
    def test_parallel_method(self):
        """Can run in parallel."""
        assert True

    def test_sequential_method(self):
        """Runs sequentially."""
        assert True

    @skip("Not implemented yet")
    def test_skipped_method(self):
        """Should be skipped."""
        assert False
