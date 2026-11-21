use agent_desktop_core::{AppError, ErrorCode};
use clap::Parser;

use super::reject_outer_options;

#[test]
fn outer_global_options_are_rejected_before_sandbox_exec() {
    let plain =
        crate::Cli::try_parse_from(["agent-desktop", "sandbox", "exec", "dev", "--", "version"])
            .unwrap();
    assert!(reject_outer_options(&plain).is_ok());

    let outer = crate::Cli::try_parse_from([
        "agent-desktop",
        "--session",
        "abc",
        "sandbox",
        "exec",
        "dev",
        "--",
        "version",
    ])
    .unwrap();
    let AppError::Adapter(error) = reject_outer_options(&outer).unwrap_err() else {
        panic!("outer options must be an adapter error");
    };

    assert_eq!(error.code, ErrorCode::InvalidArgs);
    assert!(
        error.suggestion.is_some(),
        "the refusal must say where the options go"
    );
}

struct IdleAdapter;
impl agent_desktop_core::ObservationOps for IdleAdapter {}
impl agent_desktop_core::ActionOps for IdleAdapter {}
impl agent_desktop_core::InputOps for IdleAdapter {}
impl agent_desktop_core::SystemOps for IdleAdapter {}

#[test]
fn a_sandbox_exec_refusal_reports_that_nothing_was_delivered() {
    let cli = crate::Cli::try_parse_from(["agent-desktop", "sandbox", "exec", "BAD", "--", "ls"])
        .unwrap();
    let Some(crate::cli::Commands::Sandbox(args)) = &cli.command else {
        panic!("sandbox command");
    };
    let crate::cli_args::sandbox::SandboxActionCli::Exec(exec) = &args.action else {
        panic!("exec action");
    };

    let AppError::Adapter(error) = super::exec_passthrough(&cli, exec, &IdleAdapter).unwrap_err()
    else {
        panic!("an invalid name must be an adapter error");
    };

    assert_eq!(error.code, ErrorCode::InvalidArgs);
    assert_eq!(
        error.disposition,
        agent_desktop_core::DeliverySemantics::not_delivered()
    );
}

#[test]
fn sandbox_start_window_reaches_the_provider_as_a_setting_beside_the_others() {
    let cli = crate::Cli::try_parse_from([
        "agent-desktop",
        "sandbox",
        "start",
        "dev-1",
        "--window",
        "--set",
        "title=Review build",
    ])
    .unwrap();
    let Some(crate::cli::Commands::Sandbox(args)) = cli.command else {
        panic!("sandbox command");
    };

    let agent_desktop_core::commands::sandbox_action::SandboxAction::Start { set, .. } =
        super::to_core(args.action)
    else {
        panic!("start action");
    };

    assert_eq!(set, ["window=on", "title=Review build"]);
}
