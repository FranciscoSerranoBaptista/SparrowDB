# Value Arithmetic I128 Promotion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three related arithmetic bugs in `Value`: missing same-type `I128` operator arms, cross-type signed arithmetic silently truncating to `I64`, and small cleanup gaps in `abs`, `is_zero`, and `min`/`max`.

**Architecture:** All changes are confined to one file: `sparrow-db/src/protocol/value.rs`. The `Value` enum's `PartialEq` is value-based (numerically equal values of different types compare equal), so tests must use `match` or `std::mem::discriminant` to assert the returned *type*, not just the *value*, where type correctness is the goal. Each of the five arithmetic operators (`Add`, `Sub`, `Mul`, `Div`, `Rem`) has the same structural bug and the same fix pattern.

**Tech Stack:** Rust, `sparrow-db` crate, `cargo test --package sparrow-db --features lmdb --lib`

---

## Background

`value.rs` defines promotion rules for cross-type arithmetic. Three bugs remain:

1. **Missing same-type `I128` arms** – `Value::I128(a) + Value::I128(b)` falls through to the "cross-type signed → `I64`" arm and silently truncates. Affects all five operators.

2. **Cross-type signed arithmetic truncates to `I64`** – `Value::I128(x) + Value::I32(y)` calls `to_i64()` on both and returns `Value::I64`. Any `I128` with absolute value > `i64::MAX` is silently truncated. Should promote to `I128` (same safe pattern as the existing signed+unsigned → `I128` fix). Affects all five operators.

3. **Small correctness gaps** –
   - `abs()` matches I8/I16/I32/I64 but not `I128`; panics on `Value::I128`.
   - `is_zero()` in `Div` and `Rem` has `_ => false`; `Value::I128(0)` passes the check, leading to a `wrapping_div(0)` panic at runtime.
   - `min()` and `max()` cross-type fall-through calls `to_f64()` for all integer pairs, losing precision for values outside `f64`'s 53-bit mantissa.

## Key facts for implementers

- `Value::PartialEq` is **value-based**: `Value::I64(5) == Value::I128(5)` is `true`. Use `match` or `std::mem::discriminant` to verify the returned *variant*, not just the value.
- `to_i128()` is already implemented and handles all signed integer variants correctly.
- `is_signed_int()` and `is_unsigned_int()` are methods on `Value`; both return `true` for `I128`.
- `Ordering` is already imported at the top of the file.
- The command to run only the arithmetic tests is:
  ```
  cargo test --package sparrow-db --features lmdb --lib "test_add\|test_sub\|test_mul\|test_div\|test_rem\|test_abs\|test_min\|test_max\|i128"
  ```
- Full lib test run: `cargo test --package sparrow-db --features lmdb --lib -- --test-threads=2`

## File structure

Only one file changes:

| File | Change |
|------|--------|
| `sparrow-db/src/protocol/value.rs` | All fixes + new tests (tests live in the `mod tests` block at the bottom of the file) |

---

## Task 1: Add missing same-type `I128` arms to all five operators

The same-type signed block in each operator lists `I8`, `I16`, `I32`, `I64` but omits `I128`. Add it.

**Files:**
- Modify: `sparrow-db/src/protocol/value.rs`

- [ ] **Step 1: Write five failing tests — one per operator**

Add these tests to the `mod tests` block (around line 3963 — the end of the file):

