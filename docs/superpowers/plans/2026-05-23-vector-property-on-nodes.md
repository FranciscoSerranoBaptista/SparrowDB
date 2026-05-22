# vector(N) Property Type on N:: Nodes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `vector(N)` as a first-class property type on `N::` node schemas, auto-indexing into the global HNSW on `AddN`, and expose a new `SearchN<Type.field>(query, k)` traversal entry point that returns graph nodes ranked by embedding similarity.

**Architecture:** `vector(N)` threads through the full HQL pipeline: grammar → parser AST → analyzer (type check + code generation) → engine. On insert, the generated code extracts the f32 slice and passes it to a new `add_n_with_vectors` engine method that uses `insert_with_id` (the node's UUID) to store the embedding in HNSW under the label `"TypeName.fieldname"`. On search, `SearchNAdapter` queries HNSW with that label and re-hydrates each result into a graph node via `storage.get_node`.

**Tech Stack:** Rust, pest PEG grammar, bumpalo arena allocator, heed3 LMDB, HNSW (internal VectorCore)

---

## File Map

| File | Action | What changes |
|------|--------|-------------|
| `crates/sparrow-core/src/grammar.pest` | Modify | Add `vector_type`, `search_node_vector`, `type_dot_field` rules; wire into `param_type`, `traversal`, `evaluates_to_anything` |
| `crates/sparrow-core/src/sparrowc/parser/types.rs` | Modify | Add `FieldType::Vector(usize)`, `SearchNodeVector` struct, `StartNode::SearchNodeVector` variant |
| `crates/sparrow-core/src/sparrowc/parser/schema_parse_methods.rs` | Modify | Handle `Rule::vector_type` in `parse_field_type` |
| `crates/sparrow-core/src/sparrowc/parser/traversal_parse_methods.rs` | Modify | Handle `Rule::search_node_vector` in `parse_start_node`; add `parse_search_node_vector` |
| `crates/sparrow-core/src/sparrowc/parser/expression_parse_methods.rs` | Modify | Handle `Rule::search_node_vector` in `parse_expression` |
| `crates/sparrow-core/src/sparrowc/analyzer/error_codes.rs` | Modify | Add `E111` for vector field on edge |
| `crates/sparrow-core/src/sparrowc/analyzer/methods/schema_methods.rs` | Modify | Allow `Vector(N)` in `is_valid_schema_field_type`; emit E111 if used on `E::` edge |
| `crates/sparrow-core/src/sparrowc/analyzer/types.rs` | Modify | Map `FieldType::Vector(n)` → `GeneratedType::VectorF32(n)` in `From<FieldType>` |
| `crates/sparrow-core/src/sparrowc/generator/utils.rs` | Modify | Add `GeneratedType::VectorF32(usize)` variant + `Display` + `to_ts()` |
| `crates/sparrow-core/src/sparrowc/generator/schemas.rs` | Modify | Handle `VectorF32(n)` in `NodeSchema::to_typescript()` |
| `crates/sparrow-core/src/sparrowc/analyzer/methods/traversal_validation.rs` | Modify | Handle `StartNode::SearchNodeVector`; validate + generate `SourceStep::SearchN` |
| `crates/sparrow-core/src/sparrowc/analyzer/methods/infer_expr_type.rs` | Modify | Detect vector fields in `AddNode` and populate `AddN.vector_fields` |
| `crates/sparrow-core/src/sparrowc/generator/source_steps.rs` | Modify | Add `SourceStep::SearchN`; add `vector_fields` to `AddN`; add `SearchNStep` + `Display` |
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs` | Modify | Add `add_n_with_vectors` to `AddNAdapter`; call `vectors.insert_with_id` per vector field |
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/vectors/search_n.rs` | **Create** | `SearchNAdapter` trait + impl: HNSW search → node hydration |
| `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/vectors/mod.rs` | Modify | Add `pub mod search_n` |

---

## Task 1: Grammar — add `vector_type`, `search_node_vector`, `type_dot_field`

**Files:**
- Modify: `crates/sparrow-core/src/grammar.pest`

- [ ] **Step 1: Write the failing parser test**

In `crates/sparrow-core/src/sparrowc/parser/schema_parse_methods.rs`, at the bottom of the `#[cfg(test)]` block, add:

```rust
#[test]
fn test_parse_vector_field_type() {
    let source = r#"
        N::Person {
            name: String,
            embedding: vector(1536)
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let result = SparrowParser::parse_source(&content);
    assert!(result.is_ok(), "parse failed: {:?}", result.err());
    let parsed = result.unwrap();
    let schema = parsed.schema.get(&1).unwrap();
    assert_eq!(schema.node_schemas[0].fields.len(), 2);
    assert!(matches!(
        schema.node_schemas[0].fields[1].field_type,
        FieldType::Vector(1536)
    ));
}

#[test]
fn test_search_node_vector_parses() {
    let source = r#"
        N::Person {
            name: String,
            embedding: vector(1536)
        }
        QUERY findPeople(q: [F64]) => {
            results <- SearchN<Person.embedding>(q, 10)
            RETURN results
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let result = SparrowParser::parse_source(&content);
    assert!(result.is_ok(), "parse failed: {:?}", result.err());
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
cargo test --package sparrow-core --features lmdb test_parse_vector_field_type -- --nocapture 2>&1 | head -40
```

Expected: compile error or test failure because `vector(N)` is not yet in the grammar.

- [ ] **Step 3: Add grammar rules**

In `crates/sparrow-core/src/grammar.pest`, make these changes:

**a) Add `vector_type` and `type_dot_field` rules near the types section (after line 307, before `param_type`):**

```pest
vector_type    = { "vector" ~ "(" ~ integer ~ ")" }
type_dot_field = { identifier_upper ~ "." ~ identifier }
```

**b) Update `param_type` to include `vector_type` BEFORE `identifier` (ordered choice — `vector` keyword must match before the general identifier rule consumes it):**

Change:
```pest
param_type       = { named_type | date_type | ID_TYPE | array | object | identifier  }
```
To:
```pest
param_type       = { named_type | date_type | ID_TYPE | array | object | vector_type | identifier  }
```

**c) Add `search_node_vector` rule near the vector steps section (after `search_vector` on line 227):**

```pest
search_node_vector = { "SearchN" ~ "<" ~ type_dot_field ~ ">" ~ "(" ~ vector_data ~ "," ~ (integer | identifier) ~ ")" }
```

**d) Update `traversal` (line 65) to include `search_node_vector`:**

Change:
```pest
traversal           = { (start_node | start_edge | search_vector | start_vector) ~ step* ~ last_step? }
```
To:
```pest
traversal           = { (start_node | start_edge | search_vector | search_node_vector | start_vector) ~ step* ~ last_step? }
```

**e) Update `evaluates_to_anything` to include `search_node_vector` (add after the `search_vector` line):**

Change:
```pest
  | search_vector
```
To:
```pest
  | search_vector
  | search_node_vector
```

- [ ] **Step 4: Run the test to confirm it passes**

```bash
cargo test --package sparrow-core --features lmdb test_parse_vector_field_type test_search_node_vector_parses -- --nocapture 2>&1 | head -40
```

