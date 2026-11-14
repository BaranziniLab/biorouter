//! `serve`'s half of an update installed from the browser page (SD-19).
//!
//! The daemon installs the update and exits with
//! [`biorouter::serve_update::RESTART_EXIT_CODE`], leaving a restart plan. This
//! module reads the plan and **re-executes `serve` itself** with its own
//! command line, so the process keeps its pid (a terminal, `nohup` or a systemd
//! unit sees nothing end), binds the same address, and carries the launch token
//! across in a resume file, so the browser's cookie still opens the page. The
//! new image then starts the new daemon, and either confirms the update once it
//! answers or rolls it back and re-executes the version it replaced.
//!
//! Re-executing rather than only restarting the daemon matters: `serve` decides
//! how its daemon is started (its environment, its flags), and an update that
//! changed that must take effect now, not at some later restart by hand.
//!
//! Unix only. Windows has no `exec`, and a Windows daemon is never started with
//! the restart contract, so it never asks.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitStatus;

use anyhow::{bail, Result};
use biorouter::serve_update::{
    apply,
    records::{self, Outcome, RestartPlan, Resume, RESUME_ENV},
    RESTART_EXIT_CODE,
};

/// How to start this same `serve` again: the executable it was started from
/// and its arguments, captured before an update can move either.
#[derive(Debug, Clone)]
pub(crate) struct Restartable {
    exe: Option<PathBuf>,
    args: Vec<OsString>,
}

impl Restartable {
    /// Capture this process's executable and arguments. Must run before an
    /// update replaces the file: on Linux `current_exe` names a replaced file
    /// `… (deleted)`.
    pub(crate) fn capture() -> Self {
        Self {
            exe: std::env::current_exe().ok(),
            args: std::env::args_os().skip(1).collect(),
        }
    }
}

/// The resume file this image was started with, read and removed. Its
/// variable is cleared from this process too, so no child inherits it.
pub(crate) fn take_resume() -> Option<Resume> {
    remove_stale_resume_records();
    let path = std::env::var_os(RESUME_ENV)?;
    // SAFETY (edition 2021: a safe call): this runs at the top of `serve`,
    // before any thread this process starts reads the environment.
    std::env::remove_var(RESUME_ENV);
    let path = PathBuf::from(path);
    // Only ever a file of our own folder: a path anything else named is not ours to read
    // or delete.
    if path.parent() != Some(records::records_dir().as_path()) {
        return None;
    }
    records::take(&path)
}

