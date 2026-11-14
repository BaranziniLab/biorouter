//! What kind of installation is running, and what updating it takes.
//!
//! Decided once from where the daemon's own executable lives, because that is
//! the installation an update has to replace:
//!
//! | Kind | Found by | Updated with |
//! |---|---|---|
//! | Linux package (`biorouter-cli` or the desktop `biorouter`, deb or rpm) | `dpkg-query -S` / `rpm -qf` owns the executable | the package manager, on the release's package for this kind and CPU |
//! | macOS app bundle | the executable is `<X>.app/Contents/Resources/bin/…` | the release's signed `Biorouter-darwin-<arch>-<v>.zip`, swapped in beside a kept copy |
//! | Built from source | the executable sits in a cargo `target/` directory | `git pull` and a build; nothing to download |
//! | Anything else (Windows, an unpacked copy) | none of the above | by hand, said in one line |
//!
//! ⚠ A package is updated through its package manager, never by replacing its
//! files: the package database would then name a version that is not on disk,
//! and the next `apt` or `dnf` run would act on a lie.

use std::path::{Path, PathBuf};
use std::process::Stdio;

// Every child this module starts gets no console window on Windows (counted by
// `no_console_window_census.rs` at each site) and none of the daemon's private
// variables: a package manager runs maintainer scripts, which have no business
// with the daemon secret or the browser token.
use biorouter_sandbox::console::no_console_window;
use biorouter_sandbox::environment::strip_daemon_private_env;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How a release's files reach this installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallKind {
    /// Owned by a Linux package.
    Package {
        format: PackageFormat,
        /// The package's name, `biorouter-cli` or the desktop package.
        name: String,
        /// The package architecture word (`amd64`, `x86_64`, `arm64`, `aarch64`).
        arch: String,
        /// The directory holding `biorouter` and `biorouterd`.
        bin_dir: PathBuf,
    },
    /// A macOS application bundle.
    AppBundle {
        bundle: PathBuf,
        /// `arm64` or `x64`, as the release names its zips.
        arch: String,
    },
    /// A cargo build in a source tree.
    Source,
    /// Nothing this updater can replace; `reason` says what it is.
    Manual { reason: ManualReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageFormat {
    Deb,
    Rpm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManualReason {
    /// Windows: the zip or installer is updated by the Biorouter app.
    Windows,
    /// Not a package, a bundle or a build tree.
    Unrecognised,
}

/// Who may write the installation, which decides whether one click can do it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Privilege {
    /// This process is root; the package manager runs directly.
    Root,
    /// `sudo -n` works without a password; the package manager runs under it.
    PasswordlessSudo,
    /// This account owns what has to change (an app bundle in a folder it can write).
    Owner,
    /// Nobody this process can act as; an administrator runs one command.
    None,
}

impl Privilege {
    pub fn one_click(self) -> bool {
        !matches!(self, Privilege::None)
    }
}

/// Detect the kind of installation whose daemon executable is `exe`, which
/// must be the resolved path (no symlinks), as `current_exe` gives on Linux.
pub async fn detect(exe: &Path) -> InstallKind {
    if cfg!(windows) {
        return InstallKind::Manual {
            reason: ManualReason::Windows,
        };
    }
    if let Some(bundle) = app_bundle_of(exe) {
        return InstallKind::AppBundle {
            bundle,
            arch: mac_arch().to_string(),
        };
    }
    if is_build_tree(exe) {
        return InstallKind::Source;
    }
    if cfg!(target_os = "linux") {
        if let Some((format, name)) = owning_package(exe).await {
            let bin_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
            return InstallKind::Package {
                arch: package_arch(format).to_string(),
                format,
                name,
                bin_dir,
            };
        }
    }
    InstallKind::Manual {
        reason: ManualReason::Unrecognised,
    }
}

/// `<X>.app` when `exe` is `<X>.app/Contents/Resources/bin/<file>`.
pub fn app_bundle_of(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    let resources = bin.parent()?;
    let contents = resources.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = bin.file_name()? == "bin"
        && resources.file_name()? == "Resources"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

/// A cargo `target/<profile>/` directory, or a cross build's
/// `target/<triple>/<profile>/`.
pub fn is_build_tree(exe: &Path) -> bool {
    exe.ancestors().skip(1).take(3).any(|dir| {
        dir.file_name().is_some_and(|name| name == "target")
            && dir.join("..").join("Cargo.toml").is_file()
    })
}

fn mac_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

fn package_arch(format: PackageFormat) -> &'static str {
    match (format, cfg!(target_arch = "aarch64")) {
        (PackageFormat::Deb, false) => "amd64",
        (PackageFormat::Deb, true) => "arm64",
        (PackageFormat::Rpm, false) => "x86_64",
        (PackageFormat::Rpm, true) => "aarch64",
    }
}

/// The package that owns `exe`, asked of dpkg first and rpm second.
async fn owning_package(exe: &Path) -> Option<(PackageFormat, String)> {
    let path = exe.to_string_lossy().to_string();
    if let Some(out) = run_quiet("dpkg-query", &["-S", &path]).await {
        // `biorouter-cli: /usr/bin/biorouterd` (a diversion line names two).
        let name = out.lines().next()?.split(':').next()?.trim().to_string();
        if !name.is_empty() && !name.contains(' ') {
            return Some((PackageFormat::Deb, name));
        }
    }
    if let Some(out) = run_quiet("rpm", &["-qf", "--qf", "%{NAME}\n", &path]).await {
        let name = out.lines().next()?.trim().to_string();
        if !name.is_empty() && !name.contains(' ') {
            return Some((PackageFormat::Rpm, name));
        }
    }
    None
}

/// The release asset that updates `kind` to `version`, or `None` for a kind
/// that downloads nothing.
///
/// ⚠ These names are the release's own (CLAUDE.md, "Release assets"), which the
/// usage dashboard's classifier also reads. This function only looks them up;
/// it adds and renames none.
pub fn asset_name(kind: &InstallKind, version: &str) -> Option<String> {
    let version = super::normalize_version(version);
    match kind {
        InstallKind::Package {
            format: PackageFormat::Deb,
            name,
            arch,
            ..
        } => Some(format!("{name}_{version}_{arch}.deb")),
        InstallKind::Package {
            format: PackageFormat::Rpm,
            name,
            arch,
            ..
        } => Some(format!("{name}-{version}-1.{arch}.rpm")),
        InstallKind::AppBundle { arch, .. } => {
            Some(format!("Biorouter-darwin-{arch}-{version}.zip"))
        }
        InstallKind::Source | InstallKind::Manual { .. } => None,
    }
}

/// Who can write `kind`. Probes `sudo -n true` for a non-root package install,
/// which on a machine whose sudo asks for a password fails at once and asks
/// nothing (`-n`).
pub async fn privilege(kind: &InstallKind) -> Privilege {
    match kind {
        InstallKind::Package { .. } => {
            if is_root() {
                Privilege::Root
            } else if passwordless_sudo().await {
                Privilege::PasswordlessSudo
            } else {
                Privilege::None
            }
        }
        InstallKind::AppBundle { bundle, .. } => {
            let parent_writable = bundle.parent().is_some_and(writable);
            if parent_writable && writable(bundle) {
                Privilege::Owner
            } else {
                Privilege::None
            }
        }
        InstallKind::Source | InstallKind::Manual { .. } => Privilege::None,
    }
}

#[cfg(unix)]
pub fn is_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}

