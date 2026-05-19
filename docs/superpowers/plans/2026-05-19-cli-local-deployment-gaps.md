# CLI Local Deployment Gaps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three gaps that prevent building and running a local bare-metal HelixDB binary with the RocksDB backend and correct build profiles.

**Architecture:** Three independent changes that compose into a complete local deployment story: (1) expose the `rocks` cargo feature in `helix-container` so the storage backend is selectable at build time, (2) wire a `StorageBackend` config field and `BuildMode` through the CLI's build pipeline so `--release` and `--features rocks` are passed correctly, (3) add a `helix run` command that executes a pre-built binary directly without Docker.

**Tech Stack:** Rust / Cargo features, Clap v4, `helix-container` (Axum server binary), `helix-db` (lmdb/rocks feature flags), `helix-cli` (config + build + Docker generation)

---

## File Map

| File | Change |
|------|--------|
| `helix-container/Cargo.toml` | Add `lmdb`/`rocks` features; disable `helix-db` default-features |
| `helix-cli/src/config.rs` | Add `StorageBackend` enum; add `storage_backend` field to `LocalInstanceConfig`; add `storage_backend()` on `InstanceInfo` |
| `helix-cli/src/commands/build.rs` | Update `build_binary_using_cargo` signature + body to pass `--release` / `--no-default-features --features rocks` |
| `helix-cli/src/docker.rs` | Update `generate_dockerfile` + `image_name` to incorporate storage backend |
| `helix-cli/src/main.rs` | Add `Commands::Run` variant |
| `helix-cli/src/commands/run.rs` | New file — bare-metal binary executor |
| `helix-cli/src/tests/docker_tests.rs` | Tests for new Dockerfile flag combinations |
| `helix-cli/src/tests/run_tests.rs` | New test file for `run` command logic |
| `helix-cli/src/tests/mod.rs` | Register `run_tests` module |

---

## Task 1: Add `rocks` feature to `helix-container`

**Files:**
- Modify: `helix-container/Cargo.toml`

Currently `helix-db` is pulled in with its default features (`lmdb`). Adding a `rocks` feature alongside would compile both backends simultaneously, which causes duplicate trait impl errors. The fix is to disable `helix-db`'s default features and declare `lmdb` and `rocks` as explicit features.

- [ ] **Step 1: Write the failing check**

Run this — it should currently fail because `rocks` isn't a valid feature:

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-container
cargo check --no-default-features --features rocks 2>&1 | head -5
```

Expected: error about unknown feature `rocks` or missing items.

- [ ] **Step 2: Update `helix-container/Cargo.toml`**

Current `[dependencies]` and `[features]`:
```toml
[dependencies]
helix-db = { path = "../helix-db" }

[features]
dev = ["helix-db/dev-instance"]
production = ["helix-db/production"]
```

Replace with:
```toml
[dependencies]
helix-db = { path = "../helix-db", default-features = false }

[features]
default = ["lmdb"]
lmdb = ["helix-db/lmdb"]
rocks = ["helix-db/rocks"]
dev = ["helix-db/dev-instance"]
production = ["helix-db/production"]
```

- [ ] **Step 3: Verify lmdb (default) still compiles**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-container
cargo check 2>&1 | tail -3
```

Expected: `Finished` with no errors.

- [ ] **Step 4: Verify rocks feature compiles**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-container
cargo check --no-default-features --features rocks 2>&1 | tail -3
```

Expected: `Finished` with no errors.

- [ ] **Step 5: Verify lmdb+dev and rocks+dev compile**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-container
cargo check --features dev 2>&1 | tail -3
cargo check --no-default-features --features rocks,dev 2>&1 | tail -3
```

Expected: both finish with no errors.

- [ ] **Step 6: Commit**

```bash
git -C /Users/franciscobaptista/Development/helix-db-snapshot add helix-container/Cargo.toml
git -C /Users/franciscobaptista/Development/helix-db-snapshot commit -m "feat(helix-container): add rocks feature flag for RocksDB backend selection"
```

---

## Task 2: Wire `StorageBackend` through the CLI build pipeline

**Files:**
- Modify: `helix-cli/src/config.rs`
- Modify: `helix-cli/src/commands/build.rs`
- Modify: `helix-cli/src/docker.rs`
- Modify: `helix-cli/src/tests/docker_tests.rs`

This task fixes both gap 1 (RocksDB unreachable) and gap 2 (`--release` never passed for `--bin` builds).

- [ ] **Step 1: Write failing tests in `helix-cli/src/tests/docker_tests.rs`**

