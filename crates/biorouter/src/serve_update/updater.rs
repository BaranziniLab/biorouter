//! The daemon's side of a browser update: one state machine, read by the
//! status route and driven by a background check and two person actions.
//!
//! ```text
//! idle ─check─▶ checking ─┬─▶ up_to_date
//!                         ├─▶ downloading ─┬─▶ ready        (one click installs and restarts)
//!                         │                └─▶ needs_admin  (one line for an administrator)
//!                         ├─▶ installed    (a newer version is already on disk: one click restarts)
//!                         ├─▶ manual       (nothing here can be downloaded; said in one line)
//!                         └─▶ error
//! ready ─install─▶ installing ─▶ restarting ─▶ (the daemon exits; `serve` restarts it)
//!                       └─▶ error (nothing restarted; a package was put back if it had changed)
//! ```
//!
//! A newer release is downloaded and verified as soon as it is found, as the
//! desktop app's updater does on macOS, so the person's click only installs and
//! restarts.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{bail, Result};
use serde::Serialize;

use super::apply::{self, Staged};
use super::download::{self, downloads_dir};
use super::feed::{Feed, Release};
use super::install::{self, InstallKind, ManualReason, Privilege};
use super::records::{self, LastResult, RestartPlan};
use super::{is_newer, normalize_version, running_version};

/// How often a running daemon looks for a release, as the desktop app does.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(3 * 60 * 60);

/// How long after start the first look waits, so it never competes with the
/// first page load.
pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Downloading,
    Ready,
    NeedsAdmin,
    Installed,
    Manual,
    Installing,
    Restarting,
    Error,
}

/// What the page is told.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub current: String,
    pub latest: Option<String>,
    pub phase: Phase,
    /// 0-100 while downloading.
    pub percent: u8,
    /// The first line of what went wrong.
    pub error: Option<String>,
    /// `package`, `app_bundle`, `source` or `manual`.
    pub install_kind: &'static str,
    /// Why a `manual` installation cannot update from here.
    pub manual_reason: Option<&'static str>,
    /// The one line an administrator runs, in `needs_admin`.
    pub admin_command: Option<String>,
    /// How the last update ended, when it ended recently enough to mention.
    pub last_result: Option<LastResult>,
    /// True when the feed is not the official one.
    pub feed_overridden: bool,
}

#[derive(Debug, Clone)]
struct State {
    phase: Phase,
    latest: Option<Release>,
    percent: u8,
    error: Option<String>,
    update: Option<Staged>,
    previous: Option<Staged>,
    privilege: Option<Privilege>,
    admin_command: Option<String>,
    /// The version on disk when it is newer than the one running.
    installed: Option<String>,
    /// Why a `manual` phase came about when the installation itself is one
    /// this updater can replace (`no_package`: the release has no file for
    /// this processor).
    manual_reason: Option<&'static str>,
}

/// One daemon's updater.
pub struct Updater {
    feed: Feed,
    client: reqwest::Client,
    port: u16,
    serve_pid: u32,
    daemon_exe: PathBuf,
    kind: tokio::sync::OnceCell<InstallKind>,
    state: Mutex<State>,
    work: tokio::sync::Mutex<()>,
    /// Tests name the privilege rather than probe it: the probe runs `sudo -n`.
    #[cfg(test)]
    privilege_override: Option<Privilege>,
}

/// What [`Updater::install`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// The restart plan is written; the daemon must now exit with
    /// [`super::RESTART_EXIT_CODE`].
    RestartNow,
}

