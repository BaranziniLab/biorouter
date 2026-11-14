//! Installing a verified release, proving it runs, and undoing it.
//!
//! Each install ends in a [`Rollback`], the one thing `serve` needs to put the
//! previous version back if the new one does not come up after the restart:
//!
//! - **A package** is installed by its own package manager, then its two
//!   binaries are run with `--version` while the old daemon is still serving.
//!   A binary that does not run here (the wrong CPU, too old a C library) is
//!   caught at this point, the previous release's package is reinstalled at
//!   once, and the page says so without anything having restarted.
//! - **An app bundle** is unpacked beside the installed one, its code signature
//!   checked (the same team as the bundle it replaces, when that one names a
//!   team), its binaries run with `--version`, and only then swapped in with
//!   two renames. The bundle it replaced is kept beside it until the new one has
//!   answered.

use std::path::{Path, PathBuf};
use std::process::Stdio;

// Every child this module starts gets no console window on Windows (counted by
// `no_console_window_census.rs` at each site) and none of the daemon's private
// variables: a package manager runs maintainer scripts, which have no business
// with the daemon secret or the browser token.
use biorouter_sandbox::console::no_console_window;
use biorouter_sandbox::environment::strip_daemon_private_env;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use super::download::still_verified;
use super::install::{which, InstallKind, PackageFormat, Privilege};
use super::records::Rollback;

/// A downloaded, verified file and the version it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub file: PathBuf,
    pub sha256: String,
    pub version: String,
}

/// How long the package manager may take. Generous, because killing it part
/// way through leaves a half-configured package with nothing to roll back to.
const PACKAGE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// How long one `--version` may take.
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);

/// Install `update` over `kind`, answering how to undo it.
///
/// `previous` is the running version's own package, needed for a package
/// install and ignored otherwise: without it there is no way back, so a
/// package update without one is refused rather than attempted.
pub async fn install(
    kind: &InstallKind,
    privilege: Privilege,
    update: &Staged,
    previous: Option<&Staged>,
) -> Result<Rollback> {
    if !privilege.one_click() {
        bail!("this account cannot install updates here");
    }
    still_verified(&update.file, &update.sha256).await?;
    match kind {
        InstallKind::Package {
            format, bin_dir, ..
        } => {
            let previous = previous.ok_or_else(|| {
                anyhow!(
                    "no copy of the running version could be kept, so there would be no way back"
                )
            })?;
            still_verified(&previous.file, &previous.sha256).await?;
            run_package_manager(*format, privilege, &update.file, false)
                .await
                .context("the package manager did not install the update")?;
            if let Err(error) = binaries_report(bin_dir, &update.version).await {
                let undo = run_package_manager(*format, privilege, &previous.file, true).await;
                return Err(match undo {
                    Ok(()) => error.context(format!(
                        "Biorouter {} does not run on this server, so {} was put back",
                        update.version, previous.version
                    )),
                    Err(undo_error) => error.context(format!(
                        "Biorouter {} does not run on this server, and putting {} back failed too: {undo_error:#}",
                        update.version, previous.version
                    )),
                });
            }
            Ok(Rollback::Package {
                format: *format,
                file: previous.file.clone(),
                sha256: previous.sha256.clone(),
                privilege,
            })
        }
        InstallKind::AppBundle { bundle, .. } => swap_bundle(bundle, update).await,
        InstallKind::Source | InstallKind::Manual { .. } => {
            bail!("this installation is not one Biorouter can update")
        }
    }
}

/// Put back what [`install`] replaced.
pub async fn rollback(rollback: &Rollback) -> Result<()> {
    match rollback {
        Rollback::Package {
            format,
            file,
            sha256,
            privilege,
        } => {
            still_verified(file, sha256).await?;
            run_package_manager(*format, *privilege, file, true).await
        }
        Rollback::AppBundle { bundle, backup } => {
            if !backup.exists() {
                bail!("the previous version is no longer at {}", backup.display());
            }
            let failed = sibling(bundle, "failed");
            let _ = tokio::fs::remove_dir_all(&failed).await;
            if bundle.exists() {
                tokio::fs::rename(bundle, &failed).await?;
            }
            tokio::fs::rename(backup, bundle).await?;
            let _ = tokio::fs::remove_dir_all(&failed).await;
            Ok(())
        }
    }
}

