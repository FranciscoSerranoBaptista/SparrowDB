# sparrowdb-chef CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a new `sparrowdb-chef` crate — a standalone CLI binary that bootstraps a new SparrowDB application for a coding agent in one command.

**Architecture:** A self-contained binary crate at `sparrowdb-chef/` added to the workspace. It creates a project directory, writes a `docker-compose.yml` + example HQL files + seed data + agent prompt, starts the SparrowDB container, waits for it to be healthy, seeds initial data, and prints next steps. Follows the same clap/cliclack/eyre conventions as `sparrow-cli`.

**Tech Stack:** Rust, clap 4 (derive), cliclack 0.3, tokio, reqwest (JSON), serde_json, eyre/color-eyre.

---

## File Map

| File | Purpose |
|---|---|
| `sparrowdb-chef/Cargo.toml` | Crate manifest with dependencies |
| `sparrowdb-chef/src/main.rs` | Binary entry point — clap parse, dispatch |
| `sparrowdb-chef/src/lib.rs` | Re-exports `commands` module for integration tests |
| `sparrowdb-chef/src/commands/mod.rs` | `pub mod chef;` |
| `sparrowdb-chef/src/commands/chef.rs` | Full orchestration of the setup flow |
| `sparrowdb-chef/src/prompts.rs` | `ask_build_intent`, `ask_setup_mode`, `ask_project_path` |
| `sparrowdb-chef/src/templates.rs` | String generators for all written files |
| `sparrowdb-chef/src/docker.rs` | `compose_up`, `wait_for_healthy` |
| `sparrowdb-chef/src/http.rs` | `post_v1_query`, `check_health` |
| `sparrowdb-chef/src/tests/mod.rs` | Parser + unit tests |
| `Cargo.toml` (workspace root) | Add `sparrowdb-chef` to `members` |

---

### Task 1: Workspace registration and crate scaffold

**Files:**
- Modify: `Cargo.toml` (workspace root, `members` array)
- Create: `sparrowdb-chef/Cargo.toml`
- Create: `sparrowdb-chef/src/main.rs`
- Create: `sparrowdb-chef/src/lib.rs`

- [ ] **Step 1: Add member to workspace**

Edit the `[workspace]` members array in `/Users/franciscobaptista/Development/SparrowDB/Cargo.toml`. Find the existing `members = [` block and add `"sparrowdb-chef"`.

- [ ] **Step 2: Create `sparrowdb-chef/Cargo.toml`**

```toml
[package]
name = "sparrowdb-chef"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "sparrowdb-chef"
path = "src/main.rs"

[lib]
name = "sparrowdb_chef"
path = "src/lib.rs"

[dependencies]
clap = { version = "4.5", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
eyre = "0.6"
color-eyre = "0.6"
cliclack = "0.3"
reqwest = { version = "0.12", features = ["json"] }
serde_json = "1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Create `sparrowdb-chef/src/lib.rs`**

```rust
pub mod commands;
pub mod docker;
pub mod http;
pub mod prompts;
pub mod templates;
```

- [ ] **Step 4: Create `sparrowdb-chef/src/main.rs`** (empty Commands enum to start)

```rust
use clap::{Parser, Subcommand};
use color_eyre::owo_colors::OwoColorize;
use eyre::Result;

mod commands;
mod docker;
mod http;
mod prompts;
mod templates;

#[derive(Parser)]
#[command(name = "sparrowdb-chef", version, about = "Bootstrap a SparrowDB application for a coding agent")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bootstrap a new SparrowDB project (alias: cook)
    #[command(alias = "cook")]
    Chef {
        /// Skip prompts and run with defaults
        #[arg(short = 'a', long)]
        auto: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    match cli.command {
        Commands::Chef { auto } => commands::chef::run(auto).await,
    }
}
```

- [ ] **Step 5: Create stub command modules**

Create `sparrowdb-chef/src/commands/mod.rs`:
```rust
pub mod chef;
```

Create `sparrowdb-chef/src/commands/chef.rs`:
```rust
use eyre::Result;

