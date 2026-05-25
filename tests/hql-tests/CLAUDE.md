# hql-tests CLAUDE.md

Integration test harness for HQL. Tests run against a **live SparrowDB instance** over HTTP.
No internal sparrow-* crate deps — pure external HTTP test client.

---

## Prerequisites

A running SparrowDB instance is required before executing any tests.

```bash
sparrow run          # start a local instance
# or
sparrow push dev && sparrow start dev
```

See `docs/skills/setup.md` for full setup options.

---

## Running the tests

```bash
./run.sh             # run all test files
./run.sh 26          # run a single test file by number
./run.sh branch fixinghql-error-file26   # run all tests on a branch
./run.sh 26 branch fixinghql-error-file26
./run.sh batch 10 1  # batch mode: batch-size start-index

# or directly via cargo:
cargo run --profile dev --bin test -- [options]
```

---

## Writing tests

- Tests POST HQL queries to the running SparrowDB instance and assert on the JSON response.
- HQL syntax reference: `docs/skills/querying.md`
- Follow existing patterns in the `tests/` directory.
- **Always assert specific field values**, not just a non-empty response — `silent-failure-hunter`
  flags weak assertions that silently pass on wrong results.

---

## Agent invocation guide

| Situation | Agent |
|-----------|-------|
| Test harness fails to compile | `rust-build-resolver` |
| Tests pass on wrong results / weak assertions | `silent-failure-hunter` |

Agents live in `.agents/` at the workspace root.

---

## Skills reference

| Need | Skill |
|------|-------|
| HQL query syntax | `docs/skills/querying.md` |
| Spin up a SparrowDB instance | `docs/skills/setup.md` |
| Unexpected or flaky test results | `docs/skills/debugging.md` |

---

## Code graph

| Tool | When to use |
|------|-------------|
| `get_architecture_overview_tool` | Understand the overall test harness structure |
| `get_flow_tool` (entry: `main`) | Trace how tests are dispatched end-to-end |
| `semantic_search_nodes_tool` | Find tests by the HQL feature they exercise |
| `get_minimal_context_tool` | Understand a single test function without reading everything |
