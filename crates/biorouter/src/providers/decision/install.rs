//! Installing a decision model that runs on this computer.
//!
//! A private chat may be read only by a decision model on this computer: Gate
//! H refuses every Public model for it, and Baseten's are always Public. So a
//! private chat on a computer with no local decision model gets no
//! supervision. This module downloads one of the models Biorouter offers into
//! the Ollama that `OLLAMA_HOST` names, and calls it installed only once that
//! Ollama answers decisions with it.
//!
//! - **Offered, never automatic.** A download of 1.5 to 12 GB is the person's
//!   choice. No agent tool calls the installer; its doors are the install
//!   route (`/decisions/models/install`, which the Monitors dialog uses) and
//!   `biorouter models decision pull`.
//! - **Only [`INSTALLABLE_DECISION_MODELS`].** The list bounds what a caller
//!   holding the daemon secret can make the daemon download. `ollama pull`
//!   stays for any other model.
//! - **Only into an Ollama on this computer.** A model pulled into an Ollama
//!   elsewhere is Public by the self-hosted rule and could not read a private
//!   chat, so that is refused before anything is sent ([`ollama_place`]).
//! - **In the daemon, not the renderer.** The renderer's own pull reaches only
//!   `127.0.0.1` under the CSP and only a local daemon's Ollama, so it cannot
//!   work in `biorouter serve`. The daemon also owns the catalog the new model
//!   must appear in ([`super::catalog::forget_discovery`]).
//!
//! `OLLAMA_HOST` is read through [`super::catalog::ollama_base_url`], under
//! `providers/`, where the destination-key scan looks.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use url::Url;

use crate::decision::error::{DecisionError, DecisionResult};
use crate::decision::model::DecisionModelEntry;
use crate::decision::types::DecisionAdapter;
use crate::privacy::ProviderTier;
use crate::providers::api_client::{ApiClient, AuthMethod, AuthProvider};

use super::catalog::{
    discover_ollama, forget_discovery, ollama_base_url, ollama_entry, ollama_facts,
    ollama_load_refusal,
};
use super::chat::OLLAMA_DEFAULT_DECISION_MODEL;

/// A decision model Biorouter offers to install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallableDecisionModel {
    /// The name Ollama knows it by, and the name the catalog lists it under.
    pub name: &'static str,
    /// What a person reads: the same name the model picker shows afterwards.
    pub display_name: &'static str,
    /// The download, roughly. Apple Silicon pulls the MLX build and other
    /// computers the smaller GGUF one, so the figure is the larger of the two.
    pub approx_size: &'static str,
    /// One phrase on what it is for.
    pub note: &'static str,
}

impl InstallableDecisionModel {
    /// Whether this is the one offered first, and installed when a caller
    /// names none: the shipped default decision model.
    pub fn is_default(&self) -> bool {
        self.name == OLLAMA_DEFAULT_DECISION_MODEL
    }
}

/// The models Biorouter installs, the default first. Both answer System One
/// decisions in Ollama (`/api/show` capabilities include `decision`).
pub const INSTALLABLE_DECISION_MODELS: &[InstallableDecisionModel] = &[
    InstallableDecisionModel {
        name: OLLAMA_DEFAULT_DECISION_MODEL,
        display_name: OLLAMA_DEFAULT_DECISION_MODEL,
        approx_size: "about 1.5 GB",
        note: "small and fast",
    },
    InstallableDecisionModel {
        name: "clef-flash:latest",
        display_name: "clef-flash:latest",
        approx_size: "about 12 GB",
        note: "larger, also reads images",
    },
];

/// The model installed when a caller names none.
pub const DEFAULT_INSTALL_MODEL: &str = OLLAMA_DEFAULT_DECISION_MODEL;

/// How long a pull may go without a byte from Ollama. There is no overall
/// deadline: 12 GB on a slow line takes hours, and that is fine while bytes
/// keep coming.
pub const PULL_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long [`ollama_place`] waits for `/api/version`.
const PLACE_TIMEOUT: Duration = Duration::from_secs(3);

/// The longest progress line kept while waiting for its end. Ollama's are
/// about 200 bytes; a server that never ends a line is not Ollama, and must
/// not grow the buffer without bound.
const MAX_PULL_LINE: usize = 64 * 1024;

const OLLAMA_DOWNLOAD_URL: &str = "https://ollama.com/download";

/// The listed model `name` names: its exact name, or the name without the
/// `:latest` tag Ollama adds to an untagged one (`clef-flash`).
pub fn installable(name: &str) -> Option<&'static InstallableDecisionModel> {
    let name = name.trim();
    INSTALLABLE_DECISION_MODELS
        .iter()
        .find(|model| same_model(model.name, name))
}

/// Whether two Ollama names name the same model, `x` being `x:latest`.
fn same_model(a: &str, b: &str) -> bool {
    let full = |name: &str| {
        if name.contains(':') {
            name.to_string()
        } else {
            format!("{name}:latest")
        }
    };
    full(a) == full(b)
}

/// Where the Ollama `OLLAMA_HOST` names is, as far as installing into it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OllamaPlace {
    /// On this computer, and answering `/api/version`.
    Running { version: String },
    /// On this computer, but nothing answered as Ollama does.
    NotRunning,
    /// Not on this computer. Nothing was sent to it.
    NotThisComputer,
    /// `OLLAMA_HOST` does not resolve to a server address; the reason.
    InvalidHost(String),
}

impl OllamaPlace {
    /// The wire name: `running`, `not_running`, `not_this_computer` or
    /// `invalid_host`.
    pub fn state(&self) -> &'static str {
        match self {
            Self::Running { .. } => "running",
            Self::NotRunning => "not_running",
            Self::NotThisComputer => "not_this_computer",
            Self::InvalidHost(_) => "invalid_host",
        }
    }

    /// Ollama's version while it runs.
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Running { version } => Some(version),
            _ => None,
        }
    }
}

/// Where the Ollama at `base` is. A base that is not this computer is
/// [`OllamaPlace::NotThisComputer`], decided by the same rule that makes a
/// model there Public ([`crate::providers::self_hosted_tier`]), with no
/// request sent. Otherwise `GET /api/version` is asked, with a 3 s bound.
pub async fn ollama_place(base: &Url) -> OllamaPlace {
    if crate::providers::self_hosted_tier(base.as_str()) != ProviderTier::Private {
        return OllamaPlace::NotThisComputer;
    }
    let Ok(client) = ApiClient::with_timeout(
        base.to_string(),
        AuthMethod::Custom(Box::new(NoAuth)),
        PLACE_TIMEOUT,
    ) else {
        return OllamaPlace::NotRunning;
    };
    let ask = async {
        let response = client.response_get("api/version").await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body = response.json::<Value>().await.ok()?;
        body.get("version")?.as_str().map(str::to_string)
    };
    match tokio::time::timeout(PLACE_TIMEOUT, ask).await {
        Ok(Some(version)) => OllamaPlace::Running { version },
        _ => OllamaPlace::NotRunning,
    }
}