/// Drop what was kept for [`rollback`] once the new version has answered.
pub async fn finish(rollback: &Rollback) {
    match rollback {
        Rollback::Package { .. } => {}
        Rollback::AppBundle { backup, .. } => {
            let _ = tokio::fs::remove_dir_all(backup).await;
        }
    }
}

/// `<parent>/.<name>.<tag>`: hidden, and on the bundle's own volume so a rename
/// is one atomic step.
fn sibling(bundle: &Path, tag: &str) -> PathBuf {
    let name = bundle
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Biorouter.app".into());
    bundle
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.{tag}"))
}

/// The kept copy of the bundle [`swap_bundle`] replaced.
pub fn bundle_backup(bundle: &Path) -> PathBuf {
    sibling(bundle, "previous")
}

async fn swap_bundle(bundle: &Path, update: &Staged) -> Result<Rollback> {
    if let Some(pid) = desktop_app_running_from(bundle).await {
        bail!(
            "the Biorouter app is open on this Mac (process {pid}); quit it first, or let it update itself"
        );
    }
    let incoming = sibling(bundle, "incoming");
    let _ = tokio::fs::remove_dir_all(&incoming).await;
    tokio::fs::create_dir_all(&incoming).await?;
    let outcome = async {
        run_checked(
            "ditto",
            &[
                "-x",
                "-k",
                &update.file.to_string_lossy(),
                &incoming.to_string_lossy(),
            ],
            Duration::from_secs(300),
        )
        .await
        .context("could not unpack the update")?;
        let new_bundle = single_app_in(&incoming).await?;
        verify_signature(bundle, &new_bundle).await?;
        binaries_report(
            &new_bundle.join("Contents").join("Resources").join("bin"),
            &update.version,
        )
        .await?;
        let backup = bundle_backup(bundle);
        let _ = tokio::fs::remove_dir_all(&backup).await;
        tokio::fs::rename(bundle, &backup)
            .await
            .context("could not move the installed version aside")?;
        if let Err(error) = tokio::fs::rename(&new_bundle, bundle).await {
            let _ = tokio::fs::rename(&backup, bundle).await;
            return Err(anyhow!(error).context("could not move the update into place"));
        }
        Ok(Rollback::AppBundle {
            bundle: bundle.to_path_buf(),
            backup,
        })
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&incoming).await;
    outcome
}

async fn single_app_in(dir: &Path) -> Result<PathBuf> {
    let mut found = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().extension().is_some_and(|e| e == "app") {
            found.push(entry.path());
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => bail!("the update holds no application"),
        _ => bail!("the update holds more than one application"),
    }
}

/// The new bundle's signature must verify, and when the installed bundle names
/// a team, the new one must name the same team: the rule Squirrel.Mac applies
/// to the desktop app's own updates.
async fn verify_signature(installed: &Path, new_bundle: &Path) -> Result<()> {
    run_checked(
        "codesign",
        &[
            "--verify",
            "--deep",
            "--strict",
            &new_bundle.to_string_lossy(),
        ],
        Duration::from_secs(120),
    )
    .await
    .context("the update's code signature does not verify")?;
    if let Some(team) = team_identifier(installed).await {
        let new_team = team_identifier(new_bundle).await;
        if new_team.as_deref() != Some(team.as_str()) {
            bail!(
                "the update is signed by {} rather than {team}, the team that signed the installed app",
                new_team.as_deref().unwrap_or("no team")
            );
        }
    }
    Ok(())
}

async fn team_identifier(bundle: &Path) -> Option<String> {
    let mut command = tokio::process::Command::new("codesign");
    command
        .args(["-dv", "--verbose=2"])
        .arg(bundle)
        .stdin(Stdio::null());
    no_console_window(&mut command);
    strip_daemon_private_env(&mut command);
    let output = command.output().await.ok()?;
    parse_team_identifier(&String::from_utf8_lossy(&output.stderr))
}

/// `TeamIdentifier=F3YYBXAFJ8` → `F3YYBXAFJ8`; `not set` (ad hoc) → `None`.
pub fn parse_team_identifier(codesign_output: &str) -> Option<String> {
    codesign_output.lines().find_map(|line| {
        let team = line.trim().strip_prefix("TeamIdentifier=")?.trim();
        (!team.is_empty() && team != "not set").then(|| team.to_string())
    })
}