Append to the end of the file:

```rust
#[test]
fn test_dockerfile_lmdb_release_uses_release_flag() {
    use crate::config::BuildMode;
    let (_temp_dir, mut context) = setup_test_project();
    context
        .config
        .local
        .get_mut("dev")
        .unwrap()
        .build_mode = BuildMode::Release;
    let docker = DockerManager::new(&context);
    let instance_config = context.config.get_instance("dev").unwrap();
    let dockerfile = docker.generate_dockerfile("dev", instance_config).unwrap();
    assert!(
        dockerfile.contains("cargo build --release\n") || dockerfile.contains("cargo build --release "),
        "release lmdb build should pass --release flag, got:\n{dockerfile}"
    );
    assert!(
        !dockerfile.contains("--features rocks"),
        "lmdb build must not include rocks feature"
    );
}

#[test]
fn test_dockerfile_rocks_release_uses_rocks_no_default_features() {
    use crate::config::{BuildMode, StorageBackend};
    let (_temp_dir, mut context) = setup_test_project();
    let dev = context.config.local.get_mut("dev").unwrap();
    dev.build_mode = BuildMode::Release;
    dev.storage_backend = StorageBackend::Rocks;
    let docker = DockerManager::new(&context);
    let instance_config = context.config.get_instance("dev").unwrap();
    let dockerfile = docker.generate_dockerfile("dev", instance_config).unwrap();
    assert!(
        dockerfile.contains("--no-default-features --features rocks"),
        "rocks release build should pass --no-default-features --features rocks, got:\n{dockerfile}"
    );
    assert!(
        dockerfile.contains("--release"),
        "rocks release build should also pass --release"
    );
}

#[test]
fn test_dockerfile_rocks_dev_uses_rocks_no_default_features() {
    use crate::config::{BuildMode, StorageBackend};
    let (_temp_dir, mut context) = setup_test_project();
    context
        .config
        .local
        .get_mut("dev")
        .unwrap()
        .storage_backend = StorageBackend::Rocks;
    // build_mode is Dev by default
    let docker = DockerManager::new(&context);
    let instance_config = context.config.get_instance("dev").unwrap();
    let dockerfile = docker.generate_dockerfile("dev", instance_config).unwrap();
    assert!(
        dockerfile.contains("--no-default-features --features rocks,dev"),
        "rocks dev build should pass --no-default-features --features rocks,dev, got:\n{dockerfile}"
    );
}

#[test]
fn test_image_name_differs_between_lmdb_and_rocks() {
    use crate::config::{BuildMode, StorageBackend};
    let (_temp_dir, context) = setup_test_project();
    let docker = DockerManager::new(&context);

    let lmdb_name = docker.image_name("dev", BuildMode::Release, StorageBackend::Lmdb);
    let rocks_name = docker.image_name("dev", BuildMode::Release, StorageBackend::Rocks);
    assert_ne!(lmdb_name, rocks_name, "rocks and lmdb images must have distinct names");
    assert!(rocks_name.contains("rocks"), "rocks image name should contain 'rocks'");
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-cli
cargo test --lib tests::docker_tests 2>&1 | grep -E "FAILED|error\[" | head -10
```

Expected: compile errors because `StorageBackend` doesn't exist yet.

- [ ] **Step 3: Add `StorageBackend` enum and field to `helix-cli/src/config.rs`**