/// What an install refusal maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallRefusalKind {
    /// The model is not one Biorouter installs.
    Unlisted,
    /// Another model is still being installed.
    Busy,
    /// `OLLAMA_HOST` names an Ollama on another computer.
    NotThisComputer,
    /// The Ollama on this computer did not answer.
    NotRunning,
    /// `OLLAMA_HOST` does not resolve.
    InvalidHost,
}

impl InstallRefusalKind {
    /// The HTTP status the install route answers with: 400 for a model not
    /// offered, 409 for what waits on the configuration or on the running
    /// install, 503 for an Ollama that is not running.
    pub fn http_status(self) -> u16 {
        match self {
            Self::Unlisted => 400,
            Self::Busy | Self::NotThisComputer | Self::InvalidHost => 409,
            Self::NotRunning => 503,
        }
    }
}

/// Why an install did not start, as a sentence a person can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct InstallRefusal {
    pub kind: InstallRefusalKind,
    pub message: String,
}

impl InstallRefusal {
    fn new(kind: InstallRefusalKind, message: String) -> Self {
        Self { kind, message }
    }

    fn unlisted(name: &str) -> Self {
        let names = INSTALLABLE_DECISION_MODELS
            .iter()
            .map(|model| model.name)
            .collect::<Vec<_>>()
            .join(" and ");
        Self::new(
            InstallRefusalKind::Unlisted,
            format!(
                "`{name}` is not a decision model Biorouter installs. It installs {names}; pull \
                 any other with `ollama pull {name}`."
            ),
        )
    }

    fn busy(running: &str) -> Self {
        Self::new(
            InstallRefusalKind::Busy,
            format!("{running} is still downloading. Wait for it to finish, then try again."),
        )
    }
}

/// The sentence for an Ollama that cannot take an install, or `None` while it
/// runs on this computer. `model` is named in the advice for a remote server.
pub fn place_sentence(place: &OllamaPlace, base: Option<&Url>, model: &str) -> Option<String> {
    let base = base.map(Url::as_str).unwrap_or("(unset)");
    match place {
        OllamaPlace::Running { .. } => None,
        OllamaPlace::NotRunning => Some(not_running_sentence(base)),
        OllamaPlace::NotThisComputer => Some(format!(
            "Biorouter installs decision models only into an Ollama on this computer, and \
             OLLAMA_HOST is {base}. A model there could not read a private chat. Pull it on that \
             server with `ollama pull {model}`, or point OLLAMA_HOST at this computer."
        )),
        OllamaPlace::InvalidHost(reason) => Some(format!(
            "OLLAMA_HOST could not be read ({reason}), so Biorouter cannot tell which Ollama to \
             install into. Correct it, or remove it to use the Ollama on this computer, then try \
             again."
        )),
    }
}

fn not_running_sentence(base: &str) -> String {
    format!(
        "Ollama is not running at {base}. Install it from {OLLAMA_DOWNLOAD_URL} or start it, \
         then try again."
    )
}

/// Every refusal an install can meet before it starts, other than another
/// install running: a model not offered (checked first, sending nothing),
/// then an `OLLAMA_HOST` that does not resolve, is not this computer, or does
/// not answer. On success, the listed model and the base to pull into.
///
/// The CLI (`biorouter models decision pull`) asks this before [`pull`], so it
/// refuses what the route refuses; [`DecisionInstaller::start`] asks it too.
pub async fn preflight(
    base: Result<Url, String>,
    name: &str,
) -> Result<(&'static InstallableDecisionModel, Url), InstallRefusal> {
    let model = installable(name).ok_or_else(|| InstallRefusal::unlisted(name))?;
    let base = base.map_err(|reason| {
        let place = OllamaPlace::InvalidHost(reason);
        InstallRefusal::new(
            InstallRefusalKind::InvalidHost,
            place_sentence(&place, None, model.name).unwrap_or_default(),
        )
    })?;
    let place = ollama_place(&base).await;
    let kind = match place {
        OllamaPlace::Running { .. } => return Ok((model, base)),
        OllamaPlace::NotRunning => InstallRefusalKind::NotRunning,
        OllamaPlace::NotThisComputer => InstallRefusalKind::NotThisComputer,
        OllamaPlace::InvalidHost(_) => InstallRefusalKind::InvalidHost,
    };
    Err(InstallRefusal::new(
        kind,
        place_sentence(&place, Some(&base), model.name).unwrap_or_default(),
    ))
}

/// The configured Ollama's address (`OLLAMA_HOST`), or why it does not
/// resolve.
pub fn configured_ollama_base() -> Result<Url, String> {
    ollama_base_url().map_err(|error| error.to_string())
}

/// Which part of an install a progress report belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullStage {
    /// Ollama is downloading.
    Downloading,
    /// Ollama finished; Biorouter is checking that it answers decisions with
    /// the model.
    Verifying,
}

/// One progress report from [`pull`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullProgress {
    pub stage: PullStage,
    /// Ollama's own words for the step (`pulling manifest`,
    /// `pulling 6a0746a1ec1a`, `verifying sha256 digest`, `success`).
    pub status: String,
    /// Bytes downloaded, summed over every layer (digest) seen so far.
    pub completed_bytes: u64,
    /// Bytes to download, summed the same way. It grows as Ollama names more
    /// layers.
    pub total_bytes: u64,
}

/// Download `name` into the Ollama at `base` and check it: `POST /api/pull`,
/// streamed, then the model must answer decisions there.
///
/// - There is no overall deadline; [`PULL_IDLE_TIMEOUT`] of silence fails it.
///   Ollama keeps what it downloaded, so installing again resumes.
/// - An `error` line fails it with Ollama's own words. A stream that ends
///   without Ollama's final `success` fails too: nothing counts as installed
///   on a hope.
/// - After `success` the model must have the `decision` capability (an older
///   Ollama lists none) and a copy Ollama will load
///   ([`ollama_load_refusal`]). Only then is the catalog's last discovery
///   forgotten, so the model lists at once, and its entry returned.
///
/// It refuses nothing about the place: callers ask [`preflight`] first.
pub async fn pull(
    base: &Url,
    name: &str,
    on_progress: impl FnMut(PullProgress),
) -> DecisionResult<DecisionModelEntry> {
    pull_with_idle(base, name, PULL_IDLE_TIMEOUT, on_progress).await
}

