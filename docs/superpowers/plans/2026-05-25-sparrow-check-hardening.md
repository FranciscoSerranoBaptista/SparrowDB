# sparrow check Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden `sparrow check` so generated `queries.rs` is always validated before being written to disk, with a version stamp, HQL-attributed error messages, and a new `sparrow doctor` pre-flight command.

**Architecture:** Four ordered stages: (0) HQL analysis returns a code string in-memory, (1) self-assertions scan it for known-bad patterns, (2) cargo check runs against a temp file in the hermetic pinned workspace, (3) atomic rename to the final path with a version stamp header. `sparrow doctor` is a separate command that runs in <2 seconds with no cargo invocations.

**Tech Stack:** Rust, tokio, `crates/sparrow-cli`, `crates/sparrow-core/src/sparrowc`, `std::fs`, `tokio::process::Command`.

---

## File Map

| Action | File |
|--------|------|
| Create | `crates/sparrow-cli/build.rs` |
| Modify | `crates/sparrow-core/src/sparrowc/generator/queries.rs` |
| Modify | `crates/sparrow-core/src/sparrowc/analyzer/methods/query_validation.rs` |
| Modify | `crates/sparrow-cli/src/commands/build.rs` |
| Modify | `crates/sparrow-cli/src/commands/check.rs` |
| Create | `crates/sparrow-cli/src/commands/doctor.rs` |
| Modify | `crates/sparrow-cli/src/commands/mod.rs` |
| Modify | `crates/sparrow-cli/src/main.rs` |

---

## Codebase Context

**`check.rs` current flow (to be replaced in Task 5):**
1. `validate_project_syntax()` — parse + analyze .hx files, returns nothing on success
2. `ensure_sparrow_repo_cached()` — syncs `~/.sparrow/repo`
3. `prepare_instance_workspace()` — copies from global cache into instance workspace
4. `compile_project()` — generates `queries.rs` directly to disk at `<instance>/.sparrow/<name>/sparrow-container/src/queries.rs`
5. Copies generated file into `sparrow-repo-copy/crates/sparrow-container/src/` for cargo check
6. Runs `cargo check --color=never --target-dir .sparrow/check-cache` in `sparrow-repo-copy/crates/sparrow-container/`
7. Restores original files in `sparrow-repo-copy`

**`build.rs` compile functions (relevant to Task 3):**
- `compile_helix_files(files, instance_src_dir) -> Result<(GeneratedSource, MetricsData)>` — parses + analyzes HQL, reads config, returns IR
- `compile_project(project, instance_name) -> Result<MetricsData>` — calls above, formats IR as string, writes to disk
- `get_sparrow_repo_cache() -> Result<PathBuf>` — in `project.rs`, returns `~/.sparrow/repo`

**`generator::queries::Query` struct (to be modified in Task 2):**
- File: `crates/sparrow-core/src/sparrowc/generator/queries.rs`
- `pub name: String` — already there
- `fn print_query()` at line 109 — generates the Rust function; the function body starts after `writeln!(f, "pub fn {} (input: HandlerInput) -> ...", self.name)?;` at line 120
- First line after function signature: `writeln!(f, "let db = Arc::clone(&input.graph.storage);")?;`

**`parser::types::Query` (source of Loc info for Task 2):**
- File: `crates/sparrow-core/src/sparrowc/parser/types.rs`
- `pub loc: Loc` — has `loc.filepath: Option<String>` and `loc.start.line: usize`

**`validate_query()` in analyzer (Task 2 call site):**
- File: `crates/sparrow-core/src/sparrowc/analyzer/methods/query_validation.rs`, line 1157
- Takes `original_query: &'a Query` (the parser Query with Loc)
- Constructs `GeneratedQuery { name: original_query.name.clone(), ..Default::default() }`

---

### Task 1: Embed `SPARROW_BUILD_HASH` via `build.rs`

**Files:**
- Create: `crates/sparrow-cli/build.rs`

The Rust build system runs `build.rs` before compilation. It can print `cargo:rustc-env=VAR=value` to set compile-time env vars. No extra `[build-dependencies]` are needed — `std::process::Command` is available in build scripts.

- [ ] **Step 1: Create the build script**

```rust
// crates/sparrow-cli/build.rs
fn main() {
    let hash = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=SPARROW_BUILD_HASH={hash}");
    // Re-run if commits or staged changes happen
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
```

- [ ] **Step 2: Write a compile-time assertion test**

Add to `crates/sparrow-cli/src/lib.rs` (or any existing `#[cfg(test)]` module):

```rust
#[cfg(test)]
mod build_hash_tests {
    #[test]
    fn sparrow_build_hash_is_set() {
        let hash = env!("SPARROW_BUILD_HASH");
        assert!(!hash.is_empty(), "SPARROW_BUILD_HASH must not be empty");
        // "unknown" is acceptable in environments without git
        // but the variable must always be set
    }
}
```

- [ ] **Step 3: Run the test to verify it passes**

```bash
cargo test --package sparrow-cli sparrow_build_hash_is_set
```

Expected: `PASS` — the env var is set by build.rs.

- [ ] **Step 4: Commit**

```bash
git add crates/sparrow-cli/build.rs crates/sparrow-cli/src/lib.rs
git commit -m "feat(cli): embed SPARROW_BUILD_HASH via build.rs using git describe --always --dirty"
```

---

### Task 2: Add Query Source Location Marker

**Files:**
- Modify: `crates/sparrow-core/src/sparrowc/generator/queries.rs`
- Modify: `crates/sparrow-core/src/sparrowc/analyzer/methods/query_validation.rs`

The `generator::queries::Query` struct needs a `source_loc` field. When serialized, the first line of each query's function body should be a `// sparrow:query=NAME source=FILE:LINE` comment. This comment is used by `check.rs` Stage 2 to attribute cargo check errors back to the HQL source.

- [ ] **Step 1: Write the failing test**

