# Configuration

Taut runs without configuration. Add settings to the nearest project's `pyproject.toml` when you need consistent defaults:

```toml
[tool.taut]
max-workers = 4
async-concurrency = 1
isolation = "process-per-run"
timeout = 30
fail-fast = false
```

| Setting | Default | Purpose |
| --- | --- | --- |
| `max-workers` | CPU count | Maximum Python worker processes |
| `async-concurrency` | `1` | Concurrent async tests per worker loop |
| `isolation` | `"process-per-run"` | Reuse workers, or choose `"process-per-test"` |
| `timeout` | None | Positive, finite per-test timeout in seconds |
| `fail-fast` | `false` | Stop scheduling after the first failure |
| `python` | Automatically selected | Interpreter executable or path |

The aliases `max_workers`, `async_concurrency`, and `fail_fast` are also accepted. Unknown settings, invalid TOML, zero worker counts, and invalid timeouts are errors. Taut locates configuration from the first selected path and stops at its nearest `pyproject.toml`, even when that file has no `[tool.taut]` table.

CLI values override corresponding configured values:

```sh
taut -j 2 --timeout 5
```

`--no-parallel` requires `async-concurrency = 1`. Process-per-test isolation also requires async concurrency of one. If your project config enables overlap, add `--async-concurrency 1` when selecting either mode.

## Select Python

```sh
taut --python .venv/bin/python
python -m taut
```

Interpreter selection uses this order:

1. `--python`, or the configured `python` value.
2. `TAUT_PYTHON`.
3. The active `VIRTUAL_ENV` interpreter.
4. A Python executable beside the installed `taut` command.
5. The nearest `.venv` in the current directory or its ancestors.
6. `python3` on PATH, or `python` on Windows.

`python -m taut` supplies its interpreter through `TAUT_PYTHON` when that variable is unset. Explicit interpreter configuration still takes precedence. A relative configuration path such as `.venv/bin/python` is resolved against its `pyproject.toml`; a bare command such as `python3.13` uses PATH. On Windows, a virtual environment's interpreter is `.venv\Scripts\python.exe`.

## Changed runs

A normal `taut` run executes the full selected suite without dependency tracing. `--no-cache` makes that default explicit. Opt into incremental selection with:

```sh
taut --changed
taut --changed
taut watch tests --changed
```

The first run records results and tracked Python dependencies. Later runs may omit previously passing tests when the tracked source snapshot and execution context are unchanged. Failed tests rerun, with known failures scheduled first.

Selection is deliberately conservative: changes to tracked project Python files, selected configuration/lock files, or observed Python dependencies invalidate cached results. Source additions and deletions also invalidate the snapshot. This is not a promise that only the smallest affected test set runs, and it is not function-level change isolation.

The cache does not observe every input. Network services, databases, time, randomness, non-Python data files, native libraries, and changes to an environment's installed packages can affect results without a tracked source change. Use a full `taut` run for CI and after such changes. Configuration and environment digests help invalidate results but do not make external dependencies deterministic.

Skipped-by-marker, unchanged, and not-run-after-failure are reported separately. An unchanged test is a reused prior result, not a newly executed passing test.

```sh
taut cache info
taut cache clear
```

Run cache commands from the same working directory as your test runs. `cache info` prints the actual cache location; caches use the operating system's cache directory and a project-path identity. Clearing the cache causes the next `--changed` run to rebuild its baseline.

## Machine-readable results

```sh
taut --json > results.json
python -m json.tool results.json
taut list tests --json
```

A completed run emits one JSON document with `schema_version`, `summary`, `tests`, and `selected_out`. The summary distinguishes `collected`, `executed`, `passed`, `failed`, `skipped`, `unchanged`, and `not_run`. Test records include their ID, source location, status, duration, captured streams, and error or skip reason.

`--json` cannot be combined with `-v` or `-q`. CLI syntax errors use stderr and exit code 2. With `watch --json`, each run produces a separate JSON document on its own line.

| Exit code | Meaning |
| --- | --- |
| `0` | Selected execution succeeded, or requested command completed |
| `1` | Test failures |
| `2` | Usage, collection, configuration, or runner setup error |
| `5` | No tests collected or matched |