async fn pull_with_idle(
    base: &Url,
    name: &str,
    idle: Duration,
    mut on_progress: impl FnMut(PullProgress),
) -> DecisionResult<DecisionModelEntry> {
    // The client's own stall bound sits a little above `idle`, so the bound
    // below, which says what happened, is the one that fires.
    let client_idle = Duration::from_secs(idle.as_secs() + 5);
    let client = ApiClient::with_idle_timeout(
        base.to_string(),
        AuthMethod::Custom(Box::new(NoAuth)),
        client_idle,
    )
    .map_err(|error| pull_failed(name, format!("Ollama's client could not start: {error}.")))?;
    let request = json!({ "model": name, "stream": true });
    let send = client.response_post("api/pull", &request);
    let response = match tokio::time::timeout(idle, send).await {
        Err(_) => return Err(pull_failed(name, silent_sentence(name, idle))),
        Ok(Err(error)) => return Err(pull_failed(name, unreached_sentence(base, name, &error))),
        Ok(Ok(response)) => response,
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let said = tokio::time::timeout(idle, response.json::<Value>())
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|body| body.get("error")?.as_str().map(str::to_string));
        let message = match said {
            Some(said) => ollama_said(name, &said),
            None => format!("Ollama could not download `{name}` (HTTP {status})."),
        };
        return Err(DecisionError::Backend {
            model: name.to_string(),
            status: Some(status),
            error_type: None,
            message,
        });
    }
    let chunks = futures::stream::unfold(response, |mut response| async move {
        match response.chunk().await {
            Ok(Some(chunk)) => Some((Ok(chunk), response)),
            Ok(None) => None,
            Err(error) if error.is_timeout() => Some((Err(StreamBreak::Silent), response)),
            Err(error) => Some((Err(StreamBreak::Broken(error.to_string())), response)),
        }
    });
    let done = follow_pull(name, Box::pin(chunks), idle, &mut on_progress).await?;
    on_progress(PullProgress {
        stage: PullStage::Verifying,
        ..done
    });
    verify(base, name).await
}

/// Why the body stopped arriving.
#[derive(Debug)]
enum StreamBreak {
    /// The client's own stall bound fired.
    Silent,
    /// The connection failed; the transport's words.
    Broken(String),
}

/// The per-layer byte counts of one pull.
#[derive(Default)]
struct PullTally {
    /// digest to (completed, total), each the largest Ollama reported, so the
    /// sums never go back.
    layers: HashMap<String, (u64, u64)>,
}

impl PullTally {
    fn note(&mut self, line: &Value) {
        let Some(digest) = line.get("digest").and_then(Value::as_str) else {
            return;
        };
        let read = |key: &str| line.get(key).and_then(Value::as_u64).unwrap_or(0);
        let layer = self.layers.entry(digest.to_string()).or_default();
        layer.0 = layer.0.max(read("completed"));
        layer.1 = layer.1.max(read("total"));
    }

    fn progress(&self, status: &str) -> PullProgress {
        PullProgress {
            stage: PullStage::Downloading,
            status: status.to_string(),
            completed_bytes: self.layers.values().map(|layer| layer.0).sum(),
            total_bytes: self.layers.values().map(|layer| layer.1).sum(),
        }
    }
}

/// Read Ollama's NDJSON pull stream to its final `success`, reporting
/// progress per line, and return the last report. Lines are buffered across
/// chunks, so a line split anywhere reads whole.
async fn follow_pull<S, B>(
    name: &str,
    mut chunks: S,
    idle: Duration,
    on_progress: &mut impl FnMut(PullProgress),
) -> DecisionResult<PullProgress>
where
    S: Stream<Item = Result<B, StreamBreak>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut pending: Vec<u8> = Vec::new();
    let mut tally = PullTally::default();
    let mut last = tally.progress("");
    loop {
        let next = tokio::time::timeout(idle, chunks.next())
            .await
            .map_err(|_| pull_failed(name, silent_sentence(name, idle)))?;
        let at_end = match next {
            Some(Ok(chunk)) => {
                pending.extend_from_slice(chunk.as_ref());
                false
            }
            Some(Err(StreamBreak::Silent)) => {
                return Err(pull_failed(name, silent_sentence(name, idle)))
            }
            Some(Err(StreamBreak::Broken(error))) => {
                return Err(pull_failed(
                    name,
                    format!(
                        "The download of `{name}` from Ollama broke off ({error}). Ollama keeps \
                         what it downloaded, so installing it again resumes it."
                    ),
                ))
            }
            None => true,
        };
        // Every complete line, and at the end whatever is left: a last line
        // may come without its newline.
        while let Some(end) = pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            if let Some(done) = read_line(name, &line, &mut tally, &mut last, on_progress)? {
                return Ok(done);
            }
        }
        if at_end {
            let line = std::mem::take(&mut pending);
            if let Some(done) = read_line(name, &line, &mut tally, &mut last, on_progress)? {
                return Ok(done);
            }
            let after = match last.status.as_str() {
                "" => String::new(),
                status => format!(" (its last step was `{status}`)"),
            };
            return Err(pull_failed(
                name,
                format!(
                    "Ollama stopped before it finished downloading `{name}`{after}, so it was \
                     not installed. Install it again to resume the download."
                ),
            ));
        }
        if pending.len() > MAX_PULL_LINE {
            return Err(pull_failed(
                name,
                format!(
                    "The server at the Ollama address answered the download of `{name}` with a \
                     line longer than {MAX_PULL_LINE} bytes, which is not Ollama's progress \
                     format, so it was stopped."
                ),
            ));
        }
    }
}

/// One NDJSON line: an `error` fails the pull with Ollama's words; `success`
/// ends it; anything else is progress. A line that is not a JSON object
/// cannot be either, and is skipped.
fn read_line(
    name: &str,
    line: &[u8],
    tally: &mut PullTally,
    last: &mut PullProgress,
    on_progress: &mut impl FnMut(PullProgress),
) -> DecisionResult<Option<PullProgress>> {
    let Ok(line) = serde_json::from_slice::<Value>(line.trim_ascii()) else {
        return Ok(None);
    };
    if let Some(said) = line.get("error").and_then(Value::as_str) {
        return Err(pull_failed(name, ollama_said(name, said)));
    }
    let Some(status) = line.get("status").and_then(Value::as_str) else {
        return Ok(None);
    };
    tally.note(&line);
    *last = tally.progress(status);
    if status == "success" {
        return Ok(Some(last.clone()));
    }
    on_progress(last.clone());
    Ok(None)
}

/// After Ollama's `success`: the model must answer decisions there.
async fn verify(base: &Url, name: &str) -> DecisionResult<DecisionModelEntry> {
    let facts = ollama_facts(base, name).await?.ok_or_else(|| {
        DecisionError::NoModel(format!(
            "Ollama downloaded `{name}`, but did not describe it when asked, so it could not be \
             checked. Install it again."
        ))
    })?;
    if !facts.decision {
        return Err(DecisionError::Unsupported(format!(
            "Ollama downloaded `{name}`, but it does not answer decisions there (its capabilities \
             lack `decision`). Update Ollama from {OLLAMA_DOWNLOAD_URL}, then install it again."
        )));
    }
    if let Some(refusal) = ollama_load_refusal(base, name, None).await {
        return Err(DecisionError::Unsupported(refusal));
    }
    forget_discovery();
    Ok(ollama_entry(base, name, &facts, DecisionAdapter::SystemOne))
}