At the bottom of `crates/sparrow-core/src/sparrowc/generator/queries.rs`, add:

```rust
#[cfg(test)]
mod marker_tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn query_emits_source_marker_in_function_body() {
        let q = Query {
            name: "GetUser".to_string(),
            source_loc: Some("users.hx:12".to_string()),
            ..Default::default()
        };
        let rendered = format!("{q}");
        assert!(
            rendered.contains("// sparrow:query=GetUser source=users.hx:12"),
            "Expected source marker in rendered output, got:\n{rendered}"
        );
    }

    #[test]
    fn query_without_source_loc_omits_marker() {
        let q = Query {
            name: "GetUser".to_string(),
            source_loc: None,
            ..Default::default()
        };
        let rendered = format!("{q}");
        assert!(
            !rendered.contains("// sparrow:query="),
            "Expected no marker when source_loc is None"
        );
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test --package sparrow-core --features compiler -- marker_tests
```

Expected: FAIL — `source_loc` field doesn't exist yet.

- [ ] **Step 3: Add `source_loc` field to `generator::queries::Query`**

In `crates/sparrow-core/src/sparrowc/generator/queries.rs`, find the `pub struct Query` at line 9 and add the field:

```rust
pub struct Query {
    pub embedding_model_to_use: Option<String>,
    pub mcp_handler: Option<String>,
    pub name: String,
    pub source_loc: Option<String>,  // ← add this line
    pub statements: Vec<Statement>,
    pub parameters: Vec<Parameter>,
    pub sub_parameters: Vec<(String, Vec<Parameter>)>,
    pub return_values: Vec<(String, ReturnValue)>,
    pub return_structs: Vec<ReturnValueStruct>,
    pub use_struct_returns: bool,
    pub is_mut: bool,
    pub hoisted_embedding_calls: Vec<EmbedData>,
}
```

Also add `source_loc: None` to the `Default` impl at line 2045:

```rust
impl Default for Query {
    fn default() -> Self {
        Self {
            embedding_model_to_use: None,
            mcp_handler: None,
            name: "".to_string(),
            source_loc: None,  // ← add this line
            statements: vec![],
            parameters: vec![],
            sub_parameters: vec![],
            return_values: vec![],
            return_structs: vec![],
            use_struct_returns: true,
            is_mut: false,
            hoisted_embedding_calls: vec![],
        }
    }
}
```

- [ ] **Step 4: Emit the marker in `print_query()`**

In `fn print_query()` at line 109, immediately after the line that writes the function signature (around line 123), add the marker emission. The function signature line currently is:

```rust
writeln!(
    f,
    "pub fn {} (input: HandlerInput) -> Result<Response, GraphError> {{",
    self.name
)?;
```

Add this block right after it (before the `let db = Arc::clone(...)` line):

```rust
// Emit source attribution marker for cargo check error attribution
if let Some(loc) = &self.source_loc {
    writeln!(f, "    // sparrow:query={} source={}", self.name, loc)?;
}
```

- [ ] **Step 5: Populate `source_loc` in the analyzer**

In `crates/sparrow-core/src/sparrowc/analyzer/methods/query_validation.rs`, find `validate_query()` at line 1157. The construction of `GeneratedQuery` uses:

```rust
let mut query = GeneratedQuery {
    name: original_query.name.clone(),
    ..Default::default()
};
```

Change it to:

```rust
// Build source location string from the parser Loc struct.
// filepath is the full canonicalized path; use only the filename component.
let source_loc = {
    let line = original_query.loc.start.line;
    match &original_query.loc.filepath {
        Some(fp) => {
            let filename = std::path::Path::new(fp)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| fp.clone());
            Some(format!("{filename}:{line}"))
        }
        None => Some(format!("queries.hx:{line}")),
    }
};

let mut query = GeneratedQuery {
    name: original_query.name.clone(),
    source_loc,
    ..Default::default()
};
```

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cargo test --package sparrow-core --features compiler -- marker_tests
```

Expected: PASS — both marker tests pass.

- [ ] **Step 7: Run the full compiler test suite**

```bash
cargo test --package sparrow-core --features compiler
```

Expected: All tests pass (346 compiler tests as of last run).

- [ ] **Step 8: Commit**

```bash
git add crates/sparrow-core/src/sparrowc/generator/queries.rs \
        crates/sparrow-core/src/sparrowc/analyzer/methods/query_validation.rs
git commit -m "feat(generator): emit sparrow:query= source marker in generated function bodies"
```

---

### Task 3: Add `compile_to_string()` to `build.rs`

**Files:**
- Modify: `crates/sparrow-cli/src/commands/build.rs`

`compile_project()` currently generates the code string and immediately writes it to disk. We need a variant that returns the string without writing, so `check.rs` can run assertions and temp-file cargo check before deciding whether to write the final file.

- [ ] **Step 1: Write the failing test**

In `crates/sparrow-cli/src/commands/build.rs`, at the bottom, add:

```rust
#[cfg(test)]
mod compile_to_string_tests {
    // compile_to_string is hard to unit test without real .hx files,
    // so we just verify the function exists and has the right signature.
    // Integration tests in tests/ cover the full round-trip.
    use super::*;

