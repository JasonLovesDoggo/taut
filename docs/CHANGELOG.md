# Changelog

## Unreleased

### Execution

- Run the full selected suite by default, using reusable Python worker processes and parallel scheduling.
- Add explicit same-loop async concurrency, per-test timeouts, fail-fast scheduling, and serial markers.
- Await async tests without a plugin; support tests that start their own threads.
- Keep fresh process-per-test isolation available for tests that need a new interpreter.
- Capture Python and native output separately from the worker protocol, attribute async task output, and report setup, execution, and cleanup failures.
- Select the test interpreter from CLI/configuration, environment, installed executable location, or a project virtual environment.

### Test authoring and collection

- Add function-scoped fixtures, fixture dependencies, `conftest.py`, autouse fixtures, sync/async `yield` cleanup, `tmp_path`, and `monkeypatch`.
- Collect functions and class methods from `test_*.py`, `_test*.py`, and `*_test.py`; accept explicit Python files and exact node IDs.
- Deduplicate overlapping collection roots and skip common dependency/build directories during directory discovery.
- Keep `taut list` free of test-module imports.

### CLI and configuration

- Add quiet output, versioned JSON reports, explicit exit codes, and strict configuration validation.
- Run once when watch starts, then rerun on Python or project configuration changes; keep watching after collection errors.
- Make dependency selection an explicit `--changed` mode with conservative tracked-source invalidation. Plain `taut` and `--no-cache` perform full runs without dependency tracing.
- Distinguish intentionally skipped, unchanged, executed, and not-run tests in reports.

### Packaging and verification

- Ship a native CLI and pure Python package in platform wheels, without a PyO3 command trampoline.
- Support `python -m taut` and Python 3.12–3.14.
- Add installed-wheel checks and runtime regression coverage.
- Add reproducible comparisons against pytest, pytest-asyncio, and pytest-xdist. Benchmark samples must verify exact test execution before their timings are accepted.