/// A resume record holds a launch token. One left behind (a `serve` killed
/// between writing it and re-executing) is removed by the next start, once it
/// is old enough that no re-execution can still be reading it.
fn remove_stale_resume_records() {
    let Ok(entries) = std::fs::read_dir(records::records_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(600));
        if name.starts_with("resume-") && old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Did the daemon exit asking to be restarted on an installed update?
pub(crate) fn is_restart(status: &ExitStatus) -> bool {
    status.code() == Some(RESTART_EXIT_CODE)
}

/// What a re-executed `serve` must carry: the token this launch minted (an
/// operator's own token is found again where it came from) and the approval
/// digests, which are never asked for twice.
#[derive(Debug, Clone, Default)]
pub(crate) struct Carried {
    pub(crate) minted_token: Option<String>,
    pub(crate) computer_use_digest: Option<String>,
    pub(crate) folder_access_digest: Option<String>,
}

impl Carried {
    /// What a resume record carried, for a start that fails before it has
    /// decided its own token (the record holds a token only when it was minted).
    pub(crate) fn from_resume(resume: Option<&Resume>) -> Self {
        Self {
            minted_token: resume.and_then(|r| r.token.clone()),
            computer_use_digest: resume.and_then(|r| r.computer_use_digest.clone()),
            folder_access_digest: resume.and_then(|r| r.folder_access_digest.clone()),
        }
    }
}

/// The daemon exited with the restart code. Re-execute `serve` on the version
/// now installed; returns only when that could not happen.
pub(crate) async fn restart_on_update(
    port: u16,
    carried: &Carried,
    restartable: &Restartable,
) -> Result<()> {
    let Some(plan) = records::take_restart_plan(port, std::process::id()) else {
        bail!("biorouterd exited asking to be restarted, but left no update to restart on");
    };
    println!(
        "\n  Biorouter {} is installed. Restarting on it; open pages reconnect by themselves.",
        plan.to
    );
    let Some(exe) = restartable.exe.as_ref() else {
        return give_up_or_roll_back(
            &plan,
            carried,
            restartable,
            "this process cannot find its own executable".into(),
        )
        .await;
    };
    if plan.rollback.is_none() {
        // An administrator installed this version, and there is nothing here to
        // go back to: start whatever is on disk rather than stop with nothing
        // running because `biorouter` and `biorouterd` disagree on a version.
        let error = reexec(carried, Some(plan.clone()), restartable);
        return give_up_or_roll_back(
            &plan,
            carried,
            restartable,
            format!("could not restart: {error:#}"),
        )
        .await;
    }
    // The new `biorouter` must run before this process becomes it: a binary that
    // cannot run here would leave nothing to roll back from.
    match apply::binary_version(exe).await {
        Ok(version)
            if biorouter::serve_update::normalize_version(&version)
                == biorouter::serve_update::normalize_version(&plan.to) => {}
        Ok(version) => {
            let reason = format!(
                "{} reports {version} rather than {}",
                exe.display(),
                plan.to
            );
            return give_up_or_roll_back(&plan, carried, restartable, reason).await;
        }
        Err(error) => {
            let reason = format!("{} does not run: {error:#}", exe.display());
            return give_up_or_roll_back(&plan, carried, restartable, reason).await;
        }
    }
    let error = reexec(carried, Some(plan.clone()), restartable);
    give_up_or_roll_back(
        &plan,
        carried,
        restartable,
        format!("could not restart: {error:#}"),
    )
    .await
}

/// The daemon of a restart that finishes `plan` answered: keep the update.
pub(crate) async fn confirm(plan: &RestartPlan) {
    if let Some(rollback) = &plan.rollback {
        apply::finish(rollback).await;
    }
    records::record_result(plan, Outcome::Updated, None);
    println!("  Updated from {} to {}.", plan.from, plan.to);
}

/// The daemon of a restart that finishes `plan` did not answer: put the
/// previous version back and re-execute it. Returns only when neither could
/// happen.
pub(crate) async fn undo(
    plan: &RestartPlan,
    carried: &Carried,
    restartable: &Restartable,
    reason: String,
) -> Result<()> {
    give_up_or_roll_back(plan, carried, restartable, reason).await
}

async fn give_up_or_roll_back(
    plan: &RestartPlan,
    carried: &Carried,
    restartable: &Restartable,
    reason: String,
) -> Result<()> {
    eprintln!("  Biorouter {} did not start: {reason}", plan.to);
    let Some(rollback) = &plan.rollback else {
        records::record_result(plan, Outcome::Failed, Some(reason.clone()));
        bail!(
            "Biorouter {} was installed by an administrator and did not start ({reason}); \
             reinstall {} to go back",
            plan.to,
            plan.from
        );
    };
    if let Err(error) = apply::rollback(rollback).await {
        records::record_result(
            plan,
            Outcome::Failed,
            Some(format!(
                "{reason}; putting {} back failed: {error:#}",
                plan.from
            )),
        );
        bail!(
            "Biorouter {} did not start ({reason}), and putting {} back failed: {error:#}",
            plan.to,
            plan.from
        );
    }
    records::record_result(plan, Outcome::RolledBack, Some(reason));
    eprintln!("  Put {} back; restarting on it.", plan.from);
    let error = reexec(carried, None, restartable);
    bail!(
        "{} is back in place, but restarting on it failed: {error:#}",
        plan.from
    )
}

/// Replace this process with `serve` again. Answers only on failure.
#[cfg(unix)]
fn reexec(
    carried: &Carried,
    post_update: Option<RestartPlan>,
    restartable: &Restartable,
) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    let Some(exe) = restartable.exe.as_ref() else {
        return anyhow::anyhow!("this process cannot find its own executable");
    };
    let resume = Resume {
        token: carried.minted_token.clone(),
        computer_use_digest: carried.computer_use_digest.clone(),
        folder_access_digest: carried.folder_access_digest.clone(),
        post_update,
    };
    let path = records::resume_path(std::process::id());
    if let Err(error) = records::write_private(&path, &resume) {
        return error.context("could not write the resume record");
    }
    let error = std::process::Command::new(exe)
        .args(&restartable.args)
        .env(RESUME_ENV, &path)
        .exec();
    let _ = std::fs::remove_file(&path);
    anyhow::Error::new(error).context(format!("could not start {}", exe.display()))
}

#[cfg(not(unix))]
fn reexec(
    _carried: &Carried,
    _post_update: Option<RestartPlan>,
    _restartable: &Restartable,
) -> anyhow::Error {
    anyhow::anyhow!("restarting in place needs a Unix system")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn only_the_restart_code_is_a_restart() {
        use std::os::unix::process::ExitStatusExt;
        assert!(is_restart(&ExitStatus::from_raw(RESTART_EXIT_CODE << 8)));
        assert!(!is_restart(&ExitStatus::from_raw(0)));
        assert!(!is_restart(&ExitStatus::from_raw(1 << 8)));
        // Killed by a signal: no code at all.
        assert!(!is_restart(&ExitStatus::from_raw(9)));
    }

    #[test]
    fn a_resume_path_outside_the_records_folder_is_ignored() {
        let _env = env_lock::lock_env([(RESUME_ENV, Some("/tmp/somebody-elses.json"))]);
        assert!(take_resume().is_none());
        assert!(std::env::var_os(RESUME_ENV).is_none(), "cleared either way");
    }

    #[test]
    fn a_resume_record_is_read_once() {
        let path = records::resume_path(4_000_000_001);
        records::write_private(
            &path,
            &Resume {
                token: Some("t".into()),
                ..Resume::default()
            },
        )
        .unwrap();
        let _env = env_lock::lock_env([(RESUME_ENV, Some(path.display().to_string()))]);
        assert_eq!(take_resume().and_then(|r| r.token).as_deref(), Some("t"));
        assert!(!path.exists());
    }
}
