use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub(crate) struct SandboxArgs {
    #[command(subcommand)]
    pub action: SandboxActionCli,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SandboxActionCli {
    #[command(about = "Report or install what a sandbox provider needs on this host")]
    Install(SandboxInstallArgs),
    #[command(about = "Create a sandbox, reusing whatever the provider already installed")]
    Create(SandboxCreateArgs),
    #[command(about = "Start a stopped sandbox")]
    Start(SandboxStartArgs),
    #[command(about = "Stop a running sandbox")]
    Stop(SandboxNameArgs),
    #[command(about = "Report one sandbox's state and its next command")]
    Status(SandboxNameArgs),
    #[command(about = "List every sandbox with its provider and state")]
    List,
    #[command(about = "Remove a stopped sandbox")]
    Rm(SandboxNameArgs),
    #[command(about = "Run agent-desktop inside a sandbox and pass its output through")]
    Exec(SandboxExecArgs),
}

#[derive(Args, Debug)]
pub(crate) struct SandboxInstallArgs {
    #[arg(
        value_name = "PROVIDER",
        help = "Provider to install; omit it to report every provider's state"
    )]
    pub provider: Option<String>,
}

#[derive(Args, Debug)]
pub(crate) struct SandboxNameArgs {
    #[arg(help = "Sandbox name: lowercase letters, digits and hyphens")]
    pub name: String,
}

#[derive(Args, Debug)]
pub(crate) struct SandboxCreateArgs {
    #[arg(help = "Sandbox name: lowercase letters, digits and hyphens")]
    pub name: String,
    #[arg(
        long,
        value_name = "PROVIDER",
        help = "Provider that owns the sandbox; `sandbox install` lists the providers in this build"
    )]
    pub provider: String,
    #[arg(
        long = "set",
        value_name = "KEY=VALUE",
        help = "Provider option; repeat for more than one"
    )]
    pub set: Vec<String>,
}

#[derive(Args, Debug)]
pub(crate) struct SandboxStartArgs {
    #[arg(help = "Sandbox name: lowercase letters, digits and hyphens")]
    pub name: String,
    #[arg(
        long,
        help = "Show the sandbox's desktop in a window on this host instead of running it headless"
    )]
    pub window: bool,
    #[arg(
        long = "set",
        value_name = "KEY=VALUE",
        help = "Provider option for this start, such as title=<text>; repeat for more than one"
    )]
    pub set: Vec<String>,
}

#[derive(Args, Debug)]
pub(crate) struct SandboxExecArgs {
    #[arg(help = "Sandbox name: lowercase letters, digits and hyphens")]
    pub name: String,
    #[arg(
        last = true,
        required = true,
        value_name = "ARGS",
        help = "Arguments for agent-desktop inside the sandbox, after --"
    )]
    pub args: Vec<String>,
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