fn pull_failed(name: &str, message: String) -> DecisionError {
    DecisionError::Backend {
        model: name.to_string(),
        status: None,
        error_type: None,
        message,
    }
}

/// Ollama's own words about a failed pull. They name the model and the
/// registry, never a chat.
fn ollama_said(name: &str, said: &str) -> String {
    let said = said.trim();
    let stop = if said.ends_with(['.', '!', '?']) {
        ""
    } else {
        "."
    };
    format!("Ollama could not download `{name}`: {said}{stop}")
}

fn silent_sentence(name: &str, idle: Duration) -> String {
    format!(
        "Ollama sent nothing for {} while downloading `{name}`, so the download was stopped. \
         Ollama keeps what it downloaded, so installing it again resumes it.",
        human_duration(idle)
    )
}

fn human_duration(span: Duration) -> String {
    if span.as_millis() < 1000 {
        format!("{} ms", span.as_millis())
    } else {
        format!("{} s", span.as_secs())
    }
}

fn unreached_sentence(base: &Url, name: &str, error: &anyhow::Error) -> String {
    let refused = error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(reqwest::Error::is_connect);
    if refused {
        not_running_sentence(base.as_str())
    } else {
        format!("Biorouter could not ask the Ollama at {base} to download `{name}`: {error}.")
    }
}

struct NoAuth;

#[async_trait]
impl AuthProvider for NoAuth {
    async fn get_auth_header(&self) -> anyhow::Result<(String, String)> {
        Ok(("X-No-Auth".to_string(), "true".to_string()))
    }
}

/// Where an install stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallState {
    Downloading,
    Verifying,
    Installed,
    Failed,
}

impl InstallState {
    /// The wire name: `downloading`, `verifying`, `installed` or `failed`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Downloading => "downloading",
            Self::Verifying => "verifying",
            Self::Installed => "installed",
            Self::Failed => "failed",
        }
    }

    /// Whether the install is still going.
    pub fn is_running(self) -> bool {
        matches!(self, Self::Downloading | Self::Verifying)
    }
}

/// The current or last install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallJob {
    pub model: String,
    pub state: InstallState,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    /// Ollama's last step, in its words.
    pub status: Option<String>,
    /// Why it failed, once it has.
    pub error: Option<String>,
}

/// One offered model and whether it is installed already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOffer {
    pub model: &'static InstallableDecisionModel,
    /// Listed by the configured Ollama on this computer as a decision model
    /// it loads. Never asked of an Ollama elsewhere.
    pub installed: bool,
}

/// The configured Ollama, as the install status reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaStatus {
    /// `OLLAMA_HOST` resolved; `None` when it does not resolve.
    pub base_url: Option<Url>,
    pub place: OllamaPlace,
    /// [`place_sentence`] for the default model: `None` while it runs here.
    pub message: Option<String>,
}

/// What the install route reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionInstallSnapshot {
    pub offers: Vec<InstallOffer>,
    pub ollama: OllamaStatus,
    pub job: Option<InstallJob>,
}

type BaseResolver = Box<dyn Fn() -> Result<Url, String> + Send + Sync>;

/// The job slot: the job and the id of the start that made it, so a finished
/// pull never writes over a later one.
type JobSlot = Arc<Mutex<Option<(u64, InstallJob)>>>;

/// Runs one install at a time in the background and reports it. The daemon
/// uses the process-global [`installer`]; tests build their own over a
/// base-URL resolver.
pub struct DecisionInstaller {
    base: BaseResolver,
    idle: Duration,
    job: JobSlot,
    starts: AtomicU64,
}

impl DecisionInstaller {
    /// An installer for the Ollama `resolve` names when asked.
    pub fn new(resolve: impl Fn() -> Result<Url, String> + Send + Sync + 'static) -> Self {
        Self {
            base: Box::new(resolve),
            idle: PULL_IDLE_TIMEOUT,
            job: Arc::new(Mutex::new(None)),
            starts: AtomicU64::new(0),
        }
    }

    /// The installer for the configured `OLLAMA_HOST`, read at each start.
    pub fn configured() -> Self {
        Self::new(configured_ollama_base)
    }

    /// The same installer with another stall bound (tests).
    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    /// The current or last install. A finished one is kept until the next
    /// start.
    pub fn job(&self) -> Option<InstallJob> {
        lock(&self.job).as_ref().map(|(_, job)| job.clone())
    }

    /// Start installing `name` in the background. Joining is not an error:
    /// asking for the model already being installed answers `Ok` and starts
    /// nothing. Refused, with a sentence, for a model not offered, while
    /// another model installs, and for an Ollama that is not this computer's
    /// or does not answer ([`preflight`]).
    pub async fn start(&self, name: &str) -> Result<(), InstallRefusal> {
        let wanted = installable(name).ok_or_else(|| InstallRefusal::unlisted(name))?;
        if self.join_or_busy(wanted)? {
            return Ok(());
        }
        let (model, base) = preflight((self.base)(), name).await?;
        let id = {
            let mut slot = lock(&self.job);
            if let Some((_, job)) = slot.as_ref().filter(|(_, job)| job.state.is_running()) {
                // Another start won the race while the place was asked.
                return if job.model == model.name {
                    Ok(())
                } else {
                    Err(InstallRefusal::busy(&job.model))
                };
            }
            let id = self.starts.fetch_add(1, Ordering::Relaxed) + 1;
            *slot = Some((
                id,
                InstallJob {
                    model: model.name.to_string(),
                    state: InstallState::Downloading,
                    completed_bytes: 0,
                    total_bytes: 0,
                    status: None,
                    error: None,
                },
            ));
            id
        };
        tracing::info!(model = model.name, base = %base, "installing a decision model");
        let slot = Arc::clone(&self.job);
        let idle = self.idle;
        tokio::spawn(async move {
            let progress_slot = Arc::clone(&slot);
            let outcome = pull_with_idle(&base, model.name, idle, move |progress| {
                update(&progress_slot, id, |job| {
                    job.state = match progress.stage {
                        PullStage::Downloading => InstallState::Downloading,
                        PullStage::Verifying => InstallState::Verifying,
                    };
                    job.completed_bytes = progress.completed_bytes;
                    job.total_bytes = progress.total_bytes;
                    if !progress.status.is_empty() {
                        job.status = Some(progress.status);
                    }
                });
            })
            .await;
            match &outcome {
                Ok(_) => tracing::info!(model = model.name, "decision model installed"),
                Err(error) => {
                    tracing::warn!(model = model.name, %error, "decision model install failed")
                }
            }
            update(&slot, id, |job| match outcome {
                Ok(_) => job.state = InstallState::Installed,
                Err(error) => {
                    job.state = InstallState::Failed;
                    job.error = Some(error.to_string());
                }
            });
        });
        Ok(())
    }

