//! Tests for lifecycle commands (start, stop, restart, status, delete)
//!
//! These tests focus on error paths and project configuration validation
//! that don't require Docker to be actually running.

mod test_utils;

use sparrow_cli::commands::{restart, start, status, stop};
use sparrow_cli::config::SparrowConfig;
use test_utils::TestContext;
use std::fs;

// ============================================================================
// Start Command Tests
// ============================================================================

#[tokio::test]
async fn test_start_fails_without_sparrow_project() {
    let ctx = TestContext::new();
    // Don't set up any project

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = start::run(Some("dev".to_string())).await;
    assert!(
        result.is_err(),
        "Start should fail when not in a sparrow project"
    );
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("not found") || error_msg.contains("sparrow.toml"),
        "Error should mention missing project configuration"
    );
}

#[tokio::test]
async fn test_start_fails_with_nonexistent_instance() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = start::run(Some("nonexistent".to_string())).await;
    assert!(
        result.is_err(),
        "Start should fail for nonexistent instance"
    );
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("not found") || error_msg.contains("nonexistent"),
        "Error should mention instance not found"
    );
}

#[tokio::test]
async fn test_start_fails_without_build_artifacts() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    // dev instance exists in config but has not been built
    let result = start::run(Some("dev".to_string())).await;

    // This should fail because docker-compose.yml doesn't exist
    // (either due to not being built OR Docker not being available)
    assert!(result.is_err(), "Start should fail without build artifacts");
    let error_msg = format!("{:?}", result.err().unwrap());
    // Could fail due to missing build OR missing Docker
    assert!(
        error_msg.contains("built")
            || error_msg.contains("docker")
            || error_msg.contains("Docker")
            || error_msg.contains("not found"),
        "Error should indicate missing build or Docker: {error_msg}"
    );
}

#[tokio::test]
async fn test_start_requires_instance_when_non_interactive() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    // In non-interactive mode, start without instance should fail
    // because prompts::is_interactive() returns false in tests
    let result = start::run(None).await;

    // This should fail asking for an instance
    assert!(result.is_err(), "Start should fail without instance name");
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("No instance") || error_msg.contains("instance"),
        "Error should mention no instance specified"
    );
}

// ============================================================================
// Stop Command Tests
// ============================================================================

#[tokio::test]
async fn test_stop_fails_without_sparrow_project() {
    let ctx = TestContext::new();
    // Don't set up any project

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = stop::run(Some("dev".to_string())).await;
    assert!(
        result.is_err(),
        "Stop should fail when not in a sparrow project"
    );
}

#[tokio::test]
async fn test_stop_fails_with_nonexistent_instance() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = stop::run(Some("nonexistent".to_string())).await;
    assert!(result.is_err(), "Stop should fail for nonexistent instance");
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("not found") || error_msg.contains("nonexistent"),
        "Error should mention instance not found"
    );
}

#[tokio::test]
async fn test_stop_requires_instance_when_non_interactive() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = stop::run(None).await;
    assert!(result.is_err(), "Stop should fail without instance name");
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("No instance") || error_msg.contains("instance"),
        "Error should mention no instance specified"
    );
}

// ============================================================================
// Restart Command Tests
// ============================================================================

#[tokio::test]
async fn test_restart_fails_without_sparrow_project() {
    let ctx = TestContext::new();
    // Don't set up any project

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = restart::run(Some("dev".to_string())).await;
    assert!(
        result.is_err(),
        "Restart should fail when not in a sparrow project"
    );
}

#[tokio::test]
async fn test_restart_fails_with_nonexistent_instance() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = restart::run(Some("nonexistent".to_string())).await;
    assert!(
        result.is_err(),
        "Restart should fail for nonexistent instance"
    );
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("not found") || error_msg.contains("nonexistent"),
        "Error should mention instance not found"
    );
}

#[tokio::test]
async fn test_restart_fails_without_build_artifacts() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    // dev instance exists but has not been built
    let result = restart::run(Some("dev".to_string())).await;

    // This should fail because docker-compose.yml doesn't exist
    assert!(
        result.is_err(),
        "Restart should fail without build artifacts"
    );
    let error_msg = format!("{:?}", result.err().unwrap());
    // Could fail due to missing build OR missing Docker
    assert!(
        error_msg.contains("built")
            || error_msg.contains("docker")
            || error_msg.contains("Docker")
            || error_msg.contains("not found"),
        "Error should indicate missing build or Docker: {error_msg}"
    );
}

#[tokio::test]
async fn test_restart_requires_instance_when_non_interactive() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = restart::run(None).await;
    assert!(result.is_err(), "Restart should fail without instance name");
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("No instance") || error_msg.contains("instance"),
        "Error should mention no instance specified"
    );
}

