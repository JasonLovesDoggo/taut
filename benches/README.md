# Benchmarks

The [2026-10-03 report](results/2026-10-03/benchmark-report.md) records the current development implementation against the original runner and pytest, with raw samples, validation, and measurement limits.

Build an optimized binary, then run the harness in an isolated Python environment:

```sh
cargo build --release
uv venv .venv-bench
uv pip install --python .venv-bench/bin/python pytest pytest-asyncio pytest-xdist
.venv-bench/bin/python benches/compare_execution.py \
  --taut target/release/taut --jobs 4 --warmups 2 --repeats 7 \
  --output benchmark-results.json
```

Use the same Python version and pinned dependency versions when comparing runs.
The JSON records the actual interpreter, environment, dependency versions, platform,
CPU count, binary paths, binary hashes, commands, warmups, every measured sample,
and medians with minimum/maximum ranges. A failed sample aborts with a nonzero exit
status and leaves a JSON artifact marked `failed`; it never becomes a timing result.

To compare a previous release, add `--baseline /absolute/path/to/old-taut`. Repeat
`--taut /path/to/binary` to compare multiple candidates in the same session. Binaries
use their default execution mode. Set explicit execution options with, for example,
`--taut-arg=--isolation=process-per-run` or
`--baseline-arg=--isolation=process-per-test`. Extra arguments apply only to execution,
not collection. The baseline's default mode may differ from the candidate's; check
the recorded commands before attributing gains to a particular implementation.

Each execution workload runs taut with one and `--jobs` workers, pytest, and
pytest-xdist with `--jobs` workers. `-j1` means one Python worker, not necessarily
one in-flight async test if the selected binary supports async concurrency.

| Workload | What is measured |
| --- | --- |
| `noop` | Minimal test bodies, runner startup, discovery, dispatch, reporting, and shutdown |
| `cpu` | Pure Python integer arithmetic; `--cpu-iterations` iterations per test |
| `blocking` | Blocking `time.sleep`; `--sleep` seconds per test |
| `threaded` | Each test starts two threads, waits for both sleeps, and joins its executor |
| `async` | `async def` tests awaiting `asyncio.sleep`; pytest uses pytest-asyncio in auto mode |
| `collection` | Actual taut `list` and pytest `--collect-only` CLI processes |

All workloads generate exactly `--tests` tests across at most `--files` modules,
including ordinary functions and class methods. The generated `@parallel` identity
decorator requests parallel execution in taut versions that require opt-in; pytest
executes identical test bodies. Collection is deliberately measured separately:
taut statically parses tests, while pytest imports modules and runs its collection
hooks. These have different capabilities and their timings are not interchangeable
with complete test execution. Pytest-asyncio also works with xdist: its async tests
are distributed across processes, and normally run sequentially within each worker.

Every sample must exit successfully, report exactly the expected test count, and
complete each expected test body exactly once. Completion is recorded after the
await/thread join. A shared in-memory list records IDs, and each Python process
writes an audit file at normal exit. This adds the same small per-test append and
per-process shutdown I/O to all runners; the `noop` figure includes this audit cost.
It catches skipped cached results, missing awaits, duplicate execution, incorrect
collection, and forced worker termination that bypasses clean shutdown. Collection
samples verify exact test IDs and that no test body ran. The harness also checks
that every executing worker used the same Python interpreter and environment.

Taut receives `--no-cache`; pytest's cache provider and unrelated plugin autoload
are disabled. Required async/xdist plugins are loaded explicitly. Each sample is a
fresh CLI process with warm filesystem/import caches after warmup; timings include
startup and shutdown. Runner order is interleaved and shuffled with a recorded seed
for each round. These are **not cold operating-system cache measurements**.

Run benchmarks on an idle machine, using consistent power settings, after builds
finish. Compare the full distribution and workload tradeoffs. Tiny synthetic
workloads do not establish a universal "fastest test runner" claim.

A quick validation run (its timings are not performance evidence):

```sh
.venv-bench/bin/python -m unittest discover -s benches -p 'test_*.py'
.venv-bench/bin/python benches/compare_execution.py \
  --taut target/release/taut --tests 5 --files 3 --jobs 2 \
  --warmups 0 --repeats 1 --sleep .001 --cpu-iterations 100 \
  --output benchmark-smoke.json
```

Select workloads by repeating `--workload`, or use `benches/compare_pytest.py` for
collection only. `--skip-xdist` and `--skip-pytest` allow narrower local experiments;
the selected comparison set is recorded in the artifact.

`cargo bench --bench integration` measures internal collection and execution paths
with Criterion. Its fixtures are built outside the measurement, collection performs
both file discovery and parsing, and every run asserts exact nonzero counts and
successful execution. The `collection_*` names describe warm repeated collection;
`execute_*` excludes discovery and CLI overhead. Compare these numbers only against
other runs of the same internal benchmark, never against pytest CLI wall time.
