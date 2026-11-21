use agent_desktop_core::{
    AppError, ExecPassthrough, PlatformAdapter, commands::sandbox,
    commands::sandbox_action::SandboxAction,
};
use serde_json::Value;
use std::io::Write;
use std::process::ExitCode;

use crate::cli_args::sandbox::{SandboxActionCli, SandboxArgs, SandboxExecArgs};

pub(crate) fn dispatch(
    args: SandboxArgs,
    adapter: &dyn PlatformAdapter,
) -> Result<Value, AppError> {
    sandbox::execute(to_core(args.action), adapter)
}

fn reject_outer_options(cli: &crate::Cli) -> Result<(), AppError> {
    let any_set = [
        cli.visual_debug.verbose,
        cli.visual_debug.debug,
        cli.visual_debug.screenshot.is_some(),
        cli.identity.session.is_some(),
        cli.identity.agent_id.is_some(),
        cli.trace.is_some(),
        cli.trace_strict,
        cli.interaction.headed,
        cli.post_action_wait.wait_for.is_some(),
        cli.post_action_wait.wait_for_gone.is_some(),
        cli.post_action_wait.wait_timeout.is_some(),
    ]
    .contains(&true);
    if !any_set {
        return Ok(());
    }
    Err(AppError::invalid_input_with_suggestion(
        "Options before `sandbox exec` are not applied inside the sandbox.",
        "Put agent-desktop options after `--`, for example `agent-desktop sandbox exec dev -- snapshot --app Finder --session <id>`.",
    ))
}

pub(crate) fn run_exec(
    cli: &crate::Cli,
    args: &SandboxExecArgs,
    cmd_name: &str,
    adapter: &dyn PlatformAdapter,
) -> ExitCode {
    match exec_passthrough(cli, args, adapter) {
        Ok(passthrough) => match write_stdout(&passthrough.stdout) {
            Ok(()) => exit_code(passthrough.status),
            Err(error) => crate::report_output_failure(error),
        },
        Err(error) => crate::finish(cmd_name, Err(error)),
    }
}

fn exec_passthrough(
    cli: &crate::Cli,
    args: &SandboxExecArgs,
    adapter: &dyn PlatformAdapter,
) -> Result<ExecPassthrough, AppError> {
    reject_outer_options(cli)
        .and_then(|()| {
            agent_desktop_core::validate_state_root_env().map_err(crate::pre_dispatch_error)
        })
        .and_then(|()| sandbox::execute_exec(args.name.clone(), args.args.clone(), adapter))
        .map_err(|error| {
            let before_the_provider_ran = !matches!(error, AppError::Adapter(_));
            super::disposition::settle_unknown_delivery(error, before_the_provider_ran)
        })
}

fn write_stdout(bytes: &[u8]) -> std::io::Result<()> {
    let stdout = std::io::stdout();
    let mut writer = std::io::BufWriter::new(stdout.lock());
    writer.write_all(bytes)?;
    writer.flush()
}

fn exit_code(status: i32) -> ExitCode {
    match u8::try_from(status) {
        Ok(code) => ExitCode::from(code),
        Err(_) => ExitCode::FAILURE,
    }
}

fn to_core(action: SandboxActionCli) -> SandboxAction {
    match action {
        SandboxActionCli::Install(args) => SandboxAction::Install {
            provider: args.provider,
        },
        SandboxActionCli::Create(create) => SandboxAction::Create {
            name: create.name,
            provider: create.provider,
            set: create.set,
        },
        SandboxActionCli::Start(args) => SandboxAction::Start {
            name: args.name,
            set: args
                .window
                .then(|| "window=on".to_string())
                .into_iter()
                .chain(args.set)
                .collect(),
        },
        SandboxActionCli::Stop(args) => SandboxAction::Stop { name: args.name },
        SandboxActionCli::Status(args) => SandboxAction::Status { name: args.name },
        SandboxActionCli::List => SandboxAction::List,
        SandboxActionCli::Rm(args) => SandboxAction::Rm { name: args.name },
        SandboxActionCli::Exec(exec) => SandboxAction::Exec {
            name: exec.name,
            args: exec.args,
        },
    }
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