After the `BuildMode` enum (around line 313), add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    #[default]
    Lmdb,
    Rocks,
}
```

In `LocalInstanceConfig` (around line 182), add the new field:

```rust
pub struct LocalInstanceConfig {
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default = "default_dev_build_mode")]
    pub build_mode: BuildMode,
    #[serde(default)]
    pub storage_backend: StorageBackend,
    #[serde(flatten)]
    pub db_config: DbConfig,
}
```

In the `InstanceInfo` impl block, add a `storage_backend()` method after `build_mode()` (around line 411):

```rust
pub fn storage_backend(&self) -> StorageBackend {
    match self {
        InstanceInfo::Local(LocalInstanceConfig { storage_backend, .. }) => *storage_backend,
        _ => StorageBackend::Lmdb,
    }
}
```

- [ ] **Step 4: Update `generate_dockerfile` in `helix-cli/src/docker.rs`**

The current `build_flag` / `build_mode` block (lines 480–493) becomes:

```rust
let (build_flag, build_mode_str) = match (instance_config.build_mode(), instance_config.storage_backend()) {
    (BuildMode::Debug, _) => unreachable!(
        "Please report as a bug. BuildMode::Debug should have been caught in validation."
    ),
    (BuildMode::Release, StorageBackend::Lmdb) => {
        ("--release".to_string(), "release")
    }
    (BuildMode::Dev, StorageBackend::Lmdb) => {
        ("--features dev".to_string(), "debug")
    }
    (BuildMode::Release, StorageBackend::Rocks) => {
        ("--release --no-default-features --features rocks".to_string(), "release")
    }
    (BuildMode::Dev, StorageBackend::Rocks) => {
        ("--no-default-features --features rocks,dev".to_string(), "debug")
    }
};
```

Replace the two `build_flag` / `build_mode` variable references in the template string:
- `{build_flag}` stays as-is (already in the template)
- `{build_mode}` → rename the local variable to `build_mode_str` and update the template to `{build_mode_str}`

Add the import at the top of `docker.rs` if not already present:
```rust
use crate::config::StorageBackend;
```

Update `image_name` to include storage backend in the tag:

```rust
pub(crate) fn image_name(&self, instance_name: &str, build_mode: BuildMode, storage_backend: StorageBackend) -> String {
    let tag = match (build_mode, storage_backend) {
        (BuildMode::Debug, StorageBackend::Lmdb) => "debug".to_string(),
        (BuildMode::Release, StorageBackend::Lmdb) => "latest".to_string(),
        (BuildMode::Dev, StorageBackend::Lmdb) => "dev".to_string(),
        (BuildMode::Debug, StorageBackend::Rocks) => "debug-rocks".to_string(),
        (BuildMode::Release, StorageBackend::Rocks) => "latest-rocks".to_string(),
        (BuildMode::Dev, StorageBackend::Rocks) => "dev-rocks".to_string(),
    };
    let project_name = self.compose_project_name(instance_name);
    format!("{project_name}:{tag}")
}
```

Find all call sites of `image_name` in `docker.rs` and update them to pass `storage_backend`:

```bash
grep -n "image_name(" /Users/franciscobaptista/Development/helix-db-snapshot/helix-cli/src/docker.rs
```

At each call site, pass `instance_config.storage_backend()` as the third argument.

- [ ] **Step 5: Update `build_binary_using_cargo` in `helix-cli/src/commands/build.rs`**

Change the function signature and body (lines 571–600):

```rust
fn build_binary_using_cargo(
    project: &ProjectContext,
    instance_name: &str,
    binary_output: &str,
    build_mode: BuildMode,
    storage_backend: StorageBackend,
) -> Result<()> {
    use crate::config::StorageBackend;

    let binary_output_path = std::path::Path::new(binary_output);
    std::fs::create_dir_all(binary_output_path)?;

    // <path-to-.helix>/<instance_name>/helix-repo-copy/helix-container/
    let current_dir = project
        .helix_dir
        .join(instance_name)
        .join("helix-repo-copy")
        .join("helix-container");

    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("--target-dir")
        .arg(binary_output_path.as_os_str())
        .current_dir(current_dir);

    match (build_mode, storage_backend) {
        (BuildMode::Debug, _) => unreachable!(
            "Please report as a bug. BuildMode::Debug should have been caught in validation."
        ),
        (BuildMode::Release, StorageBackend::Lmdb) => {
            cmd.arg("--release");
        }
        (BuildMode::Dev, StorageBackend::Lmdb) => {
            cmd.arg("--features").arg("dev");
        }
        (BuildMode::Release, StorageBackend::Rocks) => {
            cmd.arg("--release")
                .arg("--no-default-features")
                .arg("--features")
                .arg("rocks");
        }
        (BuildMode::Dev, StorageBackend::Rocks) => {
            cmd.arg("--no-default-features")
                .arg("--features")
                .arg("rocks,dev");
        }
    }

    let status = cmd.status()?;

    if !status.success() {
        return Err(eyre!(
            "Cargo build failed with exit code: {:?}",
            status.code()
        ));
    }
    Ok(())
}
```

Update the call site around line 171:

```rust
match build_binary_using_cargo(
    project,
    instance_name,
    binary_output,
    instance_config.build_mode(),
    instance_config.storage_backend(),
) {
```

Add import at top of `build.rs` if missing:
```rust
use crate::config::{BuildMode, StorageBackend};
```

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-cli
cargo test --lib tests::docker_tests 2>&1 | tail -10
```

Expected: all docker_tests pass including the 4 new tests.

- [ ] **Step 7: Run full CLI check**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-cli
cargo check 2>&1 | tail -5
```

Expected: `Finished` with no errors.

- [ ] **Step 8: Commit**

```bash
git -C /Users/franciscobaptista/Development/helix-db-snapshot add \
  helix-cli/src/config.rs \
  helix-cli/src/commands/build.rs \
  helix-cli/src/docker.rs \
  helix-cli/src/tests/docker_tests.rs
git -C /Users/franciscobaptista/Development/helix-db-snapshot commit -m "feat(helix-cli): wire StorageBackend through build pipeline; fix --release for --bin builds"
```

---

## Task 3: Add `helix run` command for bare-metal binary execution

**Files:**
- Create: `helix-cli/src/commands/run.rs`
- Modify: `helix-cli/src/main.rs`
- Create: `helix-cli/src/tests/run_tests.rs`
- Modify: `helix-cli/src/tests/mod.rs`

`helix run --bin <dir>` finds the binary built by `helix build --bin <dir>`, reads port and data-dir from the instance config (or from flags), and execs it directly — no Docker required.

- [ ] **Step 1: Write failing tests in `helix-cli/src/tests/run_tests.rs`** (new file)

```rust
use crate::commands::run::{resolve_binary, resolve_data_dir};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_resolve_binary_prefers_release_over_debug() {
    let tmp = TempDir::new().unwrap();
    let release_dir = tmp.path().join("release");
    let debug_dir = tmp.path().join("debug");
    fs::create_dir_all(&release_dir).unwrap();
    fs::create_dir_all(&debug_dir).unwrap();

    // Create both binaries
    let release_bin = release_dir.join("helix-container");
    let debug_bin = debug_dir.join("helix-container");
    fs::write(&release_bin, b"").unwrap();
    fs::write(&debug_bin, b"").unwrap();

    let found = resolve_binary(tmp.path()).unwrap();
    assert_eq!(found, release_bin, "release binary should be preferred over debug");
}

#[test]
fn test_resolve_binary_falls_back_to_debug() {
    let tmp = TempDir::new().unwrap();
    let debug_dir = tmp.path().join("debug");
    fs::create_dir_all(&debug_dir).unwrap();
    let debug_bin = debug_dir.join("helix-container");
    fs::write(&debug_bin, b"").unwrap();

    let found = resolve_binary(tmp.path()).unwrap();
    assert_eq!(found, debug_bin);
}

#[test]
fn test_resolve_binary_errors_when_neither_exists() {
    let tmp = TempDir::new().unwrap();
    let result = resolve_binary(tmp.path());
    assert!(result.is_err(), "should error when no binary found");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("No binary found"), "error should mention missing binary, got: {msg}");
}