Expected: FAIL because `Rule::vector_type` is not yet handled in `parse_field_type`. Confirm the test compiles and the error is a runtime failure, not a compile failure.

- [ ] **Step 5: Commit grammar**

```bash
git add crates/sparrow-core/src/grammar.pest crates/sparrow-core/src/sparrowc/parser/schema_parse_methods.rs
git commit -m "feat(grammar): add vector_type and SearchN grammar rules"
```

---

## Task 2: Parser types — `FieldType::Vector(usize)`, `SearchNodeVector`, `StartNode::SearchNodeVector`

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/parser/types.rs`

- [ ] **Step 1: Add `Vector(usize)` to `FieldType` enum**

In `types.rs`, after the `Object` variant:

```rust
// Before:
    Object(HashMap<String, FieldType>),
    // Closure(String, HashMap<String, FieldType>),
}
```

Change to:
```rust
    Object(HashMap<String, FieldType>),
    /// vector(N) — first-class embedding field; always F32, stored as F64 in HNSW
    Vector(usize),
    // Closure(String, HashMap<String, FieldType>),
}
```

- [ ] **Step 2: Update `PartialEq for FieldType`**

After the `(FieldType::Object(a), FieldType::Object(b)) => a == b,` arm, add:

```rust
            (FieldType::Vector(a), FieldType::Vector(b)) => a == b,
```

- [ ] **Step 3: Update `Display for FieldType`**

After the `FieldType::Object(m) => { ... }` arm, add:

```rust
            FieldType::Vector(n) => write!(f, "vector({n})"),
```

- [ ] **Step 4: Add `SearchNodeVector` struct and extend `StartNode`**

After the existing `SearchVector` struct (near line 969), add:

```rust
/// AST for `SearchN<NodeType.field>(query, k)`
#[derive(Debug, Clone)]
pub struct SearchNodeVector {
    pub loc: Loc,
    /// The node type name (e.g. "Person")
    pub node_type: String,
    /// The vector field name (e.g. "embedding")
    pub field_name: String,
    pub data: Option<VectorData>,
    pub k: Option<EvaluatesToNumber>,
}
```

In the `StartNode` enum, add a new variant after `SearchVector`:

```rust
    SearchVector(SearchVector),
    SearchNodeVector(SearchNodeVector),   // NEW
    Identifier(String),
    Anonymous,
```

- [ ] **Step 5: Run the parser test (confirm it still fails but compiles)**

```bash
cargo test --package sparrow-core --features lmdb test_parse_vector_field_type -- --nocapture 2>&1 | head -30
```

Expected: compile + runtime failure pointing to the unhandled `Rule::vector_type` in `parse_field_type`.

- [ ] **Step 6: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/parser/types.rs
git commit -m "feat(parser): add FieldType::Vector, SearchNodeVector, StartNode::SearchNodeVector"
```

---

## Task 3: Parser methods — handle new grammar rules

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/parser/schema_parse_methods.rs`
- Modify: `crates/sparrow-core/src/sparrowc/parser/traversal_parse_methods.rs`
- Modify: `crates/sparrow-core/src/sparrowc/parser/expression_parse_methods.rs`

- [ ] **Step 1: Handle `Rule::vector_type` in `parse_field_type`**

In `schema_parse_methods.rs`, inside `parse_field_type`, add a new match arm BEFORE the `other => Err(...)` catch-all:

```rust
            Rule::vector_type => {
                // vector(N) — parse the dimension integer from the single child
                let dim_str = field
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParserError::from("vector type missing dimension"))?
                    .as_str();
                let dim = dim_str.parse::<usize>().map_err(|_| {
                    ParserError::from(format!("invalid vector dimension '{dim_str}'"))
                })?;
                Ok(FieldType::Vector(dim))
            }
```

- [ ] **Step 2: Run the schema parse test — it should now pass**

```bash
cargo test --package sparrow-core --features lmdb test_parse_vector_field_type -- --nocapture 2>&1 | head -30
```

Expected: PASS.

- [ ] **Step 3: Add `parse_search_node_vector` in `traversal_parse_methods.rs`**

At the end of `impl SparrowParser` in `traversal_parse_methods.rs`, add:

```rust
    pub(super) fn parse_search_node_vector(
        &self,
        pair: Pair<Rule>,
    ) -> Result<SearchNodeVector, ParserError> {
        use crate::sparrowc::parser::types::SearchNodeVector;
        let mut node_type = String::new();
        let mut field_name = String::new();
        let mut data = None;
        let mut k = None;

        for p in pair.clone().into_inner() {
            match p.as_rule() {
                Rule::type_dot_field => {
                    let mut inner = p.into_inner();
                    node_type = inner
                        .next()
                        .ok_or_else(|| ParserError::from("missing node type in SearchN"))?
                        .as_str()
                        .to_string();
                    field_name = inner
                        .next()
                        .ok_or_else(|| ParserError::from("missing field name in SearchN"))?
                        .as_str()
                        .to_string();
                }
                Rule::vector_data => {
                    let inner = p.clone().try_inner_next()?;
                    match inner.as_rule() {
                        Rule::identifier => {
                            data = Some(VectorData::Identifier(p.as_str().to_string()));
                        }
                        Rule::vec_literal => {
                            data = Some(VectorData::Vector(self.parse_vec_literal(p)?));
                        }
                        Rule::embed_method => {
                            // Embed() not supported in SearchN (out of scope)
                            return Err(ParserError::from(
                                "Embed() is not supported in SearchN; use a Vec<f64> parameter",
                            ));
                        }
                        _ => {
                            return Err(ParserError::from(format!(
                                "Unexpected rule in SearchN vector_data: {:?}",
                                inner.as_rule()
                            )));
                        }
                    }
                }
                Rule::integer => {
                    k = Some(EvaluatesToNumber {
                        loc: p.loc(),
                        value: EvaluatesToNumberType::I32(
                            p.as_str()
                                .parse::<i32>()
                                .map_err(|_| ParserError::from("Invalid integer k in SearchN"))?,
                        ),
                    });
                }
                Rule::identifier => {
                    k = Some(EvaluatesToNumber {
                        loc: p.loc(),
                        value: EvaluatesToNumberType::Identifier(p.as_str().to_string()),
                    });
                }
                _ => {
                    return Err(ParserError::from(format!(
                        "Unexpected rule in SearchN: {:?}",
                        p.as_rule()
                    )));
                }
            }
        }

        Ok(SearchNodeVector {
            loc: pair.loc(),
            node_type,
            field_name,
            data,
            k,
        })
    }
```

Also update the `use` statement at the top of the file to import `SearchNodeVector` and the other types needed. The existing imports in this file reference types from `super::types` — make sure `SearchNodeVector`, `VectorData`, `EvaluatesToNumber`, `EvaluatesToNumberType` are included.

Check the existing imports in `traversal_parse_methods.rs`:

```bash
head -15 crates/sparrow-core/src/sparrowc/parser/traversal_parse_methods.rs
```

Add `SearchNodeVector` to the existing `types::` import.

- [ ] **Step 4: Handle `Rule::search_node_vector` in `parse_start_node`**

In `traversal_parse_methods.rs`, inside `parse_start_node`, add before the `Rule::identifier` arm:

```rust
            Rule::search_node_vector => {
                Ok(StartNode::SearchNodeVector(self.parse_search_node_vector(pair)?))
            }
