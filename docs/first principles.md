# Guiding Principles

## Core Axioms

1. **Determinism** — Same inputs, same result. Always. Flaky tests are bugs in the framework.

2. **Independence** — Tests don't share mutable state. Order doesn't matter. Parallel by default.

3. **Speed** — Collection is O(n), not O(n²). Import nothing during discovery. Parallelize execution.

4. **Locality** — Failures point to the problem. No hunting. Assertion output is a complete diagnostic.

5. **Composability** — Fixtures compose. Markers compose. Plugins extend without modifying core.

## Design Constraints

- Static collection (AST) for speed, dynamic fallback for compatibility
- Rust orchestrates, Python executes
- Work-stealing over static partitioning
- Fixture scopes: function < class < module < session (only depend on equal or broader)
- Teardown is reverse setup order

## What We're Fixing

| pytest                        | us                        |
|-------------------------------|---------------------------|
| imports everything to collect | parse AST, import nothing |
| single-threaded default       | parallel default          |
| xdist bolted on               | native worker pool        |
| noisy output                  | quiet unless failure      |

## Non-Goals

- 100% pytest compatibility (aim for 95% of common usage)
- Plugin system as powerful as pytest's (too much rope)
- Supporting every edge case over being fast
