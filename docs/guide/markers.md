# Markers

Markers let you add metadata to tests - skip them, mark them as slow, group them, or enable parallel execution.

## Installation

Install the taut Python package to use markers:

```bash
pip install taut
```

## @skip

Skip a test entirely:

```python
from taut import skip

@skip
def test_not_implemented():
    pass

@skip("Waiting for API v2")
def test_new_feature():
    pass

@skip(reason="Flaky on CI")
def test_integration():
    pass
```

Skipped tests show in output:

```
taut S.
1 passed, 1 skipped, in 0.03s
```

## @mark

Add arbitrary metadata to tests:

```python
from taut import mark

@mark(slow=True)
def test_database_migration():
    # Takes a long time
    pass

@mark(group="api")
def test_api_endpoint():
    pass

@mark(group=["api", "integration"])
def test_api_integration():
    pass

@mark(priority=1, owner="alice")
def test_critical_path():
    pass
```

### Filtering by Markers

Use the `-m` flag to filter tests by markers:

```bash
# Run only slow tests
taut -m slow

# Run only api group tests  
taut -m "group=api"

# Run tests in multiple groups (OR logic)
taut -m "group=api or group=db"

# Run tests matching multiple conditions (AND logic)
taut -m "slow and group=integration"

# Exclude slow tests
taut -m "not slow"

# Complex expressions
taut -m "group=api and not slow"
taut -m "(group=api or group=db) and not slow"
```

#### Marker Expression Syntax

| Expression | Matches |
|------------|---------|
| `slow` | Tests with `@mark(slow=True)` |
| `not slow` | Tests without `slow=True` marker |
| `group=api` | Tests with `@mark(group="api")` or `@mark(group=["api", ...])` |
| `a and b` | Tests matching both `a` AND `b` |
| `a or b` | Tests matching either `a` OR `b` |
| `(a or b) and c` | Parentheses for grouping |

#### Examples

```python
from taut import mark

@mark(slow=True, group="db")
def test_database_migration():
    pass

@mark(group="api")
def test_fast_api():
    pass

@mark(slow=True, group="api") 
def test_slow_api():
    pass
```

```bash
# Run all api tests (fast and slow)
taut -m "group=api"
# Runs: test_fast_api, test_slow_api

# Run only fast api tests
taut -m "group=api and not slow"
# Runs: test_fast_api

# Run anything that's not slow
taut -m "not slow"
# Runs: test_fast_api

# Run db OR api tests
taut -m "group=db or group=api"
# Runs: test_database_migration, test_fast_api, test_slow_api
```

## @parallel

Mark tests as safe to run in parallel:

```python
from taut import parallel

@parallel()
def test_pure_function():
    """This test has no side effects"""
    assert add(1, 2) == 3

@parallel()
def test_isolated_api_call():
    """Uses unique test data"""
    result = api.create_user(f"user_{uuid4()}")
    assert result.ok
```

By default, taut runs tests sequentially for maximum isolation. Tests marked with `@parallel` run concurrently after all sequential tests complete.

```python
class TestAPI:
    @parallel()
    def test_get_users(self):
        pass
    
    @parallel()
    def test_get_posts(self):
        pass
    
    def test_delete_all(self):
        """Not parallel - modifies shared state"""
        pass
```

### Class-Level Parallel

Apply `@parallel` to a class to mark all its methods as parallel-safe:

```python
from taut import parallel

@parallel()
class TestMathFunctions:
    def test_add(self):
        assert add(1, 2) == 3
    
    def test_subtract(self):
        assert subtract(5, 3) == 2
    
    def test_multiply(self):
        assert multiply(4, 3) == 12
```

## Combining Markers

Stack multiple markers on a single test:

```python
from taut import skip, mark, parallel

@mark(slow=True)
@mark(group="integration")
def test_slow_integration():
    pass

@skip("Flaky")
@mark(group="api")
def test_flaky_api():
    pass

@parallel()
@mark(group="unit")
def test_pure_computation():
    pass
```

## Markers Without the Python Package

If you don't want to install the taut Python package, you can define markers inline:

```python
# Define your own skip decorator
def skip(reason=None):
    def decorator(fn):
        fn._taut_skip = True
        fn._taut_skip_reason = reason
        return fn
    if callable(reason):
        fn = reason
        reason = None
        return decorator(fn)
    return decorator

@skip
def test_skipped():
    pass

@skip("Not implemented")  
def test_also_skipped():
    pass
```

taut looks for `_taut_skip`, `_taut_parallel`, and `_taut_markers` attributes on test functions.