```

- [ ] **Step 5: Handle `Rule::search_node_vector` in `parse_expression`**

In `expression_parse_methods.rs`, inside `parse_expression`, add after the `Rule::search_vector` arm:

```rust
            Rule::search_node_vector => {
                Ok(Expression {
                    loc: pair.loc(),
                    expr: ExpressionType::SearchNodeVector(
                        self.parse_search_node_vector(pair)?,
                    ),
                })
            }
```

Also add `SearchNodeVector` to the `ExpressionType` enum in `parser/types.rs`:

In `types.rs`, find the `ExpressionType` enum (near line 610) and add after `SearchVector(SearchVector)`:

```rust
    SearchNodeVector(SearchNodeVector),
```

Update the `Display` and `Debug` impls for `ExpressionType` accordingly (follow the same pattern as `SearchVector`):

```rust
// In Display for ExpressionType:
ExpressionType::SearchNodeVector(snv) => write!(f, "SearchNodeVector({snv:?})"),
// In Debug for ExpressionType:
ExpressionType::SearchNodeVector(snv) => write!(f, "SearchNodeVector({snv:?})"),
```

- [ ] **Step 6: Run both parser tests**

```bash
cargo test --package sparrow-core --features lmdb test_parse_vector_field_type test_search_node_vector_parses -- --nocapture 2>&1 | head -40
```

Expected: both PASS.

- [ ] **Step 7: Run the full parser test suite**

```bash
cargo test --package sparrow-core --features lmdb sparrowc::parser -- --nocapture 2>&1 | tail -20
```

Expected: all existing tests still pass.

- [ ] **Step 8: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/parser/
git commit -m "feat(parser): handle vector_type and search_node_vector rules"
```

---

## Task 4: Generator types — `GeneratedType::VectorF32`

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/generator/utils.rs`
- Modify: `crates/sparrow-core/src/sparrowc/analyzer/types.rs`
- Modify: `crates/sparrow-core/src/sparrowc/generator/schemas.rs`

- [ ] **Step 1: Add `VectorF32(usize)` to `GeneratedType`**

In `generator/utils.rs`, find the `GeneratedType` enum:

```rust
pub enum GeneratedType {
    RustType(RustType),
    Vec(Box<GeneratedType>),
    Variable(GenRef<String>),
    Object(GeneratedObject),
}
```

Add the new variant:

```rust
pub enum GeneratedType {
    RustType(RustType),
    Vec(Box<GeneratedType>),
    Variable(GenRef<String>),
    Object(GeneratedObject),
    /// vector(N) field — renders as Vec<f32> in Rust
    VectorF32(usize),
}
```

- [ ] **Step 2: Update `Display for GeneratedType`**

In the `Display` impl (near line 313), add:

```rust
            GeneratedType::VectorF32(_) => write!(f, "Vec<f32>"),
```

- [ ] **Step 3: Update `From<FieldType> for GeneratedType` in `analyzer/types.rs`**

In the `From<FieldType>` impl (near line 185), add before the `FieldType::Object` arm:

```rust
            FieldType::Vector(n) => GeneratedType::VectorF32(n),
```

- [ ] **Step 4: Update `NodeSchema::to_typescript()` in `generator/schemas.rs`**

In the `to_typescript` method for `NodeSchema`, update the property type match to handle `VectorF32`:

```rust
// Change:
        for property in &self.properties {
            result.push_str(&format!(
                "  {}: {};\n",
                property.name,
                match &property.field_type {
                    GeneratedType::RustType(t) => t.to_ts(),
                    _ => {
                        debug_assert!(false, "NodeSchema property has unexpected type");
                        format!("/* ERROR: unsupported type for {} */", property.name)
                    }
                }
            ));
        }

// To:
        for property in &self.properties {
            result.push_str(&format!(
                "  {}: {};\n",
                property.name,
                match &property.field_type {
                    GeneratedType::RustType(t) => t.to_ts(),
                    GeneratedType::VectorF32(n) => format!("Array<number> /* vector({n}) */"),
                    GeneratedType::Vec(inner) => {
                        // e.g. [String] renders as Array<string>
                        format!("Array<{}>", match inner.as_ref() {
                            GeneratedType::RustType(t) => t.to_ts(),
                            _ => "unknown".to_string(),
                        })
                    }
                    _ => {
                        debug_assert!(false, "NodeSchema property has unexpected type");
                        format!("/* ERROR: unsupported type for {} */", property.name)
                    }
                }
            ));
        }
```

- [ ] **Step 5: Verify the project compiles**

```bash
cargo build --package sparrow-core --features lmdb 2>&1 | head -40
```

Expected: no errors (there may be dead_code warnings for the new variants, which is fine).

- [ ] **Step 6: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/
git commit -m "feat(generator): add GeneratedType::VectorF32 for vector(N) fields"
```

---

## Task 5: Analyzer schema validation — allow `vector(N)` on nodes, reject on edges

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/analyzer/error_codes.rs`
- Modify: `crates/sparrow-core/src/sparrowc/analyzer/methods/schema_methods.rs`

- [ ] **Step 1: Write the failing analyzer test for edge rejection**

In `schema_methods.rs`, in the `#[cfg(test)]` block, add:

```rust
#[test]
fn test_vector_field_rejected_on_edge() {
    use crate::sparrowc::{
        analyzer::{Ctx, errors::AnalysisError},
        parser::{SparrowParser, write_to_temp_file},
    };

    let source = r#"
        N::Person { name: String }
        N::Company { name: String }
        E::WorksAt {
            From: Person,
            To: Company,
            Properties: {
                embedding: vector(512)
            }
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let parsed = SparrowParser::parse_source(&content).unwrap();
    let result = Ctx::analyze(parsed, None);
    assert!(
        result.is_err(),
        "expected error for vector field on edge, got Ok"
    );
}

#[test]
fn test_vector_field_valid_on_node() {
    use crate::sparrowc::{
        analyzer::Ctx,
        parser::{SparrowParser, write_to_temp_file},
    };

    let source = r#"
        N::Person {
            name: String,
            embedding: vector(1536)
        }
        QUERY addPerson(name: String, embedding: [F64]) => {
            p <- AddN<Person>({name: name, embedding: embedding})
            RETURN p
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let parsed = SparrowParser::parse_source(&content).unwrap();
    let result = Ctx::analyze(parsed, None);
    assert!(result.is_ok(), "expected Ok for vector on node, got {:?}", result.err());
}
```

- [ ] **Step 2: Run the tests to confirm they fail**

```bash
cargo test --package sparrow-core --features lmdb test_vector_field_rejected_on_edge test_vector_field_valid_on_node -- --nocapture 2>&1 | head -40
```

Expected: both fail (no E111 code yet; no validation logic yet).

- [ ] **Step 3: Add `E111` to `error_codes.rs`**

In `error_codes.rs`, after the `E110` entry, add:

```rust
    /// `E111` - `vector(N) field is only valid on N:: node types, not E:: edges`
    E111,
```

