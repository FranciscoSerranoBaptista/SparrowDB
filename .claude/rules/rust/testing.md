# Rust Testing Conventions

## Test Placement

- **Unit tests**: `#[cfg(test)] mod tests { ... }` inside the source file, immediately after the code under test. Keep them close to the code they exercise.
- **Integration tests**: `tests/` directory at the crate root. Each `.rs` file compiles as a separate binary with access only to the crate's public API.
- **End-to-end tests**: `tests/hql-tests` harness for full query pipeline validation.

## Async Tests

Always use `#[tokio::test]` for async test functions. Do not manually construct a `tokio::runtime::Runtime` — the attribute handles setup and teardown correctly.

```rust
#[tokio::test]
async fn test_insert_returns_new_node_id() -> Result<(), Box<dyn std::error::Error>> {
    let id = store.insert(node).await?;
    assert!(id.0 > 0);
    Ok(())
}
```

## Parameterised Tests

Use `rstest` with `#[rstest]` and `#[case(input, expected)]` instead of copy-pasting nearly-identical test functions. This keeps test intent clear and reduces boilerplate.

```rust
#[rstest]
#[case("", false)]
#[case("valid_name", true)]
#[case("invalid name!", false)]
fn test_is_valid_identifier(#[case] input: &str, #[case] expected: bool) {
    assert_eq!(is_valid_identifier(input), expected);
}
```

## Property-Based Tests

Use `proptest` for invariant testing — round-trip serialization, commutative operations, and any property that should hold for all inputs. Mark these with the `proptest!` macro.

```rust
proptest! {
    #[test]
    fn test_node_serialization_roundtrip(id in 0u64..u64::MAX) {
        let node = NodeRecord::new(NodeId(id));
        let serialized = node.serialize();
        let deserialized = NodeRecord::deserialize(&serialized)?;
        prop_assert_eq!(node, deserialized);
    }
}
```

## Mocking

Use `mockall` for mocking traits in unit tests. Apply `#[automock]` to the trait definition (or use `mock!` for external traits). Define mocks in the `#[cfg(test)]` test module.

```rust
#[cfg_attr(test, automock)]
trait NodeStore {
    fn get(&self, id: NodeId) -> Result<Option<NodeRecord>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_service_returns_error_on_store_failure() {
        let mut mock = MockNodeStore::new();
        mock.expect_get().returning(|_| Err(StoreError::WriteTxnConflict));
        // ...
    }
}
```

## LMDB Write Serialization

Tests that open LMDB write transactions must use `#[serial]` from the `serial_test` crate to avoid OS-level single-writer deadlocks. LMDB panics if two write transactions are opened simultaneously within the same process. See the running-tests section of CLAUDE.md for the required `--test-threads=1` invocation.

```rust
use serial_test::serial;

#[test]
#[serial]
fn test_write_transaction_commits() { ... }
```

## Test Naming

Test names should read as assertions, describing specifically what the test verifies. Avoid vague names like `test_thing_works`.

```
test_insert_returns_new_node_id        ✓
test_query_with_invalid_filter_returns_error  ✓
test_thing_works                       ✗  (what does "works" mean?)
test_insert                            ✗  (what aspect of insert?)
```

## Assertions — No Silent `.unwrap()`

Do not call `.unwrap()` in test assertions without justification. Prefer the `?` operator in tests returning `Result`, or use `assert!` with a descriptive message:

```rust
// Preferred — propagates errors with context
fn test_something() -> Result<(), Box<dyn std::error::Error>> {
    let result = do_thing()?;
    assert_eq!(result.id, expected_id);
    Ok(())
}

// Acceptable when .unwrap() is justified
let val = config.get("key").unwrap(); // SAFETY: "key" is always present; set in test setup above
```

## Coverage Target

Aim for 80%+ line coverage on new code. Measure with:

```bash
cargo llvm-cov --workspace
# requires: cargo install cargo-llvm-cov
```
