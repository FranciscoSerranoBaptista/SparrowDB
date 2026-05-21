use crate::commands::chef::write_project_files;
use std::fs;
use tempfile::TempDir;

#[test]
fn write_project_files_creates_all_expected_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    write_project_files(path, "build a graph app").unwrap();

    assert!(
        path.join("docker-compose.yml").exists(),
        "docker-compose.yml must exist"
    );
    assert!(
        path.join("db/schema.hx").exists(),
        "db/schema.hx must exist"
    );
    assert!(
        path.join("db/queries.hx").exists(),
        "db/queries.hx must exist"
    );
    assert!(
        path.join("examples/seed.json").exists(),
        "examples/seed.json must exist"
    );
    assert!(
        path.join("examples/read.json").exists(),
        "examples/read.json must exist"
    );
    assert!(
        path.join("SPARROWDB_CHEF_PROMPT.md").exists(),
        "prompt file must exist"
    );
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
