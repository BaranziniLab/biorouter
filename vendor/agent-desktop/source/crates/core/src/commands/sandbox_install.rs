use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::sandbox::provider_dirs::resolve;
use crate::sandbox::provider_protocol::parse_reply;
use crate::sandbox::provider_reply::ProviderReply;
use crate::sandbox::provider_run::{ProviderCall, success_value};
use crate::sandbox::provider_spec::ProviderSpec;
use crate::sandbox::provider_table::{PROVIDERS, find_provider};
use crate::sandbox::sandbox_state::SandboxState;
use crate::storage::state_root::resolve_configured_state_root;
use crate::{AppError, PlatformAdapter};

const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

pub fn execute(provider: Option<String>, adapter: &dyn PlatformAdapter) -> Result<Value, AppError> {
    let root = resolve_configured_state_root()?;
    match provider {
        Some(name) => install_one(&root, &name, adapter),
        None => report_providers(&root, adapter),
    }
}

fn install_one(root: &Path, name: &str, adapter: &dyn PlatformAdapter) -> Result<Value, AppError> {
    let spec = find_provider(name)?;
    let (provider_dir, sandboxes_root) = resolve(root, spec)?;
    let call = ProviderCall::new(spec, &provider_dir, &sandboxes_root, None, &[]);
    let output = call.run(adapter, vec!["install".to_string()], INSTALL_TIMEOUT)?;
    let reply = parse_reply(spec.name, None, true, &output)?;
    Ok(success_value(spec.name, None, reply))
}

fn report_providers(root: &Path, adapter: &dyn PlatformAdapter) -> Result<Value, AppError> {
    let mut providers = Vec::with_capacity(PROVIDERS.len());
    for spec in PROVIDERS {
        providers.push(provider_entry(root, spec, adapter));
    }
    Ok(json!({ "providers": providers }))
}

fn provider_entry(root: &Path, spec: &ProviderSpec, adapter: &dyn PlatformAdapter) -> Value {
    match check(root, spec, adapter) {
        Ok(reply) => success_value(spec.name, None, reply),
        Err(error) => unavailable_entry(spec, &error),
    }
}

fn check(
    root: &Path,
    spec: &ProviderSpec,
    adapter: &dyn PlatformAdapter,
) -> Result<ProviderReply, AppError> {
    let (provider_dir, sandboxes_root) = resolve(root, spec)?;
    let call = ProviderCall::new(spec, &provider_dir, &sandboxes_root, None, &[]);
    let output = call.run(adapter, vec!["check".to_string()], CHECK_TIMEOUT)?;
    parse_reply(spec.name, None, false, &output)
}

fn unavailable_entry(spec: &ProviderSpec, error: &AppError) -> Value {
    let mut value = json!({
        "name": spec.name,
        "state": SandboxState::Unavailable,
        "message": error.to_string(),
    });
    if let (Some(next), Some(object)) = (error.suggestion(), value.as_object_mut()) {
        object.insert("next".to_string(), Value::String(next.to_string()));
    }
    value
}

#[cfg(test)]
#[path = "sandbox_install_tests.rs"]
mod tests;
