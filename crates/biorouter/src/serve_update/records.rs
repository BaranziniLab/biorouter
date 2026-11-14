//! The three small files the daemon and `serve` leave each other across a
//! restart. Nothing else crosses: the daemon has exited by the time `serve`
//! reads the first, and `serve` has re-executed itself by the time the daemon
//! reads the last.
//!
//! | File | Written by | Read by | Says |
//! |---|---|---|---|
//! | `restart-<port>.json` | the daemon, just before it exits with the restart code | `serve` | which version to expect and how to undo the install |
//! | `resume-<pid>.json` | `serve`, just before it re-executes itself | the same `serve`, one image later | the launch token and approval digests to carry over, and the plan |
//! | `last-<port>.json` | `serve`, once the new version answered or was rolled back | the daemon's status | how the last update ended, for the page to say once |
//!
//! All three are this account's alone (`0600`), and the resume file, which
//! holds the launch token, is deleted the moment it is read. They live in the
//! state folder, so `BIOROUTER_PATH_ROOT` moves them with everything else.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::install::{PackageFormat, Privilege};

/// The environment variable naming the resume file to the re-executed `serve`.
pub const RESUME_ENV: &str = "BIOROUTER_SERVE_RESUME";

/// The folder all three live in.
pub fn records_dir() -> PathBuf {
    crate::config::paths::Paths::in_state_dir("serve-update")
}

/// What the daemon leaves `serve` when it exits to be restarted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartPlan {
    /// The port of the daemon that wrote it. Two `serve`s of one installation
    /// on one account keep apart by it.
    pub port: u16,
    /// The `serve` that started that daemon. A plan any other process finds is
    /// not its to act on.
    pub serve_pid: u32,
    /// The version that was running.
    pub from: String,
    /// The version that should answer after the restart.
    pub to: String,
    /// How to put `from` back if `to` does not come up. `None` when the update
    /// was installed by an administrator, which this process cannot undo.
    pub rollback: Option<Rollback>,
}

/// How an install is undone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rollback {
    /// Reinstall the previous release's package, with the privilege the
    /// update used.
    Package {
        format: PackageFormat,
        file: PathBuf,
        sha256: String,
        privilege: Privilege,
    },
    /// Put the kept bundle back.
    AppBundle { bundle: PathBuf, backup: PathBuf },
}

/// What `serve` carries across its own re-execution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
    /// The launch token, when this launch minted it. A token the operator chose
    /// (`--token`, `BIOROUTER_BROWSER_TOKEN`) is found again where it was, and is
    /// not copied here.
    pub token: Option<String>,
    pub computer_use_digest: Option<String>,
    pub folder_access_digest: Option<String>,
    /// The update this restart finishes, which the new image confirms or undoes.
    pub post_update: Option<RestartPlan>,
}

/// How the last update on a port ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastResult {
    pub outcome: Outcome,
    pub from: String,
    pub to: String,
    /// What went wrong, when something did.
    pub reason: Option<String>,
    /// Seconds since the epoch.
    pub at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The new version answered.
    Updated,
    /// The new version did not answer and the previous one was put back.
    RolledBack,
    /// The new version did not answer and nothing could be put back.
    Failed,
}

pub fn restart_plan_path(port: u16) -> PathBuf {
    records_dir().join(format!("restart-{port}.json"))
}

pub fn last_result_path(port: u16) -> PathBuf {
    records_dir().join(format!("last-{port}.json"))
}

pub fn resume_path(pid: u32) -> PathBuf {
    records_dir().join(format!("resume-{pid}.json"))
}

/// Write `value` as JSON, readable by this account alone, replacing any older
/// copy in one rename.
pub fn write_private<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("could not create {}", dir.display()))?;
    }
    let body = serde_json::to_vec_pretty(value)?;
    let partial = path.with_extension("json.part");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options
            .open(&partial)
            .with_context(|| format!("could not write {}", partial.display()))?;
        file.write_all(&body)?;
        file.sync_all()?;
    }
    std::fs::rename(&partial, path)?;
    Ok(())
}

/// Read and delete `path`. Deleted even when it does not parse, so a damaged
/// record is not acted on twice.
pub fn take<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let body = std::fs::read(path).ok()?;
    let _ = std::fs::remove_file(path);
    serde_json::from_slice(&body).ok()
}

/// Read `path` and leave it.
pub fn peek<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The restart plan the daemon left for the `serve` with pid `serve_pid` on
/// `port`, removed once read. One written for another process is left alone.
pub fn take_restart_plan(port: u16, serve_pid: u32) -> Option<RestartPlan> {
    let path = restart_plan_path(port);
    let plan: RestartPlan = peek(&path)?;
    if plan.port != port || plan.serve_pid != serve_pid {
        return None;
    }
    let _ = std::fs::remove_file(&path);
    Some(plan)
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Record how the update in `plan` ended.
pub fn record_result(plan: &RestartPlan, outcome: Outcome, reason: Option<String>) {
    let result = LastResult {
        outcome,
        from: plan.from.clone(),
        to: plan.to.clone(),
        reason,
        at: now(),
    };
    if let Err(error) = write_private(&last_result_path(plan.port), &result) {
        tracing::warn!(%error, "could not record how the update ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(port: u16, serve_pid: u32) -> RestartPlan {
        RestartPlan {
            port,
            serve_pid,
            from: "1.94.1".into(),
            to: "1.95.0".into(),
            rollback: Some(Rollback::AppBundle {
                bundle: "/a/Biorouter.app".into(),
                backup: "/a/.Biorouter.app.previous".into(),
            }),
        }
    }

    #[test]
    fn a_record_is_private_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("restart-1.json");
        write_private(&path, &plan(1, 2)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(peek::<RestartPlan>(&path), Some(plan(1, 2)));
        assert_eq!(take::<RestartPlan>(&path), Some(plan(1, 2)));
        assert!(!path.exists(), "taken means gone");
    }

    #[test]
    fn a_damaged_record_is_removed_rather_than_read_twice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resume-9.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(take::<Resume>(&path), None);
        assert!(!path.exists());
    }

    #[test]
    fn the_wire_names_are_stable() {
        let value = serde_json::to_value(plan(8765, 42)).unwrap();
        assert_eq!(value["rollback"]["kind"], "app_bundle");
        let package = Rollback::Package {
            format: PackageFormat::Deb,
            file: "/x.deb".into(),
            sha256: "ab".into(),
            privilege: Privilege::PasswordlessSudo,
        };
        let value = serde_json::to_value(package).unwrap();
        assert_eq!(value["kind"], "package");
        assert_eq!(value["format"], "deb");
        assert_eq!(value["privilege"], "passwordless_sudo");
    }
}
