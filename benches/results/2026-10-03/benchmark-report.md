# Taut performance and validation — 2026-10-03

Local measurements of the unreleased implementation. These synthetic workloads establish measured improvements, not a universal fastest-runner claim or full pytest compatibility.

Measured source: `f89fc5bcf37eddac18639f5f4ad52b943d49b04d` on `json/fast-runner`; original baseline: `84ed1e2`. Subsequent report commits change documentation and saved evidence only.

## Full execution

128 tests across eight files; CPython 3.13.16; one or four worker processes. Medians in milliseconds from seven measured rounds after two warmups. Every sample includes CLI startup, collection, execution, reporting, and shutdown, and verifies each expected test body completed exactly once.

| Workload | Taut, 1 worker | Taut, 4 workers | pytest | pytest-xdist, 4 workers | Original, 1 worker | Original, 4 workers |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| noop | 39.16 | 39.15 | 108.91 | 287.46 | 4518.78 | 1250.17 |
| cpu | 410.13 | 134.76 | 481.74 | 386.09 | 4945.15 | 1364.47 |
| blocking | 1787.50 | 486.90 | 1903.50 | 751.94 | 6346.12 | 1694.71 |
| threaded | 1944.86 | 540.90 | 2161.38 | 804.73 | 6380.60 | 1718.13 |
| async | 1519.05 | 424.21 | 1670.15 | 732.18 | 6210.09 | 1664.10 |

`noop` measures minimal bodies and runner overhead. CPU cases each execute 100,000 arithmetic iterations. Blocking, threaded, and async cases each wait 10 ms; threaded cases start and join two threads per test. Async concurrency is one in this table. The original commit (`84ed1e2`) defaults to a fresh interpreter per test; the new runner defaults to reusable workers. This comparison includes that intentional default-policy change.

## Concurrent async I/O

Same 128 async tests and 10 ms awaits, with `--async-concurrency 16` for Taut. This overlaps independent tests inside each worker's loop. Pytest-asyncio works with xdist; xdist distributes tests across processes and does not add this same-loop overlap.

| Runner | Median, ms |
| --- | ---: |
| taut-j1 | 155.71 |
| taut-j4 | 82.07 |
| pytest | 1725.13 |
| pytest-xdist-j4 | 744.79 |

## Collection and command startup

Collection measures 10,000 tests in one generated file as a complete CLI command. Taut parses statically; pytest imports modules and runs collection hooks, so their capabilities differ. Neither collection timing measures test execution.

| Collection command | Median, ms |
| --- | ---: |
| taut | 42.77 |
| baseline | 1873.59 |
| pytest | 446.56 |

Installed `--help` commands, 31 interleaved samples after five warmups:

- original-installed-command: **13.10 ms**.
- native-installed-command: **3.98 ms**.

## Build provenance

The execution/collection tables measure the release CLI build with SHA-256 `a55b7e1e74b155cb74beceb83cdd1a4f05b593384ff1ecc3fff5762ac9ba537e`. The installed wheel uses a separate release build of the same source, SHA-256 `80b8ada8dfa3184c5936bd9069b05c910a09b32fec139ac3a619a3af08f903f1`; startup measures that installed executable. A final interleaved comparison checks both builds with the same body-completion validation:

| Workload, 4 workers | Measured CLI build, ms | Installed wheel, ms |
| --- | ---: | ---: |
| Minimal bodies | 39.73 | 39.98 |
| Async, concurrency 16 | 82.82 | 84.49 |

## Method and limits

- Platform: `macOS-27.0.1-arm64-arm-64bit-Mach-O`, `arm64`, 12 logical CPUs.
- Commands are fresh processes with warm filesystem/import caches. Runner order is randomized and interleaved; no build jobs run during these measurements. This remains a live desktop, not an isolated laboratory host.
- All runners use the same interpreter/environment. Pytest plugin autoload is disabled; required asyncio/xdist plugins are loaded explicitly. Taut receives `--no-cache`.
- Raw JSON includes commands, binary/source hashes, dependency versions, every sample, exact expected IDs, and min/max ranges. Failed, missing, duplicate, skipped, or incomplete execution rejects the sample.
- The original runner panicked on its cache-directory hash when the first batch reached async tests. The default comparison is an explicit per-workload aggregation: four fully completed workloads from that attempt, plus async/collection from a successful follow-up. The failed attempt and follow-up are preserved; no incomplete samples enter the table.
- The completion audit adds a small per-test append and per-worker exit write to every runner. Startup-only measurements validate successful help output separately.
- Reproduce with `benches/compare_execution.py`; see the benchmark README. Absolute temporary benchmark paths in JSON describe the measured invocation and are recreated by the harness.

## Validation and review

- Rust formatting and strict all-target Clippy pass.
- 368 Rust tests plus all three manual collection checks pass; six internal benchmark smoke checks also verify nonempty, successful execution.
- Installed native wheels pass smoke checks on Python 3.12.15, 3.13.16, and 3.14.8, including both launchers, both isolation modes, fixtures, parameters, failure output, cleanup, and interpreter selection.
- Taut runs its own 38 Python contract cases on all three versions: 37 pass and one optional-pytest case skips in each minimal environment.
- All 28 fixture contracts pass with pytest installed, covering the optional compatibility path. A fresh `uv sync --frozen --no-dev --python 3.13` succeeds, and `uv run --no-sync taut tests/test_parametrize_helper.py --no-cache --json` passes all four cases.
- All 22 documentation cases behave correctly in four modes: 21 pass and one intentional skip.
- Independent standards and requirements reviews found no remaining blockers in their final focused rechecks. Earlier findings produced regression tests for collection omissions, fake skip decorators, optimized assertions, partial parametrization, stale bytecode, fixture boundaries, isolated-loop errors, cache aliases, and process cleanup.
- Windows/Linux CI coverage is configured but was not run locally. Unix cleanup owns the worker process group; detached processes and Windows descendants are not guaranteed terminated. Class fixtures, non-function fixture scopes, pytest plugins/usefixtures/xfail, and dynamic test generation remain documented compatibility limits.

## Raw evidence

- [Default execution](benchmark-default.json)
- [Original interrupted attempt](benchmark-default-attempt.json) and [successful remaining workloads](benchmark-default-completion.json)
- [Concurrent async execution](benchmark-async.json)
- [Large collection](benchmark-collection.json)
- [Installed startup](benchmark-startup.json)
- [Release CLI and installed wheel comparison](benchmark-builds.json)