#[test]
fn test_resolve_data_dir_uses_override_when_provided() {
    let override_dir = "/custom/data/path".to_string();
    let result = resolve_data_dir(Some(override_dir.clone()), None, None);
    assert_eq!(result, override_dir);
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-cli
cargo test --lib tests::run_tests 2>&1 | grep -E "error\[|FAILED" | head -5
```

Expected: compile error — `run` module and `resolve_binary`/`resolve_data_dir` don't exist yet.

- [ ] **Step 3: Create `helix-cli/src/commands/run.rs`**

```rust
use eyre::{eyre, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::ProjectContext;

/// Find the helix-container binary in a `helix build --bin <dir>` output directory.
/// Prefers `<dir>/release/helix-container` over `<dir>/debug/helix-container`.
pub fn resolve_binary(bin_dir: &Path) -> Result<PathBuf> {
    for profile in ["release", "debug"] {
        let candidate = bin_dir.join(profile).join("helix-container");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(eyre!(
        "No binary found in {}\nRun 'helix build --bin {}' first",
        bin_dir.display(),
        bin_dir.display()
    ))
}

/// Resolve the data directory to pass as HELIX_DATA_DIR.
/// Priority: explicit `--data-dir` flag > project instance volume path > ~/.helix/user
pub fn resolve_data_dir(
    data_dir_override: Option<String>,
    project: Option<&ProjectContext>,
    instance_name: Option<&str>,
) -> String {
    if let Some(dir) = data_dir_override {
        return dir;
    }
    if let (Some(proj), Some(inst)) = (project, instance_name) {
        return proj.instance_volume(inst).to_string_lossy().into_owned();
    }
    dirs::home_dir()
        .map(|h| h.join(".helix/user").to_string_lossy().into_owned())
        .unwrap_or_else(|| "/tmp/helix-data".to_string())
}

pub async fn run(
    bin: String,
    instance: Option<String>,
    data_dir: Option<String>,
    port: Option<u16>,
) -> Result<()> {
    let bin_dir = Path::new(&bin);
    let binary_path = resolve_binary(bin_dir)?;

    // Load project context if available, for port and data-dir defaults
    let project = ProjectContext::find_and_load(None).ok();
    let inst = instance.as_deref().unwrap_or("dev");

    let data_dir_val = resolve_data_dir(
        data_dir,
        project.as_ref(),
        project.as_ref().map(|_| inst),
    );

    let port_val = port
        .or_else(|| {
            project
                .as_ref()
                .and_then(|p| p.config.get_instance(inst).ok())
                .and_then(|c| c.port())
        })
        .unwrap_or(6969);

    println!("Starting helix-container:");
    println!("  binary:   {}", binary_path.display());
    println!("  data dir: {data_dir_val}");
    println!("  port:     {port_val}");

    // Ensure the data directory exists
    std::fs::create_dir_all(&data_dir_val)?;

    let status = Command::new(&binary_path)
        .env("HELIX_DATA_DIR", &data_dir_val)
        .env("HELIX_PORT", port_val.to_string())
        .status()
        .map_err(|e| eyre!("Failed to start {}: {e}", binary_path.display()))?;

    if !status.success() {
        return Err(eyre!(
            "helix-container exited with code {:?}",
            status.code()
        ));
    }
    Ok(())
}
```

- [ ] **Step 4: Register `run` in `helix-cli/src/commands/mod.rs`**

Find `helix-cli/src/commands/mod.rs` and add:
```rust
pub mod run;
```

- [ ] **Step 5: Add `Commands::Run` to `helix-cli/src/main.rs`**

In the `Commands` enum (after `Commands::Start`, around line 118), add:

```rust
/// Run a pre-built binary directly without Docker
Run {
    /// Directory containing the built binary (output of `helix build --bin <dir>`)
    #[arg(long)]
    bin: String,
    /// Instance name for config lookup (port, data-dir defaults)
    #[arg(short, long)]
    instance: Option<String>,
    /// Override the data directory (sets HELIX_DATA_DIR)
    #[arg(long)]
    data_dir: Option<String>,
    /// Override the port (sets HELIX_PORT)
    #[arg(long)]
    port: Option<u16>,
},
```

In the `match cmd` arm (after `Commands::Start`), add:

```rust
Commands::Run {
    bin,
    instance,
    data_dir,
    port,
} => commands::run::run(bin, instance, data_dir, port).await,
```

- [ ] **Step 6: Register `run_tests` in `helix-cli/src/tests/mod.rs`**

Add:
```rust
#[cfg(test)]
pub mod run_tests;
```

- [ ] **Step 7: Run tests**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-cli
cargo test --lib tests::run_tests 2>&1 | tail -10
```

Expected: 4 tests pass.

- [ ] **Step 8: Full CLI check and all tests**

```bash
cd /Users/franciscobaptista/Development/helix-db-snapshot/helix-cli
cargo check 2>&1 | tail -3
cargo test --lib 2>&1 | tail -5
```

Expected: clean compile, all tests pass.

- [ ] **Step 9: Commit**

```bash
git -C /Users/franciscobaptista/Development/helix-db-snapshot add \
  helix-cli/src/commands/run.rs \
  helix-cli/src/main.rs \
  helix-cli/src/tests/run_tests.rs \
  helix-cli/src/tests/mod.rs
git -C /Users/franciscobaptista/Development/helix-db-snapshot commit -m "feat(helix-cli): add 'helix run' command for bare-metal binary execution"
```

---

## Self-Review

**Spec coverage check:**

- Gap 1 (RocksDB not wired into build): ✅ Task 1 adds the feature to helix-container; Task 2 wires it through config + build.rs + docker.rs
- Gap 2 (`--release` never passed for `--bin`): ✅ Task 2 fixes `build_binary_using_cargo` to match on `BuildMode::Release`
- Gap 3 (no bare-metal start): ✅ Task 3 adds `helix run`

**Placeholder scan:** None found — all steps contain concrete code.

**Type consistency:**
- `StorageBackend` defined in Task 2 Step 3, used in Steps 4 and 5 of the same task ✅
- `resolve_binary` / `resolve_data_dir` defined in Task 3 Step 3, tested in Step 1 (tests reference them) ✅
- `image_name` updated in Task 2 Step 4 — call sites also updated in same step ✅
- `build_binary_using_cargo` updated in Task 2 Step 5 — call site updated in same step ✅
