use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::sandbox::{recorded_provider, verb_args};
use crate::sandbox::provider_dirs::{SANDBOXES_DIR, resolve};
use crate::sandbox::provider_protocol::parse_reply;
use crate::sandbox::provider_reply::ProviderReply;
use crate::sandbox::provider_run::{ProviderCall, success_value};
use crate::sandbox::sandbox_record::SandboxRecord;
use crate::sandbox::sandbox_state::SandboxState;
use crate::sandbox::sandbox_store;
use crate::{AppError, PlatformAdapter};

const STATUS_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) fn execute(root: &Path, adapter: &dyn PlatformAdapter) -> Result<Value, AppError> {
    let entries = sandbox_store::list(&root.join(SANDBOXES_DIR))?;
    let mut sandboxes = Vec::with_capacity(entries.len());
    for entry in &entries {
        match &entry.record {
            Ok(record) => sandboxes.push(record_entry(root, record, adapter)),
            Err(error) => sandboxes.push(broken_record_entry(&entry.name, error)),
        }
    }
    Ok(json!({ "sandboxes": sandboxes }))
}

fn record_entry(root: &Path, record: &SandboxRecord, adapter: &dyn PlatformAdapter) -> Value {
    match status_reply(root, record, adapter) {
        Ok(reply) => success_value(&record.name, Some(&record.provider), reply),
        Err(error) => broken_entry(record, &error),
    }
}

fn status_reply(
    root: &Path,
    record: &SandboxRecord,
    adapter: &dyn PlatformAdapter,
) -> Result<ProviderReply, AppError> {
    let spec = recorded_provider(root, record)?;
    let (provider_dir, sandboxes_root) = resolve(root, spec)?;
    let call = ProviderCall::new(
        spec,
        &provider_dir,
        &sandboxes_root,
        Some(&record.name),
        &[],
    );
    let output = call.run(adapter, verb_args(&record.name, "status"), STATUS_TIMEOUT)?;
    parse_reply(spec.name, Some(&record.name), false, &output)
}

fn broken_entry(record: &SandboxRecord, error: &AppError) -> Value {
    broken_value(&record.name, Some(&record.provider), error)
}

fn broken_record_entry(name: &str, error: &AppError) -> Value {
    broken_value(name, None, error)
}

fn broken_value(name: &str, provider: Option<&str>, error: &AppError) -> Value {
    let mut value = json!({
        "name": name,
        "state": SandboxState::Broken,
        "message": error.to_string(),
    });
    if let Some(next) = error.suggestion() {
        value["next"] = json!(next);
    }
    if let Some(provider) = provider {
        value["provider"] = json!(provider);
    }
    value
}
