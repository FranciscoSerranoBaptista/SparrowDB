# HNSW Stability Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two critical HNSW correctness bugs (AVX2 cosine divide-by-zero, entry point error swallowing) and add two correctness features (HNSW health diagnostics via BFS, PREFILTER during traversal).

**Architecture:** Each fix is isolated to its own file(s). The cosine fix touches `vector_distance.rs` in both the `lmdb` and `rocks` backends and `types.rs` for a new error variant. The entry point fix is a one-function change in `lmdb/vector_core.rs`. The diagnostics BFS is a pure addition to `diagnostics.rs`. The PREFILTER is a single boolean change in `traversal_core/ops/vectors/search.rs`.

**Tech Stack:** Rust, cargo test, bumpalo arena, heed3 (LMDB backend), `#[cfg(target_feature = "avx2")]` SIMD

---

## Roadmap & Priority

| # | Priority | Item | Root Cause | Status in SparrowDB |
|---|----------|------|-----------|---------------------|
| 1 | **P0 Critical** | AVX2 cosine divide-by-zero | `cosine_similarity_avx2` has no zero-magnitude guard; divides by zero on modern x86 → NaN/Inf silently | PRESENT |
| 2 | **P1 High** | Entry point error swallowing | `Err(_)` in insert swallows all errors; only `EntryPointNotFound` should be treated as first-insert | PRESENT |
| 3 | **P2 Medium** | HNSW diagnostics BFS health check | No way to detect unreachable vectors post-deletion | ABSENT |
| 4 | **P2 Medium** | PREFILTER during HNSW traversal | `should_trickle` hardcoded `false`; filtered SearchV returns < K results | DISABLED |

---

## File Map

| File | Change |
|---|---|
| `sparrow-db/src/sparrow_engine/types.rs` | Add `ZeroMagnitudeVector` variant to `VectorError` enum |
| `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_distance.rs` | Fix `cosine_similarity_avx2` return type + zero-magnitude guard; fix scalar path to return error |
| `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_distance.rs` | Same cosine fix for RocksDB backend |
| `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` | Fix `Err(_)` → `Err(VectorError::EntryPointNotFound)` in insert; propagate other errors |
| `sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs` | Add tests for zero-magnitude cosine (both AVX2 and scalar paths) |
| `sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs` | Add BFS health check: count unreachable vectors, classify health |
| `sparrow-db/src/sparrow_engine/traversal_core/ops/vectors/search.rs` | Change `should_trickle` from `false` to `true` |

---

## Task 1 — Fix AVX2 Cosine Divide-by-Zero (P0 Critical)

**Problem:** `cosine_similarity_avx2` (lines 91–141 in both `lmdb/vector_distance.rs` and `rocks/vector_distance.rs`) returns bare `f64` and divides by zero when either magnitude is zero, silently producing `NaN` or `Inf`. On any x86 machine with AVX2, the non-SIMD scalar guards on lines 81–83 are never reached. The scalar path also incorrectly returns `Ok(-1.0)` for zero-magnitude vectors instead of an error.

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/types.rs:168-195`
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_distance.rs`
- Modify: `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_distance.rs`
- Test: `sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs`

---

- [ ] **Step 1.1: Write failing tests for zero-magnitude cosine**

Add these tests to the bottom of `sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs`:

