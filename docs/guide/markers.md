# Markers

Import decorators from `taut`. They annotate existing functions and classes; they do not wrap test execution.

## Skip a test

```python
from taut import skip


@skip("Waiting for the next API version")
def test_future_api():
    raise AssertionError("this test is not executed")
```

`@skip`, `@skip("reason")`, and `@skip(reason="reason")` are supported. The result is reported as skipped, with the reason where supplied.

## Run a test exclusively

Tests run in parallel by default. Mark a test that needs exclusive access to shared state:

```python
from taut import mark


@mark(serial=True)
def test_global_migration():
    assert True
```

The scheduler waits for active tests to finish before running a serial test, and schedules no other test until it completes. Apply the marker to a test class to make its test methods serial.

Serial execution does not reset a worker's Python state. Use `--isolation process-per-test` for a fresh interpreter, and clean up shared files or external services explicitly.

## Attach metadata

```python
from taut import mark


@mark(slow=True, group="integration")
def test_service():
    assert True
```

`mark` accepts metadata such as `slow`, `group`, or project-specific values. Metadata alone does not filter tests. There is no `-m` marker expression flag; select by path, node ID, or `-k` name pattern.

## Existing parallel markers

`@parallel` and `@parallel()` remain accepted for existing suites:

```python
from taut import parallel


@parallel
def test_independent():
    assert 1 + 1 == 2
```

They are unnecessary for the default parallel mode. Use `--no-parallel` to run the whole selected suite sequentially, or `@mark(serial=True)` for individual barriers.

## Static collection

`pytest.mark.usefixtures` and `pytest.mark.xfail` are unsupported and raise errors. Request fixtures through test arguments or `@fixture(autouse=True)` instead of `usefixtures`.

Taut reads supported decorator syntax while collecting tests. Prefer direct imports and literal arguments as shown above. Runtime-generated markers, arbitrary decorator wrappers, and pytest plugins are not a general compatibility layer.