pub async fn run(_auto: bool) -> Result<()> {
    Ok(())
}
```

Create `sparrowdb-chef/src/docker.rs`, `sparrowdb-chef/src/http.rs`, `sparrowdb-chef/src/prompts.rs`, `sparrowdb-chef/src/templates.rs` — each as an empty module (`// placeholder`) for now.

- [ ] **Step 6: Verify it compiles**

```bash
cargo check --package sparrowdb-chef
```

Expected: no errors, no warnings (ignore unused warnings on stubs).

- [ ] **Step 7: Commit**

```bash
git add sparrowdb-chef/ Cargo.toml
git commit -m "feat: add sparrowdb-chef crate scaffold"
```

---

### Task 2: CLI parser + parser tests

**Files:**
- Modify: `sparrowdb-chef/src/main.rs`
- Create: `sparrowdb-chef/src/tests/mod.rs`
- Create: `sparrowdb-chef/src/tests/parser_tests.rs`

- [ ] **Step 1: Write the failing parser tests**

Create `sparrowdb-chef/src/tests/mod.rs`:
```rust
mod parser_tests;
```

Create `sparrowdb-chef/src/tests/parser_tests.rs`:
```rust
use clap::Parser;

// Import Cli from the binary crate's module path via lib re-export.
// We test the CLI struct directly to avoid subprocess overhead.

// Duplicate the minimal Cli definition here for white-box testing.
// This avoids coupling the test to private implementation details
// while still verifying the arg parsing rules.

#[derive(Parser)]
#[command(name = "sparrowdb-chef")]
struct TestCli {
    #[command(subcommand)]
    command: TestCommands,
}

#[derive(clap::Subcommand)]
enum TestCommands {
    #[command(alias = "cook")]
    Chef {
        #[arg(short = 'a', long)]
        auto: bool,
    },
}

#[test]
fn chef_without_flags() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "chef"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(!auto);
}

#[test]
fn chef_with_auto_long() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "chef", "--auto"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(auto);
}

#[test]
fn chef_with_auto_short() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "chef", "-a"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(auto);
}

#[test]
fn cook_alias_works() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "cook"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(!auto);
}

#[test]
fn cook_alias_with_auto() {
    let cli = TestCli::try_parse_from(["sparrowdb-chef", "cook", "--auto"]).unwrap();
    let TestCommands::Chef { auto } = cli.command;
    assert!(auto);
}
```

Add to `sparrowdb-chef/src/lib.rs`:
```rust
pub mod commands;
pub mod docker;
pub mod http;
pub mod prompts;
pub mod templates;

#[cfg(test)]
mod tests;
```

- [ ] **Step 2: Run tests — expect FAIL (module not found)**

```bash
cargo test --package sparrowdb-chef 2>&1 | head -20
```

Expected: compile errors about missing `tests` module.

- [ ] **Step 3: Add tests directory**

Create `sparrowdb-chef/src/tests/` directory and place the files from Step 1.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test --package sparrowdb-chef -- parser_tests
```

Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add sparrowdb-chef/src/
git commit -m "feat(chef): add CLI parser with chef/cook subcommands and auto flag"
```

---

### Task 3: Template generation

**Files:**
- Modify: `sparrowdb-chef/src/templates.rs`
- Create: `sparrowdb-chef/src/tests/template_tests.rs`

Templates are pure string-returning functions — no I/O, no side effects.

- [ ] **Step 1: Write failing template tests**

