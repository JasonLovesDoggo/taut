# Writing Tests

## Basic Tests

Tests are just Python functions that start with `test_` or `_test`:

```python
def test_addition():
    assert 1 + 1 == 2

def _test_also_runs():
    assert True
```

## Assertions

taut uses plain Python assertions. When a test fails, taut shows the assertion message:

```python
def test_with_message():
    result = calculate_total([1, 2, 3])
    assert result == 6, f"Expected 6, got {result}"
```

Output on failure:

```
✗ test_math.py::test_with_message (15ms)
  Expected 6, got 5
  Traceback (most recent call last):
    File "test_math.py", line 3, in test_with_message
      assert result == 6, f"Expected 6, got {result}"
  AssertionError: Expected 6, got 5
```

## Async Tests

Async tests work automatically - no decorators needed:

```python
import asyncio

async def test_async_operation():
    result = await fetch_data()
    assert result is not None

async def test_sleep():
    await asyncio.sleep(0.1)
    assert True
```

## Class-Based Tests

Group related tests in classes starting with `Test`:

```python
class TestUserAPI:
    def test_create(self):
        user = create_user("alice")
        assert user.name == "alice"
    
    def test_delete(self):
        user = create_user("bob")
        delete_user(user.id)
        assert get_user(user.id) is None
```

## Setup and Teardown

Use `setUp` and `tearDown` methods in test classes:

```python
class TestDatabase:
    def setUp(self):
        """Runs before each test method"""
        self.db = connect_database()
        self.db.begin_transaction()
    
    def tearDown(self):
        """Runs after each test method (even if test fails)"""
        self.db.rollback()
        self.db.close()
    
    def test_insert(self):
        self.db.insert({"name": "test"})
        assert self.db.count() == 1
    
    def test_query(self):
        self.db.insert({"name": "alice"})
        result = self.db.query(name="alice")
        assert len(result) == 1
```

!!! note
    `tearDown` always runs, even if the test fails or raises an exception. This ensures cleanup happens.

## Importing From Your Project

taut adds the test file's directory to Python's path, so relative imports work:

```
myproject/
  utils/
    helpers.py
  tests/
    test_helpers.py
```

```python
# tests/test_helpers.py
from utils.helpers import format_name

def test_format_name():
    assert format_name("alice") == "Alice"
```

## Exceptions

Test for expected exceptions using try/except:

```python
def test_raises_value_error():
    try:
        int("not a number")
        assert False, "Should have raised ValueError"
    except ValueError:
        pass  # Expected
```

Or use a helper pattern:

```python
def raises(exc_type, fn, *args, **kwargs):
    try:
        fn(*args, **kwargs)
        return False
    except exc_type:
        return True

def test_invalid_input():
    assert raises(ValueError, int, "not a number")
```

## Test Output

Capture stdout/stderr in your tests:

```python
def test_prints_hello():
    print("Hello, World!")
    # taut captures this output
```

When verbose mode is enabled (`-v`), captured output is shown for failed tests.
