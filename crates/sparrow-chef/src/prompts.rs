use eyre::{Result, bail};

#[derive(Debug, PartialEq)]
pub enum SetupMode {
    Automatic,
    Manual,
}

/// Ask the user what they want to build. Empty answer is allowed.
pub fn ask_build_intent() -> Result<String> {
    let intent: String = cliclack::input("What do you want to build? (press Enter to skip)")
        .placeholder("e.g. a social graph, a recommendation engine")
        .required(false)
        .interact()?;
    Ok(intent)
}

/// Ask whether to run automatic or manual setup.
pub fn ask_setup_mode() -> Result<SetupMode> {
    let choice: String = cliclack::select("How would you like to set up?")
        .item(
            "auto".to_string(),
            "Automatic setup",
            "run the full flow with defaults",
        )
        .item(
            "manual".to_string(),
            "Manual setup",
            "confirm or customise each step",
        )
        .interact()?;
    Ok(match choice.as_str() {
        "auto" => SetupMode::Automatic,
        "manual" => SetupMode::Manual,
        other => bail!("unexpected setup mode: {other}"),
    })
}

/// Ask for the project path, defaulting to `~/my-first-sparrow-project`.
pub fn ask_project_path() -> Result<std::path::PathBuf> {
    let default = dirs_next::home_dir()
        .map(|h| {
            h.join("my-first-sparrow-project")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| "./my-first-sparrow-project".to_string());

    let raw: String = cliclack::input("Where should the project be created?")
        .placeholder(&default)
        .default_input(&default)
        .interact()?;

    Ok(std::path::PathBuf::from(raw))
}

/// In manual mode: ask the user to confirm before running a step.
pub fn confirm_step(step_name: &str) -> Result<bool> {
    let ok: bool = cliclack::confirm(format!("Run step: {step_name}?"))
        .initial_value(true)
        .interact()?;
    Ok(ok)
}

/// Returns false when stdout is not a TTY (e.g. in tests or piped output).
pub fn is_interactive() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdout())
}
