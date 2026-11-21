use clap::Parser;

use super::SandboxActionCli;
use crate::cli::{Cli, Commands};

fn exec_action(argv: &[&str]) -> super::SandboxExecArgs {
    let mut full = vec!["agent-desktop", "sandbox"];
    full.extend_from_slice(argv);
    let cli = Cli::try_parse_from(full).expect("the sandbox command parses");
    let Commands::Sandbox(args) = cli.command.expect("the sandbox command is present") else {
        panic!("expected the sandbox command");
    };
    let SandboxActionCli::Exec(exec) = args.action else {
        panic!("expected the exec action");
    };
    exec
}

#[test]
fn sandbox_exec_hands_every_argument_after_the_separator_to_the_guest() {
    let exec = exec_action(&[
        "exec",
        "dev",
        "--",
        "snapshot",
        "--app",
        "Finder",
        "--verbose",
    ]);

    assert_eq!(exec.name, "dev");
    assert_eq!(exec.args, ["snapshot", "--app", "Finder", "--verbose"]);
}

#[test]
fn sandbox_install_takes_an_optional_provider_and_no_settings() {
    let cli = Cli::try_parse_from(["agent-desktop", "sandbox", "install", "macos"])
        .expect("sandbox install parses");
    assert!(matches!(
        cli.command,
        Some(Commands::Sandbox(super::SandboxArgs {
            action: SandboxActionCli::Install(ref args),
        })) if args.provider.as_deref() == Some("macos")
    ));

    let error = Cli::try_parse_from([
        "agent-desktop",
        "sandbox",
        "install",
        "macos",
        "--set",
        "binary=/tmp/agent-desktop",
    ])
    .expect_err("install takes no provider settings");
    assert_eq!(error.exit_code(), 2);

    let error = Cli::try_parse_from(["agent-desktop", "install"])
        .expect_err("install is not a top-level command");
    assert_eq!(error.exit_code(), 2);
}