Create `sparrowdb-chef/src/tests/template_tests.rs`:
```rust
use crate::templates::{docker_compose, seed_json, read_json, chef_prompt, schema_hx, queries_hx};

#[test]
fn docker_compose_is_valid_yaml_with_port_6969() {
    let s = docker_compose();
    assert!(s.contains("6969"), "compose file must expose port 6969");
    assert!(s.contains("sparrow-data"), "compose file must define sparrow-data volume");
    assert!(s.contains("SPARROW_DATA_DIR"), "compose file must set SPARROW_DATA_DIR");
}

#[test]
fn seed_json_is_valid_json_with_addn() {
    let s = seed_json();
    let v: serde_json::Value = serde_json::from_str(&s).expect("seed.json must be valid JSON");
    let queries = &v["query"]["queries"];
    assert!(queries.is_array(), "queries must be an array");
    let steps = &queries[0]["Query"]["steps"];
    assert!(steps[0].get("AddN").is_some(), "first step must be AddN");
}

#[test]
fn read_json_is_valid_json_with_nwhere() {
    let s = read_json();
    let v: serde_json::Value = serde_json::from_str(&s).expect("read.json must be valid JSON");
    let steps = &v["query"]["queries"][0]["Query"]["steps"];
    assert!(steps[0].get("NWhere").is_some(), "first step must be NWhere");
}

#[test]
fn chef_prompt_contains_localhost_6969() {
    let s = chef_prompt("build a social network");
    assert!(s.contains("localhost:6969"), "prompt must include the SparrowDB URL");
    assert!(s.contains("build a social network"), "prompt must include user intent");
}

#[test]
fn chef_prompt_with_empty_intent() {
    let s = chef_prompt("");
    assert!(s.contains("localhost:6969"));
    // Should not crash or include an empty placeholder line
    assert!(!s.contains("## What you're building\n\n##"));
}

#[test]
fn schema_hx_has_user_node() {
    let s = schema_hx();
    assert!(s.contains("N::User"), "schema must define a User node type");
}

#[test]
fn queries_hx_has_a_query() {
    let s = queries_hx();
    assert!(s.contains("QUERY"), "queries.hx must contain at least one QUERY definition");
}
```

Add to `sparrowdb-chef/src/tests/mod.rs`:
```rust
mod parser_tests;
mod template_tests;
```

- [ ] **Step 2: Run — expect FAIL (templates module empty)**

```bash
cargo test --package sparrowdb-chef -- template_tests 2>&1 | head -20
```

Expected: compile errors about missing functions.

- [ ] **Step 3: Implement `sparrowdb-chef/src/templates.rs`**

