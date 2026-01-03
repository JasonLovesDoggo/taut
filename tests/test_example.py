"""Example tests for taut to run on itself."""

import time


def test_simple_pass():
    assert 1 + 1 == 2


def test_string_equality():
    assert "hello" == "hello"


def test_list_contains():
    items = [1, 2, 3, 4, 5]
    assert 3 in items


def test_with_small_delay():
    time.sleep(0.01)
    assert True


class TestMath:
    def test_addition(self):
        assert 2 + 2 == 4

    def test_multiplication(self):
        assert 3 * 4 == 12


class TestStrings:
    def setUp(self):
        self.greeting = "hello"

    def test_upper(self):
        assert self.greeting.upper() == "HELLO"

    def test_length(self):
        assert len(self.greeting) == 5


def helper_function():
    """Not a test - should be ignored."""
    pass


class HelperClass:
    """Not a test class - should be ignored."""

    def test_method(self):
        pass
