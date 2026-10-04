# First five minutes

Start in a uv project with Python 3.12+, uv, and a current stable Rust toolchain. These commands build the unreleased [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1); the first build may take a few minutes. [Other installation options](installation.md).

## Collect, run, rerun

Choose one existing directory, such as `tests/unit`, and inspect the selection:

```sh
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut list tests/unit
```

`list` parses the files without importing tests or running fixtures. Copy a printed test ID to run that exact test:

```sh
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut tests/unit/test_example.py::test_addition
```

Then run the directory:

```sh
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut tests/unit
```

Substitute a real ID from your own collection. No Taut dependency is added to `pyproject.toml` or `uv.lock`; uv may create or sync the project environment and lockfile as usual. Keep the `--with` argument on each trial command. After [adding Taut to your development dependencies](installation.md#keep-taut-in-the-project), the commands shorten to `uv run taut`.

### Need a first test?

Create `test_example.py` in your project and select that file instead:

```python
import asyncio


def test_addition():
    assert 1 + 1 == 2


async def test_async():
    await asyncio.sleep(0)
    assert "hello".upper() == "HELLO"
```

Both tests execute without plugins or async markers. A normal run executes the full selected suite every time.

## Bring an existing pytest suite

Keep pytest installed during the trial. Imports such as `import pytest` still need it, even when Taut runs the test. Keep pytest's existing CI job, choose one directory, and compare collected IDs, case counts, skips, and outcomes before expanding the trial.

| Status | Existing tests | What to expect |
| --- | --- | --- |
| Supported | Ordinary `test_*` functions, plain test classes, async functions | Async tests are awaited automatically; no plugin is required. |
| Supported | Module-level `pytest.fixture` and `conftest.py` fixtures | Function scope, dependencies, `autouse`, `name`, `yield` cleanup, and async setup/cleanup work. |
| Supported subset | `pytest.mark.parametrize` decorators | Use literal case data and IDs. Stacked decorators and class/method cases are supported. [Value limits](../guide/writing-tests.md#parameterized-tests). |
| Supported | `pytest.mark.skip`, `skipif`, and `pytest.skip()` | Recognized as skips. Keep pytest installed for these imports. |
| Different | Assertions and failure output | Normal Python assertions; no pytest assertion rewriting. Add useful assertion messages. |
| Different | `-k`, markers, and configuration | `-k` matches substrings/globs, not boolean expressions. No `-m`. Defaults come from `[tool.taut]`, not pytest options. |
| Different | Fixture built-ins | `tmp_path` and a subset of `monkeypatch` are provided. `capsys`, `capfd`, `caplog`, `request`, and `tmp_path_factory` are not. |
| Different | Scheduling | Parallel by default, with reusable workers. Start with `--no-parallel` if your suite assumes sequential execution. |
| Unsupported | Session/module/class-scoped, parametrized, or class-defined fixtures | Adapt only the trial directory to function-scoped fixtures, or keep it on pytest. |
| Unsupported | Dynamic cases, `pytest.param`, indirect parametrization | Static collection needs supported literal values; generated cases need adaptation. |
| Unsupported | Pytest plugins/hooks, `xfail`, `usefixtures`, module/class setup hooks | Plugin-provided fixtures and behavior do not carry over. Keep those tests on pytest until adapted. |

Taut is not a drop-in replacement for every pytest suite. A successful `list` checks collection; execution is still needed to validate imports, fixtures, and cleanup. See [writing tests](../guide/writing-tests.md) for exact lifecycle and class support.

When the directory passes under both runners, add Taut as a development dependency and expand one directory at a time. Leave plugin-dependent tests on pytest for as long as you need.

## Stay in the feedback loop

The examples below assume Taut is installed in the project. During a temporary trial, retain the `--with` argument above.

```sh
uv run taut watch tests/unit
uv run taut tests/unit -k 'test_add*'
uv run taut tests/unit -x --timeout 10
uv run taut doctor tests/unit
```

`watch` runs immediately and reruns after Python or project configuration changes. Stop it with Ctrl+C. `-x` stops scheduling after a failure; tests already running may finish. `--timeout` is per test, in seconds, and hard termination cannot run Python cleanup.

Failure reports show the failing test and a command to rerun it. Run that command through the same `uv run` or trial prefix so it keeps your environment. The command uses `--no-config` and preserves effective execution settings so a nested configuration cannot change the rerun. Insert `-v` before `--` for the full traceback. A collection syntax error points to the file and line to fix. [Environment and configuration troubleshooting](../guide/configuration.md#troubleshoot-a-first-run).

## Try it in CI

Add a separate trial job while keeping your current test job. This example assumes a checked-in, current `uv.lock` and uses the immutable preview commit `77cd681acffcbd0d8e88d704d4d44f8aae5fa5d7`, including environment diagnostics and exact failure reruns. Replace the pin only after reviewing a newer commit; there is no published release with this feature set yet.

```yaml
name: Taut trial
on: [push, pull_request]
jobs:
  taut:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: astral-sh/setup-uv@v6
        with:
          version: "0.12.23"
          python-version: "3.13"
      - uses: dtolnay/rust-toolchain@stable
      - name: Run the trial directory
        run: >-
          uv run --locked
          --with "taut @ git+https://github.com/JasonLovesDoggo/taut@77cd681acffcbd0d8e88d704d4d44f8aae5fa5d7"
          taut tests/unit --json > taut-results.json
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: taut-results
          path: taut-results.json
```

Select your actual trial path and add your test dependency group or extras to `uv run` if needed. `--locked` verifies the existing project lockfile without updating it; the source-built Taut version is pinned separately. JSON output retains test outcomes and captured output, and test failures still exit nonzero. Use full runs in CI; `--changed` is an opt-in local workflow.

[Fixtures and async cleanup](../guide/writing-tests.md#fixtures) · [Execution and isolation](../guide/writing-tests.md#choose-your-execution-model) · [Every CLI option](../reference/cli.md)