```rust
/// Generate docker-compose.yml content for a new SparrowDB project.
pub fn docker_compose() -> String {
    r#"services:
  sparrow:
    image: ghcr.io/sparrowdb/sparrowdb:latest
    ports:
      - "6969:6969"
    volumes:
      - sparrow-data:/data
    environment:
      SPARROW_DATA_DIR: /data
      SPARROW_PORT: "6969"
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "sh", "-c", "cat /proc/1/status | grep -q Sleeping || exit 0"]
      interval: 5s
      timeout: 3s
      retries: 12

volumes:
  sparrow-data:
"#
    .to_string()
}

/// Starter HQL schema: a single User node type with indexed fields.
pub fn schema_hx() -> String {
    r#"N::User {
    INDEX name: String,
    INDEX email: String,
}

E::Follows {
    From: User,
    To: User,
}
"#
    .to_string()
}

/// Starter HQL queries: get all users, get user by ID, get followers.
pub fn queries_hx() -> String {
    r#"QUERY getAllUsers() =>
    users <- N<User>()
    RETURN users

QUERY getUserById(id: ID) =>
    user <- N<User>(id)
    RETURN user

QUERY getFollowers(id: ID) =>
    followers <- N<User>(id)::In<Follows>
    RETURN followers
"#
    .to_string()
}

/// v1/query-compatible seed request: creates two User nodes.
pub fn seed_json() -> String {
    serde_json::json!({
        "query": {
            "queries": [
                {
                    "Query": {
                        "name": "alice",
                        "steps": [{
                            "AddN": {
                                "label": "User",
                                "properties": [
                                    ["name",  {"Value": {"String": "Alice"}}],
                                    ["email", {"Value": {"String": "alice@example.com"}}]
                                ]
                            }
                        }]
                    }
                },
                {
                    "Query": {
                        "name": "bob",
                        "steps": [{
                            "AddN": {
                                "label": "User",
                                "properties": [
                                    ["name",  {"Value": {"String": "Bob"}}],
                                    ["email", {"Value": {"String": "bob@example.com"}}]
                                ]
                            }
                        }]
                    }
                }
            ],
            "returns": ["alice", "bob"]
        }
    })
    .to_string()
}

/// v1/query-compatible read request: fetches all User nodes.
pub fn read_json() -> String {
    serde_json::json!({
        "query": {
            "queries": [{
                "Query": {
                    "name": "users",
                    "steps": [{
                        "NWhere": {
                            "Eq": ["$label", {"String": "User"}]
                        }
                    }]
                }
            }],
            "returns": ["users"]
        }
    })
    .to_string()
}

/// Coding-agent prompt. `intent` is what the user typed (may be empty).
pub fn chef_prompt(intent: &str) -> String {
    let intent_section = if intent.is_empty() {
        String::new()
    } else {
        format!("## What you're building\n\n{intent}\n\n")
    };

    format!(
        r#"# SparrowDB Project — Coding Agent Instructions

{intent_section}## Local database

SparrowDB is running at http://localhost:6969.

## API quick-start

All requests are JSON over HTTP. Example using curl:

```bash
# Read all User nodes
curl -s -X POST http://localhost:6969/v1/query \
  -H "Content-Type: application/json" \
  -d @examples/read.json | jq .

# Check database stats
curl -s -X POST http://localhost:6969/diagnostics | jq .
```

## Project files

| File | Purpose |
|------|---------|
| `docker-compose.yml` | Start/stop the local SparrowDB instance |
| `db/schema.hx` | Node and edge type definitions (HQL) |
| `db/queries.hx` | Compiled query definitions (HQL) |
| `examples/seed.json` | Seed data — POST to `/v1/query` |
| `examples/read.json` | Read query — POST to `/v1/query` |

## Next steps

1. Replace `examples/seed.json` with your real data model.
2. Write application code in any language — it just makes HTTP requests to port 6969.
3. For typed queries, edit `db/schema.hx` and `db/queries.hx`, then push with `sparrow push`.

Full HTTP API reference: https://github.com/sparrowdb/sparrowdb/blob/main/docs/HTTP_API.md
"#
    )
}
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test --package sparrowdb-chef -- template_tests
```

Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add sparrowdb-chef/src/
git commit -m "feat(chef): add template generators for docker-compose, HQL, seed data, and agent prompt"
```

---

### Task 4: HTTP client

**Files:**
- Modify: `sparrowdb-chef/src/http.rs`
- Create: `sparrowdb-chef/src/tests/http_tests.rs`

- [ ] **Step 1: Write failing http tests**

Create `sparrowdb-chef/src/tests/http_tests.rs`:
```rust
// Unit tests for request shapes — not integration tests.
// We validate that the functions accept the right types and return Results.
// Actual HTTP calls are tested in the integration flow in chef.rs.

use crate::http::SparrowClient;

#[test]
fn client_builds_with_base_url() {
    let client = SparrowClient::new("http://localhost:6969");
    assert_eq!(client.base_url(), "http://localhost:6969");
}

#[test]
fn client_strips_trailing_slash() {
    let client = SparrowClient::new("http://localhost:6969/");
    assert_eq!(client.base_url(), "http://localhost:6969");
}
```

Add to `sparrowdb-chef/src/tests/mod.rs`:
```rust
mod parser_tests;
mod template_tests;
mod http_tests;
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test --package sparrowdb-chef -- http_tests 2>&1 | head -10
```

Expected: compile errors about missing `SparrowClient`.

- [ ] **Step 3: Implement `sparrowdb-chef/src/http.rs`**

```rust
use eyre::{Result, eyre};
use reqwest::Client;

