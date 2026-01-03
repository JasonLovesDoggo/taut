# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
