use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::sandbox_action::SandboxAction;
use crate::sandbox::exec_passthrough::ExecPassthrough;
use crate::sandbox::provider_dirs::{SANDBOXES_DIR, resolve};
use crate::sandbox::provider_protocol::{exec_outcome, parse_reply, provider_refused};
use crate::sandbox::provider_run::{ProviderCall, success_value};
use crate::sandbox::provider_settings::parse_settings;
use crate::sandbox::provider_spec::ProviderSpec;
use crate::sandbox::provider_table::find_provider;
use crate::sandbox::sandbox_name::validate_sandbox_name;
use crate::sandbox::sandbox_record::SandboxRecord;
use crate::sandbox::sandbox_state::SandboxState;
use crate::sandbox::sandbox_store;
use crate::storage::state_root::resolve_configured_state_root;
use crate::{AdapterError, AppError, DeliverySemantics, ErrorCode, PlatformAdapter};

const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
const START_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);
const REMOVE_TIMEOUT: Duration = Duration::from_secs(30);
const EXEC_TIMEOUT: Duration = Duration::from_secs(300);

type Target = (&'static ProviderSpec, PathBuf, PathBuf);

pub fn execute(action: SandboxAction, adapter: &dyn PlatformAdapter) -> Result<Value, AppError> {
    let root = resolve_configured_state_root()?;
    match action {
        SandboxAction::Install { provider } => super::sandbox_install::execute(provider, adapter),
        SandboxAction::Create {
            name,
            provider,
            set,
        } => create(&root, &name, &provider, &set, adapter),
        SandboxAction::Start { name, set } => {
            let settings = parse_settings(&set)?;
            verb_value(&root, &name, ("start", START_TIMEOUT), &settings, adapter)
        }
        SandboxAction::Stop { name } => {
            verb_value(&root, &name, ("stop", STOP_TIMEOUT), &[], adapter)
        }
        SandboxAction::Status { name } => {
            verb_value(&root, &name, ("status", STATUS_TIMEOUT), &[], adapter)
        }
        SandboxAction::List => super::sandbox_list::execute(&root, adapter),
        SandboxAction::Rm { name } => remove(&root, &name, adapter),
        SandboxAction::Exec { name, .. } => Err(exec_without_envelope(&name)),
    }
}

pub fn execute_exec(
    name: String,
    args: Vec<String>,
    adapter: &dyn PlatformAdapter,
) -> Result<ExecPassthrough, AppError> {
    let root = resolve_configured_state_root()?;
    let (spec, provider_dir, sandboxes_root) = locate(&root, &name)?;
    let call = ProviderCall::new(spec, &provider_dir, &sandboxes_root, Some(&name), &[]);
    let mut guest_args = vec!["exec".to_string(), name.clone(), "--".to_string()];
    guest_args.extend(args);
    let output = call.run(adapter, guest_args, EXEC_TIMEOUT)?;
    exec_outcome(spec.name, &output)
}

fn create(
    root: &Path,
    name: &str,
    provider: &str,
    set: &[String],
    adapter: &dyn PlatformAdapter,
) -> Result<Value, AppError> {
    validate_sandbox_name(name)?;
    let spec = find_provider(provider)?;
    let settings = parse_settings(set)?;
    let (provider_dir, sandboxes_root) = resolve(root, spec)?;
    let _busy = sandbox_store::hold(&sandboxes_root, name)?;
    sandbox_store::create(
        &sandboxes_root,
        &SandboxRecord {
            name: name.to_string(),
            provider: provider.to_string(),
            created_at: unix_seconds(),
        },
    )?;
    let call = ProviderCall::new(spec, &provider_dir, &sandboxes_root, Some(name), &settings);
    let output = call
        .run(adapter, verb_args(name, "create"), CREATE_TIMEOUT)
        .inspect_err(|error| {
            if never_ran(error) {
                discard_untouched_record(&sandboxes_root, name);
            }
        })?;
    let refused = provider_refused(&output);
    parse_reply(spec.name, Some(name), true, &output)
        .and_then(|reply| {
            if reply.state == SandboxState::Unavailable {
                discard_untouched_record(&sandboxes_root, name);
                Err(unavailable_refusal(reply.message))
            } else {
                Ok(success_value(name, Some(spec.name), reply))
            }
        })
        .map_err(|error| {
            if refused && discard_untouched_record(&sandboxes_root, name) {
                not_delivered(error)
            } else {
                error
            }
        })
}

fn not_delivered(error: AppError) -> AppError {
    match error {
        AppError::Adapter(source) => {
            AppError::Adapter(source.with_disposition(DeliverySemantics::not_delivered()))
        }
        other => other,
    }
}

fn never_ran(error: &AppError) -> bool {
    !matches!(error, AppError::Adapter(source) if source.disposition != DeliverySemantics::not_delivered())
}

/// Drops the record of a create the provider refused and reports whether it did. Every other failed create
/// keeps its record, because the provider may have made something that only `rm` can clear.
fn discard_untouched_record(sandboxes_root: &Path, name: &str) -> bool {
    let dir = sandboxes_root.join(name);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    let untouched = entries
        .filter_map(Result::ok)
        .all(|entry| entry.file_name() == sandbox_store::RECORD_FILE);
    untouched && sandbox_store::remove(sandboxes_root, name).is_ok()
}

fn verb_value(
    root: &Path,
    name: &str,
    (verb, timeout): (&str, Duration),
    settings: &[(String, String)],
    adapter: &dyn PlatformAdapter,
) -> Result<Value, AppError> {
    let (spec, provider_dir, sandboxes_root) = locate(root, name)?;
    let _busy = match is_mutating(verb) {
        true => Some(sandbox_store::hold(&sandboxes_root, name)?),
        false => None,
    };
    let call = ProviderCall::new(spec, &provider_dir, &sandboxes_root, Some(name), settings);
    let output = call.run(adapter, verb_args(name, verb), timeout)?;
    let reply = parse_reply(spec.name, Some(name), is_mutating(verb), &output)?;
    Ok(success_value(name, Some(spec.name), reply))
}

fn remove(root: &Path, name: &str, adapter: &dyn PlatformAdapter) -> Result<Value, AppError> {
    let (spec, provider_dir, sandboxes_root) = locate(root, name)?;
    let _busy = sandbox_store::hold(&sandboxes_root, name)?;
    let call = ProviderCall::new(spec, &provider_dir, &sandboxes_root, Some(name), &[]);
    let running = call
        .run(adapter, verb_args(name, "status"), STATUS_TIMEOUT)
        .and_then(|output| parse_reply(spec.name, Some(name), false, &output))
        .is_ok_and(|reply| reply.state == SandboxState::Running);
    if running {
        return Err(running_refusal(name));
    }
    let output = call.run(adapter, verb_args(name, "rm"), REMOVE_TIMEOUT)?;
    let reply = parse_reply(spec.name, Some(name), true, &output)?;
    sandbox_store::remove(&sandboxes_root, name)?;
    Ok(success_value(name, Some(spec.name), reply))
}

fn locate(root: &Path, name: &str) -> Result<Target, AppError> {
    let record = sandbox_store::load(&root.join(SANDBOXES_DIR), name)?;
    let spec = recorded_provider(root, &record)?;
    let (provider_dir, sandboxes_root) = resolve(root, spec)?;
    Ok((spec, provider_dir, sandboxes_root))
}

pub(super) fn recorded_provider(
    root: &Path,
    record: &SandboxRecord,
) -> Result<&'static ProviderSpec, AppError> {
    find_provider(&record.provider).map_err(|_| {
        sandbox_store::broken(
            &root.join(SANDBOXES_DIR),
            &record.name,
            &format!(
                "it records provider {}, which this build does not include",
                record.provider
            ),
        )
    })
}

