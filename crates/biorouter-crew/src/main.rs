use anyhow::{bail, Result};

#[cfg(unix)]
const USAGE: &str = "usage: biorouter-crew serve|start|status|stop (--state-dir PATH | --name NAME) [--name NAME] [--bootstrap-key HEX]; bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID";

/// Where a named workspace keeps its state when `--state-dir` is not given:
/// `$HOME/.local/share/biorouter-crew/<name>`. The name is validated first, so it is a plain
/// ASCII slug and can never climb out of that directory.
#[cfg(unix)]
fn default_state_dir(name: &str) -> Result<std::path::PathBuf> {
    biorouter_crew::validate_workspace_name(name).map_err(|error| anyhow::anyhow!(error.wire()))?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| {
            anyhow::anyhow!("--state-dir required: HOME is not set to an absolute path")
        })?;
    Ok(home
        .join(".local")
        .join("share")
        .join("biorouter-crew")
        .join(name))
}

#[cfg(unix)]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");
    if matches!(command, "--version" | "-V") {
        println!("biorouter-crew {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if matches!(command, "help" | "--help" | "-h") {
        println!("Usage: biorouter-crew start --name NAME --bootstrap-key HEX [--state-dir PATH]\n       biorouter-crew serve|start|status|stop --state-dir PATH [--name NAME] [--bootstrap-key HEX]\n       biorouter-crew bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID\n       biorouter-crew --version\n--name names the workspace (lowercase letters, numbers and hyphens); without --state-dir its state lives in ~/.local/share/biorouter-crew/NAME. start prints the workspace's invitation line (brcrew1:...) to paste into Crew.\nBroker and bridge operations require Linux.");
        return Ok(());
    }

    let option = |name: &str| -> Option<&str> {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].as_str())
    };
    if command == "__crew-exec" {
        biorouter_crew::remote::exec_helper()?;
    } else if command == "bridge" {
        biorouter_crew::bridge(
            std::path::Path::new(
                option("--socket").ok_or_else(|| anyhow::anyhow!("--socket required"))?,
            ),
            option("--owner-uid")
                .ok_or_else(|| anyhow::anyhow!("--owner-uid required"))?
                .parse()?,
            option("--workspace-id").ok_or_else(|| anyhow::anyhow!("--workspace-id required"))?,
        )?;
    } else if matches!(command, "serve" | "start" | "status" | "stop") {
        let name = option("--name");
        if args.iter().any(|arg| arg == "--name") && name.is_none() {
            bail!("--name requires a workspace name");
        }
        let root = match (option("--state-dir"), name) {
            (Some(root), _) => std::path::PathBuf::from(root),
            (None, Some(name)) => {
                let root = default_state_dir(name)?;
                if matches!(command, "serve" | "start") {
                    if let Some(parent) = root.parent() {
                        use std::os::unix::fs::DirBuilderExt;
                        std::fs::DirBuilder::new()
                            .recursive(true)
                            .mode(0o700)
                            .create(parent)?;
                    }
                }
                root
            }
            (None, None) => bail!(
                "--state-dir required (existing private parent under HOME), or --name to use ~/.local/share/biorouter-crew/NAME"
            ),
        };
        let key = option("--bootstrap-key").unwrap_or("");
        if command == "serve" {
            biorouter_crew::serve(&root, key, name)?;
        } else {
            let name = if command == "start" { name } else { None };
            println!("{}", biorouter_crew::lifecycle(command, &root, key, name)?);
        }
    } else {
        bail!("{USAGE}");
    }
    Ok(())
}
#[cfg(not(unix))]
fn main() -> Result<()> {
    bail!("Crew broker and bridge require Linux; use the desktop SSH connection manager on this platform")
}