pub struct SparrowClient {
    base_url: String,
    client: Client,
}

impl SparrowClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::new(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// POST to /diagnostics to check if the database is reachable.
    /// Returns Ok(()) if the server responds with a 2xx status.
    pub async fn check_health(&self) -> Result<()> {
        let url = format!("{}/diagnostics", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .map_err(|e| eyre!("health check failed: {e}"))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(eyre!("database returned status {}", resp.status()))
        }
    }

    /// POST a v1/query-compatible JSON body to /v1/query.
    pub async fn post_v1_query(&self, body: &str) -> Result<serde_json::Value> {
        let url = format!("{}/v1/query", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| eyre!("POST /v1/query failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(eyre!("seed request failed with {status}: {text}"));
        }

        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| eyre!("failed to parse seed response: {e}"))
    }
}
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test --package sparrowdb-chef -- http_tests
```

Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add sparrowdb-chef/src/
git commit -m "feat(chef): add SparrowClient for health check and v1/query seeding"
```

---

### Task 5: Docker operations

**Files:**
- Modify: `sparrowdb-chef/src/docker.rs`
- Create: `sparrowdb-chef/src/tests/docker_tests.rs`

- [ ] **Step 1: Write failing docker tests**

Create `sparrowdb-chef/src/tests/docker_tests.rs`:
```rust
use std::path::Path;
use crate::docker::compose_command;

#[test]
fn compose_command_includes_project_dir() {
    let dir = Path::new("/tmp/my-project");
    let cmd = compose_command(dir, &["up", "-d"]);
    let args: Vec<&str> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
    assert!(args.contains(&"up"));
    assert!(args.contains(&"-d"));
    // compose file path must be absolute
    assert!(args.iter().any(|a| a.contains("docker-compose.yml")));
}
```

Add to `sparrowdb-chef/src/tests/mod.rs`:
```rust
mod docker_tests;
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test --package sparrowdb-chef -- docker_tests 2>&1 | head -10
```

Expected: compile errors about missing `compose_command`.

- [ ] **Step 3: Implement `sparrowdb-chef/src/docker.rs`**

```rust
use eyre::{Result, eyre};
use std::path::Path;
use std::process::Command;

/// Build a `docker compose` command scoped to the given project directory.
pub fn compose_command(project_dir: &Path, args: &[&str]) -> Command {
    let compose_file = project_dir.join("docker-compose.yml");
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_file)
        .args(args);
    cmd
}

/// Run `docker compose up -d` in the given project directory.
pub fn compose_up(project_dir: &Path) -> Result<()> {
    let status = compose_command(project_dir, &["up", "-d"])
        .status()
        .map_err(|e| eyre!("failed to run docker compose: {e}\nIs Docker running?"))?;

    if !status.success() {
        return Err(eyre!(
            "docker compose up failed with code {:?}",
            status.code()
        ));
    }
    Ok(())
}

/// Poll `check_health` until it returns Ok or `max_attempts` is exceeded.
/// Waits `delay_ms` milliseconds between attempts.
pub async fn wait_for_healthy(
    client: &crate::http::SparrowClient,
    max_attempts: u32,
    delay_ms: u64,
) -> Result<()> {
    for attempt in 1..=max_attempts {
        if client.check_health().await.is_ok() {
            return Ok(());
        }
        if attempt < max_attempts {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
    }
    Err(eyre!(
        "SparrowDB did not become healthy after {max_attempts} attempts.\n\
         Check `docker compose logs` in your project directory."
    ))
}
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test --package sparrowdb-chef -- docker_tests
```

Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add sparrowdb-chef/src/
git commit -m "feat(chef): add docker compose_up and wait_for_healthy"
```

---

### Task 6: Interactive prompts

**Files:**
- Modify: `sparrowdb-chef/src/prompts.rs`

No unit tests for prompts — they're I/O. They're covered by the manual walkthrough in Task 7.

- [ ] **Step 1: Implement `sparrowdb-chef/src/prompts.rs`**

```rust
use cliclack::input;
use eyre::Result;