```rust
#[cfg(test)]
mod cosine_tests {
    use crate::sparrow_engine::{
        types::VectorError,
        vector_core::vector_distance::cosine_similarity,
    };

    #[test]
    #[cfg(feature = "cosine")]
    fn test_cosine_zero_magnitude_a_returns_error() {
        let a = vec![0.0f64, 0.0, 0.0, 0.0];
        let b = vec![1.0f64, 0.0, 0.0, 0.0];
        let result = cosine_similarity(&a, &b);
        assert!(
            matches!(result, Err(VectorError::ZeroMagnitudeVector)),
            "expected ZeroMagnitudeVector, got {result:?}"
        );
    }

    #[test]
    #[cfg(feature = "cosine")]
    fn test_cosine_zero_magnitude_b_returns_error() {
        let a = vec![1.0f64, 0.0, 0.0, 0.0];
        let b = vec![0.0f64, 0.0, 0.0, 0.0];
        let result = cosine_similarity(&a, &b);
        assert!(
            matches!(result, Err(VectorError::ZeroMagnitudeVector)),
            "expected ZeroMagnitudeVector, got {result:?}"
        );
    }

    #[test]
    #[cfg(feature = "cosine")]
    fn test_cosine_both_zero_magnitude_returns_error() {
        let a = vec![0.0f64, 0.0, 0.0, 0.0];
        let b = vec![0.0f64, 0.0, 0.0, 0.0];
        let result = cosine_similarity(&a, &b);
        assert!(
            matches!(result, Err(VectorError::ZeroMagnitudeVector)),
            "expected ZeroMagnitudeVector, got {result:?}"
        );
    }

    #[test]
    #[cfg(feature = "cosine")]
    fn test_cosine_identical_unit_vectors() {
        let a = vec![1.0f64, 0.0, 0.0, 0.0];
        let b = vec![1.0f64, 0.0, 0.0, 0.0];
        let result = cosine_similarity(&a, &b).unwrap();
        assert!((result - 1.0).abs() < 1e-10, "identical vectors should have cosine 1.0, got {result}");
    }

    #[test]
    #[cfg(feature = "cosine")]
    fn test_cosine_orthogonal_vectors() {
        let a = vec![1.0f64, 0.0, 0.0, 0.0];
        let b = vec![0.0f64, 1.0, 0.0, 0.0];
        let result = cosine_similarity(&a, &b).unwrap();
        assert!(result.abs() < 1e-10, "orthogonal vectors should have cosine ~0.0, got {result}");
    }

    #[test]
    #[cfg(feature = "cosine")]
    fn test_cosine_result_is_finite() {
        // Regression: AVX2 path must never return NaN or Inf
        let a = vec![0.5f64; 128];
        let b = vec![0.5f64; 128];
        let result = cosine_similarity(&a, &b).unwrap();
        assert!(result.is_finite(), "cosine result must be finite, got {result}");
    }
}
```

- [ ] **Step 1.2: Run to verify tests fail**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors cosine_tests 2>&1 | tail -20
```

Expected: compile error `ZeroMagnitudeVector` not found (since the variant doesn't exist yet).

- [ ] **Step 1.3: Add `ZeroMagnitudeVector` to `VectorError`**

In `sparrow-db/src/sparrow_engine/types.rs`, the `VectorError` enum starts at line 169. Add the new variant and its Display arm:

Find:
```rust
pub enum VectorError {
    VectorNotFound(String),
    VectorDeleted,
    InvalidVectorLength,
    InvalidVectorData,
    EntryPointNotFound,
    ConversionError(String),
    VectorCoreError(String),
    VectorAlreadyDeleted(String),
}
```

Replace with:
```rust
pub enum VectorError {
    VectorNotFound(String),
    VectorDeleted,
    InvalidVectorLength,
    InvalidVectorData,
    EntryPointNotFound,
    ConversionError(String),
    VectorCoreError(String),
    VectorAlreadyDeleted(String),
    ZeroMagnitudeVector,
}
```

Find the `Display` impl:
```rust
            VectorError::VectorAlreadyDeleted(id) => write!(f, "Vector already deleted: {id}"),
        }
    }
```

Replace with:
```rust
            VectorError::VectorAlreadyDeleted(id) => write!(f, "Vector already deleted: {id}"),
            VectorError::ZeroMagnitudeVector => write!(f, "Vector has zero magnitude; cosine similarity is undefined"),
        }
    }
```

- [ ] **Step 1.4: Fix `lmdb/vector_distance.rs` — scalar path and AVX2 path**

Replace the entire contents of `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_distance.rs` with:

```rust
use crate::sparrow_engine::{types::VectorError, vector_core::vector::HVector};

pub const MAX_DISTANCE: f64 = 2.0;
pub const ORTHOGONAL: f64 = 1.0;
pub const MIN_DISTANCE: f64 = 0.0;

