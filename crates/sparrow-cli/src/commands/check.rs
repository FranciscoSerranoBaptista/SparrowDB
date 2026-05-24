//! Check command - validates project configuration, queries, and generated Rust code.

use crate::commands::build;
use crate::metrics_sender::MetricsSender;
use crate::output::{Operation, Step};
use crate::project::ProjectContext;
use crate::utils::sparrowc_utils::{
    analyze_source, collect_hx_files, generate_content, parse_content,
};
use crate::utils::{print_error, print_warning};
use eyre::Result;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tokio::process::Command;

/// Output from running cargo check.
struct CargoCheckOutput {
    success: bool,
    #[allow(dead_code)] // May be useful for debugging
    full_output: String,
    errors_only: String,
}

pub async fn run(instance: Option<String>, metrics_sender: &MetricsSender) -> Result<()> {
    // Load project context
    let project = ProjectContext::find_and_load(None)?;

    match instance {
        Some(instance_name) => check_instance(&project, &instance_name, metrics_sender).await,
        None => check_all_instances(&project, metrics_sender).await,
    }
}

async fn check_instance(
    project: &ProjectContext,
    instance_name: &str,
    metrics_sender: &MetricsSender,
) -> Result<()> {
    let start_time = Instant::now();

    let op = Operation::new("Checking", instance_name);

    // Validate instance exists in config
    let _instance_config = project.config.get_instance(instance_name)?;

    // Step 1: Validate syntax first (quick check)
    let mut syntax_step = Step::with_messages("Validating syntax", "Syntax validated");
    syntax_step.start();
    validate_project_syntax(project)?;
    syntax_step.done();

    // Step 2: Ensure sparrow repo is cached (reuse from build.rs)
    let mut repo_step = Step::with_messages("Syncing repository", "Repository synced");
    repo_step.start();
    build::ensure_sparrow_repo_cached().await?;
    repo_step.done();

    // Step 3: Prepare instance workspace (reuse from build.rs)
    build::prepare_instance_workspace(project, instance_name).await?;

    // Step 4: Compile project - generate queries.rs (reuse from build.rs)
    let mut compile_step = Step::with_messages("Compiling queries", "Queries compiled");
    compile_step.start();
    let metrics_data = build::compile_project(project, instance_name).await?;
    compile_step.done_with_info(&format!("{} queries", metrics_data.num_of_queries));

    // Step 5: Copy generated files to sparrow-repo-copy for cargo check.
    // Snapshot the originals first so we can restore them afterwards and keep
    // sparrow-repo-copy clean for subsequent docker builds (dirty files bust
    // the `COPY sparrow-repo-copy/ ./` Docker layer cache on every check run).
    let instance_workspace = project.instance_workspace(instance_name);
    let generated_src = instance_workspace.join("sparrow-container/src");
    let cargo_check_src = instance_workspace.join("sparrow-repo-copy/crates/sparrow-container/src");

    let original_queries = fs::read(cargo_check_src.join("queries.rs")).ok();
    let original_config = fs::read(cargo_check_src.join("config.hx.json")).ok();

    fs::copy(
        generated_src.join("queries.rs"),
        cargo_check_src.join("queries.rs"),
    )?;
    fs::copy(
        generated_src.join("config.hx.json"),
        cargo_check_src.join("config.hx.json"),
    )?;

    // Step 6: Run cargo check — capture result without propagating yet so we
    // can always restore the originals before returning.
    //
    // Use a shared, persistent target directory so compiled dependency
    // artifacts are reused across every `sparrow check` invocation on this
    // machine. Without this, cargo check recompiles all dependencies from
    // scratch each time (sparrow-repo-copy has no persistent target/).
    let mut cargo_step = Step::with_messages("Running cargo check", "Cargo check passed");
    cargo_step.start();
    Step::verbose_substep("Running cargo check on generated code...");
    let sparrow_container_dir = instance_workspace.join("sparrow-repo-copy/crates/sparrow-container");
    let check_target_dir = project.sparrow_dir.join("check-cache");
    let cargo_result = run_cargo_check(&sparrow_container_dir, &check_target_dir).await;

    // Restore sparrow-repo-copy to its original state so that future docker
    // builds are not forced to re-run `COPY sparrow-repo-copy/ ./` due to a
    // stale generated queries.rs left behind by this check.
    match original_queries {
        Some(content) => {
            if let Err(e) = fs::write(cargo_check_src.join("queries.rs"), &content) {
                print_warning(&format!("Failed to restore queries.rs: {e}. Run `sparrow build` to clean up."));
            }
        }
        None => { let _ = fs::remove_file(cargo_check_src.join("queries.rs")); }
    }
    match original_config {
        Some(content) => {
            if let Err(e) = fs::write(cargo_check_src.join("config.hx.json"), &content) {
                print_warning(&format!("Failed to restore config.hx.json: {e}. Run `sparrow build` to clean up."));
            }
        }
        None => { let _ = fs::remove_file(cargo_check_src.join("config.hx.json")); }
    }

    let cargo_output = cargo_result?;

    let compile_time = start_time.elapsed().as_secs() as u32;

    if !cargo_output.success {
        cargo_step.fail();
        op.failure();

        // Send failure telemetry
        metrics_sender.send_compile_event(
            instance_name.to_string(),
            metrics_data.queries_string,
            metrics_data.num_of_queries,
            compile_time,
            false,
            Some(cargo_output.errors_only.clone()),
        );

        // Read generated Rust for issue reporting.
        // Use generated_src (not cargo_check_src) because the original has
        // already been restored in sparrow-repo-copy at this point.
        let generated_rust = fs::read_to_string(generated_src.join("queries.rs"))
            .unwrap_or_else(|_| String::from("[Could not read generated code]"));

        // Handle failure - print errors and offer GitHub issue
        handle_cargo_check_failure(&cargo_output, &generated_rust, project)?;

        return Err(eyre::eyre!("Cargo check failed on generated Rust code"));
    }

    cargo_step.done();

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

async fn check_all_instances(
    project: &ProjectContext,
    metrics_sender: &MetricsSender,
) -> Result<()> {
    let instances: Vec<String> = project
        .config
        .list_instances()
        .into_iter()
        .map(String::from)
        .collect();

    if instances.is_empty() {
        return Err(eyre::eyre!(
            "No instances found in sparrow.toml. Add at least one instance to check."
        ));
    }

    // Check each instance
    for instance_name in &instances {
        check_instance(project, instance_name, metrics_sender).await?;
    }

    crate::output::success("All instances checked successfully");
    Ok(())
}

/// Validate project syntax by parsing queries and schema (similar to build.rs but without generating files)
fn validate_project_syntax(project: &ProjectContext) -> Result<()> {
    // Collect all .hx files for validation
    let hx_files = collect_hx_files(&project.root, &project.config.project.queries)?;

    // Generate content and validate using sparrow-db parsing logic
    let content = generate_content(&hx_files)?;
    let source = parse_content(&content)?;

    // Check if schema is empty before analyzing
    if source.schema.is_empty() {
        let error = crate::errors::CliError::new("no schema definitions found in project")
            .with_context("searched all .hx files in the queries directory but found no N:: (node) or E:: (edge) definitions")
            .with_hint("add at least one schema definition like 'N::User { name: String }' to your .hx files");
        return Err(eyre::eyre!("{}", error.render()));
    }

    // Run static analysis to catch validation errors
    analyze_source(source, &content.files)?;

    Ok(())
}

/// Run cargo check on the generated code.
///
/// `target_dir` is a shared, persistent directory (`.sparrow/check-cache`)
/// reused across all `sparrow check` invocations so dependency artifacts are
/// not recompiled from scratch on every run.
async fn run_cargo_check(sparrow_container_dir: &Path, target_dir: &Path) -> Result<CargoCheckOutput> {
    let output = Command::new("cargo")
        .arg("check")
        .arg("--color=never") // Disable color codes for cleaner output
        .arg("--target-dir")
        .arg(target_dir)
        .current_dir(sparrow_container_dir)
        .output()
        .await
        .map_err(|e| eyre::eyre!("Failed to run cargo check: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    // stderr contains the actual errors, stdout has JSON if using message-format
    let full_output = format!("{}\n{}", stderr, stdout);

    let errors_only: String = stderr
        .lines()
        .filter(|l| l.starts_with("error"))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(CargoCheckOutput {
        success: output.status.success(),
        full_output,
        errors_only,
    })
}

/// Handle cargo check failure - print errors and offer GitHub issue creation.
fn handle_cargo_check_failure(
    cargo_output: &CargoCheckOutput,
    _generated_rust: &str,
    _project: &ProjectContext,
) -> Result<()> {
    print_error("Cargo check failed on generated Rust code");
    println!();

    if !cargo_output.errors_only.is_empty() {
        eprintln!("{}", cargo_output.errors_only);
        println!();
    }

    println!("This may indicate a bug in the SparrowDB code generator.");
    println!();

    print_warning("Please report this issue at https://github.com/SparrowDB/sparrowdb/issues");

    Ok(())
}
