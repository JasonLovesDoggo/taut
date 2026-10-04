# taut

**Tests, without the overhead.**

Taut collects Python tests in Rust, then executes them in reusable Python worker processes. The default command runs the full suite in parallel. Async tests work without plugins.

```sh
uv pip install taut
taut
```

These are development-branch docs. The published release does not include these changes yet; [build from source](getting-started/installation.md#build-from-source) to try them.

Use your project's Python 3.12+ environment. The installed CLI is a native executable, and the Python package provides decorators and `python -m taut`.

## A small command surface

| Task | Command |
| --- | --- |
| Run the suite | `taut` |
| Run a file or exact test | `taut tests/test_api.py::test_get` |
| Filter by name | `taut -k 'test_*login'` |
| Inspect collection without imports | `taut list tests` |
| Choose worker count | `taut -j 4` |
| Overlap async tests in each worker | `taut --async-concurrency 8` |
| Start a fresh process for every test | `taut --isolation process-per-test` |
| Stop scheduling after a failure | `taut -x` |
| Save machine-readable results | `taut --json > results.json` |
| Rerun while editing | `taut watch tests` |

Warm workers avoid repeated interpreter startup and module imports. They also reuse Python state, so tests must clean up after themselves. Same-loop async concurrency is opt-in and intended for independent I/O-bound tests. [Choose an execution model](guide/writing-tests.md#choose-your-execution-model).

The default run does not skip previously passing tests. `--changed` explicitly enables conservative dependency-based selection; it cannot observe every external input. [Understand changed runs](guide/configuration.md#changed-runs).

## Learn by doing

- [Install taut](getting-started/installation.md), including source and editable builds.
- [Run your first suite](getting-started/quickstart.md), select a test, and save results.
- [Write tests and fixtures](guide/writing-tests.md), including async setup and teardown.
- [Use markers](guide/markers.md) for skips and exclusive execution.
- [Configure a project](guide/configuration.md) and choose its Python interpreter.
- [Look up every flag](reference/cli.md).

## Performance you can check

Use the repository's [benchmark harness](https://github.com/JasonLovesDoggo/taut/blob/main/benches/README.md) to compare full execution against pytest, pytest-asyncio, and pytest-xdist on your machine. It measures several workloads and rejects runs with missing, duplicated, or incomplete test bodies. Static collection and full execution are reported separately.

Pytest-asyncio supports xdist's process-based parallelism. Taut's additional async concurrency setting overlaps independent tests within each worker's event loop; it solves a different scheduling problem.