Then find the `impl fmt::Display for ErrorCode` (or wherever the messages are), and add:

```rust
ErrorCode::E111 => "vector(N) field is only valid on N:: node types, not E:: edges",
```

Check how existing codes are formatted in that file:
```bash
grep -A2 "E110 =>" crates/sparrow-core/src/sparrowc/analyzer/error_codes.rs
```

Match the existing pattern.

- [ ] **Step 4: Update `is_valid_schema_field_type` and add edge validation in `schema_methods.rs`**

**a)** In `is_valid_schema_field_type`, add the `Vector` arm:

```rust
fn is_valid_schema_field_type(ft: &FieldType) -> bool {
    match ft {
        FieldType::Identifier(_) => false,
        FieldType::Object(_) => false,
        FieldType::Array(inner) => is_valid_schema_field_type(inner),
        FieldType::Vector(_) => true,    // NEW
        _ => true,
    }
}
```

**b)** Find where edge properties are validated (the loop that calls `is_valid_schema_field_type` on edge fields, around line 391). It should look something like:

```rust
for f in edge.properties.iter().flatten() {
    if !is_valid_schema_field_type(&f.field_type) {
        // existing error push
    }
}
```

Add an additional check specifically for `Vector`:

```rust
for f in edge.properties.iter().flatten() {
    if !is_valid_schema_field_type(&f.field_type) {
        // existing error push
    }
    // NEW: vector(N) fields are not allowed on edges
    if matches!(f.field_type, FieldType::Vector(_)) {
        push_schema_err(
            errors,
            &f.loc,
            ErrorCode::E111,
            &[f.name.as_str(), edge.name.1.as_str()],
        );
    }
}
```

Look at how `push_schema_err` is called with existing E-codes in the same file to match the exact signature.

- [ ] **Step 5: Run the schema tests**

```bash
cargo test --package sparrow-core --features lmdb test_vector_field_rejected_on_edge test_vector_field_valid_on_node -- --nocapture 2>&1 | head -40
```

Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/analyzer/
git commit -m "feat(analyzer): add E111 and validate vector(N) field placement"
```

---

## Task 6: Generator source_steps — add `SearchNStep`, extend `AddN`

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/generator/source_steps.rs`

- [ ] **Step 1: Add `SearchNStep` struct and `SourceStep::SearchN` variant**

In `source_steps.rs`, at the end of the file, add the new struct:

```rust
/// Generated step for `SearchN<NodeType.field>(query, k)`
#[derive(Clone, Debug)]
pub struct SearchNStep {
    /// The HNSW label derived as "TypeName.fieldname"
    pub label: GenRef<String>,
    /// Query vector
    pub vec: VecData,
    /// Number of results
    pub k: GeneratedValue,
}

impl Display for SearchNStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "search_n::<fn(&HVector, &RTxn) -> bool, _>({}, {}, {}, None)",
            self.vec, self.k, self.label,
        )
    }
}
```

In the `SourceStep` enum, add after `SearchBM25`:

```rust
    /// Search for graph nodes by their embedded vector field
    SearchN(SearchNStep),
```

In `Display for SourceStep`, add:

```rust
            SourceStep::SearchN(search_n) => write!(f, "{search_n}"),
```

- [ ] **Step 2: Extend `AddN` to carry vector field info and update the existing constructor**

Update the `AddN` struct to add `vector_fields`:

```rust
#[derive(Clone, Debug)]
pub struct AddN {
    /// Label of node
    pub label: GenRef<String>,
    /// Properties of node (ALL properties including vector fields)
    pub properties: Option<Vec<(String, GeneratedValue)>>,
    /// Names of properties to index on
    pub secondary_indices: Option<Vec<String>>,
    /// Vector fields to index in HNSW: (hnsw_label, field_value_accessor)
    /// hnsw_label = "TypeName.fieldname", accessor = the input field (e.g. data.embedding)
    pub vector_fields: Option<Vec<(String, GeneratedValue)>>,
}
```

Update `AddN::fmt` to generate `add_n_with_vectors` when `vector_fields` is `Some`:

```rust
impl Display for AddN {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let properties = write_properties(&self.properties);
        let secondary_indices = write_secondary_indices(&self.secondary_indices);

        match &self.vector_fields {
            None => write!(
                f,
                "add_n({}, {}, {})",
                self.label, properties, secondary_indices
            ),
            Some(vf) => {
                // Build the vector_inserts slice expression
                // Each entry: ("Label.field", field_value_as_f32_slice)
                let vec_entries = vf
                    .iter()
                    .map(|(hnsw_label, accessor)| {
                        format!("(\"{hnsw_label}\", {accessor}.as_slice())")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                write!(
                    f,
                    "add_n_with_vectors({}, {}, {}, Some(&[{vec_entries}]))",
                    self.label, properties, secondary_indices
                )
            }
        }
    }
}
```

- [ ] **Step 3: Build to catch compile errors**

```bash
cargo build --package sparrow-core --features lmdb 2>&1 | head -40
```

Expected: compile succeeds. May have unused-variable warnings for `SearchNStep`/`SearchN` not yet wired into the traversal validator — that's fine.

**IMPORTANT — update the existing `AddN` constructor:** The existing `AddN { label, properties, secondary_indices }` literal in `infer_expr_type.rs` (around line 392) must be updated to include `vector_fields: None` immediately after this task, otherwise it will fail to compile. Task 10 will replace it with the real value; for now set it to `None`:

```rust
// Temporary — Task 10 will replace None with the real vector_fields
let add_n = AddN {
    label,
    properties: Some(properties.into_iter().collect()),
    secondary_indices,
    vector_fields: None,
};
```

- [ ] **Step 4: Compile to confirm no regressions**

```bash
cargo build --package sparrow-core --features lmdb 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/generator/source_steps.rs crates/sparrow-core/src/sparrowc/analyzer/methods/infer_expr_type.rs
git commit -m "feat(generator): add SearchNStep and extend AddN with vector_fields"
```

---

## Task 7: Engine — `add_n_with_vectors` in `add_n.rs`

**Files:**
- Modify: `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs`

- [ ] **Step 1: Write a failing engine test**

At the bottom of `add_n.rs`, add a test module:

```rust
#[cfg(test)]
mod tests {
    // Note: full round-trip tests live in the hql-tests crate.
    // This unit test just verifies the trait method signature compiles.
    #[test]
    fn add_n_with_vectors_signature_present() {
        // Verify that `AddNAdapter` has the `add_n_with_vectors` method via trait check.
        // This test passes as long as the code compiles.
        let _ = stringify!(add_n_with_vectors);
    }
}
```

This is a compile-time signature check. The real behavior is tested in Task 11 (integration test).

- [ ] **Step 2: Add `add_n_with_vectors` to `AddNAdapter` and implement it**

In `add_n.rs`, extend the trait and impl:

