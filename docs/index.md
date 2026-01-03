# taut

**Fast Python test runner. Tests, without the overhead.**

taut is a high-performance test runner for Python written in Rust. It uses smart dependency tracking to only re-run tests that could have been affected by your changes.

## Features

- **Fast startup** - Sub-100ms startup time, no Python import overhead
- **Smart caching** - Only re-runs tests when their dependencies change
- **Block-level tracking** - Tracks dependencies at function/class level, not file level
- **Zero config** - Works out of the box with no setup required
- **Parallel execution** - Runs tests in parallel with process isolation
- **Async support** - `async def test_*` functions just work

## Quick Example

```python
# test_example.py
def test_addition():
    assert 1 + 1 == 2

def test_subtraction():
    assert 5 - 3 == 2

async def test_async_operation():
    result = await some_async_func()
    assert result == "expected"
```

```bash
$ taut
taut ..
2 passed, in 0.05s

# Change test_addition, only it re-runs:
$ taut
taut .
1 passed, 1 skipped (unchanged), in 0.03s
```

## Why taut?

Traditional test runners re-run all tests when any file changes. taut tracks which code each test actually executes, so it knows exactly which tests need to re-run.

| Feature | taut | pytest |
|---------|------|--------|
| Startup time | ~50ms | ~500ms+ |
| Incremental runs | Function-level | File-level (with plugins) |
| Default isolation | Process per test | Shared process |

## Getting Started

- [Installation](getting-started/installation.md) - Install taut
- [Quick Start](getting-started/quickstart.md) - Run your first tests
