# Quick Start

## Writing Your First Test

Create a file named `test_example.py`:

```python
def test_addition():
    assert 1 + 1 == 2

def test_string():
    assert "hello".upper() == "HELLO"
```

## Running Tests

Run taut in your project directory:

```bash
taut
```

Output:

```
taut ..
2 passed, in 0.05s
```

## Test Discovery

taut automatically discovers tests by looking for:

- Files matching `test_*.py` or `*_test.py`
- Functions starting with `test_` or `_test`
- Methods in classes starting with `Test`

```python
# test_users.py

def test_create_user():
    """Discovered: starts with test_"""
    pass

def _test_helper():
    """Discovered: starts with _test"""
    pass

class TestUserAPI:
    """Class starting with Test"""
    
    def test_get_user(self):
        """Discovered: method starting with test_"""
        pass
    
    def _test_internal(self):
        """Discovered: method starting with _test"""
        pass
```

## Filtering Tests

Run specific tests using the `-k` flag:

```bash
# Run tests containing "user"
taut -k user

# Run tests in a specific file
taut test_users.py

# Run a specific test function
taut test_users.py::test_create_user

# Use glob patterns
taut -k "test_*_api"
```

## Verbose Output

See individual test results:

```bash
taut -v
```

Output:

```
taut 
  ✓ test_example.py::test_addition (12ms)
  ✓ test_example.py::test_string (8ms)

2 passed, in 0.05s
```

## Watching for Changes

Automatically re-run tests when files change:

```bash
taut watch
```

## Next Steps

- [Writing Tests](../guide/writing-tests.md) - Learn about async tests, classes, and setup/teardown
- [Markers](../guide/markers.md) - Skip tests, mark them as slow, or enable parallel execution
- [Configuration](../guide/configuration.md) - Configure taut via pyproject.toml