pub(super) fn verb_args(name: &str, verb: &str) -> Vec<String> {
    vec![verb.to_string(), name.to_string()]
}

fn is_mutating(verb: &str) -> bool {
    matches!(verb, "create" | "start" | "stop" | "rm")
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn running_refusal(name: &str) -> AppError {
    AppError::Adapter(
        AdapterError::new(
            ErrorCode::InvalidArgs,
            format!("Sandbox {name} is running, so it was not removed."),
        )
        .with_suggestion(format!("Run `agent-desktop sandbox stop {name}` first."))
        .with_disposition(DeliverySemantics::not_delivered()),
    )
}

fn unavailable_refusal(message: String) -> AppError {
    AppError::Adapter(
        AdapterError::new(ErrorCode::PlatformNotSupported, message)
            .with_suggestion("Run `agent-desktop sandbox install` to see what each provider needs.")
            .with_disposition(DeliverySemantics::not_delivered()),
    )
}

fn exec_without_envelope(name: &str) -> AppError {
    AppError::Adapter(
        AdapterError::new(
            ErrorCode::ActionNotSupported,
            "`sandbox exec` prints the sandboxed program's own output, so it has no data envelope of its own.",
        )
        .with_suggestion(format!(
            "Run `agent-desktop sandbox exec {name} -- snapshot --app Finder` to run a command in the sandbox."
        ))
        .with_disposition(DeliverySemantics::not_delivered()),
    )
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sandbox_guard_tests.rs"]
mod guard_tests;
