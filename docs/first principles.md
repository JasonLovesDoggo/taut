# Design principles

## Run the work the command promises

A normal run executes the selected tests. Cached results are an explicit opt-in, and reports distinguish executed, skipped, unchanged, and not-run tests. Failed or incomplete execution is never a successful benchmark sample.

## Keep the common path small

Rust collects tests without importing Python. A native command avoids starting Python just to parse CLI flags. Reusable workers amortize interpreter startup and module imports. Dependency tracing is reserved for `--changed` runs.

These choices have boundaries: static collection does not reproduce arbitrary runtime-generated tests, and reusable interpreters retain process state. Make those boundaries visible.

## Concurrency must be understandable

Worker processes provide the default parallelism. Same-loop async overlap is a separate opt-in for independent I/O-bound tests. Serial markers create scheduling barriers. Fresh-process isolation provides a stronger state boundary at an explicit cost.

Do not silently trade correct results for a faster number. Tests still need to await tasks, join threads, restore shared state, and release external resources.

## Make failures actionable

Keep the test ID, source location, exception, traceback, and captured output together. Reject invalid configuration instead of falling back silently. Keep structured output parseable and exit codes useful to scripts.

## Start with a coherent supported surface

Ordinary functions, async tests, test classes, function-scoped fixtures, and explicit cleanup should compose predictably. Unsupported fixture lifetimes and plugin hooks should remain clear limits. Compatibility claims should describe tested behavior, not an invented percentage of another runner's ecosystem.

## Benchmark complete execution

Measure startup, collection, scheduling, test bodies, reporting, and shutdown when comparing user-visible runs. Also measure individual stages, but label them separately. Verify the work completed, record the environment and commands, interleave runner order, and retain raw samples.

Pytest-asyncio and pytest-xdist are useful comparisons, including their combined process-based async execution. A result on one synthetic workload does not establish a universal fastest-runner claim.