// ============================================================================
// Status Command Tests
// ============================================================================

#[tokio::test]
async fn test_status_handles_no_project_gracefully() {
    let ctx = TestContext::new();
    // Don't set up any project

    let _guard = std::env::set_current_dir(&ctx.project_path);

    // Status command should handle missing project gracefully (not panic)
    let result = status::run().await;
    // Status should succeed but print an error message
    assert!(result.is_ok(), "Status should not panic without project");
}

#[tokio::test]
async fn test_status_shows_project_info() {
    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    // Status command should succeed when in a valid project
    let result = status::run().await;
    // Status will try to check Docker which may not be available,
    // but it should still succeed by handling Docker errors gracefully
    assert!(result.is_ok(), "Status should succeed in valid project");
}

#[tokio::test]
async fn test_status_with_multiple_instances() {
    let ctx = TestContext::new();

    // Create project with multiple instances
    let mut config = SparrowConfig::default_config("multi-instance-project");
    config.local.insert(
        "staging".to_string(),
        sparrow_cli::config::LocalInstanceConfig {
            port: Some(6970),
            build_mode: sparrow_cli::config::BuildMode::Dev,
            storage_backend: sparrow_cli::config::StorageBackend::Lmdb,
            db_config: sparrow_cli::config::DbConfig::default(),
        },
    );
    config.local.insert(
        "production".to_string(),
        sparrow_cli::config::LocalInstanceConfig {
            port: Some(6971),
            build_mode: sparrow_cli::config::BuildMode::Release,
            storage_backend: sparrow_cli::config::StorageBackend::Lmdb,
            db_config: sparrow_cli::config::DbConfig::default(),
        },
    );

    config
        .save_to_file(&ctx.project_path.join("sparrow.toml"))
        .expect("Failed to save config");
    fs::create_dir_all(ctx.project_path.join(".sparrow")).expect("Failed to create .sparrow");

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = status::run().await;
    assert!(
        result.is_ok(),
        "Status should succeed with multiple instances"
    );
}

// ============================================================================
// Delete Command Tests (limited - requires confirmation)
// ============================================================================

#[tokio::test]
async fn test_delete_fails_with_nonexistent_instance_dev() {
    use sparrow_cli::commands::delete;

    let ctx = TestContext::new();

    // Create sparrow.toml but clear all instances - this prevents the test from
    // walking up directories and finding another test's sparrow.toml
    let mut config = SparrowConfig::default_config("test-project");
    config.local.clear(); // Remove the default "dev" instance
    config
        .save_to_file(&ctx.project_path.join("sparrow.toml"))
        .expect("Failed to save config");
    fs::create_dir_all(ctx.project_path.join(".sparrow")).expect("Failed to create .sparrow");

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = delete::run("dev".to_string()).await;
    assert!(
        result.is_err(),
        "Delete should fail when instance doesn't exist"
    );
}

#[tokio::test]
async fn test_delete_fails_with_nonexistent_instance() {
    use sparrow_cli::commands::delete;

    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let _guard = std::env::set_current_dir(&ctx.project_path);

    let result = delete::run("nonexistent".to_string()).await;
    assert!(
        result.is_err(),
        "Delete should fail for nonexistent instance"
    );
    let error_msg = format!("{:?}", result.err().unwrap());
    assert!(
        error_msg.contains("not found") || error_msg.contains("nonexistent"),
        "Error should mention instance not found"
    );
}

// ============================================================================
// Instance Configuration Tests
// ============================================================================

#[tokio::test]
async fn test_instance_validation_rejects_empty_name() {
    let _ctx = TestContext::new();

    // Create config manually with validation
    let config = SparrowConfig::default_config("test-project");

    // The get_instance method should fail for empty string
    let result = config.get_instance("");
    assert!(
        result.is_err(),
        "get_instance should fail for empty instance name"
    );
}

#[tokio::test]
async fn test_project_context_finds_sparrow_toml() {
    use sparrow_cli::project::ProjectContext;

    let ctx = TestContext::new();
    ctx.setup_valid_project();

    let result = ProjectContext::find_and_load(Some(&ctx.project_path));
    assert!(
        result.is_ok(),
        "ProjectContext should find sparrow.toml: {:?}",
        result.err()
    );

    let project = result.unwrap();
    assert_eq!(project.config.project.name, "test-project");
}

#[tokio::test]
async fn test_project_context_fails_without_sparrow_toml() {
    use sparrow_cli::project::ProjectContext;

    let ctx = TestContext::new();
    // Don't create sparrow.toml

    let result = ProjectContext::find_and_load(Some(&ctx.project_path));
    assert!(
        result.is_err(),
        "ProjectContext should fail without sparrow.toml"
    );
}