pub trait DistanceCalc {
    fn distance(from: &HVector, to: &HVector) -> Result<f64, VectorError>;
}
impl<'a> DistanceCalc for HVector<'a> {
    /// Calculates the distance between two vectors.
    ///
    /// It normalizes the distance to be between 0 and 2.
    ///
    /// - 1.0 (most similar) → Distance 0.0 (closest)
    /// - 0.0 (orthogonal) → Distance 1.0
    /// - -1.0 (most dissimilar) → Distance 2.0 (furthest)
    #[inline(always)]
    #[cfg(feature = "cosine")]
    fn distance(from: &HVector, to: &HVector) -> Result<f64, VectorError> {
        cosine_similarity(from.data, to.data).map(|sim| 1.0 - sim)
    }
}

#[inline]
#[cfg(feature = "cosine")]
pub fn cosine_similarity(from: &[f64], to: &[f64]) -> Result<f64, VectorError> {
    let len = from.len();
    let other_len = to.len();

    if len != other_len {
        return Err(VectorError::InvalidVectorLength);
    }

    #[cfg(target_feature = "avx2")]
    {
        return cosine_similarity_avx2(from, to);
    }

    let mut dot_product = 0.0;
    let mut magnitude_a = 0.0;
    let mut magnitude_b = 0.0;

    const CHUNK_SIZE: usize = 8;
    let chunks = len / CHUNK_SIZE;
    let remainder = len % CHUNK_SIZE;

    for i in 0..chunks {
        let offset = i * CHUNK_SIZE;
        let a_chunk = &from[offset..offset + CHUNK_SIZE];
        let b_chunk = &to[offset..offset + CHUNK_SIZE];

        let mut local_dot = 0.0;
        let mut local_mag_a = 0.0;
        let mut local_mag_b = 0.0;

        for j in 0..CHUNK_SIZE {
            let a_val = a_chunk[j];
            let b_val = b_chunk[j];
            local_dot += a_val * b_val;
            local_mag_a += a_val * a_val;
            local_mag_b += b_val * b_val;
        }

        dot_product += local_dot;
        magnitude_a += local_mag_a;
        magnitude_b += local_mag_b;
    }

    let remainder_offset = chunks * CHUNK_SIZE;
    for i in 0..remainder {
        let a_val = from[remainder_offset + i];
        let b_val = to[remainder_offset + i];
        dot_product += a_val * b_val;
        magnitude_a += a_val * a_val;
        magnitude_b += b_val * b_val;
    }

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return Err(VectorError::ZeroMagnitudeVector);
    }

    Ok(dot_product / (magnitude_a.sqrt() * magnitude_b.sqrt()))
}

// SIMD implementation using AVX2 (256-bit vectors)
#[cfg(target_feature = "avx2")]
#[inline(always)]
pub fn cosine_similarity_avx2(a: &[f64], b: &[f64]) -> Result<f64, VectorError> {
    use std::arch::x86_64::*;

    let len = a.len();
    let chunks = len / 4; // AVX2 processes 4 f64 values at once

    let (dot_product_total, magnitude_a_total, magnitude_b_total) = unsafe {
        let mut dot_product = _mm256_setzero_pd();
        let mut magnitude_a = _mm256_setzero_pd();
        let mut magnitude_b = _mm256_setzero_pd();

        for i in 0..chunks {
            let offset = i * 4;

            let a_chunk = _mm256_loadu_pd(&a[offset]);
            let b_chunk = _mm256_loadu_pd(&b[offset]);

            dot_product = _mm256_add_pd(dot_product, _mm256_mul_pd(a_chunk, b_chunk));
            magnitude_a = _mm256_add_pd(magnitude_a, _mm256_mul_pd(a_chunk, a_chunk));
            magnitude_b = _mm256_add_pd(magnitude_b, _mm256_mul_pd(b_chunk, b_chunk));
        }

        let dot_sum = horizontal_sum_pd(dot_product);
        let mag_a_sum = horizontal_sum_pd(magnitude_a);
        let mag_b_sum = horizontal_sum_pd(magnitude_b);

        let mut dot_remainder = 0.0;
        let mut mag_a_remainder = 0.0;
        let mut mag_b_remainder = 0.0;

        let remainder_offset = chunks * 4;
        for i in remainder_offset..len {
            let a_val = a[i];
            let b_val = b[i];
            dot_remainder += a_val * b_val;
            mag_a_remainder += a_val * a_val;
            mag_b_remainder += b_val * b_val;
        }

        (
            dot_sum + dot_remainder,
            (mag_a_sum + mag_a_remainder).sqrt(),
            (mag_b_sum + mag_b_remainder).sqrt(),
        )
    };

    if magnitude_a_total == 0.0 || magnitude_b_total == 0.0 {
        return Err(VectorError::ZeroMagnitudeVector);
    }

    Ok(dot_product_total / (magnitude_a_total * magnitude_b_total))
}