    #[test]
    fn compile_to_string_fn_exists() {
        // This is a compile-time test: if compile_to_string doesn't exist,
        // this file won't compile.
        let _: fn(&ProjectContext, &str) -> eyre::Result<(String, MetricsData)>
            = compile_to_string;
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```bash
cargo test --package sparrow-cli -- compile_to_string_fn_exists
```

Expected: FAIL — function doesn't exist yet.

- [ ] **Step 3: Add `compile_to_string()` to `build.rs`**

`compile_to_string` must mirror `compile_project` exactly up to the write step.
`compile_helix_files` reads `config.hx.json` from `instance_src_dir`, so that file must be
written before `compile_helix_files` is called — just as `compile_project` does.

Add this function in `crates/sparrow-cli/src/commands/build.rs` immediately after `compile_project()`:

```rust
/// Compile the project and return the generated Rust code as a string,
/// without writing `queries.rs` to disk.
///
/// Writes `config.hx.json` to the instance src dir (required by `compile_helix_files`)
/// but does NOT write `queries.rs` — the caller validates and decides the final path.
///
/// Used by `sparrow check` Stage 0.
pub(crate) fn compile_to_string(
    project: &ProjectContext,
    instance_name: &str,
) -> Result<(String, MetricsData)> {
    let instance_workspace = project.instance_workspace(instance_name);
    let helix_container_dir = instance_workspace.join("sparrow-container");
    let src_dir = helix_container_dir.join("src");
    fs::create_dir_all(&src_dir)?;

    // Write config.hx.json — required by compile_helix_files to read instance config
    let instance = project.config.get_instance(instance_name)?;
    let legacy_config_json = instance.to_legacy_json();
    let legacy_config_str = serde_json::to_string_pretty(&legacy_config_json)?;
    fs::write(src_dir.join("config.hx.json"), legacy_config_str)?;

    let hx_files = collect_hx_files(&project.root, &project.config.project.queries)?;
    let (analyzed_source, metrics_data) = compile_helix_files(&hx_files, &src_dir)?;

    let mut code = String::new();
    std::fmt::write(&mut code, format_args!("{analyzed_source}"))
        .map_err(|e| eyre::eyre!("Failed to format generated code: {e}"))?;

    Ok((code, metrics_data))
}
```

- [ ] **Step 4: Run test to confirm it passes**

```bash
cargo test --package sparrow-cli -- compile_to_string_fn_exists
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sparrow-cli/src/commands/build.rs
git commit -m "feat(cli/build): add compile_to_string() for in-memory code generation without disk write"
```

---

### Task 4: Stage 1 Self-Assertions

**Files:**
- Modify: `crates/sparrow-cli/src/commands/check.rs`

Stage 1 assertions catch known-bad codegen patterns before paying the `cargo check` penalty. They scan the generated code string with regex-free string patterns.

Two assertions to implement:
1. `n_from_index(` calls must not have `.clone()` in the third argument — this was Bug B; the generator now uses `into_ref_key()` but the assertion ensures it stays fixed.
2. The string must not contain `/* UNKNOWN */` or `"UNKNOWN"` from unknown `GeneratedValue` variants.

- [ ] **Step 1: Write the failing tests**

Add to `check.rs`:

```rust
#[cfg(test)]
mod assertion_tests {
    use super::*;

    #[test]
    fn assertion_passes_on_clean_code() {
        let code = r#"
pub fn get_user(input: HandlerInput) -> Result<Response, GraphError> {
    // sparrow:query=GetUser source=users.hx:1
    let db = Arc::clone(&input.graph.storage);
    let r = db.n_from_index("User", "slug", &data.slug);
}
"#;
        assert!(check_codegen_assertions(code).is_ok());
    }

    #[test]
    fn assertion_catches_clone_in_n_from_index() {
        let code = r#"
    let r = db.n_from_index("User", "slug", data.slug.clone());
"#;
        let result = check_codegen_assertions(code);
        assert!(result.is_err(), "Expected assertion error for .clone() in n_from_index");
        assert!(result.unwrap_err().to_string().contains("n_from_index"));
    }

    #[test]
    fn assertion_catches_unknown_generated_value() {
        let code = r#"
    let val = /* UNKNOWN */;
"#;
        let result = check_codegen_assertions(code);
        assert!(result.is_err(), "Expected assertion error for UNKNOWN generated value");
    }
}
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cargo test --package sparrow-cli -- assertion_tests
```

Expected: FAIL — `check_codegen_assertions` doesn't exist yet.

- [ ] **Step 3: Implement `check_codegen_assertions()`**

Add this function to `crates/sparrow-cli/src/commands/check.rs`:

```rust
/// Stage 1: scan generated Rust code for known-bad structural patterns.
///
/// Runs in-process at ~0ms. Catches known codegen bug classes before paying
/// the 30–60s cargo check penalty.
///
/// If this fires, it means a known bug has regressed. The error message
/// explicitly attributes it as a SparrowDB bug so the user knows not to
/// fix their HQL.
fn check_codegen_assertions(code: &str) -> Result<()> {
    // Assertion 1: n_from_index key must not be a `.clone()` expression.
    // The key parameter is the third argument (after two string literals).
    // Pattern: n_from_index(<str>, <str>, <expr>.clone())
    // A quick scan: find every "n_from_index(" and check if the rest of
    // the call (up to the closing paren) contains ".clone()".
    for (line_num, line) in code.lines().enumerate() {
        let line_num = line_num + 1;
        if let Some(idx) = line.find("n_from_index(") {
            let after = &line[idx..];
            // Find the third comma (separating third arg from second)
            let mut comma_count = 0;
            let mut third_arg_start = None;
            for (i, ch) in after.char_indices() {
                if ch == ',' {
                    comma_count += 1;
                    if comma_count == 2 {
                        third_arg_start = Some(i + 1);
                        break;
                    }
                }
            }
            if let Some(start) = third_arg_start {
                let third_arg = &after[start..];
                if third_arg.contains(".clone()") {
                    return Err(eyre::eyre!(
                        "codegen bug [Stage 1 assertion]: n_from_index key argument contains \
                         .clone() on line {line_num}.\n\
                         This means the WHERE optimizer emitted an owned value instead of a \
                         reference for the index key.\n\
                         This is a SparrowDB bug. Please report with your .hx files at: \
                         https://github.com/SparrowDB/sparrowdb/issues\n\
                         Run `sparrow check --debug-codegen` to inspect the full generated output."
                    ));
                }
            }
        }

        // Assertion 2: no UNKNOWN GeneratedValue placeholders.
        if line.contains("/* UNKNOWN */") || line.contains("\"UNKNOWN\"") {
            return Err(eyre::eyre!(
                "codegen bug [Stage 1 assertion]: unknown GeneratedValue on line {line_num}.\n\
                 The code generator produced a placeholder instead of a real Rust expression.\n\
                 This is a SparrowDB bug. Please report with your .hx files at: \
                 https://github.com/SparrowDB/sparrowdb/issues\n\
                 Run `sparrow check --debug-codegen` to inspect the full generated output."
            ));
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo test --package sparrow-cli -- assertion_tests
```

Expected: All 3 assertion tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sparrow-cli/src/commands/check.rs
git commit -m "feat(cli/check): add Stage 1 self-assertions for known codegen bug patterns"
```

---

### Task 5: Rewrite `check.rs` — 4-Stage Pipeline

**Files:**
- Modify: `crates/sparrow-cli/src/commands/check.rs`

This is the core change. Replace the current `check_instance()` flow with the 4-stage pipeline:
- **Stage 0**: HQL analysis returns code string (no disk write)
- **Version stamp check**: compare existing header hash with `SPARROW_BUILD_HASH`
- **Stage 1**: self-assertions on the string
- **Stage 2**: write to temp file, cargo check with HQL-attributed errors
- **Stage 3**: prepend version stamp header, atomic rename

The version stamp header (written at Stage 3) has this format:
```rust
// generated by sparrow <hash>
// sparrow-core workspace: <hash>
// Do not edit — regenerate with `sparrow check`
```

- [ ] **Step 1: Add version stamp helpers**

Add these functions to `check.rs` before `check_instance()`:

```rust
const VERSION_STAMP_PREFIX: &str = "// generated by sparrow ";

/// Write the 3-line version stamp to the beginning of a code string.
fn prepend_version_stamp(code: &str) -> String {
    let hash = env!("SPARROW_BUILD_HASH");
    let stamp = format!(
        "// generated by sparrow {hash}\n\
         // sparrow-core workspace: {hash}\n\
         // Do not edit — regenerate with `sparrow check`\n",
    );
    format!("{stamp}{code}")
}

/// Read the hash from an existing queries.rs file's first line, if present.
/// Returns `None` if the file doesn't exist or has no version stamp.
fn read_existing_stamp_hash(queries_rs: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(queries_rs).ok()?;
    let first_line = content.lines().next()?;
    let hash = first_line.strip_prefix(VERSION_STAMP_PREFIX)?;
    Some(hash.trim().to_string())
}
```

- [ ] **Step 2: Write integration test for stamp helpers**

```rust
#[cfg(test)]
mod stamp_tests {
    use super::*;

    #[test]
    fn prepend_version_stamp_adds_header() {
        let stamped = prepend_version_stamp("fn main() {}");
        assert!(stamped.starts_with("// generated by sparrow "));
        assert!(stamped.contains("// sparrow-core workspace: "));
        assert!(stamped.contains("// Do not edit"));
        assert!(stamped.ends_with("fn main() {}"));
    }

    #[test]
    fn read_existing_stamp_hash_parses_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queries.rs");
        std::fs::write(&path, "// generated by sparrow abc1234-dirty\nfn main() {}").unwrap();
        let hash = read_existing_stamp_hash(&path);
        assert_eq!(hash, Some("abc1234-dirty".to_string()));
    }

    #[test]
    fn read_existing_stamp_hash_returns_none_for_unstamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queries.rs");
        std::fs::write(&path, "// auto-generated\nfn main() {}").unwrap();
        let hash = read_existing_stamp_hash(&path);
        assert_eq!(hash, None);
    }
}
```

Run: `cargo test --package sparrow-cli -- stamp_tests`  
Expected: PASS.

- [ ] **Step 3: Add hermetic workspace check function**

The `sparrow-repo-copy` inside the instance workspace is what `cargo check` runs against. Before Stage 2, ensure it is at the binary's commit hash:

```rust
/// Check whether the sparrow-repo-copy workspace is pinned to the binary's commit hash.
/// If not (and if neither hash is "unknown"), check out the correct hash.
///
/// This ensures cargo check compiles against exactly the sparrow-core version the
/// binary was built from, preventing false-green or false-red cargo check results.
async fn ensure_hermetic_workspace(sparrow_repo_copy_dir: &std::path::Path) -> Result<()> {
    let binary_hash = env!("SPARROW_BUILD_HASH");
    if binary_hash == "unknown" {
        print_warning(
            "SPARROW_BUILD_HASH is 'unknown' — workspace hermetic pinning skipped. \
             cargo check may not use the same sparrow-core version as this binary."
        );
        return Ok(());
    }

    // Read current HEAD of the sparrow-repo-copy git repo
    let head_output = tokio::process::Command::new("git")
        .args(["-C", sparrow_repo_copy_dir.to_str().unwrap_or("."), "rev-parse", "--short=7", "HEAD"])
        .output()
        .await
        .map_err(|e| eyre::eyre!("Failed to read sparrow-repo-copy HEAD: {e}"))?;

    if !head_output.status.success() {
        // Not a git repo (e.g., created by non-git path) — skip pinning
        return Ok(());
    }

    let current_head = String::from_utf8_lossy(&head_output.stdout).trim().to_string();

    // Compare the short hash prefix of both (git describe may include tag prefix)
    // If binary_hash starts with "v" or a tag, extract the commit part
    let binary_short = binary_hash
        .split('-')
        .find(|part| part.len() >= 7 && part.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or(binary_hash);

    if current_head == binary_short || binary_hash.starts_with(&current_head) {
        return Ok(()); // Already at the right hash
    }

    print_warning(&format!(
        "sparrow-repo-copy is at {current_head} but binary is {binary_hash}. \
         Checking out {binary_hash} for hermetic cargo check…"
    ));

    let checkout = tokio::process::Command::new("git")
        .args(["-C", sparrow_repo_copy_dir.to_str().unwrap_or("."), "checkout", binary_hash])
        .output()
        .await
        .map_err(|e| eyre::eyre!("Failed to checkout {binary_hash}: {e}"))?;

    if !checkout.status.success() {
        print_warning(&format!(
            "Could not checkout {binary_hash} in sparrow-repo-copy. \
             cargo check may not be hermetic."
        ));
    }
    Ok(())
}
```

- [ ] **Step 4: Add HQL-attributed error extraction**

Replace the current `handle_cargo_check_failure()` with one that scans backward for `// sparrow:query=` markers:

```rust
/// Given cargo check error output and the generated code, find the nearest
/// `// sparrow:query=NAME source=FILE:LINE` marker above each error location
/// and emit a human-readable attributed message.
fn attribute_cargo_errors(errors_only: &str, generated_code: &str) -> String {
    let code_lines: Vec<&str> = generated_code.lines().collect();
    let mut attributed = Vec::new();

    for error_line in errors_only.lines() {
        // Cargo error lines look like: "error[E0308]: ... --> src/queries.rs:42:5"
        let line_num: Option<usize> = error_line
            .split("queries.rs:")
            .nth(1)
            .and_then(|s| s.split(':').next())
            .and_then(|n| n.parse::<usize>().ok());

        let query_attr = line_num.and_then(|n| {
            // Scan backward from line N for the nearest sparrow:query= marker
            let idx = n.saturating_sub(1); // convert to 0-based
            code_lines[..idx.min(code_lines.len())]
                .iter()
                .rev()
                .find(|l| l.contains("// sparrow:query="))
                .map(|l| l.trim().to_string())
        });

        match query_attr {
            Some(marker) => {
                // Parse "// sparrow:query=NAME source=FILE:LINE"
                let name = marker
                    .split("sparrow:query=").nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .unwrap_or("unknown");
                let source = marker
                    .split("source=").nth(1)
                    .unwrap_or("unknown");
                attributed.push(format!(
                    "codegen bug in query '{name}' ({source}):\n  {error_line}\n  \
                     This is a SparrowDB bug. Run `sparrow check --debug-codegen` to inspect output."
                ));
            }
            None => {
                attributed.push(format!(
                    "likely a codegen bug (no query marker found):\n  {error_line}"
                ));
            }
        }
    }
    attributed.join("\n")
}
```

- [ ] **Step 5: Rewrite `check_instance()` with the 4-stage pipeline**

Replace the entire `check_instance()` function with:

```rust
async fn check_instance(
    project: &ProjectContext,
    instance_name: &str,
    metrics_sender: &MetricsSender,
    debug_codegen: bool,
) -> Result<()> {
    let start_time = Instant::now();
    let op = Operation::new("Checking", instance_name);

    // Validate instance exists in config
    let _instance_config = project.config.get_instance(instance_name)?;

    // ── Version stamp comparison (before Stage 0) ─────────────────────
    let instance_workspace = project.instance_workspace(instance_name);
    let final_queries_rs = instance_workspace.join("sparrow-container/src/queries.rs");

    let binary_hash = env!("SPARROW_BUILD_HASH");
    if binary_hash != "unknown" {
        if let Some(existing_hash) = read_existing_stamp_hash(&final_queries_rs) {
            if existing_hash != binary_hash {
                print_warning(&format!(
                    "queries.rs was generated by sparrow {existing_hash} but you are running \
                     {binary_hash}. Regenerating…"
                ));
            }
        }
    }

    // ── Handle --debug-codegen flag ───────────────────────────────────
    if debug_codegen {
        let (code, _) = build::compile_to_string(project, instance_name)?;
        let debug_header = "// ⚠ DEBUG — UNVALIDATED OUTPUT — do not use in production\n\
                            // Generated with --debug-codegen. Run `sparrow check` for validated output.\n";
        fs::write(&final_queries_rs, format!("{debug_header}{code}"))?;
        crate::output::success(&format!("Debug output written to {}", final_queries_rs.display()));
        return Ok(());
    }

    // ── Stage 0: HQL analysis ─────────────────────────────────────────
    let mut syntax_step = Step::with_messages("Validating HQL", "HQL valid");
    syntax_step.start();

    // Sync repo + prepare workspace (needed for config reading inside compile_to_string)
    build::ensure_sparrow_repo_cached().await?;
    build::prepare_instance_workspace(project, instance_name).await?;

    // compile_to_string generates code as a string without writing queries.rs to disk.
    // It does write config.hx.json internally (required by compile_helix_files).
    let (generated_code, metrics_data) = build::compile_to_string(project, instance_name)
        .map_err(|e| {
            syntax_step.fail();
            op.failure();
            e
        })?;
    syntax_step.done_with_info(&format!("{} queries", metrics_data.num_of_queries));

    // ── Stage 1: Generator self-assertions ────────────────────────────
    let mut assert_step = Step::with_messages("Running codegen assertions", "Assertions passed");
    assert_step.start();
    check_codegen_assertions(&generated_code).map_err(|e| {
        assert_step.fail();
        op.failure();
        e
    })?;
    assert_step.done();

    // ── Stage 2: cargo check in hermetic workspace ────────────────────
    let cargo_check_src = instance_workspace
        .join("sparrow-repo-copy/crates/sparrow-container/src");
    let sparrow_container_dir = instance_workspace
        .join("sparrow-repo-copy/crates/sparrow-container");
    let check_target_dir = project.sparrow_dir.join("check-cache");

    // Hermetic pinning: ensure sparrow-repo-copy is at the binary's commit hash
    ensure_hermetic_workspace(&instance_workspace.join("sparrow-repo-copy")).await?;

    // Write to temp file (not the final destination)
    let temp_queries = cargo_check_src.join("queries.rs.tmp");
    let original_queries = fs::read(cargo_check_src.join("queries.rs")).ok();
    let original_config = fs::read(cargo_check_src.join("config.hx.json")).ok();

    // Also copy config.hx.json (required for cargo check to compile)
    let generated_config = src_dir.join("config.hx.json");
    if generated_config.exists() {
        fs::copy(&generated_config, cargo_check_src.join("config.hx.json"))?;
    }

    fs::write(&temp_queries, &generated_code)?;
    // Temporarily put it where cargo check expects it
    fs::copy(&temp_queries, cargo_check_src.join("queries.rs"))?;

    let mut cargo_step = Step::with_messages("Running cargo check", "Cargo check passed");
    cargo_step.start();
    Step::verbose_substep("Running cargo check on generated code…");

    let cargo_result = run_cargo_check(&sparrow_container_dir, &check_target_dir).await;

    // Restore sparrow-repo-copy originals regardless of outcome
    match original_queries {
        Some(content) => {
            if let Err(e) = fs::write(cargo_check_src.join("queries.rs"), &content) {
                print_warning(&format!("Failed to restore queries.rs in repo copy: {e}"));
            }
        }
        None => {
            let _ = fs::remove_file(cargo_check_src.join("queries.rs"));
        }
    }
    match original_config {
        Some(content) => {
            if let Err(e) = fs::write(cargo_check_src.join("config.hx.json"), &content) {
                print_warning(&format!("Failed to restore config.hx.json: {e}"));
            }
        }
        None => {
            let _ = fs::remove_file(cargo_check_src.join("config.hx.json"));
        }
    }
    // Clean up temp file
    let _ = fs::remove_file(&temp_queries);

    let cargo_output = cargo_result?;
    let compile_time = start_time.elapsed().as_secs() as u32;

    if !cargo_output.success {
        cargo_step.fail();
        op.failure();

        metrics_sender.send_compile_event(
            instance_name.to_string(),
            metrics_data.queries_string,
            metrics_data.num_of_queries,
            compile_time,
            false,
            Some(cargo_output.errors_only.clone()),
        );

        let attributed = attribute_cargo_errors(&cargo_output.errors_only, &generated_code);
        eprintln!("{attributed}");
        return Err(eyre::eyre!("sparrow check failed: codegen produced invalid Rust"));
    }

    cargo_step.done();

    // ── Stage 3: Atomic rename with version stamp ─────────────────────
    let stamped_code = prepend_version_stamp(&generated_code);
    // Write to a temp file next to the destination, then rename (atomic on POSIX)
    let final_tmp = final_queries_rs.with_extension("rs.new");
    fs::write(&final_tmp, &stamped_code)?;
    fs::rename(&final_tmp, &final_queries_rs)?;

    metrics_sender.send_compile_event(
        instance_name.to_string(),
        metrics_data.queries_string,
        metrics_data.num_of_queries,
        compile_time,
        true,
        None,
    );

    op.success();
    Ok(())
}
```

Also update the signature in `check_all_instances()` to pass `debug_codegen`:

```rust
async fn check_all_instances(
    project: &ProjectContext,
    metrics_sender: &MetricsSender,
    debug_codegen: bool,
) -> Result<()> {
    // ... same as before but pass debug_codegen to check_instance:
    for instance_name in &instances {
        check_instance(project, instance_name, metrics_sender, debug_codegen).await?;
    }
    // ...
}
```

Update `run()` to accept `debug_codegen: bool` and forward it:

```rust
pub async fn run(
    instance: Option<String>,
    metrics_sender: &MetricsSender,
    debug_codegen: bool,
) -> Result<()> {
    let project = ProjectContext::find_and_load(None)?;
    match instance {
        Some(instance_name) => check_instance(&project, &instance_name, metrics_sender, debug_codegen).await,
        None => check_all_instances(&project, metrics_sender, debug_codegen).await,
    }
}
```

- [ ] **Step 6: Add `--debug-codegen` flag to `Commands::Check` in `main.rs`**

Find the `Check` variant in `crates/sparrow-cli/src/main.rs`:

```rust
// Current:
Check {
    instance: Option<String>,
},

// Replace with:
Check {
    /// Instance to check (defaults to all instances)
    instance: Option<String>,
    /// Write generated code directly without validation (for debugging codegen bugs)
    #[arg(long)]
    debug_codegen: bool,
},
```

And update the dispatch in the match arm:

```rust
// Current:
Commands::Check { instance } => commands::check::run(instance, &metrics_sender).await,

// Replace with:
Commands::Check { instance, debug_codegen } => {
    commands::check::run(instance, &metrics_sender, debug_codegen).await
}
```

- [ ] **Step 7: Run the full test suite**

```bash
cargo test --package sparrow-cli
cargo test --package sparrow-core --features compiler
```

Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/sparrow-cli/src/commands/check.rs crates/sparrow-cli/src/main.rs
git commit -m "feat(cli/check): 4-stage pipeline — in-memory codegen, Stage1 assertions, hermetic cargo check, atomic rename"
```

---

### Task 6: `sparrow doctor` Command

**Files:**
- Create: `crates/sparrow-cli/src/commands/doctor.rs`
- Modify: `crates/sparrow-cli/src/commands/mod.rs`
- Modify: `crates/sparrow-cli/src/main.rs`

`sparrow doctor` is a standalone pre-flight command. No `cargo` invocations. Runs in under 2 seconds. Checks run in parallel where independent.

**Checks table:**
| Check | Source | Blocking |
|-------|--------|---------|
| CLI version | `env!("CARGO_PKG_VERSION")` + `env!("SPARROW_BUILD_HASH")` | No |
| `queries.rs` in sync | header hash == `SPARROW_BUILD_HASH` | Yes (for `sparrow push`) |
| Cached workspace | workspace HEAD == binary hash | Yes (for Stage 2 validation) |
| Docker running | `docker info` exit code | Yes (for `sparrow push`) |
| Instance health | `GET /diagnostics` on configured port | No |

Exit code 0: no blocking failures. Exit code 1: at least one blocking check failed.

- [ ] **Step 1: Create `doctor.rs`**

```rust
//! sparrow doctor — pre-flight health checklist.
//!
//! Runs in under 2 seconds. No cargo invocations.
//! Exit code 0 on success, 1 if any blocking check fails.

use crate::project::ProjectContext;
use eyre::Result;
use tokio::process::Command;

pub struct CheckResult {
    pub label: String,
    pub ok: bool,
    pub detail: String,
    pub hint: Option<String>,
    pub blocking: bool,
    pub skipped: Option<String>,
}

pub async fn run(json: bool) -> Result<()> {
    let mut results: Vec<CheckResult> = Vec::new();

    // Check 1: CLI version (informational)
    results.push(CheckResult {
        label: "CLI".to_string(),
        ok: true,
        detail: format!(
            "sparrow v{} ({})",
            env!("CARGO_PKG_VERSION"),
            env!("SPARROW_BUILD_HASH")
        ),
        hint: None,
        blocking: false,
        skipped: None,
    });

    // Load project context for instance-level checks (best effort)
    let project = ProjectContext::find_and_load(None).ok();

    // Check 2: queries.rs in sync with CLI (per instance)
    if let Some(ref project) = project {
        let binary_hash = env!("SPARROW_BUILD_HASH");
        let instances = project.config.list_instances();
        if instances.is_empty() {
            results.push(CheckResult {
                label: "queries.rs".to_string(),
                ok: true,
                detail: "no instances configured".to_string(),
                hint: None,
                blocking: false,
                skipped: None,
            });
        }
        for instance_name in &instances {
            let instance_workspace = project.instance_workspace(instance_name);
            let queries_rs = instance_workspace.join("sparrow-container/src/queries.rs");
            let (ok, detail, hint) = if binary_hash == "unknown" {
                (true, "version unknown — skipping hash comparison".to_string(), None)
            } else {
                match crate::commands::check::read_existing_stamp_hash_pub(&queries_rs) {
                    Some(h) if h == binary_hash => (
                        true,
                        format!("in sync with CLI ({h})"),
                        None,
                    ),
                    Some(h) => (
                        false,
                        format!("generated by sparrow {h}, CLI is {binary_hash}"),
                        Some("Run `sparrow check` to regenerate.".to_string()),
                    ),
                    None => (
                        false,
                        "no version stamp — run `sparrow check` first".to_string(),
                        Some("Run `sparrow check`.".to_string()),
                    ),
                }
            };
            results.push(CheckResult {
                label: format!("queries.rs ({instance_name})"),
                ok,
                detail,
                hint,
                blocking: true,
                skipped: None,
            });
        }
    } else {
        results.push(CheckResult {
            label: "queries.rs".to_string(),
            ok: true,
            detail: "no sparrow.toml found — skipping".to_string(),
            hint: None,
            blocking: false,
            skipped: None,
        });
    }

    // Check 3: Cached workspace HEAD (informational — affects hermetic validation)
    {
        let cache_check = check_cached_workspace().await;
        results.push(cache_check);
    }

    // Checks 4 + 5 run in parallel (Docker + instance health)
    let docker_future = check_docker();
    let instances_future = check_instances(&project);
    let (docker_result, mut instance_results) = tokio::join!(docker_future, instances_future);

    let docker_running = docker_result.ok;
    results.push(docker_result);

    // Instance health: skip if Docker not running
    if !docker_running {
        for mut r in instance_results.drain(..) {
            r.skipped = Some("Docker not available".to_string());
            results.push(r);
        }
    } else {
        results.extend(instance_results);
    }

    // Print or serialize
    if json {
        print_json(&results);
    } else {
        print_human(&results);
    }

    let has_blocking_failure = results.iter().any(|r| r.blocking && !r.ok && r.skipped.is_none());
    if has_blocking_failure {
        std::process::exit(1);
    }
    Ok(())
}

async fn check_cached_workspace() -> CheckResult {
    let binary_hash = env!("SPARROW_BUILD_HASH");
    if binary_hash == "unknown" {
        return CheckResult {
            label: "Cached workspace".to_string(),
            ok: true,
            detail: "version unknown — skipping".to_string(),
            hint: None,
            blocking: false,
            skipped: None,
        };
    }
    let cache_path = match crate::project::get_sparrow_repo_cache() {
        Ok(p) => p,
        Err(_) => {
            return CheckResult {
                label: "Cached workspace".to_string(),
                ok: false,
                detail: "could not determine cache path".to_string(),
                hint: Some("Run `sparrow build` to create the cache.".to_string()),
                blocking: true,
                skipped: None,
            };
        }
    };
    if !cache_path.exists() {
        return CheckResult {
            label: "Cached workspace".to_string(),
            ok: false,
            detail: "not present".to_string(),
            hint: Some("Run `sparrow build` to create the cache.".to_string()),
            blocking: true,
            skipped: None,
        };
    }
    let head = Command::new("git")
        .args(["-C", cache_path.to_str().unwrap_or("."), "rev-parse", "--short=7", "HEAD"])
        .output()
        .await;
    match head {
        Ok(o) if o.status.success() => {
            let head_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let pinned = binary_hash.starts_with(&head_str) || binary_hash == &head_str;
            CheckResult {
                label: "Cached workspace".to_string(),
                ok: pinned,
                detail: if pinned {
                    format!("present and pinned to {head_str}")
                } else {
                    format!("at {head_str}, binary is {binary_hash}")
                },
                hint: if pinned { None } else {
                    Some("Run `sparrow check` to re-pin the workspace.".to_string())
                },
                blocking: !pinned,
                skipped: None,
            }
        }
        _ => CheckResult {
            label: "Cached workspace".to_string(),
            ok: false,
            detail: "could not read cache HEAD".to_string(),
            hint: Some("Run `sparrow build` to recreate the cache.".to_string()),
            blocking: true,
            skipped: None,
        },
    }
}

async fn check_docker() -> CheckResult {
    let result = Command::new("docker")
        .arg("info")
        .output()
        .await;
    match result {
        Ok(o) if o.status.success() => CheckResult {
            label: "Docker".to_string(),
            ok: true,
            detail: "daemon running".to_string(),
            hint: None,
            blocking: false,
            skipped: None,
        },
        _ => CheckResult {
            label: "Docker".to_string(),
            ok: false,
            detail: "daemon not running".to_string(),
            hint: Some("Start Docker Desktop or Podman before running `sparrow push`.".to_string()),
            blocking: true,
            skipped: None,
        },
    }
}

async fn check_instances(project: &Option<ProjectContext>) -> Vec<CheckResult> {
    let Some(project) = project else {
        return vec![];
    };
    let instances = project.config.list_instances();
    let mut results = Vec::new();
    for instance_name in &instances {
        let instance_config = match project.config.get_instance(instance_name) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let port = instance_config.port.unwrap_or(6969);
        let url = format!("http://localhost:{port}/diagnostics");
        let result = match reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                // Try to extract node count from response
                let body = resp.text().await.unwrap_or_default();
                let nodes: Option<u64> = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["nodes"].as_u64());
                CheckResult {
                    label: format!("Instance '{instance_name}'"),
                    ok: true,
                    detail: match nodes {
                        Some(n) => format!("running on :{port} — {n} nodes"),
                        None => format!("running on :{port}"),
                    },
                    hint: None,
                    blocking: false,
                    skipped: None,
                }
            }
            _ => CheckResult {
                label: format!("Instance '{instance_name}'"),
                ok: false,
                detail: format!("not found on :{port}"),
                hint: Some(format!("Run `sparrow push {instance_name}` to deploy.")),
                blocking: false,
                skipped: None,
            },
        };
        results.push(result);
    }
    results
}

fn print_human(results: &[CheckResult]) {
    println!();
    let mut issues = 0;
    for r in results {
        if let Some(skip_reason) = &r.skipped {
            println!("  ? skipped: {}", skip_reason);
            continue;
        }
        if r.ok {
            println!("  ✓ {}: {}", r.label, r.detail);
        } else {
            println!("  ✗ {}: {}", r.label, r.detail);
            if let Some(hint) = &r.hint {
                println!("      → {hint}");
            }
            if r.blocking {
                issues += 1;
            }
        }
    }
    println!();
    if issues > 0 {
        println!("  {issues} blocking issue(s) found. Fix the ✗ items above before deploying.");
    } else {
        println!("  All checks passed.");
    }
    println!();
}

fn print_json(results: &[CheckResult]) {
    let obj: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "label": r.label,
                "ok": r.ok,
                "detail": r.detail,
                "blocking": r.blocking,
                "hint": r.hint,
                "skipped": r.skipped,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&serde_json::Value::Array(obj)).unwrap());
}
```

- [ ] **Step 2: Expose `read_existing_stamp_hash` as pub in `check.rs`**

In `check.rs`, rename (or add a pub wrapper for) `read_existing_stamp_hash`:

```rust
/// Public version used by `sparrow doctor`.
pub fn read_existing_stamp_hash_pub(queries_rs: &std::path::Path) -> Option<String> {
    read_existing_stamp_hash(queries_rs)
}
```

- [ ] **Step 3: Register `doctor` in `mod.rs`**

In `crates/sparrow-cli/src/commands/mod.rs`, add:

```rust
pub mod doctor;
```

- [ ] **Step 4: Register `Doctor` subcommand in `main.rs`**

In the `Commands` enum in `crates/sparrow-cli/src/main.rs`, add:

```rust
/// Pre-flight health checklist — check CLI, workspace, Docker, and instances
Doctor {
    /// Output as JSON (for CI)
    #[arg(long)]
    json: bool,
},
```

In the match arm dispatch:

```rust
Commands::Doctor { json } => commands::doctor::run(json).await,
```

- [ ] **Step 5: Write unit tests for doctor helpers**

In `doctor.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_human_does_not_panic_on_empty() {
        print_human(&[]);
    }

    #[test]
    fn print_human_shows_tick_for_ok() {
        let results = vec![CheckResult {
            label: "CLI".to_string(),
            ok: true,
            detail: "sparrow v3.0.0 (abc1234)".to_string(),
            hint: None,
            blocking: false,
            skipped: None,
        }];
        // Just verify no panic
        print_human(&results);
    }

    #[test]
    fn print_json_valid_json() {
        let results = vec![CheckResult {
            label: "Docker".to_string(),
            ok: false,
            detail: "daemon not running".to_string(),
            hint: Some("Start Docker Desktop.".to_string()),
            blocking: true,
            skipped: None,
        }];
        // Just verify no panic
        print_json(&results);
    }
}
```

Run: `cargo test --package sparrow-cli -- doctor`  
Expected: PASS.

- [ ] **Step 6: Run the full test suite**

```bash
cargo test --package sparrow-cli
```

Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/sparrow-cli/src/commands/doctor.rs \
        crates/sparrow-cli/src/commands/mod.rs \
        crates/sparrow-cli/src/commands/check.rs \
        crates/sparrow-cli/src/main.rs
git commit -m "feat(cli): add sparrow doctor pre-flight checklist command with parallel checks and --json output"
```

---

## Self-Review Checklist

After all tasks are complete:

1. Verify `cargo build --package sparrow-cli` succeeds
2. Verify `cargo test --package sparrow-cli` passes
3. Verify `cargo test --package sparrow-core --features compiler` passes
4. Run `sparrow check` against a real project and verify the 4-stage output appears
5. Run `sparrow doctor` and verify the checklist output format matches the spec
6. Verify `sparrow check --debug-codegen` writes the unvalidated header
