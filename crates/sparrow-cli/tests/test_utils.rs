//! Test utilities for sparrow-cli tests
//!
//! This module provides test infrastructure for running tests in isolation
//! without interfering with the user's environment or other parallel tests.

use std::path::PathBuf;
use tempfile::TempDir;

/// A test context that provides isolated directories for testing.
///
/// TestContext creates:
/// - A temporary project directory
/// - A temporary cache directory (set via SPARROW_CACHE_DIR env var)
/// - A temporary sparrow home directory (set via SPARROW_HOME env var)
///
/// The SPARROW_CACHE_DIR and SPARROW_HOME environment variables are automatically
/// set when the context is created and restored when it is dropped.
pub struct TestContext {
    /// The temporary directory containing everything
    pub _temp_dir: TempDir,
    /// The project path within the temp directory
    pub project_path: PathBuf,
    /// The cache directory within the temp directory
    pub cache_dir: PathBuf,
    /// The sparrow home directory within the temp directory
    pub sparrow_home: PathBuf,
    /// Guard to restore the SPARROW_CACHE_DIR env var on drop
    _cache_env_guard: EnvGuard,
    /// Guard to restore the SPARROW_HOME env var on drop
    _home_env_guard: EnvGuard,
}

/// Guard that restores an environment variable to its previous state on drop.
struct EnvGuard {
    key: &'static str,
    old_value: Option<String>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: We're restoring the environment variable to its previous state.
        // Tests using TestContext should not run in parallel with tests that
        // depend on SPARROW_CACHE_DIR, but in practice each test gets its own
        // isolated directory so this is safe.
        unsafe {
            match &self.old_value {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

impl TestContext {
    /// Create a new test context with isolated directories.
    ///
    /// This will:
    /// 1. Create a temporary directory
    /// 2. Create project, cache, and sparrow home subdirectories
    /// 3. Set the SPARROW_CACHE_DIR environment variable to the cache directory
    /// 4. Set the SPARROW_HOME environment variable to the sparrow home directory
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let project_path = temp_dir.path().join("project");
        let cache_dir = temp_dir.path().join("cache");
        let sparrow_home = temp_dir.path().join(".sparrow");

        std::fs::create_dir_all(&project_path).expect("Failed to create project dir");
        std::fs::create_dir_all(&cache_dir).expect("Failed to create cache dir");
        std::fs::create_dir_all(&sparrow_home).expect("Failed to create sparrow home dir");

        // Save old values and set new ones
        let old_cache_value = std::env::var("SPARROW_CACHE_DIR").ok();
        let old_home_value = std::env::var("SPARROW_HOME").ok();
        // SAFETY: We're setting environment variables for test isolation.
        // Each test creates its own unique temp directory, so there are no
        // data races on the actual directory contents.
        unsafe {
            std::env::set_var("SPARROW_CACHE_DIR", &cache_dir);
            std::env::set_var("SPARROW_HOME", &sparrow_home);
        }

        Self {
            _temp_dir: temp_dir,
            project_path,
            cache_dir,
            sparrow_home,
            _cache_env_guard: EnvGuard {
                key: "SPARROW_CACHE_DIR",
                old_value: old_cache_value,
            },
            _home_env_guard: EnvGuard {
                key: "SPARROW_HOME",
                old_value: old_home_value,
            },
        }
    }

    /// Create a basic sparrow project structure with valid schema and queries.
    ///
    /// This creates:
    /// - sparrow.toml configuration file
    /// - .sparrow directory
    /// - db/schema.hx with sample node and edge definitions
    /// - db/queries.hx with sample queries
    pub fn setup_valid_project(&self) {
        use sparrow_cli::config::SparrowConfig;
        use std::fs;

        // Create sparrow.toml
        let config = SparrowConfig::default_config("test-project");
        let config_path = self.project_path.join("sparrow.toml");
        config
            .save_to_file(&config_path)
            .expect("Failed to save config");

        // Create .sparrow directory
        fs::create_dir_all(self.project_path.join(".sparrow")).expect("Failed to create .sparrow");

        // Create queries directory
        let queries_dir = self.project_path.join("db");
        fs::create_dir_all(&queries_dir).expect("Failed to create queries directory");

        // Create valid schema.hx
        let schema_content = r#"
// Node types
N::User {
    name: String,
    email: String,
}

N::Post {
    title: String,
    content: String,
}

// Edge types
E::Authored {
    From: User,
    To: Post,
}

E::Likes {
    From: User,
    To: Post,
}
"#;
        fs::write(queries_dir.join("schema.hx"), schema_content)
            .expect("Failed to write schema.hx");

        // Create valid queries.hx
        let queries_content = r#"
QUERY GetUser(user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user

QUERY GetUserPosts(user_id: ID) =>
    posts <- N<User>(user_id)::Out<Authored>
    RETURN posts
"#;
        fs::write(queries_dir.join("queries.hx"), queries_content)
            .expect("Failed to write queries.hx");
    }

    /// Create a sparrow project with only schema (no queries).
    pub fn setup_schema_only_project(&self) {
        use sparrow_cli::config::SparrowConfig;
        use std::fs;

        // Create sparrow.toml
        let config = SparrowConfig::default_config("test-project");
        let config_path = self.project_path.join("sparrow.toml");
        config
            .save_to_file(&config_path)
            .expect("Failed to save config");

        // Create .sparrow directory
        fs::create_dir_all(self.project_path.join(".sparrow")).expect("Failed to create .sparrow");

        // Create queries directory with only schema
        let queries_dir = self.project_path.join("db");
        fs::create_dir_all(&queries_dir).expect("Failed to create queries directory");

        let schema_content = r#"
N::User {
    name: String,
    email: String,
}

E::Follows {
    From: User,
    To: User,
}
"#;
        fs::write(queries_dir.join("schema.hx"), schema_content)
            .expect("Failed to write schema.hx");
    }

    /// Create a sparrow project without schema definitions (queries only, should fail validation).
    pub fn setup_project_without_schema(&self) {
        use sparrow_cli::config::SparrowConfig;
        use std::fs;

        // Create sparrow.toml
        let config = SparrowConfig::default_config("test-project");
        let config_path = self.project_path.join("sparrow.toml");
        config
            .save_to_file(&config_path)
            .expect("Failed to save config");

        // Create .sparrow directory
        fs::create_dir_all(self.project_path.join(".sparrow")).expect("Failed to create .sparrow");

        // Create queries directory with only queries (no schema)
        let queries_dir = self.project_path.join("db");
        fs::create_dir_all(&queries_dir).expect("Failed to create queries directory");

        let queries_content = r#"
QUERY GetUser(user_id: ID) =>
    user <- N<User>(user_id)
    RETURN user
"#;
        fs::write(queries_dir.join("queries.hx"), queries_content)
            .expect("Failed to write queries.hx");
    }

    /// Create a sparrow project with invalid syntax in queries.
    pub fn setup_project_with_invalid_syntax(&self) {
        use sparrow_cli::config::SparrowConfig;
        use std::fs;

        // Create sparrow.toml
        let config = SparrowConfig::default_config("test-project");
        let config_path = self.project_path.join("sparrow.toml");
        config
            .save_to_file(&config_path)
            .expect("Failed to save config");

        // Create .sparrow directory
        fs::create_dir_all(self.project_path.join(".sparrow")).expect("Failed to create .sparrow");

        // Create queries directory
        let queries_dir = self.project_path.join("db");
        fs::create_dir_all(&queries_dir).expect("Failed to create queries directory");

        // Create valid schema
        let schema_content = r#"
N::User {
    name: String,
}
"#;
        fs::write(queries_dir.join("schema.hx"), schema_content)
            .expect("Failed to write schema.hx");

        // Create queries with invalid syntax
        let invalid_queries = r#"
QUERY InvalidQuery {
    this is not valid helix syntax!!!
}
"#;
        fs::write(queries_dir.join("queries.hx"), invalid_queries)
            .expect("Failed to write queries.hx");
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

