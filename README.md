# taut

**Tests, without the overhead.**

A Python test runner with native Rust collection and a pool of reusable Python workers. Runs your full suite in parallel by default. Ordinary functions, `async def` tests, and tests that start their own threads use the same command.

These docs describe the unreleased development branch. [Build from source](docs/getting-started/installation.md#build-from-source) to try these changes; the published PyPI release does not include them yet.

## Start here

In your project's Python environment (Python 3.12+):

```sh
uv pip install taut
taut
```

Or use `python -m pip install taut` and `python -m taut`. The installed `taut` command is a native executable; importing decorators does not load a native extension. See [installation](docs/getting-started/installation.md) to build the current development version from source.

```python
# test_example.py
import asyncio


def test_addition():
    assert 2 + 2 == 4


async def test_async():
    await asyncio.sleep(0)
    assert "taut".upper() == "TAUT"
```

No async plugin or test marker required.

## Commands you'll use

```sh
taut                                       # Full suite, parallel workers
taut test_example.py::test_async            # One exact test
taut -k 'test_*addition'                    # Name substring or glob
taut list .                                # Collect without importing Python
taut -j 4                                  # Four Python worker processes
taut --async-concurrency 8                  # Up to eight async tests per worker
taut --no-parallel                          # Sequential execution
taut --isolation process-per-test           # Fresh Python process for each test
taut --timeout 10 -x                        # Bound each test; stop after failure
taut --json > results.json                  # Structured results and captured output
taut watch .                               # Rerun when Python/config files change
taut --changed                             # Opt in to dependency-based selection
```

Warm workers reuse imported modules and an event loop within a run. Tests should clean up their state. Use `@mark(serial=True)` for exclusive execution, or process-per-test isolation when a test needs a fresh interpreter. Async concurrency is opt-in: concurrently running tests in one worker share process globals, environment variables, and loop state. [Execution guide](docs/guide/writing-tests.md#choose-your-execution-model).

## Fixtures without ceremony

```python
from taut import fixture


@fixture
def greeting_file(tmp_path):
    path = tmp_path / "greeting.txt"
    path.write_text("hello")
    return path


def test_greeting(greeting_file):
    assert greeting_file.read_text() == "hello"
```

Function-scoped fixtures can depend on other fixtures, use `yield` for cleanup, and be async. Share them in `conftest.py`. Built-ins include `tmp_path` and `monkeypatch`. Use `@parametrize` for literal test cases with individually selectable IDs. [Writing tests](docs/guide/writing-tests.md).

## Measure the work that matters

The [benchmark harness](benches/compare_execution.py) compares complete runs against pytest, pytest-asyncio, and pytest-xdist across minimal, CPU, blocking, threaded, and async workloads. It checks that every expected test body completed exactly once before accepting a timing.

See [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1) for benchmark charts, measured results, and reproduction commands. Generated benchmark results stay local.

Pytest-asyncio works with xdist: async tests can run across worker processes. Taut additionally supports opt-in concurrent async tests inside each worker's event loop. Performance depends on test behavior, worker count, and isolation; collection-only timings do not measure a full test run.

## Development

With a current stable Rust toolchain and Python 3.12+:

```sh
git clone https://github.com/JasonLovesDoggo/taut
cd taut
uv venv
uv pip install -e .
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
PYTHONPATH="$(pwd)/python" cargo test --locked --all-targets
```

Rebuild with `uv pip install -e .` after Rust or embedded worker changes. See [source development](docs/getting-started/installation.md#build-from-source) for Windows commands and wheel verification. Install [prek](https://github.com/astral-sh/prek) and run `prek install` to enable the formatting hook.

[Documentation](https://taut.jsn.cam) · [CLI reference](docs/reference/cli.md) · [Design principles](docs/first%20principles.md) · MIT
