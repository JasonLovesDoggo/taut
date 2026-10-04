# Quick start

Create `test_example.py`:

```python
import asyncio


def test_addition():
    assert 1 + 1 == 2


async def test_async():
    await asyncio.sleep(0)
    assert "hello".upper() == "HELLO"
```

Run it:

```sh
taut test_example.py
```

Both tests execute. No plugin, configuration, or async marker is needed. Plain `taut` discovers and runs the full suite from the current directory; a previous passing run does not suppress execution.

## Find exactly the test you need

```sh
taut list .
taut test_example.py::test_async
taut -k 'test_add*'
taut -v
```

`taut list` parses source without importing tests or running fixtures. Copy a printed node ID to run that exact function or class method. `-k` matches case-insensitive name substrings and globs; quote patterns so the shell does not expand them. It is not a Python or pytest boolean expression.

Directory discovery recognizes `test_*.py`, `_test*.py`, and `*_test.py`. Functions and methods beginning with `test_` or `_test` are collected, including async functions. Plain test classes begin with `Test`; recognized `unittest.TestCase` subclasses use unittest's `test` method-name prefix and are also collected. Explicitly selected Python files do not need a matching filename.

## Choose how much to run at once

```sh
taut -j 4
taut -j 2 --async-concurrency 8
taut --no-parallel
```

`-j` controls worker **processes**. Async concurrency controls how many async tests may share each worker's event loop; it defaults to one. Increase it for independent I/O-bound tests. Blocking calls and CPU-bound work still occupy that worker. See [execution and isolation](../guide/writing-tests.md#choose-your-execution-model) before enabling overlap in a suite that changes global state.

## Keep feedback short

```sh
taut -x --timeout 10
taut -q
taut --json > results.json
taut watch .
```

Fail-fast stops scheduling after a failure; already running tests may finish. A timeout fails the test and can terminate its worker if it does not return. Hard termination cannot run Python cleanup.

`--json` emits structured results with counts, timings, errors, and captured output. `watch` runs once immediately, then reruns on Python or `pyproject.toml` changes. Stop it with Ctrl+C.

[Fixtures and async cleanup](../guide/writing-tests.md#fixtures) · [Project configuration](../guide/configuration.md) · [Every CLI option](../reference/cli.md)