/// A desktop app process running out of `bundle`, other than this one.
async fn desktop_app_running_from(bundle: &Path) -> Option<u32> {
    let mut command = tokio::process::Command::new("ps");
    command.args(["-axo", "pid=,command="]).stdin(Stdio::null());
    no_console_window(&mut command);
    strip_daemon_private_env(&mut command);
    let output = command.output().await.ok()?;
    let own = std::process::id();
    let roots = [
        bundle.join("Contents").join("MacOS"),
        bundle.join("Contents").join("Frameworks"),
    ];
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let line = line.trim_start();
            let (pid, command) = line.split_once(' ')?;
            let pid: u32 = pid.parse().ok()?;
            let command = command.trim_start();
            (pid != own
                && roots
                    .iter()
                    .any(|root| command.starts_with(&*root.to_string_lossy())))
            .then_some(pid)
        })
}

/// Both binaries in `bin_dir` run and say `version`.
pub async fn binaries_report(bin_dir: &Path, version: &str) -> Result<()> {
    for name in ["biorouterd", "biorouter"] {
        let binary = bin_dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let reported = binary_version(&binary)
            .await
            .with_context(|| format!("{} does not run", binary.display()))?;
        if super::normalize_version(&reported) != super::normalize_version(version) {
            bail!(
                "{} reports {reported} where {version} was installed",
                binary.display()
            );
        }
    }
    Ok(())
}

/// What `binary --version` says.
pub async fn binary_version(binary: &Path) -> Result<String> {
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    no_console_window(&mut command);
    strip_daemon_private_env(&mut command);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            anyhow!(
                "it did not answer --version within {}s",
                VERSION_TIMEOUT.as_secs()
            )
        })??;
    if !output.status.success() {
        bail!("{}", last_lines(&output.stderr, 3));
    }
    super::version_from_output(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| anyhow!("it printed no version"))
}

/// The command [`run_package_manager`] runs, apart so a test can read it.
pub fn package_command(
    format: PackageFormat,
    privilege: Privilege,
    file: &Path,
    downgrade: bool,
    have: impl Fn(&str) -> bool,
) -> Vec<String> {
    let file = file.to_string_lossy().to_string();
    let mut argv: Vec<String> = match format {
        PackageFormat::Deb if have("apt-get") => {
            let mut argv = vec![
                "env".into(),
                "DEBIAN_FRONTEND=noninteractive".into(),
                "apt-get".into(),
                "install".into(),
                "-y".into(),
                // unattended-upgrades holds this lock for minutes at a time.
                "-o".into(),
                "DPkg::Lock::Timeout=120".into(),
            ];
            if downgrade {
                argv.push("--allow-downgrades".into());
            }
            argv.push(file);
            argv
        }
        PackageFormat::Deb => vec![
            "env".into(),
            "DEBIAN_FRONTEND=noninteractive".into(),
            "dpkg".into(),
            "-i".into(),
            file,
        ],
        // rpm itself for a downgrade: dnf and yum refuse to install an older
        // package with `install`.
        PackageFormat::Rpm if downgrade => vec![
            "rpm".into(),
            "-U".into(),
            "--oldpackage".into(),
            "--replacepkgs".into(),
            file,
        ],
        PackageFormat::Rpm if have("dnf") => {
            vec!["dnf".into(), "install".into(), "-y".into(), file]
        }
        PackageFormat::Rpm if have("yum") => {
            vec!["yum".into(), "install".into(), "-y".into(), file]
        }
        PackageFormat::Rpm => vec!["rpm".into(), "-U".into(), file],
    };
    if privilege == Privilege::PasswordlessSudo {
        argv.splice(0..0, ["sudo".to_string(), "-n".to_string()]);
    }
    argv
}

async fn run_package_manager(
    format: PackageFormat,
    privilege: Privilege,
    file: &Path,
    downgrade: bool,
) -> Result<()> {
    let argv = package_command(format, privilege, file, downgrade, |p| which(p).is_some());
    let (program, args) = argv.split_first().expect("a command has a program");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run_checked(program, &args, PACKAGE_TIMEOUT).await
}

async fn run_checked(program: &str, args: &[&str], timeout: Duration) -> Result<()> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    no_console_window(&mut command);
    strip_daemon_private_env(&mut command);
    tracing::info!(program, ?args, "serve update: running");
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| anyhow!("{program} did not finish within {}s", timeout.as_secs()))?
        .with_context(|| format!("could not run {program}"))?;
    if !output.status.success() {
        let mut detail = last_lines(&output.stderr, 4);
        if detail.is_empty() {
            detail = last_lines(&output.stdout, 4);
        }
        tracing::warn!(program, status = %output.status, %detail, "serve update: command failed");
        bail!("{program} failed ({}): {detail}", output.status);
    }
    Ok(())
}