impl Updater {
    pub fn new(feed: Feed, port: u16, serve_pid: u32, daemon_exe: PathBuf) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            client: super::feed::client()?,
            feed,
            port,
            serve_pid,
            daemon_exe,
            kind: tokio::sync::OnceCell::new(),
            state: Mutex::new(State {
                phase: Phase::Idle,
                latest: None,
                percent: 0,
                error: None,
                update: None,
                previous: None,
                privilege: None,
                admin_command: None,
                installed: None,
                manual_reason: None,
            }),
            work: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            privilege_override: None,
        }))
    }

    async fn kind(&self) -> &InstallKind {
        self.kind
            .get_or_init(|| async { install::detect(&self.daemon_exe).await })
            .await
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_phase(&self, phase: Phase) {
        self.lock().phase = phase;
    }

    fn fail(&self, error: &anyhow::Error) {
        let mut state = self.lock();
        state.phase = Phase::Error;
        state.error = Some(first_line(&format!("{error:#}")));
        tracing::warn!(error = %format!("{error:#}"), "serve update failed");
    }

    /// The page's view.
    pub async fn status(&self) -> Status {
        let kind = self.kind().await.clone();
        let state = self.lock().clone();
        let (install_kind, manual_reason) = match &kind {
            InstallKind::Package { .. } => ("package", None),
            InstallKind::AppBundle { .. } => ("app_bundle", None),
            InstallKind::Source => ("source", Some("source")),
            InstallKind::Manual {
                reason: ManualReason::Windows,
            } => ("manual", Some("windows")),
            InstallKind::Manual {
                reason: ManualReason::Unrecognised,
            } => ("manual", Some("unrecognised")),
        };
        let manual_reason = match (state.phase, &kind, state.manual_reason) {
            (Phase::Manual, _, Some(reason)) => Some(reason),
            // A bundle in a folder this account cannot write is updated by hand too.
            (Phase::Manual, InstallKind::AppBundle { .. }, None) => Some("not_writable"),
            _ => manual_reason,
        };
        let latest = match state.phase {
            Phase::Installed => state.installed.clone(),
            _ => state.latest.as_ref().map(|r| r.version.clone()),
        };
        Status {
            current: running_version().to_string(),
            latest,
            phase: state.phase,
            percent: state.percent,
            error: state.error,
            install_kind,
            manual_reason,
            admin_command: state.admin_command,
            last_result: records::peek(&records::last_result_path(self.port)),
            feed_overridden: self.feed.overridden,
        }
    }

    /// Look for a release now, and download it when it is newer. Returns at
    /// once, changing nothing, while another check or an install runs.
    pub async fn check(&self) {
        let Ok(_work) = self.work.try_lock() else {
            return;
        };
        let before = self.lock().clone();
        if matches!(before.phase, Phase::Installing | Phase::Restarting) {
            return;
        }
        // An update already downloaded stays installable while the feed is
        // asked again: the phase moves only when the answer changes something.
        let staged = matches!(
            before.phase,
            Phase::Ready | Phase::NeedsAdmin | Phase::Installed
        );
        {
            let mut state = self.lock();
            if !staged {
                state.phase = Phase::Checking;
            }
            state.error = None;
        }
        if let Err(error) = self.check_inner().await {
            if staged {
                // A failed look (the feed down, a rate limit) does not take away
                // an update that is already here and verified.
                tracing::warn!(error = %format!("{error:#}"), "serve update re-check failed; keeping the downloaded update");
                let mut state = self.lock();
                state.phase = before.phase;
                state.percent = before.percent;
                state.error = None;
            } else {
                self.fail(&error);
            }
        }
    }

    async fn check_inner(&self) -> Result<()> {
        let kind = self.kind().await.clone();
        let current = running_version();

        // An administrator (or an unattended upgrade) may already have put a
        // newer version on disk; all that is left is a restart. Asked before the
        // feed, so it is found even when the feed cannot be reached.
        if let Some(on_disk) = self.version_on_disk(&kind).await {
            if is_newer(&on_disk, current) {
                let mut state = self.lock();
                state.phase = Phase::Installed;
                state.installed = Some(on_disk);
                return Ok(());
            }
        }

        let release = self.feed.latest(&self.client).await?;

        if !is_newer(&release.version, current) {
            let mut state = self.lock();
            state.phase = Phase::UpToDate;
            state.latest = Some(release);
            return Ok(());
        }

        // Already downloaded and verified (a later look at the same release).
        {
            let state = self.lock();
            if matches!(state.phase, Phase::Ready | Phase::NeedsAdmin)
                && state.update.as_ref().map(|u| normalize_version(&u.version))
                    == Some(normalize_version(&release.version))
            {
                return Ok(());
            }
        }

        #[cfg(test)]
        let privilege = match self.privilege_override {
            Some(privilege) => privilege,
            None => install::privilege(&kind).await,
        };
        #[cfg(not(test))]
        let privilege = install::privilege(&kind).await;
        {
            let mut state = self.lock();
            state.latest = Some(release.clone());
            state.privilege = Some(privilege);
            state.manual_reason = None;
        }
        let Some(asset_name) = install::asset_name(&kind, &release.version) else {
            self.set_phase(Phase::Manual);
            return Ok(());
        };
        if matches!(kind, InstallKind::AppBundle { .. }) && !privilege.one_click() {
            self.set_phase(Phase::Manual);
            return Ok(());
        }
        let Some(asset) = release.asset(&asset_name).cloned() else {
            // No package for this processor (no Linux arm64 package is
            // published): nothing to offer, which is not a failure.
            tracing::info!(%asset_name, version = %release.version, "the release has no file for this server");
            let mut state = self.lock();
            state.phase = Phase::Manual;
            state.manual_reason = Some("no_package");
            return Ok(());
        };
        {
            let mut state = self.lock();
            state.phase = Phase::Downloading;
            state.percent = 0;
        }
        let dir = downloads_dir().join(normalize_version(&release.version));
        let file = download::fetch_verified(&self.client, &asset, &dir, |got, total| {
            if total > 0 {
                let percent = ((got.saturating_mul(100)) / total).min(100) as u8;
                self.lock().percent = percent;
            }
        })
        .await?;
        let update = Staged {
            file: file.clone(),
            sha256: asset.sha256.clone().unwrap_or_default(),
            version: release.version.clone(),
        };

        let mut keep = vec![normalize_version(&release.version)];
        let mut previous = None;
        if matches!(kind, InstallKind::Package { .. }) && privilege.one_click() {
            // A package's way back is the running version's own package.
            previous = Some(self.fetch_previous(&kind).await?);
            keep.push(normalize_version(current));
        }
        download::prune(&keep.iter().map(String::as_str).collect::<Vec<_>>()).await;

        let mut state = self.lock();
        state.percent = 100;
        state.update = Some(update.clone());
        state.previous = previous;
        if privilege.one_click() {
            state.phase = Phase::Ready;
            state.admin_command = None;
        } else {
            state.phase = Phase::NeedsAdmin;
            state.admin_command = install::admin_command(&kind, &file, &update.sha256);
        }
        Ok(())
    }

    async fn fetch_previous(&self, kind: &InstallKind) -> Result<Staged> {
        let current = running_version();
        let Some(release) = self.feed.release(&self.client, current).await? else {
            bail!(
                "{current}, the version running here, is not published, so it could not be kept to roll back to"
            );
        };
        let Some(name) = install::asset_name(kind, current) else {
            bail!("no package to keep for {current}");
        };
        let Some(asset) = release.asset(&name).cloned() else {
            bail!("{current} has no {name} to keep for a rollback");
        };
        let dir = downloads_dir().join(normalize_version(current));
        let file = download::fetch_verified(&self.client, &asset, &dir, |_, _| {}).await?;
        Ok(Staged {
            file,
            sha256: asset.sha256.unwrap_or_default(),
            version: current.to_string(),
        })
    }

    async fn version_on_disk(&self, kind: &InstallKind) -> Option<String> {
        let bin_dir: PathBuf = match kind {
            InstallKind::Package { bin_dir, .. } => bin_dir.clone(),
            InstallKind::AppBundle { bundle, .. } => {
                bundle.join("Contents").join("Resources").join("bin")
            }
            InstallKind::Source | InstallKind::Manual { .. } => return None,
        };
        let daemon = bin_dir.join(format!("biorouterd{}", std::env::consts::EXE_SUFFIX));
        apply::binary_version(&daemon).await.ok()
    }

    /// Is there something [`Updater::install`] would act on now?
    pub fn installable(&self) -> bool {
        matches!(self.lock().phase, Phase::Ready | Phase::Installed)
    }

    /// Install the downloaded update (or, in `installed`, only restart) and
    /// leave `serve` the plan for the restart.
    pub async fn install(&self) -> Result<Installed> {
        let Ok(_work) = self.work.try_lock() else {
            bail!("an update is already being checked or installed");
        };
        let state = self.lock().clone();
        let kind = self.kind().await.clone();
        let current = running_version().to_string();
        let plan = match state.phase {
            Phase::Ready => {
                let (Some(update), Some(privilege)) = (state.update.clone(), state.privilege)
                else {
                    bail!("the update is not ready to install");
                };
                self.set_phase(Phase::Installing);
                match apply::install(&kind, privilege, &update, state.previous.as_ref()).await {
                    Ok(rollback) => RestartPlan {
                        port: self.port,
                        serve_pid: self.serve_pid,
                        from: current,
                        to: update.version.clone(),
                        rollback: Some(rollback),
                    },
                    Err(error) => {
                        self.fail(&error);
                        return Err(error);
                    }
                }
            }
            Phase::Installed => {
                let Some(on_disk) = state.installed.clone() else {
                    bail!("no newer version is installed");
                };
                RestartPlan {
                    port: self.port,
                    serve_pid: self.serve_pid,
                    from: current,
                    to: on_disk,
                    rollback: None,
                }
            }
            _ => bail!("there is no update ready to install"),
        };
        // An update that ends here (installed and planned, or failed) leaves
        // nothing for a later check to keep.
        {
            let mut state = self.lock();
            state.update = None;
        }
        let _ = std::fs::remove_file(records::last_result_path(self.port));
        if let Err(error) = records::write_private(&records::restart_plan_path(self.port), &plan) {
            // The install already landed; without the plan `serve` would read the exit
            // as a crash. Say so, and leave the next start to pick the new version up.
            let error = error.context("the update is installed, but restarting needs Biorouter to be started again by hand");
            self.fail(&error);
            return Err(error);
        }
        self.set_phase(Phase::Restarting);
        Ok(Installed::RestartNow)
    }

    /// The daemon's background loop: a first look shortly after start, then one
    /// every [`CHECK_INTERVAL`].
    pub fn spawn_background(self: &Arc<Self>) {
        let updater = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_CHECK_DELAY).await;
            loop {
                updater.check().await;
                tokio::time::sleep(CHECK_INTERVAL).await;
            }
        });
    }

    pub fn daemon_exe(&self) -> &Path {
        &self.daemon_exe
    }
}