// Helper function to sum the 4 doubles in an AVX2 vector
#[cfg(target_feature = "avx2")]
#[inline(always)]
unsafe fn horizontal_sum_pd(__v: __m256d) -> f64 {
    use std::arch::x86_64::*;

    let sum_hi_lo = _mm_add_pd(_mm256_castpd256_pd128(__v), _mm256_extractf128_pd(__v, 1));
    let sum = _mm_add_sd(sum_hi_lo, _mm_unpackhi_pd(sum_hi_lo, sum_hi_lo));
    _mm_cvtsd_f64(sum)
}
```

Key changes from the original:
1. Removed the stale `println!` from the dimension mismatch check
2. Scalar path: `Ok(-1.0)` → `Err(VectorError::ZeroMagnitudeVector)`
3. AVX2 path: return type changed from `f64` to `Result<f64, VectorError>`, guard added before division, SIMD computation extracted to a safe `unsafe {}` block

- [ ] **Step 1.5: Apply identical fix to `rocks/vector_distance.rs`**

Replace the entire contents of `sparrow-db/src/sparrow_engine/vector_core/rocks/vector_distance.rs` with the exact same file content from Step 1.4. The two files are identical in structure and have the identical bugs.

- [ ] **Step 1.6: Run tests to verify they pass**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors cosine_tests 2>&1 | tail -20
```

Expected: all 6 tests pass.

- [ ] **Step 1.7: Run the full HNSW test suite to check for regressions**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors hnsw 2>&1 | tail -30
```

Expected: all existing HNSW tests still pass.

- [ ] **Step 1.8: Commit**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
git add sparrow-db/src/sparrow_engine/types.rs \
        sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_distance.rs \
        sparrow-db/src/sparrow_engine/vector_core/rocks/vector_distance.rs \
        sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs
git commit -m "$(cat <<'EOF'
fix(vector-core): return ZeroMagnitudeVector error instead of dividing by zero

AVX2 cosine path had no zero-magnitude guard and returned bare f64,
producing NaN/Inf silently on modern x86. Scalar path incorrectly
returned Ok(-1.0). Both paths now return Err(ZeroMagnitudeVector).
Applies to both lmdb and rocks backends.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2 — Harden Entry Point Error Handling (P1 High)

**Problem:** In `lmdb/vector_core.rs` insert function (line ~973), `Err(_)` swallows all errors and treats them as "first insert". If `get_entry_point` fails due to database corruption, a serialization error, or any other non-"not found" error, the code silently overwrites the entry point and returns early. This masks real failures and can corrupt the HNSW graph state.

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs:971-979`
- Test: `sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs`

---

- [ ] **Step 2.1: Write a failing test for entry point error propagation**

Add to the bottom of `sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs`:

```rust
#[cfg(test)]
mod entry_point_tests {
    use super::*;
    use crate::sparrow_engine::{types::VectorError, vector_core::{HNSW, HNSWConfig, VectorCore}};

    fn setup_vector_core(env: &heed3::Env) -> VectorCore {
        let mut txn = env.write_txn().unwrap();
        let vc = VectorCore::new(&mut txn, HNSWConfig::default(), "test_ep").unwrap();
        txn.commit().unwrap();
        vc
    }

    #[test]
    fn test_first_insert_sets_entry_point() {
        let (env, _tmp) = setup_env();
        let vc = setup_vector_core(&env);
        let arena = Bump::new();
        let mut txn = env.write_txn().unwrap();

        let data = vec![1.0f64, 0.0, 0.0, 0.0];
        let result = vc.insert::<Filter>(&mut txn, "test", &data, None, &arena);
        assert!(result.is_ok(), "first insert should succeed: {result:?}");

        // After first insert, entry point must be set
        let ro_txn = txn.downgrade();
        let ep = vc.get_entry_point(&ro_txn, "test", &arena);
        assert!(ep.is_ok(), "entry point should be set after first insert");
    }

    #[test]
    fn test_second_insert_does_not_overwrite_higher_level_entry_point() {
        let (env, _tmp) = setup_env();
        let vc = setup_vector_core(&env);
        let arena = Bump::new();
        let mut txn = env.write_txn().unwrap();

        // Insert many vectors — some will get higher HNSW levels by chance.
        // The entry point should always be the highest-level vector seen.
        for i in 0..20 {
            let data = vec![i as f64, 0.0, 0.0, 0.0];
            vc.insert::<Filter>(&mut txn, "test", &data, None, &arena)
                .unwrap();
        }

        let ro_txn = txn.downgrade();
        let ep = vc.get_entry_point(&ro_txn, "test", &arena).unwrap();
        // The entry point's level must be >= all vectors' levels — we just check it exists
        assert!(ep.level <= 10, "entry point level sanity check");
    }
}
```