#[cfg(unix)]
fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: a valid NUL-terminated path; `access` only reads it.
    unsafe { libc::access(c_path.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn writable(_path: &Path) -> bool {
    false
}

async fn passwordless_sudo() -> bool {
    if which("sudo").is_none() {
        return false;
    }
    let mut command = tokio::process::Command::new("sudo");
    command
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    no_console_window(&mut command);
    strip_daemon_private_env(&mut command);
    matches!(
        tokio::time::timeout(Duration::from_secs(5), command.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

/// A program on `PATH`, or `None`.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// Run a program, answering its stdout when it succeeds within ten seconds.
pub(crate) async fn run_quiet(program: &str, args: &[&str]) -> Option<String> {
    which(program)?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    no_console_window(&mut command);
    strip_daemon_private_env(&mut command);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

/// The one line an administrator runs when this process cannot install a
/// package itself.
///
/// It copies the file this daemon already downloaded and verified into a file
/// only root can write, checks the SHA-256 of THAT copy, and installs it. The
/// copy is what makes it safe to hand to root: the downloaded file sits in this
/// account's own folder, where anything running as this account, an agent's
/// shell included, could change it after it was verified. Checking the copy root
/// owns leaves no window in which a changed file is installed as root.
pub fn admin_command(kind: &InstallKind, staged: &Path, sha256: &str) -> Option<String> {
    let InstallKind::Package { format, .. } = kind else {
        return None;
    };
    let (suffix, installer) = match format {
        PackageFormat::Deb => (".deb", "apt-get install -y \"$f\""),
        PackageFormat::Rpm => (
            ".rpm",
            if which("dnf").is_some() {
                "dnf install -y \"$f\""
            } else {
                "yum install -y \"$f\""
            },
        ),
    };
    let source = double_quoted(&staged.to_string_lossy());
    let script = format!(
        "f=$(mktemp --suffix={suffix}) && cp {source} \"$f\" && chmod 644 \"$f\" && echo \"{sha256}  $f\" | sha256sum -c --quiet - && {installer}; s=$?; rm -f \"$f\"; exit $s"
    );
    Some(format!("sudo sh -c {}", single_quoted(&script)))
}

/// `value` inside double quotes, for a POSIX shell.
fn double_quoted(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// `value` inside single quotes, for a POSIX shell.
fn single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(format: PackageFormat, name: &str, arch: &str) -> InstallKind {
        InstallKind::Package {
            format,
            name: name.into(),
            arch: arch.into(),
            bin_dir: PathBuf::from("/usr/bin"),
        }
    }

    #[test]
    fn asset_names_are_the_release_s_own() {
        assert_eq!(
            asset_name(
                &package(PackageFormat::Deb, "biorouter-cli", "amd64"),
                "v1.95.0"
            )
            .as_deref(),
            Some("biorouter-cli_1.95.0_amd64.deb")
        );
        assert_eq!(
            asset_name(
                &package(PackageFormat::Rpm, "biorouter-cli", "x86_64"),
                "1.95.0"
            )
            .as_deref(),
            Some("biorouter-cli-1.95.0-1.x86_64.rpm")
        );
        // The desktop packages.
        assert_eq!(
            asset_name(&package(PackageFormat::Deb, "biorouter", "amd64"), "1.95.0").as_deref(),
            Some("biorouter_1.95.0_amd64.deb")
        );
        assert_eq!(
            asset_name(
                &package(PackageFormat::Rpm, "Biorouter", "x86_64"),
                "1.95.0"
            )
            .as_deref(),
            Some("Biorouter-1.95.0-1.x86_64.rpm")
        );
        let bundle = InstallKind::AppBundle {
            bundle: PathBuf::from("/Applications/Biorouter.app"),
            arch: "arm64".into(),
        };
        assert_eq!(
            asset_name(&bundle, "1.95.0").as_deref(),
            Some("Biorouter-darwin-arm64-1.95.0.zip")
        );
        assert_eq!(asset_name(&InstallKind::Source, "1.95.0"), None);
    }

    #[test]
    fn an_app_bundle_is_recognised_by_its_layout() {
        assert_eq!(
            app_bundle_of(Path::new(
                "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd"
            )),
            Some(PathBuf::from("/Applications/Biorouter.app"))
        );
        assert_eq!(app_bundle_of(Path::new("/usr/bin/biorouterd")), None);
        assert_eq!(
            app_bundle_of(Path::new(
                "/opt/Biorouter/Contents/Resources/bin/biorouterd"
            )),
            None
        );
    }

    #[test]
    fn a_build_tree_is_recognised_by_its_target_directory() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Cargo.toml"), "[workspace]").unwrap();
        let debug = root.path().join("target").join("debug");
        std::fs::create_dir_all(&debug).unwrap();
        assert!(is_build_tree(&debug.join("biorouterd")));
        let cross = root
            .path()
            .join("target")
            .join("x86_64-unknown-linux-gnu")
            .join("release");
        std::fs::create_dir_all(&cross).unwrap();
        assert!(is_build_tree(&cross.join("biorouterd")));
        assert!(!is_build_tree(Path::new("/usr/bin/biorouterd")));
    }

    #[test]
    fn the_admin_command_checks_a_copy_only_root_can_write() {
        let kind = package(PackageFormat::Deb, "biorouter-cli", "amd64");
        let sha = "a".repeat(64);
        let command = admin_command(
            &kind,
            Path::new("/home/ann/.local/share/biorouter/serve-update/1.95.0/biorouter-cli_1.95.0_amd64.deb"),
            &sha,
        )
        .unwrap();
        assert!(command.starts_with("sudo sh -c '"), "{command}");
        let copy = command.find("cp ").unwrap();
        let check = command.find("sha256sum -c").unwrap();
        let install = command.find("apt-get install -y").unwrap();
        assert!(
            copy < check && check < install,
            "copy, then check the copy, then install: {command}"
        );
        assert!(command.contains(&sha));
        assert!(command.contains("mktemp --suffix=.deb"));
        assert!(!command.contains('\n'), "one line");
    }

    #[test]
    fn the_admin_command_survives_a_path_with_quotes_and_dollars() {
        let kind = package(PackageFormat::Deb, "biorouter-cli", "amd64");
        let command = admin_command(
            &kind,
            Path::new("/home/o'neil/$weird \"dir\"/x.deb"),
            &"b".repeat(64),
        )
        .unwrap();
        // The single quote closes and reopens the outer quoting; the inner path is double quoted with
        // `$` and `"` escaped, so the shell copies exactly that file.
        assert!(
            command.contains(r#"/home/o'\''neil/\$weird \"dir\"/x.deb"#),
            "{command}"
        );
    }

    #[test]
    fn nothing_but_a_package_gets_an_admin_command() {
        let bundle = InstallKind::AppBundle {
            bundle: PathBuf::from("/Applications/Biorouter.app"),
            arch: "arm64".into(),
        };
        assert_eq!(admin_command(&bundle, Path::new("/x.zip"), "c"), None);
        assert_eq!(
            admin_command(&InstallKind::Source, Path::new("/x"), "c"),
            None
        );
    }
}
