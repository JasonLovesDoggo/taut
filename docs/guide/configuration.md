# Configuration

taut works with zero configuration, but you can customize behavior via `pyproject.toml`.

## pyproject.toml

Add a `[tool.taut]` section to your `pyproject.toml`:

```toml
[tool.taut]
max_workers = 4
```

### Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `max_workers` | integer | CPU count | Maximum number of parallel worker processes |

## CLI Overrides

CLI options take precedence over `pyproject.toml` settings:

```bash
# Uses 2 workers regardless of pyproject.toml
taut -j 2
```

## Example Configuration

```toml
# pyproject.toml

[project]
name = "myproject"
version = "1.0.0"

[tool.taut]
max_workers = 8
```

## Environment Variables

Currently, taut does not use environment variables for configuration. All configuration is done via `pyproject.toml` or CLI options.

## Cache Location

taut stores its cache in your system's cache directory:

- **macOS**: `~/Library/Caches/taut/<project-hash>/`
- **Linux**: `~/.cache/taut/<project-hash>/`

Each project gets its own cache directory based on a hash of its absolute path.

### Viewing Cache Info

```bash
taut cache info
```

Output:

```
Cache location: /Users/you/Library/Caches/taut/a1b2c3d4
Cache exists: true
Total size: 12.5 KB (3 files)

Dependency database:
  156 blocks tracked
  42 tests tracked
  40 passed, 2 failed
```

### Clearing Cache

```bash
taut cache clear
```

This removes all cached data, forcing all tests to re-run on the next invocation.
