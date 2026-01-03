# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Added a markers system (`src/markers.rs`) that parses Python decorators such as `@skip`, `@mark`, and `@parallel`.
- Added Python decorators in `python/taut/__init__.py` so test authors can use `skip`, `mark`, and `parallel` directly.
- Added `@skip` support for unconditional skips as well as explicit reasons via both positional and keyword arguments.
- Added the `@mark` decorator for attaching metadata like `slow=True` or `group="api"` to tests.
- Added an opt-in `@parallel` decorator so sequential tests run first and parallel-safe tests run concurrently afterward.
- Added glob-pattern test filtering in `src/filter.rs`, powering the `-k` flag with support for `test_*`, `file.py::test_name`, and `TestClass/test_method`.
- Added configuration loading in `src/config.rs` to read `max_workers` from the `[tool.taut]` table in `pyproject.toml`.
- Added fail-first sorting in `src/selection.rs` so tests that failed last run execute before the rest for faster feedback.
- `docs/ROADMAP.md` defining TAUT's async-first, minimal-authoring roadmap.
- Initialized this changelog in Keep a Changelog 1.1.0 format.
- Expanded discovery rules: test files can be `test_*.py` or `_test*.py`, and test callables can be `test_*` or `_test*`.
- First-class async support for `async def` tests.
- `--isolation` option with `process-per-test` (default) and `process-per-run`.
- Captured test output fields (`stdout`/`stderr`) in `TestResult`.
- `sys.monitoring` coverage collection in `process-per-run` mode (with `sys.settrace` fallback).
- Rust integration tests for discovery rules (`tests/discovery_rules.rs`).
- Warm worker pool (`src/worker_pool.rs`) for `process-per-run` mode: N long-lived Python workers with JSON-over-stdio protocol, crash recovery, and parallel test dispatch. ~23% faster than `process-per-test` on typical workloads.
- `-j`/`--jobs` flag now applies to worker pool in `process-per-run` mode.
- `taut list` command: show discovered tests without running them.
- `taut watch` command: watch for file changes and re-run affected tests automatically.
- `TestItem::id()` method for consistent test identification (e.g., `path/to/test.py::ClassName::test_method`).

### Fixed
- Flaky integration test `incremental_run_reruns_changed_tests` caused by Python's `__pycache__` bytecode caching.
