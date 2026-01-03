# Command-Line Help for `taut`

This document contains the help content for the `taut` command-line program.

**Command Overview:**

* [`taut`↴](#taut)
* [`taut list`↴](#taut-list)
* [`taut watch`↴](#taut-watch)
* [`taut cache`↴](#taut-cache)
* [`taut cache info`↴](#taut-cache-info)
* [`taut cache clear`↴](#taut-cache-clear)

## `taut`

Tests, without the overhead.

**Usage:** `taut [OPTIONS] [PATHS]... [COMMAND]`

###### **Subcommands:**

* `list` — List discovered tests without running them
* `watch` — Watch for changes and re-run affected tests
* `cache` — Cache management commands

###### **Arguments:**

* `<PATHS>` — Path(s) to test files or directories

  Default value: `.`

###### **Options:**

* `-k`, `--filter <FILTER>` — Filter tests by name substring
* `-v`, `--verbose` — Verbose output
* `--no-parallel` — Disable parallel execution
* `-j`, `--jobs <JOBS>` — Number of parallel jobs (default: CPU count)
* `--no-cache` — Disable dependency caching (run all tests)
* `--isolation <ISOLATION>` — Execution isolation mode

  Default value: `process-per-test`



## `taut list`

List discovered tests without running them

**Usage:** `taut list [OPTIONS] [PATHS]...`

###### **Arguments:**

* `<PATHS>` — Path(s) to test files or directories

  Default value: `.`

###### **Options:**

* `-k`, `--filter <FILTER>` — Filter tests by name substring



## `taut watch`

Watch for changes and re-run affected tests

**Usage:** `taut watch [OPTIONS] [PATHS]...`

###### **Arguments:**

* `<PATHS>` — Path(s) to test files or directories

  Default value: `.`

###### **Options:**

* `-k`, `--filter <FILTER>` — Filter tests by name substring
* `-v`, `--verbose` — Verbose output
* `-j`, `--jobs <JOBS>` — Number of parallel jobs (default: CPU count)
* `--isolation <ISOLATION>` — Execution isolation mode

  Default value: `process-per-test`
* `--no-cache` — Disable dependency caching (run all tests)



## `taut cache`

Cache management commands

**Usage:** `taut cache <COMMAND>`

###### **Subcommands:**

* `info` — Show cache statistics
* `clear` — Clear all cached data



## `taut cache info`

Show cache statistics

**Usage:** `taut cache info`



## `taut cache clear`

Clear all cached data

**Usage:** `taut cache clear`



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
