use cliclack::{intro, log, outro, spinner};
use eyre::Result;
use std::fs;
use std::path::Path;

use crate::docker::{compose_up, wait_for_healthy};
use crate::http::SparrowClient;
use crate::prompts::{SetupMode, ask_build_intent, ask_project_path, ask_setup_mode, confirm_step};
use crate::templates::{chef_prompt, docker_compose, queries_hx, read_json, schema_hx, seed_json};

const BASE_URL: &str = "http://localhost:6969";

/// Write all project files to `dir`. Public for testability.
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

    let intent = if auto {
        String::new()
    } else {
        ask_build_intent()?
    };

    let mode = if auto {
        SetupMode::Automatic
    } else {
        ask_setup_mode()?
    };

    let project_dir = if auto || mode == SetupMode::Automatic {
        dirs_next::home_dir()
            .map(|h| h.join("my-first-sparrow-project"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/my-first-sparrow-project"))
    } else {
        ask_project_path()?
    };

    let should_write = if mode == SetupMode::Manual {
        confirm_step(&format!("Write project files to {}", project_dir.display()))?
    } else {
        true
    };

    if should_write {
        let sp = spinner();
        sp.start("Writing project files…");
        if let Err(e) = write_project_files(&project_dir, &intent) {
            sp.stop("Writing project files failed");
            outro("Setup did not complete. Check the error above.")?;
            return Err(e);
        }
        sp.stop(format!(
            "Project files written to {}",
            project_dir.display()
        ));
    }

    let should_start = if mode == SetupMode::Manual {
        confirm_step("Start SparrowDB with docker compose")?
    } else {
        true
    };

    if should_start {
        let sp = spinner();
        sp.start("Starting SparrowDB…");
        if let Err(e) = compose_up(&project_dir).await {
            sp.stop("Docker compose failed");
            outro("Setup did not complete. Check the error above.")?;
            return Err(e);
        }
        sp.stop("SparrowDB container started");

        let client = SparrowClient::new(BASE_URL);
        let sp = spinner();
        sp.start("Waiting for SparrowDB to be ready…");
        if let Err(e) = wait_for_healthy(&client, 24, 2500).await {
            sp.stop("Health check timed out");
            outro("Setup did not complete. Check the error above.")?;
            return Err(e);
        }
        sp.stop("SparrowDB is ready");

        let should_seed = if mode == SetupMode::Manual {
            confirm_step("Seed example data")?
        } else {
            true
        };

        if should_seed {
            let sp = spinner();
            sp.start("Seeding example data…");
            if let Err(e) = client.post_v1_query(&seed_json()).await {
                sp.stop("Seeding failed");
                outro("Setup did not complete. Check the error above.")?;
                return Err(e);
            }
            sp.stop("Example data seeded");
        }
    }

    log::success("Done!")?;
    if should_write {
        outro(format!(
            "Your SparrowDB project is at: {}\n\nOpen SPARROWDB_CHEF_PROMPT.md and hand it to your coding agent.\nSparrowDB API: {BASE_URL}",
            project_dir.display()
        ))?;
    } else {
        outro("Nothing was changed. Run again when you're ready.")?;
    }

    Ok(())
}
