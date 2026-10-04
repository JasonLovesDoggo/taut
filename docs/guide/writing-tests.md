# Writing tests

Tests are ordinary Python functions. Use `assert` and include a message when it helps explain a failure:

```python
def test_total():
    actual = sum([1, 2, 3])
    assert actual == 6, f"expected a total of 6, got {actual}"
```

Taut reports the test ID, exception, traceback, and captured output on failure. It uses normal Python assertions; it does not rewrite them like pytest.

## Choose your execution model

| Need | Setting | Behavior |
| --- | --- | --- |
| General suite | `taut` | Reusable Python processes; one active test per worker by default |
| Control CPU or service load | `taut -j 4` | At most four worker processes |
| Overlap independent async I/O | `taut -j 2 --async-concurrency 8` | Up to eight async tests on each worker's event loop |
| Run sequentially | `taut --no-parallel` | One test at a time in a reusable worker |
| Reset the interpreter per test | `taut --isolation process-per-test` | A fresh Python process for every test |
| Protect a shared resource | `@mark(serial=True)` | An exclusive scheduling barrier for that test |

Workers live for one invocation. Imported modules and process globals survive between tests assigned to the same warm worker. `--no-parallel` controls scheduling; it does not reset state. Fresh process isolation costs more startup and imports, but is useful for tests that cannot restore interpreter state.

Process isolation does not isolate files, databases, network services, or other external resources. Use unique test resources, cleanup, and serial execution where appropriate.

## Async tests

An `async def` test is awaited automatically:

```python
import asyncio


async def test_async_result():
    async def calculate():
        await asyncio.sleep(0)
        return 42

    assert await calculate() == 42
```

No decorator or plugin is needed. Each worker reuses an event loop. By default, async tests run one at a time in each worker while different processes can run in parallel.

`--async-concurrency N` opts into overlapping async tests on the same loop. This helps when independent tests spend time awaiting I/O. It does not accelerate a CPU loop or a blocking `time.sleep()` inside async code; those block the worker. Move blocking operations to `asyncio.to_thread()` when that fits the code under test, or use additional worker processes.

Concurrent tests share process globals, environment variables, and event-loop configuration. Do not mutate those while another test uses them. Use `@mark(serial=True)` or leave async concurrency at one. Process-per-test isolation and `--no-parallel` reject an async concurrency setting greater than one.

Tests should await their work and close resources. Taut cancels unfinished tasks owned by a completed async test, but that is not a replacement for deterministic cleanup. Python-level output follows async task context. Native writes and output from threads without a test context cannot always be assigned to one overlapping test; the result that collects them marks them as shared worker output.

Pytest-asyncio can also run under pytest-xdist. Xdist distributes tests across processes; Taut's opt-in same-loop overlap is a separate capability.

## Tests that use threads

A test may start and join its own threads as normal:

```python
from concurrent.futures import ThreadPoolExecutor


def test_threaded_work():
    with ThreadPoolExecutor(max_workers=2) as executor:
        assert list(executor.map(abs, [-1, -2])) == [1, 2]
```

Taut schedules the test in a worker process. It does not impose a separate thread-based test scheduler. Join threads before returning and retrieve future results so worker-thread exceptions propagate to the test. A raw thread's exception is not automatically an exception in the thread that joined it.

## Fixtures

A fixture supplies a named test argument. Fixtures are function-scoped: each test gets its own resolved values, and a shared dependency is created only once for that test.

```python
from taut import fixture


@fixture
def numbers():
    return [1, 2, 3]


@fixture
def total(numbers):
    return sum(numbers)


def test_total(total):
    assert total == 6
```

Put shared fixtures in `conftest.py`. Taut loads applicable files from the project root down to the test directory, then test-module fixtures. A nearer definition overrides an outer one. Fixture loading happens during execution, not `taut list`.

```python
# tests/conftest.py
from taut import fixture


@fixture
def account():
    return {"name": "Ada", "active": True}
```

```python
# tests/test_account.py
def test_account_is_active(account):
    assert account["active"] is True
```

### Cleanup and async fixtures

Use `yield` to release a resource after the test. Dependencies are cleaned up in reverse setup order, including after an ordinary test failure. Already-created dependencies are cleaned up if later setup fails.

```python
from taut import fixture


@fixture
def opened_file(tmp_path):
    with (tmp_path / "message.txt").open("w+") as file:
        yield file


def test_write(opened_file):
    opened_file.write("hello")
    opened_file.seek(0)
    assert opened_file.read() == "hello"
```

