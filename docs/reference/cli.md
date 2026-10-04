# Command-Line Help for `taut`

This document contains the help content for the `taut` command-line program.

**Command Overview:**

* [`taut`↴](#taut)
* [`taut list`↴](#taut-list)
* [`taut watch`↴](#taut-watch)
* [`taut doctor`↴](#taut-doctor)
* [`taut cache`↴](#taut-cache)
* [`taut cache info`↴](#taut-cache-info)
* [`taut cache clear`↴](#taut-cache-clear)

## `taut`

Tests, without the overhead.

**Usage:** `taut [OPTIONS] [PATHS]... [COMMAND]`

Common commands:
  taut                             Run tests in the current directory
  taut tests/test_api.py::test_get  Run one exact test ID
  taut -k 'login*'                  Run names matching a substring/glob
  taut list tests                  List test IDs without importing Python
  taut watch tests                 Re-run tests when files change

Exit codes: 0 passed, 1 test failures, 2 usage or configuration error, 5 no tests collected.

###### **Subcommands:**

* `list` — List discovered tests without importing or executing Python
* `watch` — Re-run tests when Python or project configuration files change
* `doctor` — Explain project, Python and execution settings without collecting tests
* `cache` — Manage dependency selection data

###### **Arguments:**

* `<PATHS>` — Test files, directories, or exact file.py::test_name IDs [default: .]

###### **Options:**

* `-k`, `--filter <FILTER>` — Case-insensitive name substring/glob (* and ?); no boolean expressions
* `-v`, `--verbose` — Print individual test names, timings, and full tracebacks
* `-q`, `--quiet` — Print only failures and the final summary
* `--json` — Emit one machine-readable JSON document
* `--no-config` — Ignore [tool.taut] settings while preserving project discovery
* `--no-parallel` — Run tests sequentially
* `-j`, `--jobs <JOBS>` — Number of worker processes [default: CPU count]
* `--changed` — Run only tests affected by tracked dependency changes (enables tracing)
* `--no-cache` — Run all tests without dependency tracing (the default)
* `--isolation <ISOLATION>` — Worker lifetime [default: process-per-run]

  Possible values: `process-per-run`, `process-per-test`

* `--python <EXECUTABLE>` — Python executable or path (use `taut doctor` to inspect selection)
* `--async-concurrency <ASYNC_CONCURRENCY>` — Async tests sharing each worker's event loop [default: 1]
* `--timeout <SECONDS>` — Per-test timeout in seconds
* `-x`, `--fail-fast` — Stop scheduling tests after the first failure



## `taut list`

List discovered tests without importing or executing Python

**Usage:** `taut list [PATHS]...`

###### **Arguments:**

* `<PATHS>`

  Default value: `.`



## `taut watch`

Re-run tests when Python or project configuration files change

**Usage:** `taut watch [PATHS]...`

###### **Arguments:**

* `<PATHS>`

  Default value: `.`



## `taut doctor`

Explain project, Python and execution settings without collecting tests

**Usage:** `taut doctor [PATH]`

###### **Arguments:**

* `<PATH>`

  Default value: `.`



## `taut cache`

Manage dependency selection data

**Usage:** `taut cache <COMMAND>`

###### **Subcommands:**

* `info` — Show dependency selection cache statistics
* `clear` — Remove cached dependency selection data



## `taut cache info`

Show dependency selection cache statistics

**Usage:** `taut cache info`



## `taut cache clear`

Remove cached dependency selection data

**Usage:** `taut cache clear`



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