    /// `Ok(true)` when `wanted` is the model already installing, a refusal
    /// when another one is, `Ok(false)` when none is.
    fn join_or_busy(&self, wanted: &InstallableDecisionModel) -> Result<bool, InstallRefusal> {
        match lock(&self.job).as_ref() {
            Some((_, job)) if job.state.is_running() => {
                if job.model == wanted.name {
                    Ok(true)
                } else {
                    Err(InstallRefusal::busy(&job.model))
                }
            }
            _ => Ok(false),
        }
    }

    /// The offers, where the configured Ollama is, and the job. Asks that
    /// Ollama for its version and, when it runs on this computer, which
    /// decision models it lists; an Ollama elsewhere is asked nothing.
    pub async fn snapshot(&self) -> DecisionInstallSnapshot {
        let job = self.job();
        let (base_url, place) = match (self.base)() {
            Ok(base) => {
                let place = ollama_place(&base).await;
                (Some(base), place)
            }
            Err(reason) => (None, OllamaPlace::InvalidHost(reason)),
        };
        let installed: Vec<String> = match (&base_url, &place) {
            (Some(base), OllamaPlace::Running { .. }) => discover_ollama(base)
                .await
                .into_iter()
                .filter(|entry| entry.load_refusal.is_none())
                .map(|entry| entry.model)
                .collect(),
            _ => Vec::new(),
        };
        let offers = INSTALLABLE_DECISION_MODELS
            .iter()
            .map(|model| InstallOffer {
                model,
                installed: installed.iter().any(|name| same_model(name, model.name)),
            })
            .collect();
        let message = place_sentence(&place, base_url.as_ref(), DEFAULT_INSTALL_MODEL);
        DecisionInstallSnapshot {
            offers,
            ollama: OllamaStatus {
                base_url,
                place,
                message,
            },
            job,
        }
    }
}

fn lock(slot: &JobSlot) -> std::sync::MutexGuard<'_, Option<(u64, InstallJob)>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Change the job `id` started, and nothing a later start made.
fn update(slot: &JobSlot, id: u64, change: impl FnOnce(&mut InstallJob)) {
    if let Some((current, job)) = lock(slot).as_mut() {
        if *current == id {
            change(job);
        }
    }
}

static INSTALLER: LazyLock<DecisionInstaller> = LazyLock::new(DecisionInstaller::configured);