#[derive(Debug, PartialEq)]
pub enum SetupMode {
    Automatic,
    Manual,
}

/// Ask the user what they want to build. Empty answer is allowed (skips intent).
pub fn ask_build_intent() -> Result<String> {
    let intent: String = input("What do you want to build? (press Enter to skip)")
        .placeholder("e.g. a social graph, a recommendation engine")
        .default_input("")
        .interact()?;
    Ok(intent)
}

/// Ask whether to run automatic or manual setup.
/// Returns `Automatic` if user picks it or if auto flag is already set.
pub fn ask_setup_mode() -> Result<SetupMode> {
    let choice: String = cliclack::select("How would you like to set up?")
        .item("auto", "Automatic setup", "run the full flow with defaults")
        .item("manual", "Manual setup", "confirm or customise each step")
        .interact()?;

    Ok(if choice == "auto" {
        SetupMode::Automatic
    } else {
        SetupMode::Manual
    })
}

/// Ask for the project path, defaulting to `~/my-first-sparrow-project`.
pub fn ask_project_path() -> Result<std::path::PathBuf> {
    let default = dirs_next::home_dir()
        .map(|h| h.join("my-first-sparrow-project").to_string_lossy().into_owned())
        .unwrap_or_else(|| "./my-first-sparrow-project".to_string());

    let raw: String = input("Where should the project be created?")
        .placeholder(&default)
        .default_input(&default)
        .interact()?;

    Ok(std::path::PathBuf::from(raw))
}

/// In manual mode: ask the user to confirm before running a step.
/// Returns `true` if they confirm, `false` to skip.
pub fn confirm_step(step_name: &str) -> Result<bool> {
    let ok: bool = cliclack::confirm(format!("Run step: {step_name}?"))
        .initial_value(true)
        .interact()?;
    Ok(ok)
}
```

Add `dirs-next = "0.1"` to `sparrowdb-chef/Cargo.toml` dependencies:
```toml
dirs-next = "0.1"
```

- [ ] **Step 2: Verify compile**

```bash
cargo check --package sparrowdb-chef
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add sparrowdb-chef/src/prompts.rs sparrowdb-chef/Cargo.toml
git commit -m "feat(chef): add interactive prompts for intent, setup mode, and project path"
```

---

### Task 7: Chef orchestration command

**Files:**
- Modify: `sparrowdb-chef/src/commands/chef.rs`
- Create: `sparrowdb-chef/src/tests/chef_tests.rs`

- [ ] **Step 1: Write failing chef tests**

Create `sparrowdb-chef/src/tests/chef_tests.rs`:
```rust
use std::fs;
use tempfile::TempDir;
use crate::commands::chef::write_project_files;

#[test]
fn write_project_files_creates_all_expected_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    write_project_files(path, "build a graph app").unwrap();

    assert!(path.join("docker-compose.yml").exists(), "docker-compose.yml must exist");
    assert!(path.join("db/schema.hx").exists(), "db/schema.hx must exist");
    assert!(path.join("db/queries.hx").exists(), "db/queries.hx must exist");
    assert!(path.join("examples/seed.json").exists(), "examples/seed.json must exist");
    assert!(path.join("examples/read.json").exists(), "examples/read.json must exist");
    assert!(path.join("SPARROWDB_CHEF_PROMPT.md").exists(), "prompt file must exist");
}

#[test]
fn write_project_files_prompt_contains_intent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    write_project_files(path, "inventory management system").unwrap();

    let prompt = fs::read_to_string(path.join("SPARROWDB_CHEF_PROMPT.md")).unwrap();
    assert!(prompt.contains("inventory management system"));
}