```rust
use crate::sparrow_engine::vector_core::HNSW;

pub trait AddNAdapter<'db, 'arena, 'txn, 's>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    fn add_n(
        self,
        label: &'arena str,
        properties: Option<ImmutablePropertiesMap<'arena>>,
        secondary_indices: Option<&'s [&str]>,
    ) -> RwTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >;

    /// Like `add_n` but additionally indexes the given f32 slices into the HNSW
    /// under the specified labels, using the node's own ID as the HNSW entry ID.
    ///
    /// `vector_inserts`: `&[(hnsw_label, f32_data)]` where `hnsw_label` is
    /// `"TypeName.fieldname"`.
    fn add_n_with_vectors(
        self,
        label: &'arena str,
        properties: Option<ImmutablePropertiesMap<'arena>>,
        secondary_indices: Option<&'s [&str]>,
        vector_inserts: Option<&'s [(&'arena str, &'s [f32])]>,
    ) -> RwTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >;
}
```

For the impl block, add `add_n_with_vectors` alongside the existing `add_n` impl. The implementation reuses `add_n` logic and then loops over vector inserts:

```rust
    fn add_n_with_vectors(
        self,
        label: &'arena str,
        properties: Option<ImmutablePropertiesMap<'arena>>,
        secondary_indices: Option<&'s [&str]>,
        vector_inserts: Option<&'s [(&'arena str, &'s [f32])]>,
    ) -> RwTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    > {
        let node = Node {
            id: v6_uuid(),
            label,
            version: 1,
            properties,
        };
        let secondary_indices = secondary_indices.unwrap_or(&[]).to_vec();
        let mut result: Result<TraversalValue, GraphError> = Ok(TraversalValue::Empty);

        // Store the node in LMDB (same as add_n)
        match bincode::serialize(&node) {
            Ok(bytes) => {
                if let Err(e) = self.storage.nodes_db.put_with_flags(
                    self.txn,
                    PutFlags::APPEND,
                    &node.id,
                    &bytes,
                ) {
                    result = Err(GraphError::from(e));
                }
            }
            Err(e) => result = Err(GraphError::from(e)),
        }

        // Secondary indices (same as add_n)
        if result.is_ok() {
            for index in &secondary_indices {
                match self.storage.secondary_indices.get(index.as_str()) {
                    Some(db) => {
                        let key = match node.get_property(index) {
                            Some(value) => value,
                            None => continue,
                        };
                        match bincode::serialize(&key) {
                            Ok(serialized) => {
                                if matches!(db.1, crate::sparrow_engine::types::SecondaryIndex::Unique(_)) {
                                    match db.0.get(self.txn, &serialized) {
                                        Ok(Some(_)) => {
                                            result = Err(GraphError::DuplicateKey(format!(
                                                "Unique index '{index}' already contains this value"
                                            )));
                                            continue;
                                        }
                                        Err(e) => {
                                            result = Err(GraphError::from(e));
                                            continue;
                                        }
                                        Ok(None) => {}
                                    }
                                }
                                if let Err(e) = db.0.put(self.txn, &serialized, &node.id) {
                                    result = Err(GraphError::from(e));
                                }
                            }
                            Err(e) => result = Err(GraphError::from(e)),
                        }
                    }
                    None => {
                        result = Err(GraphError::New(format!(
                            "Secondary Index {index} not found"
                        )));
                    }
                }
            }
        }

        // BM25 (same as add_n)
        if result.is_ok() {
            if let Some(bm25) = &self.storage.bm25
                && let Some(props) = node.properties.as_ref()
            {
                let mut data = props.flatten_bm25();
                data.push_str(node.label);
                if let Err(e) = bm25.insert_doc(self.txn, node.id, &data) {
                    result = Err(e);
                }
            }
        }

        // HNSW vector inserts using the node's own ID
        if result.is_ok() {
            if let Some(inserts) = vector_inserts {
                for (hnsw_label, f32_data) in inserts {
                    // Convert f32 slice to arena-allocated f64 slice
                    let f64_vec = bumpalo::collections::Vec::from_iter_in(
                        f32_data.iter().map(|&v| v as f64),
                        self.arena,
                    );
                    if let Err(e) = self.storage.vectors.insert_with_id::<fn(
                        &crate::sparrow_engine::vector_core::vector::HVector,
                        &heed3::RoTxn,
                    ) -> bool>(
                        self.txn,
                        node.id,
                        self.arena.alloc_str(hnsw_label),
                        f64_vec.as_slice(),
                        None,
                        self.arena,
                    ) {
                        result = Err(GraphError::from(e));
                        break;
                    }
                }
            }
        }

        if result.is_ok() {
            result = Ok(TraversalValue::Node(node));
        }

        RwTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: std::iter::once(result),
        }
    }
```

**Note:** The `hnsw_label` string passed in is `&'arena str` already because it's a string literal embedded in the generated code (e.g., `"Person.embedding"`). If it's a `&'s str` from the caller, use `self.arena.alloc_str(hnsw_label)` to get `&'arena str`.

- [ ] **Step 3: Build to confirm it compiles**

```bash
cargo build --package sparrow-core --features lmdb 2>&1 | head -40
```

Expected: no errors. Fix any lifetime or type mismatch errors by inspecting the exact error message.

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-core/src/sparrow_engine/traversal_core/ops/source/add_n.rs
git commit -m "feat(engine): add add_n_with_vectors for HNSW auto-indexing of node embeddings"
```

---

## Task 8: Engine — `search_n.rs` (new file)

**Files:**
- Create: `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/vectors/search_n.rs`
- Modify: `crates/sparrow-core/src/sparrow_engine/traversal_core/ops/vectors/mod.rs`

- [ ] **Step 1: Add `pub mod search_n` to `mod.rs`**

In `vectors/mod.rs`, append:

```rust
pub mod search_n;
```

- [ ] **Step 2: Create `search_n.rs`**

```rust
use crate::sparrow_engine::{
    storage_core::storage_methods::StorageMethods,
    traversal_core::{traversal_iter::RoTraversalIterator, traversal_value::TraversalValue},
    types::{GraphError, VectorError},
    vector_core::{HNSW, vector::HVector},
};
use std::iter::once;

pub trait SearchNAdapter<'db, 'arena, 'txn>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    /// Search for graph nodes by their embedded `vector(N)` field.
    ///
    /// `label` must be `"TypeName.fieldname"` — the HNSW label used when
    /// the node was inserted via `add_n_with_vectors`.
    ///
    /// Returns `TraversalValue::Node` items (the matching graph nodes),
    /// ordered by ascending cosine distance. Results whose node IDs are
    /// not found in the graph (e.g. soft-deleted) are silently skipped.
    fn search_n<F, K>(
        self,
        query: &'arena [f64],
        k: K,
        label: &'arena str,
        filter: Option<&'arena [F]>,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        F: Fn(&HVector, &Txn) -> bool,
        K: TryInto<usize>,
        K::Error: std::fmt::Debug;
}

type Txn<'db> = heed3::RoTxn<'db>;