- [ ] **Step 2.2: Run to verify tests compile and pass (they should — these test existing good behavior)**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors entry_point_tests 2>&1 | tail -20
```

Expected: both tests pass. These confirm current behavior is correct, giving us a regression baseline.

- [ ] **Step 2.3: Fix the `Err(_)` catch-all in `lmdb/vector_core.rs`**

In `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs`, around line 971, find:

```rust
        let entry_point = match self.get_entry_point(txn, label, arena) {
            Ok(ep) => ep,
            Err(_) => {
                // TODO: use proper error handling
                self.set_entry_point(txn, &query)?;
                query.set_distance(0.0);

                return Ok(query);
            }
        };
```

Replace with:

```rust
        let entry_point = match self.get_entry_point(txn, label, arena) {
            Ok(ep) => ep,
            Err(VectorError::EntryPointNotFound) => {
                self.set_entry_point(txn, &query)?;
                query.set_distance(0.0);
                return Ok(query);
            }
            Err(e) => return Err(e),
        };
```

- [ ] **Step 2.4: Run the entry point tests to confirm they still pass**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors entry_point_tests 2>&1 | tail -20
```

Expected: both tests still pass.

- [ ] **Step 2.5: Run full HNSW test suite for regressions**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors hnsw 2>&1 | tail -30
```

Expected: all pass.

- [ ] **Step 2.6: Commit**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
git add sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs \
        sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs
git commit -m "$(cat <<'EOF'
fix(vector-core): propagate non-EntryPointNotFound errors in insert

Err(_) catch-all silently overwrote the entry point on any error from
get_entry_point, masking corruption and serialization failures. Now only
EntryPointNotFound triggers the first-insert path; all other errors propagate.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3 — HNSW Diagnostics BFS Health Check (P2 Medium)

**Problem:** The existing `GET /diagnostics` endpoint returns counts but has no way to detect unreachable vectors in the HNSW graph. After delete/re-insert cycles the graph can silently fragment. We need a `GET /hnsw-health` endpoint that does a BFS from the entry point and reports how many active vectors are unreachable.

**Health classification:**
- `"healthy"` — 0 unreachable active vectors
- `"degraded"` — 1–5% of active vectors are unreachable
- `"broken"` — >5% unreachable, or no entry point when vectors exist

**Files:**
- Modify: `sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs`
- Modify: `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs` (add `bfs_reachable` method)
- Modify: `sparrow-db/src/sparrow_engine/vector_core/mod.rs` (re-export new trait method if needed)

---

- [ ] **Step 3.1: Write failing test for the new `/hnsw-health` endpoint**

Add to the test section at the bottom of `sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs` (before the final `}`):

```rust
    fn make_hnsw_health_request() -> Request {
        Request {
            name: "hnsw_health".to_string(),
            req_type: RequestType::Query,
            api_key: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
            pre_computed_embedding: None,
        }
    }

    #[test]
    fn test_hnsw_health_empty_db_is_healthy() {
        let (engine, _temp_dir) = setup_test_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_hnsw_health_request(),
        };

        let result = hnsw_health_inner(input);
        assert!(result.is_ok(), "hnsw_health on empty db should succeed: {result:?}");

        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.contains("\"status\":\"healthy\""), "empty db should be healthy, got: {body}");
        assert!(body.contains("\"unreachable\":0"), "got: {body}");
    }

    #[test]
    fn test_hnsw_health_after_inserts_is_healthy() -> Result<(), Box<dyn std::error::Error>> {
        use crate::sparrow_engine::vector_core::HNSW;

        let (engine, _temp_dir) = setup_test_engine();
        let arena = bumpalo::Bump::new();
        let mut txn = engine.storage.graph_env.write_txn().unwrap();

        for i in 0..10 {
            let data = vec![i as f64, 0.0, 0.0, 0.0];
            engine.storage.vectors
                .insert::<fn(&_, &_) -> bool>(&mut txn, "test", &data, None, &arena)
                .unwrap();
        }
        txn.commit().unwrap();

        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_hnsw_health_request(),
        };

        let result = hnsw_health_inner(input)?;
        let body = String::from_utf8(result.body).unwrap();
        assert!(body.contains("\"status\":\"healthy\""), "10 inserts should be healthy, got: {body}");
        assert!(body.contains("\"total_active\":10"), "got: {body}");
        assert!(body.contains("\"unreachable\":0"), "got: {body}");
        Ok(())
    }