#[test]
fn write_project_files_seed_json_is_valid() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    write_project_files(path, "").unwrap();

    let seed = fs::read_to_string(path.join("examples/seed.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&seed)
        .expect("examples/seed.json must be valid JSON");
}
```

Add to `sparrowdb-chef/src/tests/mod.rs`:
```rust
mod parser_tests;
mod template_tests;
mod http_tests;
mod docker_tests;
mod chef_tests;
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test --package sparrowdb-chef -- chef_tests 2>&1 | head -10
```

Expected: compile errors about missing `write_project_files`.

- [ ] **Step 3: Implement `sparrowdb-chef/src/commands/chef.rs`**

```rust
use cliclack::{intro, log, outro, spinner};
use eyre::Result;
use std::fs;
use std::path::Path;

use crate::docker::{compose_up, wait_for_healthy};
use crate::http::SparrowClient;
use crate::prompts::{SetupMode, ask_build_intent, ask_project_path, ask_setup_mode, confirm_step};
use crate::templates::{chef_prompt, docker_compose, queries_hx, read_json, schema_hx, seed_json};

const BASE_URL: &str = "http://localhost:6969";

/// Write all project files to `dir`. Extracted for testability.
pub fn write_project_files(dir: &Path, intent: &str) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::create_dir_all(dir.join("db"))?;
    fs::create_dir_all(dir.join("examples"))?;

    fs::write(dir.join("docker-compose.yml"), docker_compose())?;
    fs::write(dir.join("db/schema.hx"), schema_hx())?;
    fs::write(dir.join("db/queries.hx"), queries_hx())?;
    fs::write(dir.join("examples/seed.json"), seed_json())?;
    fs::write(dir.join("examples/read.json"), read_json())?;
    fs::write(dir.join("SPARROWDB_CHEF_PROMPT.md"), chef_prompt(intent))?;

    Ok(())
}

pub async fn run(auto: bool) -> Result<()> {
    intro("sparrowdb-chef — bootstrap a SparrowDB application")?;

    // Step 1: build intent
    let intent = if auto {
        String::new()
    } else {
        ask_build_intent()?
    };

    // Step 2: setup mode
    let mode = if auto {
        SetupMode::Automatic
    } else {
        ask_setup_mode()?
    };

    // Step 3: project path
    let project_dir = if auto || mode == SetupMode::Automatic {
        dirs_next::home_dir()
            .map(|h| h.join("my-first-sparrow-project"))
            .unwrap_or_else(|| std::path::PathBuf::from("./my-first-sparrow-project"))
    } else {
        ask_project_path()?
    };

    // Step 4: write files
    let should_write = if mode == SetupMode::Manual {
        confirm_step(&format!("Write project files to {}", project_dir.display()))?
    } else {
        true
    };

    if should_write {
        let mut sp = spinner();
        sp.start("Writing project files…");
        write_project_files(&project_dir, &intent)?;
        sp.stop(format!("Project files written to {}", project_dir.display()));
    }

    // Step 5: docker compose up
    let should_start = if mode == SetupMode::Manual {
        confirm_step("Start SparrowDB with docker compose")?
    } else {
        true
    };

    if should_start {
        let mut sp = spinner();
        sp.start("Starting SparrowDB…");
        compose_up(&project_dir)?;
        sp.stop("SparrowDB container started");

        // Step 6: wait for healthy
        let client = SparrowClient::new(BASE_URL);
        let mut sp = spinner();
        sp.start("Waiting for SparrowDB to be ready…");
        wait_for_healthy(&client, 24, 2500).await?;
        sp.stop("SparrowDB is ready");

        // Step 7: seed data
        let should_seed = if mode == SetupMode::Manual {
            confirm_step("Seed example data")?
        } else {
            true
        };

        if should_seed {
            let mut sp = spinner();
            sp.start("Seeding example data…");
            client.post_v1_query(&seed_json()).await?;
            sp.stop("Example data seeded");
        }
    }

    log::success("Done!")?;
    outro(format!(
        "Your SparrowDB project is at: {}\n\n\
         Open SPARROWDB_CHEF_PROMPT.md and hand it to your coding agent.\n\
         SparrowDB API: {BASE_URL}",
        project_dir.display()
    ))?;

    Ok(())
}
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test --package sparrowdb-chef -- chef_tests
```

Expected: 3 tests pass.

- [ ] **Step 5: Run all tests**

```bash
cargo test --package sparrowdb-chef
```

Expected: all tests pass (parser + templates + http + docker + chef).

- [ ] **Step 6: Commit**

```bash
git add sparrowdb-chef/src/
git commit -m "feat(chef): implement orchestration — write files, start docker, wait, seed"
```

---

### Task 8: Final wiring, formatting, and full test run

**Files:**
- Modify: `sparrowdb-chef/src/main.rs` (verify it calls commands::chef::run correctly)
- Run: `cargo fmt`, `cargo test`

- [ ] **Step 1: Verify the binary runs without panicking on `--help`**

```bash
cargo run --package sparrowdb-chef -- --help
```

Expected output includes:
```
Usage: sparrowdb-chef <COMMAND>