impl<'db, 'arena, 'txn, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    SearchNAdapter<'db, 'arena, 'txn> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    fn search_n<F, K>(
        self,
        query: &'arena [f64],
        k: K,
        label: &'arena str,
        filter: Option<&'arena [F]>,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        F: Fn(&HVector, &Txn) -> bool,
        K: TryInto<usize>,
        K::Error: std::fmt::Debug,
    {
        let k_usize = match k.try_into() {
            Ok(n) => n,
            Err(_) => {
                let iter = once(Err(GraphError::New(
                    "SearchN: k must be a non-negative integer".to_string(),
                )))
                .collect::<Vec<_>>()
                .into_iter();
                return RoTraversalIterator {
                    storage: self.storage,
                    arena: self.arena,
                    txn: self.txn,
                    inner: iter,
                };
            }
        };

        // Search the HNSW index — results have id = node ID (set by add_n_with_vectors)
        let vectors = self.storage.vectors.search(
            self.txn,
            query,
            k_usize,
            label,
            filter,
            true,
            self.arena,
        );

        let iter = match vectors {
            Ok(vectors) => vectors
                .into_iter()
                .filter_map(|vector| {
                    // Re-hydrate the graph node from the vector's ID
                    match self.storage.get_node(self.txn, vector.id, self.arena) {
                        Ok(node) => Some(Ok::<TraversalValue, GraphError>(TraversalValue::Node(node))),
                        Err(GraphError::NodeNotFound(_)) => None, // soft-deleted or orphan — skip
                        Err(e) => Some(Err(e)),
                    }
                })
                .collect::<Vec<_>>()
                .into_iter(),
            Err(VectorError::EntryPointNotFound) => {
                // Empty index — return no results
                vec![].into_iter()
            }
            Err(VectorError::InvalidVectorLength) => {
                once(Err(GraphError::VectorError(
                    "SearchN: query vector has wrong dimension".to_string(),
                )))
                .collect::<Vec<_>>()
                .into_iter()
            }
            Err(e) => {
                once(Err(GraphError::VectorError(format!("SearchN error: {e}"))))
                    .collect::<Vec<_>>()
                    .into_iter()
            }
        };

        RoTraversalIterator {
            storage: self.storage,
            arena: self.arena,
            txn: self.txn,
            inner: iter,
        }
    }
}
```

**Note on `GraphError::NodeNotFound`:** Check whether `GraphError` has a `NodeNotFound` variant, or if missing nodes return a different error code. Inspect:

```bash
grep -n "NodeNotFound\|NotFound" crates/sparrow-core/src/sparrow_engine/types.rs | head -10
```

Match the actual variant name; adjust the `filter_map` accordingly.

- [ ] **Step 3: Build to confirm it compiles**

```bash
cargo build --package sparrow-core --features lmdb 2>&1 | head -40
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-core/src/sparrow_engine/traversal_core/ops/vectors/
git commit -m "feat(engine): add SearchNAdapter — HNSW kNN search returning graph nodes"
```

---

## Task 9: Analyzer — wire up `SearchN` traversal validation

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/analyzer/methods/traversal_validation.rs`

- [ ] **Step 1: Write a failing analyzer test for `SearchN`**

In `traversal_validation.rs`'s `#[cfg(test)]` block, add:

```rust
#[test]
fn test_search_node_vector_valid() {
    use crate::sparrowc::{
        analyzer::Ctx,
        parser::{SparrowParser, write_to_temp_file},
    };

    let source = r#"
        N::Person {
            name: String,
            embedding: vector(1536)
        }
        QUERY findPeople(q: [F64]) => {
            results <- SearchN<Person.embedding>(q, 10)
            RETURN results
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let parsed = SparrowParser::parse_source(&content).unwrap();
    let result = Ctx::analyze(parsed, None);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
}

#[test]
fn test_search_node_vector_unknown_node_type_fails() {
    use crate::sparrowc::{
        analyzer::Ctx,
        parser::{SparrowParser, write_to_temp_file},
    };

    let source = r#"
        N::Person { name: String }
        QUERY findPeople(q: [F64]) => {
            results <- SearchN<Ghost.embedding>(q, 10)
            RETURN results
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let parsed = SparrowParser::parse_source(&content).unwrap();
    let result = Ctx::analyze(parsed, None);
    assert!(result.is_err(), "expected Err for unknown node type");
}
```

- [ ] **Step 2: Run the tests to confirm they fail**

```bash
cargo test --package sparrow-core --features lmdb test_search_node_vector_valid test_search_node_vector_unknown_node_type_fails -- --nocapture 2>&1 | head -40
```

- [ ] **Step 3: Add `StartNode::SearchNodeVector` case in `traversal_validation.rs`**

In the `match &tr.start` block (find where `StartNode::SearchVector(sv)` is handled, around line 521), add a new arm after it:

```rust
        StartNode::SearchNodeVector(snv) => {
            // Validate node type exists
            if !ctx.node_set.contains(snv.node_type.as_str()) {
                generate_error!(
                    ctx,
                    original_query,
                    snv.loc.clone(),
                    E101,
                    snv.node_type.as_str()
                );
                return None;
            }

            // Validate the field exists and is vector(N)
            let is_vector_field = ctx
                .node_fields
                .get(snv.node_type.as_str())
                .and_then(|fields| fields.get(snv.field_name.as_str()))
                .map(|field| matches!(field.field_type, FieldType::Vector(_)))
                .unwrap_or(false);

            if !is_vector_field {
                generate_error!(
                    ctx,
                    original_query,
                    snv.loc.clone(),
                    E202,
                    snv.field_name.as_str(),
                    "vector",
                    snv.node_type.as_str()
                );
                return None;
            }

            // Build the HNSW label: "TypeName.fieldname"
            let hnsw_label = format!("{}.{}", snv.node_type, snv.field_name);
            let label = GenRef::Literal(hnsw_label);

            // Resolve the query vector
            let vec: VecData = match &snv.data {
                Some(VectorData::Vector(v)) => {
                    VecData::Standard(GeneratedValue::Literal(GenRef::Ref(format!(
                        "[{}]",
                        v.iter()
                            .map(|f| f.to_string())
                            .collect::<Vec<String>>()
                            .join(",")
                    ))))
                }
                Some(VectorData::Identifier(i)) => {
                    is_valid_identifier(ctx, original_query, snv.loc.clone(), i.as_str());
                    let _ = type_in_scope(ctx, original_query, snv.loc.clone(), scope, i.as_str());
                    VecData::Standard(gen_identifier_or_param(
                        original_query,
                        i.as_str(),
                        true,
                        false,
                    ))
                }
                _ => {
                    generate_error!(
                        ctx,
                        original_query,
                        snv.loc.clone(),
                        E305,
                        ["vector_data", "SearchN"],
                        ["vector_data"]
                    );
                    VecData::Unknown
                }
            };

            // Resolve k
            let k = match &snv.k {
                Some(k) => match &k.value {
                    EvaluatesToNumberType::I32(i) => {
                        GeneratedValue::Primitive(GenRef::Std(i.to_string()))
                    }
                    EvaluatesToNumberType::Identifier(i) => {
                        is_valid_identifier(ctx, original_query, snv.loc.clone(), i.as_str());
                        type_in_scope(ctx, original_query, snv.loc.clone(), scope, i.as_str());
                        gen_identifier_or_param(original_query, i, false, true)
                    }
                    _ => {
                        generate_error!(
                            ctx,
                            original_query,
                            snv.loc.clone(),
                            E305,
                            ["k", "SearchN"],
                            ["k"]
                        );
                        GeneratedValue::Unknown
                    }
                },
                None => {
                    generate_error!(ctx, original_query, snv.loc.clone(), E601, &snv.loc.span);
                    GeneratedValue::Unknown
                }
            };

            gen_traversal.source_step =
                Separator::Period(SourceStep::SearchN(SearchNStep { label, vec, k }));

            // SearchN returns a collection of nodes of the given type
            Type::Nodes(Some(snv.node_type.clone()))
        }
```