fn last_lines(bytes: &[u8], count: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join(" / ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &str) -> bool {
        true
    }

    #[test]
    fn a_deb_goes_through_apt_with_the_lock_timeout() {
        let argv = package_command(
            PackageFormat::Deb,
            Privilege::Root,
            Path::new("/v/x.deb"),
            false,
            all,
        );
        assert_eq!(
            argv[..3],
            ["env", "DEBIAN_FRONTEND=noninteractive", "apt-get"]
        );
        assert!(argv.contains(&"DPkg::Lock::Timeout=120".to_string()));
        assert!(!argv.contains(&"--allow-downgrades".to_string()));
        assert_eq!(argv.last().unwrap(), "/v/x.deb");
    }

    #[test]
    fn putting_the_previous_deb_back_allows_the_downgrade() {
        let argv = package_command(
            PackageFormat::Deb,
            Privilege::Root,
            Path::new("/v/x.deb"),
            true,
            all,
        );
        assert!(argv.contains(&"--allow-downgrades".to_string()));
    }

    #[test]
    fn passwordless_sudo_prefixes_without_ever_prompting() {
        let argv = package_command(
            PackageFormat::Deb,
            Privilege::PasswordlessSudo,
            Path::new("/v/x.deb"),
            false,
            all,
        );
        assert_eq!(argv[..2], ["sudo", "-n"]);
    }

    #[test]
    fn an_rpm_downgrade_uses_rpm_itself() {
        let argv = package_command(
            PackageFormat::Rpm,
            Privilege::Root,
            Path::new("/v/x.rpm"),
            true,
            all,
        );
        assert_eq!(
            argv,
            ["rpm", "-U", "--oldpackage", "--replacepkgs", "/v/x.rpm"]
        );
        let argv = package_command(
            PackageFormat::Rpm,
            Privilege::Root,
            Path::new("/v/x.rpm"),
            false,
            all,
        );
        assert_eq!(argv, ["dnf", "install", "-y", "/v/x.rpm"]);
        let argv = package_command(
            PackageFormat::Rpm,
            Privilege::Root,
            Path::new("/v/x.rpm"),
            false,
            |p| p == "yum",
        );
        assert_eq!(argv, ["yum", "install", "-y", "/v/x.rpm"]);
    }

    #[test]
    fn dpkg_stands_in_when_apt_is_missing() {
        let argv = package_command(
            PackageFormat::Deb,
            Privilege::Root,
            Path::new("/v/x.deb"),
            false,
            |_| false,
        );
        assert_eq!(argv[2..], ["dpkg", "-i", "/v/x.deb"]);
    }

    #[test]
    fn the_team_is_read_and_ad_hoc_reads_as_none() {
        let signed =
            "Executable=/A/Biorouter\nIdentifier=edu.ucsf.biorouter\nTeamIdentifier=F3YYBXAFJ8\n";
        assert_eq!(parse_team_identifier(signed).as_deref(), Some("F3YYBXAFJ8"));
        assert_eq!(
            parse_team_identifier("Signature=adhoc\nTeamIdentifier=not set\n"),
            None
        );
        assert_eq!(parse_team_identifier(""), None);
    }

    #[tokio::test]
    async fn a_bundle_rollback_puts_the_kept_copy_back() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Biorouter.app");
        let backup = bundle_backup(&bundle);
        std::fs::create_dir_all(bundle.join("new")).unwrap();
        std::fs::create_dir_all(backup.join("old")).unwrap();
        rollback(&Rollback::AppBundle {
            bundle: bundle.clone(),
            backup: backup.clone(),
        })
        .await
        .unwrap();
        assert!(bundle.join("old").exists());
        assert!(!bundle.join("new").exists());
        assert!(!backup.exists());
    }

    #[tokio::test]
    async fn a_binary_that_does_not_run_is_caught() {
        let dir = tempfile::tempdir().unwrap();
        let error = binaries_report(dir.path(), "1.0.0").await.unwrap_err();
        assert!(format!("{error:#}").contains("does not run"), "{error:#}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_binary_that_reports_another_version_is_caught() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for name in ["biorouterd", "biorouter"] {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\necho {name} 1.94.1\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        binaries_report(dir.path(), "1.94.1").await.unwrap();
        let error = binaries_report(dir.path(), "1.95.0").await.unwrap_err();
        assert!(error.to_string().contains("reports 1.94.1"), "{error}");
    }
}
