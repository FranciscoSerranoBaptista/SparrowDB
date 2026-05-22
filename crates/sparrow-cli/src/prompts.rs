//! Interactive prompts for the Sparrow CLI using cliclack.

use eyre::Result;
use std::path::Path;

/// Show the intro banner for interactive mode
pub fn intro(title: &str, subheader: Option<&str>) -> Result<()> {
    match subheader {
        Some(sub) => cliclack::note(title, sub)?,
        None => cliclack::intro(title.to_string())?,
    }
    Ok(())
}

/// Show note banner
#[allow(unused)]
pub fn note(message: &str) -> Result<()> {
    cliclack::log::remark(message)?;
    Ok(())
}

/// Show warning banner
#[allow(unused)]
pub fn warning(message: &str) -> Result<()> {
    cliclack::log::warning(message)?;
    Ok(())
}

/// Show the outro banner when interactive mode completes
#[allow(dead_code)]
pub fn outro(message: &str) -> Result<()> {
    cliclack::outro(message.to_string())?;
    Ok(())
}

/// Prompt user for a yes/no confirmation
pub fn confirm(message: &str) -> Result<bool> {
    let result = cliclack::confirm(message).interact()?;
    Ok(result)
}

pub fn confirm_overwrite(path: &Path) -> Result<bool> {
    confirm(&format!("File '{}' exists. Overwrite it?", path.display()))
}

/// Prompt user to enter an instance name
pub fn input_instance_name(default: &str) -> Result<String> {
    let name: String = cliclack::input("Instance name")
        .default_input(default)
        .placeholder(default)
        .validate(|input: &String| {
            if input.is_empty() {
                Err("Instance name cannot be empty")
            } else if input.len() > 32 {
                Err("Instance name must be 32 characters or less")
            } else if !input
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                Err("Instance name can only contain letters, numbers, hyphens, and underscores")
            } else {
                Ok(())
            }
        })
        .interact()?;

    Ok(name)
}

/// Check if we're running in an interactive terminal
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Prompt user to select an instance from available instances.
///
/// Auto-selects if only one instance exists; errors if none.
pub fn select_instance(instances: &[(&String, &str)]) -> Result<String> {
    if instances.is_empty() {
        return Err(eyre::eyre!(
            "No instances found in sparrow.toml. Run 'sparrow init' to create a project first."
        ));
    }

    if instances.len() == 1 {
        return Ok(instances[0].0.clone());
    }

    let mut select = cliclack::select("Select an instance");
    for (name, type_hint) in instances {
        select = select.item((*name).clone(), name.as_str(), *type_hint);
    }
    let selected = select.interact()?;
    Ok(selected)
}
