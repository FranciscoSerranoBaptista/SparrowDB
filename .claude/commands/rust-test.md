---
description: Test-driven development (TDD) red/green/refactor cycle for Rust
allowed-tools: Bash, Read, Edit
argument-hint: "[function name or module to test]"
---

# Rust Test-Driven Development

Follow the red/green/refactor cycle when implementing new functions, fixing bugs, or building features.

## Phase 1: RED — Write a failing test first

Before writing any implementation code, write a test that describes the desired behavior.

### Test location

- **Unit tests**: Use `#[cfg(test)]` modules inside the implementation file (right after the code you're testing)
- **Integration tests**: Use `tests/` directory for multi-crate workflows or end-to-end scenarios
- **Async tests**: Use `#[tokio::test]` macro for async test functions

### Test structure

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_your_function_does_something() {
        let input = ...;
        let result = your_function(input);
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn test_async_behavior() {
        let result = async_function().await;
        assert!(result.is_ok());
    }
}
```

### Feature flags for tests

Tests that touch LMDB or the HTTP gateway need explicit feature flags:

```bash
cargo test --workspace --features lmdb,server
```

Tests that touch the graph or storage engine require `lmdb`. Tests that use HTTP routes require `server`. Both are cumulative.

### Run your failing test

```bash
cargo test --package <crate-name> --features lmdb,server -- --nocapture <test-name>
```

Expect it to fail. This is RED.

## Phase 2: GREEN — Write minimal code to pass

Write the smallest amount of code to make the test pass. Don't optimize yet; focus on correctness.

```bash
cargo test --workspace --features lmdb,server
```

All tests (existing + new) must pass. This is GREEN.

## Phase 3: REFACTOR — Clean up while staying green

Now improve the code: extract functions, reduce duplication, clarify intent. Run tests after each refactor to ensure you stay green.

```bash
cargo test --workspace --features lmdb,server
```

Also run clippy and fmt:

```bash
cargo clippy --workspace --features lmdb,server -- -D warnings
cargo fmt
```

## Coverage target

Aim for 80%+ line coverage in new code. Check coverage with:

```bash
# requires: cargo install cargo-llvm-cov
cargo llvm-cov --workspace --features lmdb,server
```

## Testing frameworks available

| Framework | When to use |
|-----------|------------|
| Standard `#[test]` + `assert!` / `assert_eq!` | Simple unit tests |
| `rstest` | Parameterised tests (test the same logic with different inputs) |
| `proptest` | Property-based testing (generate random inputs and verify invariants) |
| `mockall` | Mock external dependencies (database stubs, HTTP clients) |
| `serial_test` | Force single-threaded execution (required for LMDB write tests that would deadlock) |

## Trigger

Run this command when:
- Implementing a new function or public API
- Fixing a bug (test-first to prevent regression)
- Building a new feature (TDD discipline)
- Adding helper utilities or internal refactors
