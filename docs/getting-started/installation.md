# Installation

Taut requires Python 3.12 or later. Prebuilt wheels install a native `taut` executable and the Python package together. Rust is required only when building from source or installing on a platform without a compatible wheel.

The features documented here are unreleased. Use the [source installation](#build-from-source) below to try the current branch; the package-index commands install the existing published release.

## Install in your project

With an activated virtual environment:

```sh
uv pip install taut
taut --version
```

Or use pip:

```sh
python -m pip install taut
python -m taut --version
```

For a project managed by uv, keep taut in your development dependencies:

```sh
uv add --dev taut
uv run taut
```

To start a new environment first:

```sh
uv venv
source .venv/bin/activate
```

On Windows PowerShell, activate with `.venv\Scripts\Activate.ps1`.

Install taut alongside the dependencies your tests need. `python -m taut` defaults to that Python interpreter unless you explicitly select another; the native command also discovers active and project virtual environments. Override it explicitly with `taut --python /path/to/python` when needed.

## Build from source

These docs describe the development branch. To try changes that have not yet reached PyPI, build the checkout with a current stable Rust toolchain:

```sh
git clone https://github.com/JasonLovesDoggo/taut
cd taut
uv venv
source .venv/bin/activate
uv pip install -e .
taut --version
python -c "from taut import fixture, mark, parallel, skip"
```

Use `.venv\Scripts\Activate.ps1` for the activation step on Windows. Editable installation keeps Python sources available from the checkout; rerun `uv pip install -e .` after changing Rust or the embedded Python worker.

To build a release wheel:

```sh
uvx maturin build --release --locked --out dist
```

Install the resulting `.whl` into a clean environment to test distribution behavior. A source-tree run can hide missing files or a wrong interpreter.

## Run the development checks

On macOS or Linux:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
PYTHONPATH=python cargo test --locked --all-targets
```

On Windows PowerShell:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
$env:PYTHONPATH = (Resolve-Path python).Path
cargo test --locked --all-targets
```

`cargo build --release` also builds `target/release/taut` (`taut.exe` on Windows). When running that binary directly, install the Python package into the selected environment if your tests import taut decorators or fixtures.