Async functions and async generators are also supported. Async fixture setup, the test, and cleanup use the worker's event loop:

```python
import asyncio
from taut import fixture


@fixture
async def messages():
    await asyncio.sleep(0)
    yield ["ready"]
    await asyncio.sleep(0)


async def test_messages(messages):
    assert messages == ["ready"]
```

Use `@fixture(autouse=True)` to run a fixture for every test in its applicable scope without declaring an argument. Use `@fixture(name="account")` to expose a different argument name. Wider scopes and parametrized fixtures are not supported; taut rejects them rather than silently changing their lifetime.

### Built-in fixtures

`tmp_path` is a unique `pathlib.Path` directory, removed after the test:

```python
def test_file(tmp_path):
    path = tmp_path / "result.txt"
    path.write_text("done")
    assert path.read_text() == "done"
```

`monkeypatch` restores changes after the test. It supports `setattr`, `delattr`, `setitem`, `delitem`, `setenv`, `delenv`, `chdir`, `syspath_prepend`, and `undo`:

```python
import os
from taut import mark


@mark(serial=True)
def test_environment(monkeypatch):
    monkeypatch.setenv("APP_MODE", "test")
    assert os.environ["APP_MODE"] == "test"
```

Restore actions run in reverse order. Use `setattr(object, "attribute", value)`; the dotted-string shorthand from pytest is not part of this API.

## Parameterized tests

Declare literal cases with `parametrize`. Each case is collected, scheduled, and reported independently:

```python
# test_numbers.py
from taut import parametrize


@parametrize("text, expected", [("2", 2), ("-3", -3)], ids=["positive", "negative"])
def test_parse(text, expected):
    assert int(text) == expected
```

```sh
taut list test_numbers.py
taut 'test_numbers.py::test_parse[negative]'
```

Quote node IDs containing brackets so your shell preserves them. Select the unqualified function ID to run all its cases. Stacked decorators produce a Cartesian product, and class-level cases combine with method-level cases. A parameter supplies the matching argument; other arguments can still request fixtures.

Static collection accepts literal strings, booleans, `None`, finite numbers, lists, and dictionaries with string keys. Integer values must fit a signed or unsigned 64-bit range. Tuple containers and tuple rows are valid, but tuple-valued arguments are rejected rather than converted to a different Python type.

Dynamic expressions, generated case lists, indirect parametrization, and empty case sets are collection errors. Put literal values directly in the decorator. `ids` can supply readable labels; use `taut list` to copy the exact generated IDs.

## Classes and lifecycle hooks

Group methods in a class whose name begins with `Test`. A new instance is created for each test. Plain test classes can use `setUp`/`tearDown` and `asyncSetUp`/`asyncTearDown`; cleanup follows successful setup stages.

```python
class TestCounter:
    def setUp(self):
        self.values = []

    def tearDown(self):
        self.values.clear()

    def test_append(self):
        self.values.append(1)
        assert self.values == [1]
```

Plain test classes can inherit test methods from bases in the same module. `unittest.TestCase` and `unittest.IsolatedAsyncioTestCase` subclasses are also recognized, even when their class names do not start with `Test`:

```python
import unittest


class ArithmeticCase(unittest.TestCase):
    def test_sum(self):
        self.assertEqual(1 + 2, 3)
```

Other imported or dynamically constructed base classes cannot be resolved without importing them, so collection rejects them. Define test classes and functions directly in the module instead of inside conditional statements.

Use fixtures for reusable dependencies. Taut supports a defined subset of pytest-style tests; it does not load pytest's plugin or hook system. Dynamically generated tests and unsupported fixture lifetimes need adaptation.

## Timeouts and failures

```sh
taut --timeout 5
taut -x
```

The timeout is expressed in seconds. Async work can be cancelled cooperatively; a worker that does not respond can be terminated. Cleanup cannot be guaranteed after hard termination, so external resources still need a recovery strategy.

`-x` stops scheduling after the first observed failure. Tests already running may complete. With fail-fast enabled, the runner dispatches one test at a time to each worker; same-loop async overlap is consequently reduced.

## Imports and output

Install your project and its dependencies into the chosen Python environment, for example with `uv pip install -e .`. Package test modules can use package-relative imports. Plain test modules are loaded by file identity, so matching basenames in separate directories do not need to collide.

Standard output and standard error are captured. Failure reports include captured output; `--json` retains it as `stdout` and `stderr` fields without mixing it into the JSON stream. Python output is bounded and marked when truncated.