```

- [ ] **Step 3.2: Run to verify test fails (function doesn't exist yet)**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors hnsw_health 2>&1 | tail -20
```

Expected: compile error — `hnsw_health_inner` not found.

- [ ] **Step 3.3: Add `bfs_reachable_count` to `VectorCore`**

In `sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs`, add the following method to the `VectorCore` impl block (place it after `get_all_vectors`, around line 597):

```rust
    /// Returns the count of active vectors reachable by BFS from the entry point at level 0.
    /// Used for HNSW health diagnostics.
    pub fn bfs_reachable_count<'db: 'arena, 'arena>(
        &self,
        txn: &'arena heed3::RoTxn<'db>,
        label: &'arena str,
        arena: &'arena bumpalo::Bump,
    ) -> Result<usize, VectorError> {
        let entry_point = match self.get_entry_point(txn, label, arena) {
            Ok(ep) => ep,
            Err(VectorError::EntryPointNotFound) => return Ok(0),
            Err(e) => return Err(e),
        };

        let mut visited: std::collections::HashSet<u128> = std::collections::HashSet::new();
        let mut queue: std::collections::VecDeque<u128> = std::collections::VecDeque::new();

        visited.insert(entry_point.id);
        queue.push_back(entry_point.id);

        while let Some(id) = queue.pop_front() {
            let neighbors = self.get_neighbors::<fn(&HVector, &heed3::RoTxn) -> bool>(
                txn, label, id, 0, None, arena,
            )?;
            for neighbor in neighbors {
                if visited.insert(neighbor.id) {
                    queue.push_back(neighbor.id);
                }
            }
        }

        Ok(visited.len())
    }
```

- [ ] **Step 3.4: Add `hnsw_health_inner` function to `diagnostics.rs`**

In `sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs`, add the following after the `inventory::submit!` block for `diagnostics_inner` (around line 68), before the `#[cfg(test)]` block:

```rust
// GET /hnsw-health
// curl "http://localhost:PORT/hnsw-health"
//
// Runs a BFS from the HNSW entry point (level 0) and reports unreachable active vectors.
// {
//   "status": "healthy" | "degraded" | "broken",
//   "total_active": 90,
//   "reachable": 88,
//   "unreachable": 2
// }
//
// healthy  = 0 unreachable
// degraded = 1–5% unreachable
// broken   = >5% unreachable, or no entry point when active vectors exist

pub fn hnsw_health_inner(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    let db = Arc::clone(&input.graph.storage);

    #[cfg(feature = "lmdb")]
    {
        let txn = db.graph_env.read_txn().map_err(GraphError::from)?;
        let arena = bumpalo::Bump::new();

        let vector_stats = db
            .vectors
            .stats(&txn)
            .map_err(|e| GraphError::New(e.to_string()))?;

        let total_active = vector_stats.active as usize;

        let reachable = db
            .vectors
            .bfs_reachable_count(&txn, "default", &arena)
            .map_err(|e| GraphError::New(e.to_string()))?;

        let unreachable = total_active.saturating_sub(reachable);

        let status = if total_active == 0 || unreachable == 0 {
            "healthy"
        } else if unreachable * 100 / total_active <= 5 {
            "degraded"
        } else {
            "broken"
        };

        let body = format!(
            r#"{{"status":"{status}","total_active":{total_active},"reachable":{reachable},"unreachable":{unreachable}}}"#,
        );

        return Ok(protocol::Response {
            body: body.into_bytes(),
            fmt: Default::default(),
        });
    }

    #[cfg(not(feature = "lmdb"))]
    {
        Err(GraphError::New(
            "hnsw-health endpoint requires lmdb feature".to_string(),
        ))
    }
}

inventory::submit! {
    HandlerSubmission(
        Handler::new("hnsw_health", hnsw_health_inner, false)
    )
}
```