/// The daemon's installer, for the configured `OLLAMA_HOST`.
pub fn installer() -> &'static DecisionInstaller {
    &INSTALLER
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::ModelAffiliation;
    use std::time::Instant;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEV1: &str = "tev1:0.8b";

    /// The idle bound the two stall tests give a pull.
    const STALL_BOUND: Duration = Duration::from_millis(200);

    /// How long a stall test may take before it fails. The clock also runs
    /// while the client is built and the timer is scheduled, which a loaded
    /// test run stretches (2.35 s was measured against a 200 ms bound), so
    /// this sits far above the bound and far below [`SILENT_SERVER_ANSWERS`].
    const STALL_STOPS_WITHIN: Duration = Duration::from_secs(10);

    /// When the silent server finally answers: long after
    /// [`STALL_STOPS_WITHIN`], so a pull that waited for it cannot pass.
    const SILENT_SERVER_ANSWERS: Duration = Duration::from_secs(60);

    fn ndjson(lines: &[Value]) -> String {
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>()
    }

    /// A pull of two layers that ends in `success`.
    fn two_layer_pull() -> Vec<Value> {
        vec![
            json!({"status": "pulling manifest"}),
            json!({"status": "pulling aaa", "digest": "sha256:aaa", "total": 1000, "completed": 0}),
            json!({"status": "pulling aaa", "digest": "sha256:aaa", "total": 1000, "completed": 500}),
            json!({"status": "pulling bbb", "digest": "sha256:bbb", "total": 200, "completed": 100}),
            json!({"status": "pulling aaa", "digest": "sha256:aaa", "total": 1000, "completed": 1000}),
            json!({"status": "pulling bbb", "digest": "sha256:bbb", "total": 200, "completed": 200}),
            json!({"status": "verifying sha256 digest"}),
            json!({"status": "writing manifest"}),
            json!({"status": "success"}),
        ]
    }

    fn streamed(lines: &[Value]) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("content-type", "application/x-ndjson")
            .set_body_string(ndjson(lines))
    }

    /// Ollama 0.40.14's load-probe answer for a copy it loads.
    fn loads() -> ResponseTemplate {
        ResponseTemplate::new(400)
            .set_body_json(json!({"error": "questions must contain 1\u{2013}64 fields"}))
    }

    /// Ollama 0.40.14's load-probe answer for a copy it will not load.
    fn will_not_load(name: &str) -> ResponseTemplate {
        ResponseTemplate::new(400).set_body_json(json!({
            "error": format!(
                "'{name}' was downloaded in a format that is no longer supported. \
                 To re-download, run 'ollama pull {name}'"
            )
        }))
    }

    /// An Ollama whose pull streams `lines`, whose `/api/show` reports
    /// `capabilities`, and whose load probe answers `probe`.
    async fn ollama(lines: &[Value], capabilities: &[&str], probe: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": "0.40.15"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/pull"))
            .respond_with(streamed(lines))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"capabilities": capabilities})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(probe)
            .mount(&server)
            .await;
        server
    }

    fn base(server: &MockServer) -> Url {
        Url::parse(&server.uri()).unwrap()
    }

    async fn requests_to(server: &MockServer, to: &str) -> usize {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == to)
            .count()
    }

    /// Wait for the installer's job to finish.
    async fn finished(installer: &DecisionInstaller) -> InstallJob {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(job) = installer.job().filter(|job| !job.state.is_running()) {
                return job;
            }
            assert!(Instant::now() < deadline, "the install never finished");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    #[serial_test::serial(ollama_discovery)]
    async fn a_pull_that_streams_success_and_shows_decision_installs() {
        let server = ollama(&two_layer_pull(), &["decision"], loads()).await;
        Mock::given(method("POST"))
            .and(path("/api/pull"))
            .and(body_json(json!({"model": TEV1, "stream": true})))
            .respond_with(streamed(&two_layer_pull()))
            .expect(1)
            .with_priority(1)
            .mount(&server)
            .await;
        let mut seen = Vec::new();
        let entry = pull(&base(&server), TEV1, |progress| seen.push(progress))
            .await
            .expect("installed");

        assert_eq!(entry.provider, super::super::OLLAMA_DECISION_PROVIDER);
        assert_eq!(entry.model, TEV1);
        assert_eq!(entry.tier, ProviderTier::Private);
        assert!(entry.local);
        assert_eq!(entry.affiliation, Some(ModelAffiliation::Local));
        assert_eq!(entry.adapter, DecisionAdapter::SystemOne);

        // Bytes are summed per layer, so the totals rise and never fall back
        // when Ollama moves from one layer to the other.
        let sums: Vec<(u64, u64)> = seen
            .iter()
            .map(|p| (p.completed_bytes, p.total_bytes))
            .collect();
        assert_eq!(
            sums,
            [
                (0, 0),
                (0, 1000),
                (500, 1000),
                (600, 1200),
                (1100, 1200),
                (1200, 1200),
                (1200, 1200),
                (1200, 1200),
                (1200, 1200),
            ]
        );
        assert!(sums
            .windows(2)
            .all(|w| w[0].0 <= w[1].0 && w[0].1 <= w[1].1));
        assert_eq!(seen[0].status, "pulling manifest");
        assert!(seen[..8].iter().all(|p| p.stage == PullStage::Downloading));
        assert_eq!(seen[8].stage, PullStage::Verifying);
        assert_eq!(seen[8].status, "success");
    }

    #[tokio::test]
    async fn an_error_line_fails_the_pull_with_ollamas_words() {
        let lines = [
            json!({"status": "pulling manifest"}),
            json!({"error": "pull model manifest: file does not exist"}),
        ];
        let server = ollama(&lines, &["decision"], loads()).await;
        let error = pull(&base(&server), TEV1, |_| {})
            .await
            .expect_err("Ollama refused the pull");
        assert_eq!(
            error.to_string(),
            "Ollama could not download `tev1:0.8b`: pull model manifest: file does not exist."
        );
        assert_eq!(
            requests_to(&server, "/api/show").await,
            0,
            "nothing checked"
        );
    }

    /// An `error` answered with a non-2xx status (an invalid name) is
    /// Ollama's words too.
    #[tokio::test]
    async fn a_refused_pull_request_says_what_ollama_said() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/pull"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"error": "invalid model name"})),
            )
            .mount(&server)
            .await;
        let error = pull(&base(&server), TEV1, |_| {}).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Ollama could not download `tev1:0.8b`: invalid model name."
        );
    }

    #[tokio::test]
    async fn a_stream_that_ends_without_success_fails() {
        let lines = [
            json!({"status": "pulling manifest"}),
            json!({"status": "pulling aaa", "digest": "sha256:aaa", "total": 1000, "completed": 1000}),
            json!({"status": "writing manifest"}),
        ];
        let server = ollama(&lines, &["decision"], loads()).await;
        let error = pull(&base(&server), TEV1, |_| {}).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Ollama stopped before it finished downloading `tev1:0.8b` (its last step was \
             `writing manifest`), so it was not installed. Install it again to resume the \
             download."
        );
        assert_eq!(
            requests_to(&server, "/api/show").await,
            0,
            "nothing checked"
        );
    }

    #[tokio::test]
    async fn a_pulled_model_without_the_decision_capability_is_not_installed() {
        let server = ollama(&two_layer_pull(), &["completion"], loads()).await;
        let error = pull(&base(&server), TEV1, |_| {}).await.unwrap_err();
        assert_eq!(
            error,
            DecisionError::Unsupported(
                "Ollama downloaded `tev1:0.8b`, but it does not answer decisions there (its \
                 capabilities lack `decision`). Update Ollama from https://ollama.com/download, \
                 then install it again."
                    .to_string()
            )
        );
    }

    #[tokio::test]
    async fn a_copy_ollama_will_not_load_is_not_installed() {
        let server = ollama(&two_layer_pull(), &["decision"], will_not_load(TEV1)).await;
        let error = pull(&base(&server), TEV1, |_| {}).await.unwrap_err();
        assert_eq!(
            error,
            DecisionError::Unsupported(super::super::system_one::ollama_load_refusal_sentence(
                TEV1
            ))
        );
    }

    #[tokio::test]
    async fn a_silent_stream_fails_at_the_idle_bound() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/pull"))
            .respond_with(streamed(&two_layer_pull()).set_delay(SILENT_SERVER_ANSWERS))
            .mount(&server)
            .await;
        let started = Instant::now();
        let error = pull_with_idle(&base(&server), TEV1, STALL_BOUND, |_| {})
            .await
            .unwrap_err();
        assert!(
            started.elapsed() < STALL_STOPS_WITHIN,
            "stopped at the bound, not when the server spoke: {:?}",
            started.elapsed()
        );
        // The words say which bound fired: the client's own stall bound reads
        // as "could not ask the Ollama", and a server that answered would get
        // as far as `/api/show`, which this one does not serve.
        assert_eq!(
            error.to_string(),
            "Ollama sent nothing for 200 ms while downloading `tev1:0.8b`, so the download was \
             stopped. Ollama keeps what it downloaded, so installing it again resumes it."
        );
    }

    /// The same bound between two chunks of a stream already flowing.
    #[tokio::test]
    async fn a_stall_mid_stream_fails_at_the_idle_bound() {
        let first = ndjson(&two_layer_pull()[..3]).into_bytes();
        let chunks =
            futures::stream::iter([Ok::<_, StreamBreak>(first)]).chain(futures::stream::pending());
        let started = Instant::now();
        let mut seen = 0;
        let error = follow_pull(TEV1, Box::pin(chunks), STALL_BOUND, &mut |_| seen += 1)
            .await
            .unwrap_err();
        assert_eq!(seen, 3, "what arrived was reported");
        assert!(
            started.elapsed() < STALL_STOPS_WITHIN,
            "{:?}",
            started.elapsed()
        );
        assert!(
            error
                .to_string()
                .starts_with("Ollama sent nothing for 200 ms"),
            "{error}"
        );
    }

    /// A layer's count can drop when Ollama fetches it again after a failed
    /// part. The progress keeps the largest count seen for each layer, so it
    /// never goes back, and a line without a count leaves the layer's alone.
    #[test]
    fn progress_never_goes_back_when_a_layer_is_fetched_again() {
        let mut tally = PullTally::default();
        tally.note(&json!({"digest": "sha256:aaa", "total": 1000, "completed": 600}));
        tally.note(&json!({"digest": "sha256:aaa", "total": 1000, "completed": 100}));
        tally.note(&json!({"digest": "sha256:aaa"}));
        tally.note(&json!({"digest": "sha256:bbb", "total": 200}));
        let progress = tally.progress("pulling bbb");
        assert_eq!(
            (progress.completed_bytes, progress.total_bytes),
            (600, 1200)
        );
        assert_eq!(progress.stage, PullStage::Downloading);
        assert_eq!(progress.status, "pulling bbb");
    }

    /// A line cut anywhere, a multi-byte character included, reads whole, and
    /// a last line with no newline still counts.
    #[tokio::test]
    async fn a_line_split_across_chunks_is_read_whole() {
        let mut lines = two_layer_pull();
        lines.insert(1, json!({"status": "pulling \u{2013} manifest"}));
        let whole = ndjson(&lines);
        let whole = whole.trim_end().as_bytes().to_vec();
        let chunks: Vec<Result<Vec<u8>, StreamBreak>> =
            whole.chunks(7).map(|piece| Ok(piece.to_vec())).collect();
        let mut seen = Vec::new();
        let done = follow_pull(
            TEV1,
            Box::pin(futures::stream::iter(chunks)),
            Duration::from_secs(5),
            &mut |p| seen.push(p),
        )
        .await
        .expect("the split stream reaches success");
        assert_eq!(done.status, "success");
        assert_eq!((done.completed_bytes, done.total_bytes), (1200, 1200));
        assert_eq!(seen.len(), 9);
        assert_eq!(seen[1].status, "pulling \u{2013} manifest");
    }

    /// A server that never ends a line is stopped rather than buffered
    /// without bound.
    #[tokio::test]
    async fn a_line_without_end_is_not_buffered_forever() {
        let endless = futures::stream::repeat_with(|| Ok::<_, StreamBreak>(vec![b'x'; 4096]));
        let error = follow_pull(TEV1, Box::pin(endless), Duration::from_secs(5), &mut |_| {})
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not Ollama's progress format"));
    }

    /// `::ffff:127.0.0.1` reaches this computer's loopback, but the
    /// self-hosted rule reads it lexically as another computer, as it does for
    /// a model's tier. The installer goes by that rule and sends nothing.
    #[tokio::test]
    async fn a_non_loopback_host_is_not_this_computer_and_sends_nothing() {
        let server = ollama(&two_layer_pull(), &["decision"], loads()).await;
        let port = server.address().port();
        let elsewhere = Url::parse(&format!("http://[::ffff:127.0.0.1]:{port}/")).unwrap();
        assert_eq!(
            crate::providers::self_hosted_tier(elsewhere.as_str()),
            ProviderTier::Public
        );
        assert_eq!(ollama_place(&elsewhere).await, OllamaPlace::NotThisComputer);

        let remote = elsewhere.clone();
        let installer = DecisionInstaller::new(move || Ok(remote.clone()));
        let refusal = installer.start(TEV1).await.unwrap_err();
        assert_eq!(refusal.kind, InstallRefusalKind::NotThisComputer);
        assert_eq!(refusal.kind.http_status(), 409);
        assert_eq!(
            refusal.message,
            format!(
                "Biorouter installs decision models only into an Ollama on this computer, and \
                 OLLAMA_HOST is {elsewhere}. A model there could not read \
                 a private chat. Pull it on that server with `ollama pull tev1:0.8b`, or point \
                 OLLAMA_HOST at this computer."
            )
        );
        assert_eq!(installer.job(), None);

        let snapshot = installer.snapshot().await;
        assert_eq!(snapshot.ollama.place, OllamaPlace::NotThisComputer);
        assert_eq!(
            snapshot.ollama.message.as_deref(),
            Some(refusal.message.as_str())
        );
        assert!(snapshot.offers.iter().all(|offer| !offer.installed));

        let sent = server.received_requests().await.unwrap_or_default();
        assert!(sent.is_empty(), "nothing was sent: {sent:?}");
    }

    #[tokio::test]
    async fn only_listed_models_install() {
        let server = ollama(&two_layer_pull(), &["decision"], loads()).await;
        let at = base(&server);
        let installer = DecisionInstaller::new(move || Ok(at.clone()));
        let refusal = installer.start("llama3.1:8b").await.unwrap_err();
        assert_eq!(refusal.kind, InstallRefusalKind::Unlisted);
        assert_eq!(refusal.kind.http_status(), 400);
        assert_eq!(
            refusal.message,
            "`llama3.1:8b` is not a decision model Biorouter installs. It installs tev1:0.8b and \
             clef-flash:latest; pull any other with `ollama pull llama3.1:8b`."
        );
        assert_eq!(installer.job(), None);
        assert!(server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());

        // The CLI's check refuses the same way, before anything is asked.
        let refusal = preflight(Ok(base(&server)), "tev1:4b").await.unwrap_err();
        assert_eq!(refusal.kind, InstallRefusalKind::Unlisted);

        // Ollama's untagged spelling names the listed model.
        assert_eq!(
            installable("clef-flash").map(|m| m.name),
            Some("clef-flash:latest")
        );
        assert_eq!(installable(TEV1).map(|m| m.is_default()), Some(true));
        assert_eq!(installable("tev1"), None);
    }

    #[tokio::test]
    #[serial_test::serial(ollama_discovery)]
    async fn one_install_at_a_time_and_the_same_model_joins() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version": "0.40.15"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/pull"))
            .and(body_json(json!({"model": TEV1, "stream": true})))
            .respond_with(streamed(&two_layer_pull()).set_delay(Duration::from_millis(600)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"capabilities": ["decision"]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(loads())
            .mount(&server)
            .await;
        let at = base(&server);
        let installer = DecisionInstaller::new(move || Ok(at.clone()));

        installer.start(TEV1).await.expect("starts");
        let job = installer.job().expect("a job");
        assert_eq!(job.model, TEV1);
        assert_eq!(job.state, InstallState::Downloading);

        let busy = installer.start("clef-flash:latest").await.unwrap_err();
        assert_eq!(busy.kind, InstallRefusalKind::Busy);
        assert_eq!(busy.kind.http_status(), 409);
        assert_eq!(
            busy.message,
            "tev1:0.8b is still downloading. Wait for it to finish, then try again."
        );
        installer.start(TEV1).await.expect("the same model joins");

        let done = finished(&installer).await;
        assert_eq!(done.state, InstallState::Installed, "{done:?}");
        assert_eq!((done.completed_bytes, done.total_bytes), (1200, 1200));
        assert_eq!(done.error, None);
        // The finished job is kept until the next start.
        assert_eq!(installer.job(), Some(done));
        server.verify().await;
    }

    #[tokio::test]
    async fn a_failed_install_reports_why_and_a_new_start_replaces_it() {
        let lines = [
            json!({"status": "pulling manifest"}),
            json!({"error": "pull model manifest: file does not exist"}),
        ];
        let server = ollama(&lines, &["decision"], loads()).await;
        let at = base(&server);
        let installer = DecisionInstaller::new(move || Ok(at.clone()));
        installer.start(TEV1).await.unwrap();
        let failed = finished(&installer).await;
        assert_eq!(failed.state, InstallState::Failed);
        assert_eq!(
            failed.error.as_deref(),
            Some(
                "Ollama could not download `tev1:0.8b`: pull model manifest: file does not exist."
            )
        );
        assert_eq!(failed.status.as_deref(), Some("pulling manifest"));

        // A failed install does not block the next one.
        installer.start("clef-flash").await.unwrap();
        assert_eq!(
            installer.job().map(|job| job.model),
            Some("clef-flash:latest".to_string())
        );
        finished(&installer).await;
    }

    #[tokio::test]
    async fn a_stopped_or_unreadable_ollama_refuses_with_its_sentence() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let stopped = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        assert_eq!(ollama_place(&stopped).await, OllamaPlace::NotRunning);
        let at = stopped.clone();
        let installer = DecisionInstaller::new(move || Ok(at.clone()));
        let refusal = installer.start(TEV1).await.unwrap_err();
        assert_eq!(refusal.kind, InstallRefusalKind::NotRunning);
        assert_eq!(refusal.kind.http_status(), 503);
        assert_eq!(
            refusal.message,
            format!(
                "Ollama is not running at http://127.0.0.1:{port}/. Install it from \
                 https://ollama.com/download or start it, then try again."
            )
        );
        let snapshot = installer.snapshot().await;
        assert_eq!(snapshot.ollama.place.state(), "not_running");
        assert_eq!(snapshot.ollama.message, Some(refusal.message));

        // Something that answers, but not as Ollama does, is not running.
        let other = MockServer::start().await;
        assert_eq!(ollama_place(&base(&other)).await, OllamaPlace::NotRunning);

        let unreadable =
            DecisionInstaller::new(|| Err("Invalid Ollama host: no host name".to_string()));
        let refusal = unreadable.start(TEV1).await.unwrap_err();
        assert_eq!(refusal.kind, InstallRefusalKind::InvalidHost);
        assert_eq!(refusal.kind.http_status(), 409);
        assert!(
            refusal
                .message
                .starts_with("OLLAMA_HOST could not be read (Invalid Ollama host: no host name)"),
            "{}",
            refusal.message
        );
        let snapshot = unreadable.snapshot().await;
        assert_eq!(snapshot.ollama.base_url, None);
        assert_eq!(snapshot.ollama.place.state(), "invalid_host");
        assert_eq!(unreadable.job(), None);
    }

    #[tokio::test]
    #[serial_test::serial(ollama_discovery)]
    async fn the_snapshot_reports_the_place_and_which_offers_are_installed() {
        let server = ollama(&two_layer_pull(), &["decision"], loads()).await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"name": TEV1, "model": TEV1, "digest": "snapshot-tev1"},
                {"name": "llama3.1:8b", "model": "llama3.1:8b", "digest": "snapshot-llama"}
            ]})))
            .mount(&server)
            .await;
        let at = base(&server);
        let installer = DecisionInstaller::new(move || Ok(at.clone()));
        let snapshot = installer.snapshot().await;
        assert_eq!(
            snapshot.ollama.place,
            OllamaPlace::Running {
                version: "0.40.15".to_string()
            }
        );
        assert_eq!(snapshot.ollama.place.version(), Some("0.40.15"));
        assert_eq!(snapshot.ollama.message, None);
        assert_eq!(snapshot.ollama.base_url, Some(base(&server)));
        let offers: Vec<(&str, bool, bool)> = snapshot
            .offers
            .iter()
            .map(|offer| (offer.model.name, offer.model.is_default(), offer.installed))
            .collect();
        assert_eq!(
            offers,
            [(TEV1, true, true), ("clef-flash:latest", false, false)]
        );
        assert_eq!(snapshot.job, None);
    }

    /// The `.rs` files under `roots` that name the installer, with the first
    /// name each one uses.
    fn installer_mentions(roots: &[std::path::PathBuf]) -> Vec<(std::path::PathBuf, &'static str)> {
        const NAMES: [&str; 3] = ["decision::install", "DecisionInstaller", "installer()"];
        let mut found = Vec::new();
        let mut pending: Vec<std::path::PathBuf> = roots.to_vec();
        while let Some(at) = pending.pop() {
            if at.is_dir() {
                let entries = std::fs::read_dir(&at).expect("a readable source directory");
                pending.extend(entries.map(|entry| entry.expect("a directory entry").path()));
            } else if at.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&at).expect("a readable source file");
                if let Some(name) = NAMES.iter().find(|name| text.contains(*name)) {
                    found.push((at, *name));
                }
            }
        }
        found.sort();
        found
    }

    /// A download of 1.5 to 12 GB is the person's choice, so no agent tool
    /// reaches the installer: nothing in the agent's own tools
    /// (`crates/biorouter/src/agents`) or the MCP servers
    /// (`crates/biorouter-mcp/src`) names it. Its doors are the install route
    /// and `biorouter models decision pull`, outside both.
    #[test]
    fn no_agent_tool_reaches_the_installer() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the crates directory")
            .to_path_buf();
        let roots = [
            crates.join("biorouter/src/agents"),
            crates.join("biorouter-mcp/src"),
        ];
        assert!(
            roots.iter().all(|root| root.is_dir()),
            "the scan reads real directories: {roots:?}"
        );
        let found = installer_mentions(&roots);
        assert!(
            found.is_empty(),
            "an agent tool must not install a decision model; ask the person instead (quote \
             decision::INSTALL_ADVICE): {found:?}"
        );

        // The scan finds a tool that does.
        let planted = tempfile::tempdir().unwrap();
        let tool = planted.path().join("tools").join("install_tool.rs");
        std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
        std::fs::write(
            &tool,
            "use crate::providers::decision::install;\nfn call() { install::installer(); }\n",
        )
        .unwrap();
        assert_eq!(
            installer_mentions(&[planted.path().to_path_buf()]),
            [(tool, "decision::install")]
        );
    }

    /// A model whose `/api/show` did not answer during a discovery is left out
    /// of it for up to a minute while Ollama lists the same copies. An install
    /// that verified it forgets that discovery, so it lists at once.
    #[tokio::test]
    #[serial_test::serial(ollama_discovery)]
    async fn discovery_is_forgotten_after_an_install() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"name": TEV1, "model": TEV1, "digest": "forgotten-tev1"}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .expect(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"capabilities": ["decision"]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .respond_with(loads())
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/pull"))
            .respond_with(streamed(&two_layer_pull()))
            .mount(&server)
            .await;
        let at = base(&server);
        assert!(discover_ollama(&at).await.is_empty(), "its show failed");

        pull(&at, TEV1, |_| {}).await.expect("installed");
        let listed = discover_ollama(&at).await;
        assert_eq!(
            listed.iter().map(|e| e.model.as_str()).collect::<Vec<_>>(),
            [TEV1]
        );
        server.verify().await;
    }
}
