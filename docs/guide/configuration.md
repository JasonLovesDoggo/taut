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
4. The selected project's `.venv` interpreter.
5. A Python executable beside the installed `taut` command.
6. `python3` on PATH, or `python` on Windows.

`python -m taut` supplies its interpreter through `TAUT_PYTHON` when that variable is unset. Explicit interpreter configuration still takes precedence. A relative configuration path such as `.venv/bin/python` is resolved against its `pyproject.toml`; a bare command such as `python3.13` uses PATH. On Windows, a virtual environment's interpreter is `.venv\Scripts\python.exe`.

## Inspect the environment

```sh
uv run taut doctor tests
uv run taut doctor tests --json
```

`doctor [PATH]` reports the project root, configuration path, selected Python executable and version, why that interpreter was selected, and effective run options. It does not import tests or run fixtures. Pass the same path and options as the failing command so you inspect the same selection.

Use `uv run` for a uv project so the project and its dependencies are available. During a temporary source or wheel trial, retain the `--with` argument from [installation](../getting-started/installation.md#try-the-preview-in-your-project). `doctor` diagnoses the environment; it does not install missing packages.

## Troubleshoot a first run

| Symptom | Next step |
| --- | --- |
| `doctor` or a documented option is unknown | Confirm you installed the preview from [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1), not the older PyPI release. |
| `ModuleNotFoundError` for your application or dependency | Run `doctor` with the same path; use `uv run` with the normal test groups/extras, or install dependencies into the Python environment it reports. |
| Wrong Python is selected | Inspect the selection reason in `doctor`; check explicit `--python`, `[tool.taut].python`, `TAUT_PYTHON`, and the active environment in that order. |
| No tests collected | Use `taut list` on the directory, then on an explicit file. Directory discovery recognizes `test_*.py`, `_test*.py`, and `*_test.py`; functions/methods start with `test_` or `_test`. |
| Collection reports a syntax error | Fix the reported file and line. `list` reads Python syntax even though it does not execute it. |
| Unknown fixture, marker, or parameter case | Check the [pytest migration table](../getting-started/quickstart.md#bring-an-existing-pytest-suite); pytest plugins are not loaded. |
| A failure needs more context | Copy the rerun command, keep your environment prefix, and add `-v` for the full traceback. |

Quote node IDs containing brackets, such as `'tests/test_numbers.py::test_parse[negative]'`. `-k` accepts case-insensitive substrings and globs, not pytest boolean expressions. Run `taut --help` for grouped options and `taut <command> --help` for command-specific syntax.

## Watch mode

```sh
taut watch tests
```

Watch runs once immediately, then reruns the same selection when Python files or `pyproject.toml`, `pytest.ini`, `setup.cfg`, or `setup.py` change. It keeps watching after a collection error so you can fix the file and retry. Run flags such as `-k`, `--timeout`, and `--changed` also apply in watch mode.

Watch and changed-test tracking find the nearest project boundary marked by one of those configuration files or `.git`. This includes application sources above a selected `tests` directory. Generated and environment directories are ignored. Taut's own settings still come from `[tool.taut]` in `pyproject.toml`.

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