static GLOBAL: OnceLock<Arc<Updater>> = OnceLock::new();
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Make `updater` this daemon's. Process-global because the server's route
/// modules are compiled twice (`biorouter-server`'s lib and its binary), and a
/// static there would exist twice.
pub fn install_global(updater: Arc<Updater>) {
    let _ = GLOBAL.set(updater);
}

/// This daemon's updater, when `serve` started it with the restart contract.
pub fn global() -> Option<Arc<Updater>> {
    GLOBAL.get().cloned()
}

/// Ask the daemon to exit with [`super::RESTART_EXIT_CODE`] once it has shut
/// down. Set only after [`Updater::install`] wrote the restart plan.
pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::SeqCst);
}

/// Did [`request_restart`] run? Read by the daemon's `main` after its server
/// stopped, and by its shutdown to shorten the wait for open requests.
pub fn restart_requested() -> bool {
    RESTART_REQUESTED.load(Ordering::SeqCst)
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn release_json(server: &MockServer, version: &str) -> String {
        format!(
            r#"{{"tag_name":"v{version}","assets":[{{"name":"biorouter-cli_{version}_amd64.deb","browser_download_url":"{}/dl/{version}.deb","size":3,"digest":"sha256:{}"}}]}}"#,
            server.uri(),
            "0".repeat(64)
        )
    }

    fn sha(body: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn package_kind() -> InstallKind {
        InstallKind::Package {
            format: install::PackageFormat::Deb,
            name: "biorouter-cli".into(),
            arch: "amd64".into(),
            bin_dir: PathBuf::from("/nonexistent"),
        }
    }

    /// An updater for a root-run deb install, with a feed offering `latest`
    /// (and the running version for the rollback copy), each file `body`.
    async fn package_updater(server: &MockServer, latest: &str, body: &[u8]) -> Arc<Updater> {
        let running = running_version();
        for version in [latest, running] {
            let name = format!("biorouter-cli_{version}_amd64.deb");
            let doc = format!(
                r#"{{"tag_name":"v{version}","assets":[{{"name":"{name}","browser_download_url":"{}/dl/{name}","size":{},"digest":"sha256:{}"}}]}}"#,
                server.uri(),
                body.len(),
                sha(body)
            );
            let at = if version == latest {
                "/releases/latest".to_string()
            } else {
                format!("/releases/tags/v{version}")
            };
            Mock::given(method("GET"))
                .and(path(at))
                .respond_with(ResponseTemplate::new(200).set_body_string(doc))
                .mount(server)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/dl/{name}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
                .mount(server)
                .await;
        }
        let feed = Feed::parse(&format!("{}/releases", server.uri())).unwrap();
        let mut updater =
            Arc::try_unwrap(Updater::new(feed, 18749, 1, PathBuf::from("/nonexistent")).unwrap())
                .ok()
                .unwrap();
        updater.privilege_override = Some(Privilege::Root);
        updater.kind.set(package_kind()).unwrap();
        Arc::new(updater)
    }

    #[tokio::test]
    async fn a_downloaded_update_survives_a_failed_look_at_the_feed() {
        let server = MockServer::start().await;
        let updater = package_updater(&server, "999.0.0", b"package").await;
        updater.check().await;
        assert_eq!(updater.status().await.phase, Phase::Ready);
        assert!(updater.installable());

        // The feed goes away (a rate limit, an outage): the update already here
        // and verified stays installable, and no error is shown.
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        updater.check().await;
        let status = updater.status().await;
        assert_eq!(status.phase, Phase::Ready, "{status:?}");
        assert_eq!(status.error, None);
        assert!(updater.installable());
    }

    #[tokio::test]
    async fn a_release_with_no_file_for_this_server_is_not_a_failure() {
        let server = MockServer::start().await;
        // Only an amd64 package is published; this updater looks for arm64.
        let updater = package_updater(&server, "999.0.0", b"package").await;
        let mut arm = Arc::try_unwrap(updater).ok().unwrap();
        arm.kind = tokio::sync::OnceCell::new();
        arm.kind
            .set(InstallKind::Package {
                format: install::PackageFormat::Deb,
                name: "biorouter-cli".into(),
                arch: "arm64".into(),
                bin_dir: PathBuf::from("/nonexistent"),
            })
            .unwrap();
        arm.check().await;
        let status = arm.status().await;
        assert_eq!(status.phase, Phase::Manual, "{status:?}");
        assert_eq!(status.manual_reason, Some("no_package"));
        assert_eq!(status.error, None);
    }

    async fn updater_for(server: &MockServer) -> Arc<Updater> {
        let feed = Feed::parse(&format!("{}/releases", server.uri())).unwrap();
        Updater::new(feed, 18740, 1, PathBuf::from("/nonexistent/biorouterd")).unwrap()
    }

    #[tokio::test]
    async fn an_older_or_equal_release_is_up_to_date() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(release_json(&server, "0.0.1")),
            )
            .mount(&server)
            .await;
        let updater = updater_for(&server).await;
        updater.check().await;
        let status = updater.status().await;
        assert_eq!(status.phase, Phase::UpToDate, "{status:?}");
        assert_eq!(status.latest.as_deref(), Some("0.0.1"));
        assert!(status.feed_overridden);
    }

    #[tokio::test]
    async fn a_newer_release_for_an_installation_nothing_can_replace_is_manual() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(release_json(&server, "999.0.0")),
            )
            .mount(&server)
            .await;
        let updater = updater_for(&server).await;
        updater.check().await;
        let status = updater.status().await;
        assert_eq!(status.phase, Phase::Manual, "{status:?}");
        assert_eq!(status.latest.as_deref(), Some("999.0.0"));
        assert!(status.admin_command.is_none());
        // Nothing to install, so install refuses rather than restarting anything.
        assert!(updater.install().await.is_err());
    }

    #[tokio::test]
    async fn an_unreachable_feed_is_an_error_with_one_line() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let updater = updater_for(&server).await;
        updater.check().await;
        let status = updater.status().await;
        assert_eq!(status.phase, Phase::Error);
        let error = status.error.unwrap();
        assert!(error.contains("503"), "{error}");
        assert!(!error.contains('\n'));
    }
}