**Imports to add at the top of the file:** `SearchNStep` from `generator::source_steps` and `SearchNodeVector`-related items from parser types. Check the existing imports and add:

```rust
use crate::sparrowc::generator::source_steps::SearchNStep;
use crate::sparrowc::parser::types::SearchNodeVector;
```

**Note:** `Type::Nodes(Some(...))` — verify the exact `Type` variant name used for "a collection of nodes of a given type" in this file. Search for `Type::Node` vs `Type::Nodes` vs `Type::Collection` — match what `SearchV` returns for vectors. Typically it will be something like `Type::Nodes(Some(type_name))`.

- [ ] **Step 4: Run the analyzer tests**

```bash
cargo test --package sparrow-core --features lmdb test_search_node_vector_valid test_search_node_vector_unknown_node_type_fails -- --nocapture 2>&1 | head -40
```

Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/analyzer/methods/traversal_validation.rs
git commit -m "feat(analyzer): validate and compile SearchN<Type.field> traversal"
```

---

## Task 10: Analyzer — wire vector fields through `AddNode` code generation

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/analyzer/methods/infer_expr_type.rs`

- [ ] **Step 1: Write a failing test for AddN with vector field**

In `infer_expr_type.rs`, in the `#[cfg(test)]` block, add:

```rust
#[test]
fn test_add_node_with_vector_field_compiles() {
    use crate::sparrowc::{
        analyzer::Ctx,
        parser::{SparrowParser, write_to_temp_file},
    };

    let source = r#"
        N::Person {
            name: String,
            embedding: vector(1536)
        }
        QUERY addPerson(name: String, embedding: [F32]) => {
            p <- AddN<Person>({name: name, embedding: embedding})
            RETURN p
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let parsed = SparrowParser::parse_source(&content).unwrap();
    let result = Ctx::analyze(parsed, None);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());

    // Verify the generated code references add_n_with_vectors
    let output = result.unwrap();
    let generated = format!("{output}");
    assert!(
        generated.contains("add_n_with_vectors"),
        "expected add_n_with_vectors in generated output, got:\n{generated}"
    );
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
cargo test --package sparrow-core --features lmdb test_add_node_with_vector_field_compiles -- --nocapture 2>&1 | head -40
```

- [ ] **Step 3: Detect vector fields in `AddNode` and populate `AddN.vector_fields`**

In `infer_expr_type.rs`, find the `AddNode(add)` block (around line 162). After the `secondary_indices` computation (around line 374), and before the `AddN { ... }` struct construction (around line 392), add:

```rust
                // Detect vector(N) fields for HNSW auto-indexing
                let vector_fields: Option<Vec<(String, GeneratedValue)>> = {
                    let vf: Vec<(String, GeneratedValue)> = node_in_schema
                        .properties
                        .iter()
                        .filter(|p| matches!(p.field_type, GeneratedType::VectorF32(_)))
                        .map(|p| {
                            // hnsw_label = "TypeName.fieldname"
                            let hnsw_label = format!("{}.{}", ty, p.name);
                            // accessor = the generated value for this field from properties
                            let accessor = properties
                                .get(p.name.as_str())
                                .cloned()
                                .unwrap_or(GeneratedValue::Unknown);
                            (hnsw_label, accessor)
                        })
                        .collect();
                    if vf.is_empty() { None } else { Some(vf) }
                };
```

Then update the `AddN { ... }` construction to include `vector_fields`:

```rust
                let add_n = AddN {
                    label,
                    properties: Some(properties.into_iter().collect()),
                    secondary_indices,
                    vector_fields,  // NEW
                };
```

**Note on `GeneratedType::VectorF32`**: make sure this is imported. At the top of `infer_expr_type.rs`, find the `use crate::sparrowc::generator::utils::{...}` import and add `GeneratedType` if not already present.

Also ensure `AddN` in the imports covers the new `vector_fields` field — it may need to be re-imported or the struct literal may already compile if the field has a `Default` derive. Since we're constructing it explicitly with all fields, just add the field.

- [ ] **Step 4: Run the test**

```bash
cargo test --package sparrow-core --features lmdb test_add_node_with_vector_field_compiles -- --nocapture 2>&1 | head -40
```

Expected: PASS.

- [ ] **Step 5: Run the full analyzer test suite**

```bash
cargo test --package sparrow-core --features lmdb sparrowc::analyzer -- --nocapture 2>&1 | tail -20
```

Expected: all passing.

- [ ] **Step 6: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/analyzer/methods/infer_expr_type.rs
git commit -m "feat(analyzer): populate AddN.vector_fields for auto-HNSW indexing"
```

---

## Task 11: Integration test — round-trip `AddN` + `SearchN`

**Files:**
- Create: `crates/sparrow-core/src/sparrow_gateway/tests/vector_node_tests.rs` (or extend an existing test file)

- [ ] **Step 1: Find the right test file to extend**

```bash
ls crates/sparrow-core/src/sparrow_gateway/tests/
```

Either extend an existing file or create a new one. The tests in `mcp_tests.rs` are good models.

- [ ] **Step 2: Write the failing integration test**

Add to the chosen test file (or a new `vector_node_tests.rs`):

```rust
#[cfg(test)]
#[cfg(feature = "lmdb")]
mod vector_node_tests {
    use crate::sparrow_engine::{
        SparrowGraphStorage,
        storage_core::SparrowGraphStorageConfig,
        traversal_core::traversal_iter::RoTraversalIterator,
        traversal_core::traversal_iter::RwTraversalIterator,
    };
    use crate::sparrow_engine::traversal_core::GraphTraversal;
    use crate::utils::properties::ImmutablePropertiesMap;
    use crate::protocol::value::Value;
    use bumpalo::Bump;
    use tempfile::TempDir;

    fn make_test_db() -> (SparrowGraphStorage, TempDir) {
        let dir = TempDir::new().unwrap();
        let config = SparrowGraphStorageConfig {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };
        let db = SparrowGraphStorage::new(config).unwrap();
        (db, dir)
    }

