use crate::cleanup::CleanupTracker;
use crate::config::{DbConfig, LocalInstanceConfig, SparrowConfig, StorageBackend};
use crate::errors::project_error;
use crate::output::Operation;
use crate::prompts;
use crate::utils::print_instructions;
use eyre::Result;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

pub async fn run(
    path: Option<String>,
    _template: String,
    queries_path: String,
    name: Option<String>,
) -> Result<()> {
    let mut cleanup_tracker = CleanupTracker::new();

    // Execute the init logic, capturing any errors
    let result = run_init_inner(path, queries_path, name, &mut cleanup_tracker).await;

    // If there was an error, perform cleanup
    if let Err(ref e) = result
        && cleanup_tracker.has_tracked_resources()
    {
        eprintln!("Init failed, performing cleanup: {}", e);
        let summary = cleanup_tracker.cleanup();
        summary.log_summary();
    }

    result
}

async fn run_init_inner(
    path: Option<String>,
    queries_path: String,
    name: Option<String>,
    cleanup_tracker: &mut CleanupTracker,
) -> Result<()> {
    let project_dir = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => env::current_dir()?,
    };

    let project_name = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("helix-project");

    let config_path = project_dir.join("sparrow.toml");

    if config_path.exists() {
        return Err(project_error(format!(
            "sparrow.toml already exists in {}",
            project_dir.display()
        ))
        .with_hint("use 'sparrow add' to add a new instance to the existing project")
        .into());
    }

    let op = Operation::new("Initializing", project_name);

    // Create project directory if it doesn't exist
    let project_dir_existed = project_dir.exists();
    fs::create_dir_all(&project_dir)?;
    if !project_dir_existed {
        cleanup_tracker.track_dir(project_dir.clone());
    }

    let interactive = prompts::is_interactive();

    // Create default sparrow.toml with custom queries path
    let mut config = SparrowConfig::default_config(project_name);
    config.project.queries = std::path::PathBuf::from(&queries_path);

    // Determine local instance name
    let local_instance_name = if let Some(n) = name {
        n
    } else if interactive {
        prompts::intro(
            "sparrow init",
            Some(
                "This will create a new Helix project in the current directory.\nYou can configure the project name and other settings below.",
            ),
        )?;
        // If the user didn't pass --name, prompt only for an instance name
        prompts::input_instance_name("dev")?
    } else {
        "dev".to_string()
    };

    // Rename the default "dev" instance if a custom name was given
    if local_instance_name != "dev" {
        let local_cfg = config.local.remove("dev").unwrap_or(LocalInstanceConfig {
            port: Some(6969),
            build_mode: crate::config::BuildMode::Dev,
            storage_backend: StorageBackend::Lmdb,
            db_config: DbConfig::default(),
        });
        config.local.insert(local_instance_name.clone(), local_cfg);
    }

    // Save initial config and track it
    config.save_to_file(&config_path)?;
    cleanup_tracker.track_file(config_path.clone());

    // Create project structure
    create_project_structure(&project_dir, &queries_path, interactive, cleanup_tracker)?;

    op.success();
    let queries_path_clean = queries_path.trim_end_matches('/');

    let next_steps = vec![
        format!("Edit {queries_path_clean}/schema.hx to define your data model"),
        format!("Add queries to {queries_path_clean}/queries.hx"),
        format!(
            "Run 'sparrow push {local_instance_name}' to start your development instance"
        ),
    ];

    let next_step_refs: Vec<&str> = next_steps.iter().map(String::as_str).collect();
    print_instructions("Next steps:", &next_step_refs);

    Ok(())
}

fn create_project_structure(
    project_dir: &Path,
    queries_path: &str,
    interactive: bool,
    cleanup_tracker: &mut CleanupTracker,
) -> Result<()> {
    // Create directories
    let sparrow_dir = project_dir.join(".sparrow");
    let sparrow_dir_existed = sparrow_dir.exists();
    fs::create_dir_all(&sparrow_dir)?;
    if !sparrow_dir_existed {
        cleanup_tracker.track_dir(sparrow_dir);
    }

    let queries_dir = project_dir.join(queries_path);
    let queries_dir_existed = queries_dir.exists();
    fs::create_dir_all(&queries_dir)?;
    if !queries_dir_existed {
        cleanup_tracker.track_dir(queries_dir);
    }

    // Create default schema.hx with proper Helix syntax
    let default_schema = r#"// Start building your schema here.
//
// The schema is used to to ensure a level of type safety in your queries.
//
// The schema is made up of Node types, denoted by N::,
// and Edge types, denoted by E::
//
// Under the Node types you can define fields that
// will be stored in the database.
//
// Under the Edge types you can define what type of node
// the edge will connect to and from, and also the
// properties that you want to store on the edge.
//
// Example:
//
// N::User {
//     Name: String,
//     Label: String,
//     Age: I64,
//     IsAdmin: Boolean,
// }
//
// E::Knows {
//     From: User,
//     To: User,
//     Properties: {
//         Since: I64,
//     }
// }
"#;
    let schema_path = project_dir.join(queries_path).join("schema.hx");
    write_starter_file(&schema_path, default_schema, interactive, cleanup_tracker)?;

    // Create default queries.hx with proper Helix query syntax in the queries directory
    let default_queries = r#"// Start writing your queries here.
//
// You can use the schema to help you write your queries.
//
// Queries take the form:
//     QUERY {query name}({input name}: {input type}) =>
//         {variable} <- {traversal}
//         RETURN {variable}
//
// Example:
//     QUERY GetUserFriends(user_id: String) =>
//         friends <- N<User>(user_id)::Out<Knows>
//         RETURN friends
//
//
// For more information on how to write queries,
// see the documentation at https://docs.helix-db.com
// or checkout our GitHub at https://github.com/SparrowDB/helix-db
"#;
    let queries_path_file = project_dir.join(queries_path).join("queries.hx");
    write_starter_file(
        &queries_path_file,
        default_queries,
        interactive,
        cleanup_tracker,
    )?;

    // add this to .gitignore
    let gitignore = [".sparrow/", "target/", "*.log"];
    let gitignore_path = project_dir.join(".gitignore");
    let file_existed = gitignore_path.exists();
    let existing = fs::read_to_string(&gitignore_path).unwrap_or_default();

    let missing_entries: Vec<&str> = gitignore
        .iter()
        .copied()
        .filter(|entry| !existing.lines().any(|line| line.trim() == *entry))
        .collect();

    if !missing_entries.is_empty() {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore_path)?;

        if !existing.is_empty() && !existing.ends_with('\n') {
            writeln!(file)?;
        }

        for entry in missing_entries {
            writeln!(file, "{entry}")?;
        }
    }

    if !file_existed {
        cleanup_tracker.track_file(gitignore_path);
    }

    Ok(())
}

fn write_starter_file(
    path: &Path,
    content: &str,
    interactive: bool,
    cleanup_tracker: &mut CleanupTracker,
) -> Result<()> {
    if path.exists() {
        let should_overwrite = if interactive {
            prompts::confirm_overwrite(path)?
        } else {
            false
        };

        if !should_overwrite {
            crate::output::warning(&format!("Skipping existing file: {}", path.display()));
            return Ok(());
        }
    }

    let should_track = !path.exists();
    fs::write(path, content)?;
    if should_track {
        cleanup_tracker.track_file(path.to_path_buf());
    }
    Ok(())
}
