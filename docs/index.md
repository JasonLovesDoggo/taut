# taut

**Tests, without the overhead.**

Run a file, fix a failure, and rerun the exact test. Taut runs ordinary and async Python tests with the same command, collects without importing your test modules, and runs the full suite in parallel by default.

These docs describe the unreleased [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1). The older PyPI release does not include this behavior. Trying the preview requires Python 3.12+, [uv](https://docs.astral.sh/uv/getting-started/installation/), and a current stable Rust toolchain for the first source build.

## Start with one directory

From your existing uv project's root, choose a small directory:

```sh
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut list tests/unit
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut tests/unit
```

`list` shows what will run without importing tests or fixtures. `uv run --with` makes Taut available alongside your project's dependencies without adding it as a project dependency. uv may create or sync the project environment and lockfile as usual. Keep pytest installed and keep the existing CI job during your trial.

Follow the [first five minutes](getting-started/quickstart.md) for a complete example, the [pytest migration table](getting-started/quickstart.md#bring-an-existing-pytest-suite) for compatibility limits, or [installation](getting-started/installation.md) for pip and local wheels.

## A short feedback loop

Once Taut is in your development dependencies, use `uv run taut`:

| Task | Command |
| --- | --- |
| Run a file or exact test | `uv run taut tests/test_api.py::test_get` |
| Filter names | `uv run taut -k 'test_*login'` |
| Inspect the selected environment | `uv run taut doctor tests` |
| Rerun while editing | `uv run taut watch tests` |
| Stop scheduling after a failure | `uv run taut -x` |
| Save structured results | `uv run taut --json > results.json` |

Failure reports include the test's location, captured output, and a rerun command. Use `-v` for the full traceback when you need more context. [Troubleshoot a first run](guide/configuration.md#troubleshoot-a-first-run).

## Know what runs

The default command executes the full selected suite. It does not skip previously passing tests. Reusable workers keep imported modules and Python state within a run; clean up resources and shared state between tests. [Execution and isolation](guide/writing-tests.md#choose-your-execution-model).

Function-scoped fixtures, `conftest.py`, literal parameter cases, and async setup and cleanup are supported. Taut does not load pytest's plugin or hook system. [Writing tests](guide/writing-tests.md) · [Markers](guide/markers.md) · [Configuration](guide/configuration.md) · [CLI reference](reference/cli.md).

## Performance you can check

Use the repository's [benchmark harness](https://github.com/JasonLovesDoggo/taut/blob/json/fast-runner/benches/compare_execution.py) to compare complete executions against pytest, pytest-asyncio, and pytest-xdist. It rejects missing, duplicated, or incomplete test bodies. Run it against workloads that resemble your suite; collection speed alone does not tell you how long tests take. [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1) contains measured results and reproduction commands. Generated results stay local.

Pytest-asyncio supports xdist's process-based parallelism. Taut's additional async concurrency setting overlaps independent tests within each worker's event loop; it solves a different scheduling problem.
