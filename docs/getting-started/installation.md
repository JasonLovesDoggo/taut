# Installation

## From PyPI (Recommended)

Install taut with pip: (WIP)

```bash 
pip install taut
```

This gives you both:
- The `taut` command-line tool
- The Python decorators (`@skip`, `@mark`, `@parallel`)

Verify the installation:

```bash
taut --version
```

## From Source

If you want to build from source, you'll need Rust and maturin.

### Prerequisites

Install Rust using [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Install maturin:

```bash
uv tool install maturin 
```

### Build and Install

Clone and build:

```bash
git clone https://github.com/JasonLovesDoggo/taut
cd taut
maturin develop --release
```

This builds the Rust code and installs taut in your current Python environment.

### Development Mode

For development:

```bash
maturin develop
```

## Verify Installation

```bash
# Check CLI
taut --version

# Check Python imports
python -c "from taut import skip, mark, parallel; print('OK')"
```

## System Requirements

- **Python**: 3.12 or later

## Standalone Binary

If you only need the CLI (no Python decorators), you can build a standalone binary:

```bash
git clone https://github.com/JasonLovesDoggo/taut
cd taut
cargo build --release
```

The binary will be at `target/release/taut`. Add it to your PATH:

```bash
# Option 1: Add to PATH
export PATH="$PATH:$(pwd)/target/release"

# Option 2: Copy to /usr/local/bin
sudo cp target/release/taut /usr/local/bin/
```