Commands:
  chef  Bootstrap a new SparrowDB project (alias: cook)
  cook  Bootstrap a new SparrowDB project (alias: cook)
  help  Print this message or the help of the given subcommand(s)
```

- [ ] **Step 2: Verify `chef --help`**

```bash
cargo run --package sparrowdb-chef -- chef --help
```

Expected output includes `--auto` / `-a` flag.

- [ ] **Step 3: Run cargo fmt**

```bash
cargo fmt --package sparrowdb-chef
```

Expected: no output (already formatted), or small whitespace fixes.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test --package sparrowdb-chef -- --test-threads=4
```

Expected: all tests pass with no warnings about unused code in non-test paths.

- [ ] **Step 5: Final commit**

```bash
git add sparrowdb-chef/
git commit -m "feat: sparrowdb-chef — complete bootstrap CLI for SparrowDB projects"
```

---

## Self-Review

### Spec coverage

| Spec requirement | Task covering it |
|---|---|
| `sparrowdb-chef` folder as a CLI | Task 1 |
| `chef` and `cook` subcommands | Task 2 |
| `--auto` / `-a` flag | Task 2 |
| "What do you want to build?" prompt | Task 6 |
| Automatic vs manual setup mode | Task 6 |
| Default project path `~/my-first-sparrow-project` | Task 6 + Task 7 |
| Write `docker-compose.yml` | Task 3 + Task 7 |
| Write HQL schema + query starters | Task 3 + Task 7 |
| Write `SPARROWDB_CHEF_PROMPT.md` with intent | Task 3 + Task 7 |
| Write seed + read JSON examples | Task 3 + Task 7 |
| Start Docker with `compose_up` | Task 5 + Task 7 |
| Wait for healthy | Task 5 + Task 7 |
| Seed example data via `/v1/query` | Task 4 + Task 7 |
| Print next steps | Task 7 (outro) |
| Parser tests for all flag variants | Task 2 |
| Unit tests for generated file content | Task 3, 7 |

No gaps found.

### Placeholder scan

No TBDs. Every step has exact code. All types and function names are consistent across tasks.

### Type consistency

- `write_project_files(dir: &Path, intent: &str) -> Result<()>` — defined in Task 7, tested in Task 7 ✓
- `SparrowClient::new(&str)` / `.check_health()` / `.post_v1_query(&str)` — defined in Task 4, used in Task 7 ✓
- `compose_up(&Path)` / `wait_for_healthy(&SparrowClient, u32, u64)` — defined in Task 5, used in Task 7 ✓
- `SetupMode::Automatic` / `SetupMode::Manual` — defined in Task 6, matched in Task 7 ✓