**Note on label:** The BFS call uses `"default"` as the label. If your deployment uses a different default label or multiple labels, this needs to be made label-aware. For now, `"default"` matches the standard SparrowDB schema convention. Check `sparrow-db/src/sparrow_engine/storage_core/mod.rs` to confirm the label string used during standard vector inserts if tests fail with `EntryPointNotFound`.

- [ ] **Step 3.5: Run the hnsw-health tests**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors hnsw_health 2>&1 | tail -30
```

Expected: both `test_hnsw_health_empty_db_is_healthy` and `test_hnsw_health_after_inserts_is_healthy` pass.

If `test_hnsw_health_after_inserts_is_healthy` fails with `EntryPointNotFound`, the label string used during inserts in the test is different from `"default"`. Find the correct label by checking what label the `diagnostics_with_vectors` test uses in the existing tests — if it uses `"test"`, change `"default"` to `"test"` in `bfs_reachable_count` call, or better: make the label a parameter and look it up from the stats. Check how `vector_stats` is gathered (it likely iterates all labels) and use the first active label.

- [ ] **Step 3.6: Run the full diagnostics test suite for regressions**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors diagnostics 2>&1 | tail -30
```

Expected: all pass including the original 3 diagnostics tests.

- [ ] **Step 3.7: Commit**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
git add sparrow-db/src/sparrow_engine/vector_core/lmdb/vector_core.rs \
        sparrow-db/src/sparrow_gateway/builtin/diagnostics.rs
git commit -m "$(cat <<'EOF'
feat(diagnostics): add GET /hnsw-health endpoint with BFS reachability check

Detects unreachable vectors caused by delete/re-insert fragmentation.
Reports status: healthy (0 unreachable), degraded (≤5%), or broken (>5%).
Uses BFS from HNSW entry point at level 0.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4 — Enable PREFILTER During HNSW Traversal (P2 Medium)

**Problem:** In `search.rs` line 60, `should_trickle` is hardcoded to `false`. This means WHERE-clause filters are only applied post-retrieval: the HNSW graph returns the top-K nearest vectors by distance, then only the ones matching the filter are kept. If fewer than K vectors match the filter, the caller gets back fewer results than they asked for. Setting `should_trickle = true` applies the filter *during* graph traversal so the walk only considers matching nodes, improving result completeness for filtered queries.

**Caveat:** The filter closures receive `(&HVector, &Txn)`. For property-based filters, the vector's `properties` field must be populated. Verify in a test that property filters work correctly before shipping.

**Files:**
- Modify: `sparrow-db/src/sparrow_engine/traversal_core/ops/vectors/search.rs:60`
- Test: `sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs`

---

- [ ] **Step 4.1: Write a failing test that demonstrates under-K results with post-filter**

Add to `sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs`:

```rust
#[cfg(test)]
mod prefilter_tests {
    use super::*;
    use crate::sparrow_engine::vector_core::{HNSW, HNSWConfig, VectorCore};

    fn setup_vector_core_with_data(env: &heed3::Env) -> VectorCore {
        let mut txn = env.write_txn().unwrap();
        let vc = VectorCore::new(&mut txn, HNSWConfig::default(), "test_pf").unwrap();
        txn.commit().unwrap();
        vc
    }

    #[test]
    fn test_search_with_filter_returns_k_results_when_k_exist() {
        // Insert 20 vectors, mark 10 of them as "matching" via their id parity.
        // Ask for k=5 with a filter that only accepts even-id vectors.
        // With should_trickle=true (PREFILTER), we should get 5 results.
        // This test was previously failing with should_trickle=false when the
        // nearest 5 by distance were all odd-id vectors.
        let (env, _tmp) = setup_env();
        let vc = setup_vector_core_with_data(&env);
        let arena = Bump::new();
        let mut txn = env.write_txn().unwrap();

        let mut inserted_ids = Vec::new();
        for i in 0..20i64 {
            let data = vec![i as f64, 0.0, 0.0, 0.0];
            let v = vc.insert::<Filter>(&mut txn, "test", &data, None, &arena).unwrap();
            inserted_ids.push((v.id, i));
        }
        txn.commit().unwrap();

        let ro_txn = env.read_txn().unwrap();
        let query = vec![0.0f64, 0.0, 0.0, 0.0];

        // Filter: only accept vectors whose first data element is even
        let filter_fn: Filter = |v: &HVector, _txn: &RoTxn| {
            v.data.first().map(|x| x % 2.0 == 0.0).unwrap_or(false)
        };
        let filters = [filter_fn];

        let results = vc.search(
            &ro_txn,
            &query,
            5,      // k=5
            "test",
            Some(&filters),
            true,   // should_trickle = PREFILTER mode
            &arena,
        ).unwrap();

        assert_eq!(
            results.len(), 5,
            "PREFILTER should return exactly 5 even-indexed results, got {}",
            results.len()
        );
    }
}
```

- [ ] **Step 4.2: Run the test directly against `vector_core.rs` to see it pass (the VectorCore already supports should_trickle)**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors prefilter_tests 2>&1 | tail -20
```

Expected: this test passes — the `HNSW::search` method already supports `should_trickle`. This confirms the `VectorCore` is correct.

- [ ] **Step 4.3: Change the hardcoded `false` to `true` in `search.rs`**

In `sparrow-db/src/sparrow_engine/traversal_core/ops/vectors/search.rs`, line 60, find:

```rust
        let vectors = self.storage.vectors.search(
            self.txn,
            query,
            k.try_into().unwrap(),
            label,
            filter,
            false,
            self.arena,
        );
```

Replace with:

```rust
        let vectors = self.storage.vectors.search(
            self.txn,
            query,
            k.try_into().unwrap(),
            label,
            filter,
            true,
            self.arena,
        );
```

- [ ] **Step 4.4: Run the full vector traversal test suite**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
cargo test --package sparrow-db --features lmdb,compiler,vectors 2>&1 | grep -E "^test|FAILED|error" | tail -40
```

Expected: all tests pass. If any traversal test fails specifically because it expected fewer-than-K results (testing the old post-filter behavior), update the assertion to expect K results.

- [ ] **Step 4.5: Commit**

```bash
cd /Users/franciscobaptista/Development/SparrowDB
git add sparrow-db/src/sparrow_engine/traversal_core/ops/vectors/search.rs \
        sparrow-db/src/sparrow_engine/tests/hnsw_tests.rs
git commit -m "$(cat <<'EOF'
feat(search): enable PREFILTER during HNSW traversal

should_trickle was hardcoded false, causing filtered SearchV to apply
WHERE clauses only post-retrieval and return fewer than k results when
matches were sparse. Now filters apply during graph traversal.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**Spec coverage check:**
- P0 AVX2 cosine divide-by-zero → Task 1 ✓
- P1 entry point error swallowing → Task 2 ✓
- P2 HNSW diagnostics BFS → Task 3 ✓
- P2 PREFILTER → Task 4 ✓
- Rocks backend parity → Task 1 Step 1.5 ✓

**Placeholder scan:** No TBDs, no "implement later", all code blocks are complete.

**Type consistency:**
- `VectorError::ZeroMagnitudeVector` used in Task 1 tests matches the variant added in Step 1.3 ✓
- `hnsw_health_inner` used in test matches function name in Step 3.4 ✓
- `bfs_reachable_count` used in Step 3.4 matches method added in Step 3.3 ✓
- `should_trickle: true` in Step 4.3 matches the `bool` parameter in `HNSW::search` signature ✓

**Known gaps / caveats called out inline:**
- Task 3, Step 3.5: label string `"default"` may need adjustment depending on the schema convention
- Task 4: property-based filters depend on `properties` field being populated in `HVector`; distance-only filters are safe