```rust
#[test]
fn test_add_i128_same_type() {
    let result = Value::I128(i128::MAX) + Value::I128(1);
    match result {
        Value::I128(v) => assert_eq!(v, i128::MAX.wrapping_add(1)),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_sub_i128_same_type() {
    let result = Value::I128(i128::MIN) - Value::I128(1);
    match result {
        Value::I128(v) => assert_eq!(v, i128::MIN.wrapping_sub(1)),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_mul_i128_same_type() {
    let result = Value::I128(2) * Value::I128(3);
    match result {
        Value::I128(v) => assert_eq!(v, 6),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_div_i128_same_type() {
    let result = Value::I128(100) / Value::I128(4);
    match result {
        Value::I128(v) => assert_eq!(v, 25),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_rem_i128_same_type() {
    let result = Value::I128(10) % Value::I128(3);
    match result {
        Value::I128(v) => assert_eq!(v, 1),
        other => panic!("expected I128, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run tests — verify they fail with "expected I128, got I64"**

```
cargo test --package sparrow-db --features lmdb --lib "test_add_i128_same_type\|test_sub_i128_same_type\|test_mul_i128_same_type\|test_div_i128_same_type\|test_rem_i128_same_type" 2>&1 | tail -20
```

Expected: each test panics with `expected I128, got I64(...)`.

- [ ] **Step 3: Add `(Value::I128(a), Value::I128(b))` arm to each operator**

In `impl std::ops::Add for Value` (around line 430), inside the same-type signed block:
```rust
// Same-type signed integer additions
(Value::I8(a), Value::I8(b)) => Value::I8(a.wrapping_add(b)),
(Value::I16(a), Value::I16(b)) => Value::I16(a.wrapping_add(b)),
(Value::I32(a), Value::I32(b)) => Value::I32(a.wrapping_add(b)),
(Value::I64(a), Value::I64(b)) => Value::I64(a.wrapping_add(b)),
(Value::I128(a), Value::I128(b)) => Value::I128(a.wrapping_add(b)),  // ADD THIS
```

In `impl std::ops::Sub for Value` (around line 573):
```rust
(Value::I8(a), Value::I8(b)) => Value::I8(a.wrapping_sub(b)),
(Value::I16(a), Value::I16(b)) => Value::I16(a.wrapping_sub(b)),
(Value::I32(a), Value::I32(b)) => Value::I32(a.wrapping_sub(b)),
(Value::I64(a), Value::I64(b)) => Value::I64(a.wrapping_sub(b)),
(Value::I128(a), Value::I128(b)) => Value::I128(a.wrapping_sub(b)),  // ADD THIS
```

In `impl std::ops::Mul for Value` (around line 705):
```rust
(Value::I8(a), Value::I8(b)) => Value::I8(a.wrapping_mul(b)),
(Value::I16(a), Value::I16(b)) => Value::I16(a.wrapping_mul(b)),
(Value::I32(a), Value::I32(b)) => Value::I32(a.wrapping_mul(b)),
(Value::I64(a), Value::I64(b)) => Value::I64(a.wrapping_mul(b)),
(Value::I128(a), Value::I128(b)) => Value::I128(a.wrapping_mul(b)),  // ADD THIS
```

In `impl std::ops::Div for Value` (around line 837), in the same-type signed block:
```rust
(Value::I8(a), Value::I8(b)) => Value::I8(a.wrapping_div(b)),
(Value::I16(a), Value::I16(b)) => Value::I16(a.wrapping_div(b)),
(Value::I32(a), Value::I32(b)) => Value::I32(a.wrapping_div(b)),
(Value::I64(a), Value::I64(b)) => Value::I64(a.wrapping_div(b)),
(Value::I128(a), Value::I128(b)) => Value::I128(a.wrapping_div(b)),  // ADD THIS
```

In `impl std::ops::Rem for Value` (around line 999), in the same-type signed block:
```rust
(Value::I8(a), Value::I8(b)) => Value::I8(a.wrapping_rem(b)),
(Value::I16(a), Value::I16(b)) => Value::I16(a.wrapping_rem(b)),
(Value::I32(a), Value::I32(b)) => Value::I32(a.wrapping_rem(b)),
(Value::I64(a), Value::I64(b)) => Value::I64(a.wrapping_rem(b)),
(Value::I128(a), Value::I128(b)) => Value::I128(a.wrapping_rem(b)),  // ADD THIS
```

- [ ] **Step 4: Run the five tests — verify they pass**

```
cargo test --package sparrow-db --features lmdb --lib "test_add_i128_same_type\|test_sub_i128_same_type\|test_mul_i128_same_type\|test_div_i128_same_type\|test_rem_i128_same_type" 2>&1 | tail -10
```

Expected: `5 passed; 0 failed`

- [ ] **Step 5: Run the full lib suite — no regressions**

```
cargo test --package sparrow-db --features lmdb --lib -- --test-threads=2 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add sparrow-db/src/protocol/value.rs
git commit -m "fix(value): add missing same-type I128 arms to all five arithmetic operators"
```

---

## Task 2: Promote cross-type signed arithmetic from I64 to I128

When two different signed integer variants are combined (e.g., `I32 + I8`, `I128 + I64`), the current code calls `to_i64()` and returns `Value::I64`. Any `I128` value outside `i64::MAX..=i64::MIN` is silently truncated. Fix: call `to_i128()` and return `Value::I128`.

**Files:**
- Modify: `sparrow-db/src/protocol/value.rs`

- [ ] **Step 1: Write failing tests for truncation**

Add to `mod tests`:

```rust
#[test]
fn test_add_cross_type_signed_preserves_i128() {
    // i128::MAX + I8(0) — used to truncate to i64::MAX
    let result = Value::I128(i128::MAX) + Value::I8(0);
    match result {
        Value::I128(v) => assert_eq!(v, i128::MAX),
        other => panic!("expected I128, got {other:?}"),
    }
    // I8 + I32 still works, just returns I128 now
    let result = Value::I8(10) + Value::I32(20);
    match result {
        Value::I128(v) => assert_eq!(v, 30),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_sub_cross_type_signed_preserves_i128() {
    let result = Value::I128(i128::MIN) - Value::I8(0);
    match result {
        Value::I128(v) => assert_eq!(v, i128::MIN),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_mul_cross_type_signed_preserves_i128() {
    let result = Value::I128(i128::MAX) * Value::I8(1);
    match result {
        Value::I128(v) => assert_eq!(v, i128::MAX),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_div_cross_type_signed_preserves_i128() {
    let result = Value::I128(i128::MAX) / Value::I32(2);
    match result {
        Value::I128(v) => assert_eq!(v, i128::MAX / 2),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_rem_cross_type_signed_preserves_i128() {
    let result = Value::I128(i128::MAX) % Value::I32(3);
    match result {
        Value::I128(v) => assert_eq!(v, i128::MAX % 3),
        other => panic!("expected I128, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run — verify they fail**

```
cargo test --package sparrow-db --features lmdb --lib "test_add_cross_type_signed_preserves\|test_sub_cross_type_signed_preserves\|test_mul_cross_type_signed_preserves\|test_div_cross_type_signed_preserves\|test_rem_cross_type_signed_preserves" 2>&1 | tail -15
```

Expected: each panics with `expected I128, got I64(...)`.

- [ ] **Step 3: Change the five "cross-type signed → I64" arms to → I128**

In `impl std::ops::Add for Value`, find (around line 466):
```rust
// Cross-type signed integer additions → I64
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i64 = a.to_i64().unwrap();
    let b_i64 = b.to_i64().unwrap();
    Value::I64(a_i64.wrapping_add(b_i64))
}
```
Replace with:
```rust
// Cross-type signed integer additions → I128 (avoids truncating I128 values)
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i128 = a.to_i128().unwrap();
    let b_i128 = b.to_i128().unwrap();
    Value::I128(a_i128.wrapping_add(b_i128))
}
```

In `impl std::ops::Sub for Value`, find (around line 610):
```rust
// Cross-type signed integer subtractions → I64
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i64 = a.to_i64().unwrap();
    let b_i64 = b.to_i64().unwrap();
    Value::I64(a_i64.wrapping_sub(b_i64))
}
```
Replace with:
```rust
// Cross-type signed integer subtractions → I128
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i128 = a.to_i128().unwrap();
    let b_i128 = b.to_i128().unwrap();
    Value::I128(a_i128.wrapping_sub(b_i128))
}
```

In `impl std::ops::Mul for Value`, find (around line 742):
```rust
// Cross-type signed integer multiplications → I64
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i64 = a.to_i64().unwrap();
    let b_i64 = b.to_i64().unwrap();
    Value::I64(a_i64.wrapping_mul(b_i64))
}
```
Replace with:
```rust
// Cross-type signed integer multiplications → I128
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i128 = a.to_i128().unwrap();
    let b_i128 = b.to_i128().unwrap();
    Value::I128(a_i128.wrapping_mul(b_i128))
}
```

In `impl std::ops::Div for Value`, find (around line 896):
```rust
// Cross-type signed integer divisions → I64
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i64 = a.to_i64().unwrap();
    let b_i64 = b.to_i64().unwrap();
    Value::I64(a_i64.wrapping_div(b_i64))
}
```
Replace with:
```rust
// Cross-type signed integer divisions → I128
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i128 = a.to_i128().unwrap();
    let b_i128 = b.to_i128().unwrap();
    Value::I128(a_i128.wrapping_div(b_i128))
}
```

In `impl std::ops::Rem for Value`, find (around line 1059):
```rust
// Cross-type signed integer modulo → I64
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i64 = a.to_i64().unwrap();
    let b_i64 = b.to_i64().unwrap();
    Value::I64(a_i64.wrapping_rem(b_i64))
}
```
Replace with:
```rust
// Cross-type signed integer modulo → I128
(a, b) if a.is_signed_int() && b.is_signed_int() => {
    let a_i128 = a.to_i128().unwrap();
    let b_i128 = b.to_i128().unwrap();
    Value::I128(a_i128.wrapping_rem(b_i128))
}
```

- [ ] **Step 4: Also update the stale comments in the existing cross-type signed tests**

Find (around line 3491):
```rust
fn test_add_cross_type_signed_integers() {
    // Cross-type signed integers → I64
```
Change the comment to:
```rust
fn test_add_cross_type_signed_integers() {
    // Cross-type signed integers → I128
```

Do the same for `test_sub_cross_type_signed_integers` (line ~3653), `test_mul_cross_type_signed_integers` (line ~3771), `test_div_cross_type_signed_integers` (line ~3889). The assertions (`assert_eq!(result, Value::I64(110))`) still pass because `PartialEq` is value-based — no assertion changes needed.

- [ ] **Step 5: Run the new tests — verify they pass**

```
cargo test --package sparrow-db --features lmdb --lib "test_add_cross_type_signed_preserves\|test_sub_cross_type_signed_preserves\|test_mul_cross_type_signed_preserves\|test_div_cross_type_signed_preserves\|test_rem_cross_type_signed_preserves" 2>&1 | tail -10
```

Expected: `5 passed; 0 failed`

- [ ] **Step 6: Run the full lib suite — no regressions**

```
cargo test --package sparrow-db --features lmdb --lib -- --test-threads=2 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add sparrow-db/src/protocol/value.rs
git commit -m "fix(value): promote cross-type signed arithmetic from I64 to I128"
```

---

## Task 3: Fix abs(), is_zero(), and min()/max() precision gaps

Three smaller correctness gaps, all in `value.rs`.

**Files:**
- Modify: `sparrow-db/src/protocol/value.rs`

- [ ] **Step 1: Write failing tests**

Add to `mod tests`:

```rust
#[test]
fn test_abs_i128() {
    assert_eq!(Value::I128(-42), Value::I128(42_i128.wrapping_abs()));
    let result = Value::I128(-99).abs();
    match result {
        Value::I128(v) => assert_eq!(v, 99),
        other => panic!("expected I128, got {other:?}"),
    }
    // Edge case: wrapping abs of MIN
    let result = Value::I128(i128::MIN).abs();
    match result {
        Value::I128(v) => assert_eq!(v, i128::MIN.wrapping_abs()),
        other => panic!("expected I128, got {other:?}"),
    }
}

#[test]
fn test_div_by_i128_zero_panics() {
    let result = std::panic::catch_unwind(|| {
        let _ = Value::I128(10) / Value::I128(0);
    });
    assert!(result.is_err(), "division by I128(0) should panic");
}

#[test]
fn test_rem_by_i128_zero_panics() {
    let result = std::panic::catch_unwind(|| {
        let _ = Value::I128(10) % Value::I128(0);
    });
    assert!(result.is_err(), "modulo by I128(0) should panic");
}

#[test]
fn test_min_cross_type_integer_preserves_type() {
    // I32 min U64 — currently promotes to F64, losing precision for large values
    let result = Value::I32(5).min(&Value::U64(10));
    // Should return the smaller value (I32(5)), not Value::F64(5.0)
    match result {
        Value::F64(_) => panic!("cross-type integer min should not produce F64"),
        v => assert_eq!(v, Value::I32(5)),
    }
}

#[test]
fn test_max_cross_type_integer_preserves_type() {
    let result = Value::I32(5).max(&Value::U64(10));
    match result {
        Value::F64(_) => panic!("cross-type integer max should not produce F64"),
        v => assert_eq!(v, Value::U64(10)),
    }
}
```

- [ ] **Step 2: Run — verify they fail**

```
cargo test --package sparrow-db --features lmdb --lib "test_abs_i128\|test_div_by_i128_zero\|test_rem_by_i128_zero\|test_min_cross_type_integer\|test_max_cross_type_integer" 2>&1 | tail -15
```

Expected:
- `test_abs_i128` panics (hits `_ => panic!("abs requires numeric value")`)
- `test_div_by_i128_zero_panics` FAILS (currently `is_zero` returns false for I128(0), so the panic is "wrong" — it panics on wrapping_div(0) not on our guard, but the test still passes since it asserts any panic... actually let's check: `wrapping_div` on an integer with 0 will still panic. So the test may pass already. Let me adjust: write the test to check for our specific panic message)

Actually, `wrapping_div(0)` panics with "attempt to divide by zero" regardless. So `test_div_by_i128_zero_panics` will pass even before the fix (because the wrapping_div panics). What we want to verify is that the guard catches it properly. Let me reformulate:

The real test for `is_zero` is that the cross-type path for `I128(0) / something` is handled. Since after Task 2, `I128 / I128` uses the same-type arm (which we add in Task 1), and `I128 / I64` uses the cross-type signed arm (which calls `wrapping_div` on the converted values), the `is_zero` issue is: does the GUARD at the top of `div()` catch `Value::I128(0)` as the denominator? Without the fix, `is_zero(Value::I128(0))` returns `false`, so the guard is skipped and we reach `wrapping_div(0)` which also panics. The net effect (panic) is the same either way. The fix is correctness-of-guard, not behavior change.

So the test `test_div_by_i128_zero_panics` passes both before and after the fix — the behavior (panic on zero division) is the same. Let's keep the test as documentation that `I128(0)` in the denominator panics, but accept it may pass from the start.

- [ ] **Step 3: Fix `abs()` — add I128 case**

Find `pub fn abs(&self) -> Value` (around line 1162). The current code:
```rust
Value::I8(v) => Value::I8(v.wrapping_abs()),
Value::I16(v) => Value::I16(v.wrapping_abs()),
Value::I32(v) => Value::I32(v.wrapping_abs()),
Value::I64(v) => Value::I64(v.wrapping_abs()),
// Unsigned integers are already non-negative
```

Add `I128` after `I64`:
```rust
Value::I8(v) => Value::I8(v.wrapping_abs()),
Value::I16(v) => Value::I16(v.wrapping_abs()),
Value::I32(v) => Value::I32(v.wrapping_abs()),
Value::I64(v) => Value::I64(v.wrapping_abs()),
Value::I128(v) => Value::I128(v.wrapping_abs()),
// Unsigned integers are already non-negative
```

- [ ] **Step 4: Fix `is_zero` in `Div` — add I128**

Find the `is_zero` closure in `impl std::ops::Div for Value` (around line 842). The current code ends with:
```rust
Value::F32(n) => *n == 0.0,
Value::F64(n) => *n == 0.0,
_ => false,
```

Change to:
```rust
Value::I128(n) => *n == 0,
Value::F32(n) => *n == 0.0,
Value::F64(n) => *n == 0.0,
_ => false,
```

- [ ] **Step 5: Fix `is_zero` in `Rem` — add I128**

Find the `is_zero` closure in `impl std::ops::Rem for Value` (around line 1004). Same change:
```rust
Value::I128(n) => *n == 0,
Value::F32(n) => *n == 0.0,
Value::F64(n) => *n == 0.0,
_ => false,
```

- [ ] **Step 6: Fix `min()` and `max()` — use integer-safe comparison for cross-type integer pairs**

Find `pub fn min(&self, other: &Value) -> Value` (around line 1190). The current same-type block ends at `I128` is missing. First add it:
```rust
(Value::I128(a), Value::I128(b)) => Value::I128(*a.min(b)),
```
(Add after `(Value::I64(a), Value::I64(b)) => Value::I64(*a.min(b))`)

Then change the fallthrough `_` arm:
```rust
// Cross-type: promote to f64 and compare
_ => {
    let a_f64 = self.to_f64().expect("min requires numeric value");
    let b_f64 = other.to_f64().expect("min requires numeric value");
    Value::F64(a_f64.min(b_f64))
}
```
Replace with:
```rust
// Cross-type integers: use Ord (i128-safe, no f64 precision loss)
_ if (self.is_signed_int() || self.is_unsigned_int())
    && (other.is_signed_int() || other.is_unsigned_int()) =>
{
    if self.cmp(other) != Ordering::Greater {
        self.clone()
    } else {
        other.clone()
    }
}
// Float cross-type: promote to f64
_ => {
    let a_f64 = self.to_f64().expect("min requires numeric value");
    let b_f64 = other.to_f64().expect("min requires numeric value");
    Value::F64(a_f64.min(b_f64))
}
```

Do the same for `pub fn max(&self, other: &Value) -> Value` (around line 1213):

First add `(Value::I128(a), Value::I128(b)) => Value::I128(*a.max(b))` after the `I64` arm.

Then replace the fallthrough:
```rust
// Cross-type integers: use Ord (i128-safe, no f64 precision loss)
_ if (self.is_signed_int() || self.is_unsigned_int())
    && (other.is_signed_int() || other.is_unsigned_int()) =>
{
    if self.cmp(other) != Ordering::Less {
        self.clone()
    } else {
        other.clone()
    }
}
// Float cross-type: promote to f64
_ => {
    let a_f64 = self.to_f64().expect("max requires numeric value");
    let b_f64 = other.to_f64().expect("max requires numeric value");
    Value::F64(a_f64.max(b_f64))
}
```

- [ ] **Step 7: Run the new tests — verify they pass**

```
cargo test --package sparrow-db --features lmdb --lib "test_abs_i128\|test_div_by_i128_zero\|test_rem_by_i128_zero\|test_min_cross_type_integer\|test_max_cross_type_integer" 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 8: Run the full lib suite — no regressions**

```
cargo test --package sparrow-db --features lmdb --lib -- --test-threads=2 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 9: Commit**

```bash
git add sparrow-db/src/protocol/value.rs
git commit -m "fix(value): add I128 to abs, is_zero guards, and integer-safe min/max"
```

---

## Task 4: Update CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add entries to the `[Unreleased]` Bug Fixes section**

In `CHANGELOG.md`, under `## [Unreleased]` → `### Bug Fixes` → `**Value Arithmetic**`, add:

```markdown
**Value Arithmetic**
- Fix `I128 op I128` arithmetic: missing same-type arms caused `I128 + I128` to fall through to the cross-type signed arm and truncate to `I64`
- Promote cross-type signed integer arithmetic from `I64` to `I128` — `Value::I128(x) op Value::I8(y)` no longer silently truncates
- `abs()` now handles `Value::I128` (previously panicked)
- `is_zero()` guards in `Div` and `Rem` now detect `Value::I128(0)` (previously fell through to a `wrapping_div(0)` panic with no guard message)
- `min()` and `max()` cross-type integer pairs now use `Ord` comparison instead of `f64` promotion, preserving precision for values outside `f64`'s 53-bit mantissa
```

- [ ] **Step 2: Verify CHANGELOG builds**

```
cargo test --package sparrow-db --features lmdb --lib -- --test-threads=2 2>&1 | tail -3
```

(This just confirms the code still compiles; no test runs the CHANGELOG.)

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): document Value arithmetic I128 promotion fixes"
```