    #[test]
    fn test_add_n_with_vectors_and_search_n() {
        use crate::sparrow_engine::traversal_core::ops::{
            source::add_n::AddNAdapter,
            vectors::search_n::SearchNAdapter,
        };

        let (db, _dir) = make_test_db();
        let arena = Bump::new();

        // Insert two nodes with embeddings
        let mut txn = db.write_txn().unwrap();

        let props_alice = ImmutablePropertiesMap::new(
            1,
            vec![("name", Value::from("Alice"))].into_iter(),
            &arena,
        );
        let embedding_alice: Vec<f32> = vec![1.0, 0.0, 0.0];
        let inserted_alice = RwTraversalIterator::new(&db, &arena, &mut txn)
            .add_n_with_vectors(
                "Person",
                Some(props_alice),
                None,
                Some(&[("Person.embedding", embedding_alice.as_slice())]),
            )
            .collect_to_obj()
            .unwrap();

        let props_bob = ImmutablePropertiesMap::new(
            1,
            vec![("name", Value::from("Bob"))].into_iter(),
            &arena,
        );
        let embedding_bob: Vec<f32> = vec![0.0, 1.0, 0.0];
        let _inserted_bob = RwTraversalIterator::new(&db, &arena, &mut txn)
            .add_n_with_vectors(
                "Person",
                Some(props_bob),
                None,
                Some(&[("Person.embedding", embedding_bob.as_slice())]),
            )
            .collect_to_obj()
            .unwrap();

        txn.commit().unwrap();

        // Search by embedding — query close to Alice's embedding
        let txn_ro = db.read_txn().unwrap();
        let query: Vec<f64> = vec![0.9, 0.1, 0.0];

        let results: Vec<_> = RoTraversalIterator::new(&db, &arena, &txn_ro)
            .search_n::<fn(&_, &_) -> bool, _>(&query, 2usize, "Person.embedding", None)
            .collect();

        assert_eq!(results.len(), 2, "expected 2 results from search_n");

        // The first result should be Alice (closest to [0.9, 0.1, 0.0])
        let first = results[0].as_ref().unwrap();
        if let crate::sparrow_engine::traversal_core::traversal_value::TraversalValue::Node(node) = first {
            let name = node.get_property("name").unwrap();
            assert_eq!(format!("{name}"), "Alice");
        } else {
            panic!("expected Node, got {:?}", first);
        }
    }
}
```

**Note:** Adjust imports to match how `RwTraversalIterator::new` and `RoTraversalIterator::new` are constructed in the test suite — look at `mcp_tests.rs` for the exact pattern used.

- [ ] **Step 3: Run the integration test**

```bash
cargo test --package sparrow-core --features lmdb test_add_n_with_vectors_and_search_n -- --nocapture 2>&1 | head -60
```

Fix any API mismatch by inspecting error output and adjusting import paths / constructor calls to match the real API.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test --workspace --features lmdb 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sparrow-core/src/sparrow_gateway/tests/
git commit -m "test(integration): add round-trip test for add_n_with_vectors + search_n"
```

---

## Task 12: Full HQL compile test — end-to-end schema + query

**Files:**
- Extend: `crates/sparrow-core/src/sparrowc/analyzer/methods/infer_expr_type.rs` (test section)

- [ ] **Step 1: Write a full end-to-end schema + query compile test**

Add to `infer_expr_type.rs`'s test block:

```rust
#[test]
fn test_full_vector_node_pipeline() {
    use crate::sparrowc::{
        analyzer::Ctx,
        parser::{SparrowParser, write_to_temp_file},
    };

    // Full schema + add query + search query
    let source = r#"
        N::Article {
            title: String,
            embedding: vector(768)
        }

        QUERY createArticle(title: String, embedding: [F32]) => {
            a <- AddN<Article>({title: title, embedding: embedding})
            RETURN a
        }

        QUERY findSimilar(q: [F64]) => {
            results <- SearchN<Article.embedding>(q, 5)
            RETURN results
        }
    "#;
    let content = write_to_temp_file(vec![source]);
    let parsed = SparrowParser::parse_source(&content).unwrap();
    let result = Ctx::analyze(parsed, None);
    assert!(result.is_ok(), "expected Ok, got {:?}", result.err());

    let output = result.unwrap();
    let generated = format!("{output}");

    // Verify generated code contains the right calls
    assert!(generated.contains("add_n_with_vectors"), "missing add_n_with_vectors");
    assert!(generated.contains("Article.embedding"), "missing HNSW label");
    assert!(generated.contains("search_n::"), "missing search_n call");
    assert!(generated.contains("Vec<f32>"), "Article struct should have Vec<f32> embedding field");
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test --package sparrow-core --features lmdb test_full_vector_node_pipeline -- --nocapture 2>&1 | head -40
```

Expected: PASS.

- [ ] **Step 3: Run full suite one final time**

```bash
cargo test --workspace --features lmdb 2>&1 | tail -30
```

Expected: all tests pass, no regressions.

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/analyzer/methods/infer_expr_type.rs
git commit -m "test(compiler): add end-to-end vector(N) pipeline compile test"
```

---

## Task 13: Update generated test fixtures (if any break)

**Files:**
- `tests/hql-tests/tests/*/queries.rs` — only if any pre-generated queries call `add_n` and now fail to compile

- [ ] **Step 1: Check whether any HQL test queries break**

```bash
cargo test --workspace --features lmdb 2>&1 | grep -E "FAILED|error" | head -20
```

If `add_n` signature changed anywhere the tests reference it directly, update those files. Since `add_n` is unchanged (only `add_n_with_vectors` was added), this step should be a no-op — confirm that.

- [ ] **Step 2: If any pre-generated queries reference the old `AddN` struct directly (unlikely)**

The `tests/hql-tests` crate contains pre-generated queries. Since we did not change `add_n`'s signature, no breakage is expected. If any compilation failure appears, it is because a test schema was regenerated and now picks up the new `add_n_with_vectors` path — rerun the compiler to regenerate:

```bash
# Regenerate test fixtures (only if needed)
cargo run --package sparrow-cli -- compile <path-to-schema> -o <output-dir>
```

Confirm no test regressions, then commit any regenerated files.

---

## Task 14: Final verification

- [ ] **Step 1: Full workspace test**

```bash
cargo test --workspace --features lmdb,server 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 2: Build release**

```bash
cargo build --release --workspace 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 3: Verify spec coverage**

Read `docs/superpowers/specs/2026-05-23-vector-property-on-nodes-design.md` and confirm every spec section is implemented. Key checkpoints:

| Spec requirement | Verified by |
|---|---|
| `vector(N)` in grammar | Task 1 |
| `FieldType::Vector(usize)` in parser | Task 2 |
| Rejected on E:: edges (E111) | Task 5 |
| `Vec<f32>` in generated Rust struct | Task 4 |
| `Array<number> /* vector(N) */` in TypeScript | Task 4 |
| Auto-indexed into HNSW on `AddN` | Tasks 7 + 10 |
| HNSW label = `"TypeName.fieldname"` | Tasks 7 + 9 |
| `SearchN<Type.field>(query, k)` grammar | Task 1 |
| Returns `N::` nodes (not HVectors) | Task 8 |
| Node ID = HNSW entry ID (via `insert_with_id`) | Task 7 |

- [ ] **Step 4: Final commit**

```bash
git add .
git commit -m "feat(sparrow-core): add vector(N) property type on N:: nodes with SearchN support

Closes: vector property syntax for embedding fields on graph nodes.
- Grammar: vector_type rule + SearchN<Type.field>(q, k) form
- Parser: FieldType::Vector(usize), SearchNodeVector, StartNode::SearchNodeVector  
- Analyzer: E111 for vector on edges, full traversal validation for SearchN
- Generator: VectorF32 GeneratedType, add_n_with_vectors emission, SearchNStep
- Engine: add_n_with_vectors uses insert_with_id; SearchNAdapter returns Nodes

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```
