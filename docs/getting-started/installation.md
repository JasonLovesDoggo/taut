# Installation

Taut requires Python 3.12 or later. These docs describe the unreleased [PR #1](https://github.com/JasonLovesDoggo/taut/pull/1). The source commands below select its `json/fast-runner` branch; installing `taut` from PyPI currently gets an older implementation.

## Try the preview in your project

With [uv](https://docs.astral.sh/uv/getting-started/installation/) and a current stable Rust toolchain installed, run this from your existing uv project's root:

```sh
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut list tests
uv run --with "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner" taut tests
```

Choose a smaller path for a gradual trial. The first invocation builds Taut from source; uv can reuse the build afterward. `--with` supplies Taut for that command without recording it as a project dependency. uv still performs its normal environment and lockfile sync. If your existing lockfile must remain unchanged, add uv's `--locked` option.

Use the same dependency groups and extras you normally use for tests, such as `uv run --group test --with ...`. Install your own package too if it is not already part of your project's normal sync. [`uv run` provides the project environment](https://docs.astral.sh/uv/concepts/projects/run/#requesting-additional-dependencies); [`uvx` uses an isolated tool environment](https://docs.astral.sh/uv/guides/tools/#running-tools) and does not automatically install your application or its dependencies.

The preview branch can change. Use a full commit instead of `json/fast-runner` when you need a repeatable trial. See the [pinned CI example](quickstart.md#try-it-in-ci).

## Keep Taut in the project

Once the trial works, add the preview as a development dependency:

```sh
uv add --dev "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner"
uv run taut
```

This changes `pyproject.toml` and `uv.lock`. Keep pytest alongside it if your tests still import pytest or use plugins in the remaining suite. Remove the trial dependency later with `uv remove --dev taut`.

### Existing pip environment

Activate the environment that contains your application and test dependencies, then install the same preview:

```sh
python -m pip install "taut @ git+https://github.com/JasonLovesDoggo/taut@json/fast-runner"
python -m taut tests
```

`python -m taut` selects that Python interpreter unless an explicit Taut interpreter setting overrides it. The native command also discovers virtual environments. Use [`taut doctor`](../guide/configuration.md#inspect-the-environment) to see which interpreter was selected.

### A local wheel

A compatible wheel needs no Rust compiler. Use the actual path and filename of the wheel you built or downloaded. For example, on Apple Silicon macOS:

```sh
uv run --with /path/to/taut-0.1.0-py3-none-macosx_11_0_arm64.whl taut tests
```

Or install that file into an activated environment with `python -m pip install /path/to/your-wheel.whl`. The native executable and Python helpers ship together. Use a wheel built for your operating system and architecture.

## Published package

The published PyPI package is an older release and does not implement the preview documented here. To install that release deliberately:

```sh
uv add --dev taut
uv run taut --help
```

Use its own help to inspect available commands. Switch to the preview source above before following the rest of these docs.

## Build from source

For changes to Taut itself, clone the preview and create an editable installation:

```sh
git clone --branch json/fast-runner https://github.com/JasonLovesDoggo/taut
cd taut
uv venv
uv pip install -e .
uv run taut --version
uv run python -c "from taut import fixture, mark, parametrize, skip"
```

Editable installation keeps Python sources available from the checkout. Rerun `uv pip install -e .` after changing Rust or the embedded Python worker. If you activate `.venv` instead of using `uv run`, use `source .venv/bin/activate` on macOS/Linux or `.venv\Scripts\Activate.ps1` in Windows PowerShell.

To build a release wheel:

```sh
uvx maturin build --release --locked --out dist
```

Install the resulting wheel into a separate project to verify packaging. A source-tree run can hide missing files or a wrong interpreter.

## Run the development checks

On macOS or Linux:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
PYTHONPATH="$(pwd)/python" cargo test --locked --all-targets
```

On Windows PowerShell:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
$env:PYTHONPATH = (Resolve-Path python).Path
cargo test --locked --all-targets
```

`cargo build --release` also builds `target/release/taut` (`taut.exe` on Windows). When running that binary directly, install the Python package into the selected environment if your tests import Taut decorators or fixtures.
