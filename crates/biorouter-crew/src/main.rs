use anyhow::{bail, Result};
#[cfg(unix)]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");
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
        let root = std::path::Path::new(option("--state-dir").ok_or_else(|| {
            anyhow::anyhow!("--state-dir required (existing private parent under HOME)")
        })?);
        let key = option("--bootstrap-key").unwrap_or("");
        if command == "serve" {
            biorouter_crew::serve(root, key)?;
        } else {
            println!("{}", biorouter_crew::lifecycle(command, root, key)?);
        }
    } else {
        bail!("usage: biorouter-crew serve|start|status|stop --state-dir PATH [--bootstrap-key HEX]; bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID");
    }
    Ok(())
}
#[cfg(not(unix))]
fn main() -> Result<()> {
    bail!("Crew broker and bridge require Linux; use the desktop SSH connection manager on this platform")
}
