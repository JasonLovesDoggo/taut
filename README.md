# taut

**Tests, without the overhead.**

A Python test runner for a short feedback loop: select a file, run it, and copy the failing test's command to try again. Ordinary functions and `async def` tests use the same command. Native Rust collection and reusable Python workers keep the runner out of the way.

**Preview:** these docs describe [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1), not the older package currently on PyPI. The commands below build that development branch and require Python 3.12+, [uv](https://docs.astral.sh/uv/getting-started/installation/), and a current stable Rust toolchain. The first build takes longer than subsequent runs.

## Try one directory

From an existing uv project's root, replace `tests/unit` with a small test directory:

```sh
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut list tests/unit
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut tests/unit
```

The first command lists tests without importing them. The second runs them with your project's dependencies. `--with` adds Taut for the command without adding it to your dependency declaration or lockfile; uv still performs its normal project sync. Keep your existing pytest command and CI job while you evaluate the results. [First five minutes, pytest migration, and CI](docs/getting-started/quickstart.md).

This is a moving preview branch. For a repeatable run, [pin a commit](docs/getting-started/quickstart.md#try-it-in-ci). Have a wheel or use pip? See [installation](docs/getting-started/installation.md). `uvx` uses an isolated tool environment; it does not install your project's dependencies automatically.

## Keep it when it fits

After a successful trial, record the preview in your development dependencies:

```sh
uv add --dev "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner"
uv run taut
```

You can start with ordinary tests:

```python
# test_example.py
import asyncio


def test_addition():
    assert 2 + 2 == 4


async def test_async():
    await asyncio.sleep(0)
    assert "taut".upper() == "TAUT"
```

No async plugin or test marker required. Taut supports a [defined subset of pytest-style tests](docs/getting-started/quickstart.md#bring-an-existing-pytest-suite); pytest plugins, wider fixture scopes, and dynamic parametrization need adaptation.

## Commands you'll use

```sh
uv run taut                                # Full suite, parallel workers
uv run taut test_example.py::test_async     # One exact test
uv run taut -k 'test_*addition'             # Name substring or glob
uv run taut watch tests                     # Rerun while editing
uv run taut doctor tests                    # Inspect Python, config, and run settings
uv run taut --timeout 10 -x                 # Bound each test; stop after failure
uv run taut --json > results.json           # Structured results and captured output
```

Tests run in parallel by default. Warm workers reuse imported modules and an event loop within a run, so tests should clean up their state. Use `--no-parallel` for a sequential trial, `@mark(serial=True)` for exclusive execution, or process-per-test isolation when a test needs a fresh interpreter. Async concurrency is opt-in. [Execution guide](docs/guide/writing-tests.md#choose-your-execution-model) · [CLI reference](docs/reference/cli.md).

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

## Performance you can check

The [benchmark harness](benches/compare_execution.py) compares complete runs against pytest, pytest-asyncio, and pytest-xdist across minimal, CPU, blocking, threaded, and async workloads. It checks that every expected test body completed exactly once before accepting a timing.

See [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1) for benchmark charts, measured results, and reproduction commands. Generated benchmark results stay local.

Pytest-asyncio works with xdist: async tests can run across worker processes. Taut additionally supports opt-in concurrent async tests inside each worker's event loop. Performance depends on test behavior, worker count, and isolation; collection-only timings do not measure a full test run.

## Development

With a current stable Rust toolchain and Python 3.12+:

```sh
git clone --branch json/fast-runner https://github.com/JasonLovesDoggo/taut
cd taut
uv venv
uv pip install -e .
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
PYTHONPATH="$(pwd)/python" cargo test --locked --all-targets
```

Rebuild with `uv pip install -e .` after Rust or embedded worker changes. See [source development](docs/getting-started/installation.md#build-from-source) for Windows commands and wheel verification. Install [prek](https://github.com/astral-sh/prek) and run `prek install` to enable the formatting hook.

[Preview docs](docs/index.md) · [Published site](https://taut.jsn.cam) · [Design principles](docs/first%20principles.md) · MIT
