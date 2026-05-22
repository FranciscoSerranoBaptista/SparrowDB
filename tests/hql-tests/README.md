# hql-tests

End-to-end HQL test harness. Compiles and runs each HQL test fixture through the SparrowDB compiler and server, then asserts expected results. Can optionally open GitHub issues automatically when compilation or cargo-check failures are found.

## Run

```bash
# Run all test fixtures in parallel
cargo run -p hql-tests --bin test

# Run a single fixture by number
cargo run -p hql-tests --bin test -- 42

# Using the shell helper
./run.sh
./test.sh
```

## GitHub issue integration (optional)

Set the following environment variables to enable automatic issue creation for failures:

```bash
export GITHUB_TOKEN="ghp_..."
export GITHUB_OWNER="YOUR_ORG"   # defaults to "HelixDB" if unset
export GITHUB_REPO="SparrowDB"   # defaults to "helix-db" if unset
```

Issue creation is idempotent: a SHA-2 hash of the error message is checked against open issues before creating a new one, so duplicate issues are not filed.

## Test fixtures

Fixtures live under `tests/` — each subdirectory is a self-contained HQL scenario (schema + queries). There are 100+ fixtures covering node/edge operations, vector search, BM25, graph traversal, aggregations, conditionals, path algorithms, MCP macros, and more.

## Dependencies

- `sparrow` (or `helix`) CLI tool must be installed and in `PATH`
- `cargo` for the cargo-check step
- Internet access only when GitHub issue creation is enabled
