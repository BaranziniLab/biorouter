use crate::sandbox::sandbox_entry::SandboxEntry;
use crate::sandbox::sandbox_name::validate_sandbox_name;
use crate::sandbox::sandbox_record::SandboxRecord;
use crate::storage::private_file::{read_private_bounded, write_atomic};
use crate::storage::private_file_parent::ensure_private;
use crate::{
    AdapterError, AppError, Deadline, DeliverySemantics, ErrorCode, storage::file_lock::FileLock,
};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub(crate) const RECORD_FILE: &str = "sandbox.json";
const CREATE_LOCK_FILE: &str = ".create.lock";
const MAX_RECORD_BYTES: u64 = 4096;
const BUSY_WAIT_MS: u64 = 200;

pub(crate) fn create(root: &Path, record: &SandboxRecord) -> Result<PathBuf, AppError> {
    validate_sandbox_name(&record.name)?;
    ensure_private(root)?;
    let _lock = FileLock::acquire(
        &root.join(CREATE_LOCK_FILE),
        Deadline::standard()?,
        "sandbox record create",
    )?;
    let dir = root.join(&record.name);
    if entry_exists(&dir)? && !is_crashed_publish(&dir) {
        return Err(already_exists(&load(root, &record.name)?));
    }
    ensure_private(&dir)?;
    write_atomic(&dir.join(RECORD_FILE), &serde_json::to_vec(record)?)?;
    Ok(dir)
}

/// Takes the sandbox's lock, so only one command changes a sandbox at a time. A held lock fails at once.
pub(crate) fn hold(root: &Path, name: &str) -> Result<FileLock, AppError> {
    validate_sandbox_name(name)?;
    ensure_private(root)?;
    FileLock::acquire(
        &root.join(format!(".{name}.lock")),
        Deadline::after(BUSY_WAIT_MS)?,
        "sandbox change",
    )
    .map_err(|_| busy(name))
}

/// Whether `dir` holds nothing but the temporaries of a record write that never finished.
fn is_crashed_publish(dir: &Path) -> bool {
    let is_directory = std::fs::symlink_metadata(dir).is_ok_and(|metadata| metadata.is_dir());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    is_directory
        && entries.filter_map(Result::ok).all(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(&format!(".{RECORD_FILE}.")) && name.ends_with(".tmp")
        })
}

fn entry_exists(path: &Path) -> Result<bool, AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn load(root: &Path, name: &str) -> Result<SandboxRecord, AppError> {
    validate_sandbox_name(name)?;
    let dir = root.join(name);
    match std::fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(broken(root, name, "its entry is not a regular directory")),
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(not_found(name)),
        Err(error) => return Err(broken(root, name, &error.to_string())),
    }
    match read_private_bounded(&dir.join(RECORD_FILE), MAX_RECORD_BYTES) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .ok()
            .filter(|record: &SandboxRecord| record.name == name)
            .ok_or_else(|| broken(root, name, "its record is malformed")),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            Err(broken(root, name, "its record file is missing"))
        }
        Err(error) if error.kind() == ErrorKind::InvalidData => {
            Err(broken(root, name, "its record is malformed"))
        }
        Err(error) => Err(broken(
            root,
            name,
            &format!("its record was not read: {error}"),
        )),
    }
}

pub(crate) fn list(root: &Path) -> Result<Vec<SandboxEntry>, AppError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut sandboxes = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if validate_sandbox_name(&name).is_err() {
            continue;
        }
        let record = load(root, &name);
        if record
            .as_ref()
            .is_err_and(|error| error.code() == ErrorCode::SandboxNotFound.as_str())
        {
            continue;
        }
        sandboxes.push(SandboxEntry { name, record });
    }
    sandboxes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sandboxes)
}

pub(crate) fn remove(root: &Path, name: &str) -> Result<(), AppError> {
    validate_sandbox_name(name)?;
    match std::fs::remove_dir_all(root.join(name)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn already_exists(record: &SandboxRecord) -> AppError {
    AppError::Adapter(
        AdapterError::new(
            ErrorCode::InvalidArgs,
            format!(
                "A sandbox named {} already exists (provider {}).",
                record.name, record.provider
            ),
        )
        .with_suggestion(format!(
            "Pick another name, or remove it first: run `agent-desktop sandbox list` and follow the entry for {}.",
            record.name
        ))
        .with_disposition(DeliverySemantics::not_delivered()),
    )
}

/// The one error for a sandbox agent-desktop cannot use. It has no provider to ask, so the way out is manual.
pub(crate) fn broken(root: &Path, name: &str, reason: &str) -> AppError {
    AppError::Adapter(
        AdapterError::new(
            ErrorCode::InvalidArgs,
            format!("Sandbox {name} cannot be used: {reason}."),
        )
        .with_suggestion(format!(
            "Stop anything the sandbox still runs, then delete {}.",
            root.join(name).display()
        ))
        .with_disposition(DeliverySemantics::not_delivered()),
    )
}

fn busy(name: &str) -> AppError {
    AppError::Adapter(
        AdapterError::new(
            ErrorCode::ActionFailed,
            format!("Another agent-desktop command is changing sandbox {name}."),
        )
        .with_suggestion(format!(
            "Run `agent-desktop sandbox status {name}` and retry when that command has finished."
        ))
        .with_disposition(DeliverySemantics::not_delivered()),
    )
}

pub(crate) fn not_found(name: &str) -> AppError {
    AppError::Adapter(
        AdapterError::new(
            ErrorCode::SandboxNotFound,
            format!("No sandbox named {name} exists."),
        )
        .with_suggestion("Run `agent-desktop sandbox list` to see the sandboxes that exist.")
        .with_disposition(DeliverySemantics::not_delivered()),
    )
}

#[cfg(test)]
#[path = "sandbox_store_tests.rs"]
mod tests;
