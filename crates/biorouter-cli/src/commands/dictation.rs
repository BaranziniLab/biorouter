//! `biorouter dictation`: whether this computer can dictate, and its speech models
//! (PR-10 `status`; PR-11 `list`, `install`, `remove`, `use`, `urls`, `verify`, `bench`).
//!
//! One function builds the status ([`status_json`]), and `biorouter doctor` prints the
//! same object and the same lines, so the two never disagree. Reading the status never
//! starts a worker; `--verbose` does, to show its `hello`, unless the organization's
//! policy turns dictation off.
//!
//! The model commands take catalog ids, never URLs (DD-17). Installing is a person's
//! choice (DD-20): `install`, `remove` and `use` refuse in an agent's shell
//! (`BIOROUTER_TERMINAL` set) even with `--yes`, and without a terminal they need `--yes`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use biorouter::dictation::catalog::{SttCatalogEntry, CATALOG, MODEL_SOURCE};
use biorouter::dictation::hardware::{self, HostFacts, Suitability};
use biorouter::dictation::models::{
    self, InstallError, InstallOptions, Network, Phase, Progress, Verdict, WriteMode,
};
use biorouter::dictation::paths::{self, Inventory, Location, Roots};
use biorouter::dictation::service::{self, EngineState, EngineStatus};
use biorouter::dictation::{DictationError, WorkerProbe, MSG_NO_ENGINE};
use biorouter::managed::{Allowance, DictationPolicy, ManagedPolicy};
use console::{style, Color};
use serde_json::{json, Value};

use crate::commands::needs_terminal;

/// Brand warm tan-brown accent, as in `doctor`.
const ACCENT: Color = Color::Color256(137);

/// The schema of [`status_json`]. Later pull requests add keys; readers ignore keys they
/// do not know.
pub const STATUS_SCHEMA: u32 = 1;

/// Set in an agent's shell; dictation's model commands refuse there (DD-20).
pub const AGENT_SHELL_ENV: &str = "BIOROUTER_TERMINAL";

/// MSG-INSTALL-SHELL.
pub const MSG_INSTALL_SHELL: &str = "Install the speech model yourself: run this in a terminal, or use Settings > Models > Speech models.";
/// The same refusal for `remove` and `use`, which an agent may not run either (P-7).
pub const MSG_CHANGE_SHELL: &str = "Change speech models yourself: run this in a terminal, or use Settings > Models > Speech models.";

/// MSG-OFFLINE with `<folder>` filled in: the one sentence the daemon's install job prints
/// too, from one place.
pub use biorouter::dictation::msg_offline;

/// MSG-TURBO-CPU, from a recorded bench.
pub fn msg_turbo_cpu(seconds: f64, audio_seconds: f64) -> String {
    format!(
        "This model takes about {seconds:.1} s to transcribe {audio_seconds:.1} s of speech on this computer. Install it anyway?"
    )
}

/// MSG-SYSTEM-ROOT.
pub const MSG_SYSTEM_ROOT: &str = "Installing into the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation install --system`, or leave out --system to install for your account.";
/// MSG-SYSTEM-ROOT-REMOVE.
pub const MSG_SYSTEM_ROOT_REMOVE: &str = "Removing a model from the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation remove --system`.";
/// MSG-NO-SHARED-FOLDER.
pub const MSG_NO_SHARED_FOLDER: &str = "This computer has no shared model folder.";
/// MSG-NO-RECOMMENDED.
pub const MSG_NO_RECOMMENDED: &str = "This computer has no recommended speech model.";
/// MSG-NOTHING-INSTALLED.
pub const MSG_NOTHING_INSTALLED: &str = "No speech model is installed.";

/// MSG-DETECTOR-ONLY.
pub fn msg_detector_only(id: &str) -> String {
    format!("{id} is installed with every speech model; install a speech model instead.")
}

/// MSG-DETECTOR-NEEDED: `remove` given the voice detector's id while a speech model in that
/// folder needs it, the counterpart of MSG-DETECTOR-ONLY.
pub fn msg_detector_needed(id: &str, model: &str) -> String {
    format!("{model} needs {id}; it is removed with the last speech model that needs it.")
}

/// MSG-RETIRED.
pub fn msg_retired(id: &str, replacement: &str) -> String {
    format!("{id} is retired; Biorouter uses {replacement} in its place.")
}

/// MSG-ALREADY, where the copy that serves this account was found.
pub fn msg_already_found(id: &str, location: &str, path: &Path) -> String {
    format!(
        "{id} is already installed ({location}, {}).",
        path.display()
    )
}

/// MSG-ALREADY, for the folder an install or `--from` was writing.
pub fn msg_already_in(id: &str, folder: &Path) -> String {
    format!("{id} is already installed in {}.", folder.display())
}

/// MSG-INSTALLED.
pub fn msg_installed(id: &str, path: &Path) -> String {
    format!("Installed {id} ({}).", path.display())
}

/// MSG-CONFIRM-INSTALL.
pub fn msg_confirm_install(id: &str) -> String {
    format!("Download and install {id}?")
}

/// MSG-CONFIRM-TERMINAL, for `install`, `install --from` and `remove`.
pub fn msg_confirm_terminal(command: &str, rest: &str) -> String {
    match command {
        "remove" => format!(
            "Removing a speech model needs your confirmation at a terminal. To remove it without one, run: biorouter dictation remove --yes {rest}"
        ),
        _ => format!(
            "Installing a speech model needs your confirmation at a terminal. To install without one, run: biorouter dictation install --yes {rest}"
        ),
    }
}

/// MSG-TOO-SLOW, before a bench exists.
pub fn msg_too_slow(id: &str, cores: u32) -> String {
    format!(
        "{id} is too slow for dictation on a computer with {cores} cores; after you install it, `biorouter dictation bench` measures it. Install it anyway?"
    )
}

/// MSG-BELOW-MINIMUM.
pub fn msg_below_minimum(id: &str, gib: u32) -> String {
    format!(
        "{id} needs at least {gib} GB of memory, and this computer has less. Install it anyway?"
    )
}

/// MSG-WARMING.
pub fn msg_warming(id: &str) -> String {
    format!("Preparing {id} for its first use in the background.")
}

/// MSG-WARM-UP-SYSTEM: the shaders compile into each account's own Metal cache, so a
/// warm up run by root prepares nothing for anyone else.
pub fn msg_warm_up_system(id: &str) -> String {
    format!(
        "Each account's first dictation prepares {id} for that account. To prepare it now, run `biorouter dictation warm-up {id}` as that account."
    )
}

/// MSG-IMPORT-CONFIRM.
pub fn msg_import_confirm(from: &Path, folder: &Path) -> String {
    format!(
        "Install the speech model files in {} into {}?",
        from.display(),
        folder.display()
    )
}

/// MSG-IMPORT-REFUSED, with the policy's own sentence.
pub fn msg_import_refused(path: &Path, id: &str, reason: &str) -> String {
    format!(
        "{} is {id}, which was not installed: {reason}",
        path.display()
    )
}

/// MSG-IMPORT-UNKNOWN.
pub fn msg_import_unknown(path: &Path) -> String {
    format!(
        "{} is not a Biorouter model file, so it was not installed.",
        path.display()
    )
}

/// MSG-IMPORT-NONE.
pub fn msg_import_none(from: &Path) -> String {
    format!("No file in {} matches a Biorouter model.", from.display())
}

/// MSG-WAITING-LOCK.
pub fn msg_waiting_lock(id: &str) -> String {
    format!("Another Biorouter process is installing {id}; waiting for it to finish.")
}

/// MSG-REMOVE-CONFIRM.
pub fn msg_remove_confirm(id: &str, folder: &Path) -> String {
    format!("Remove {id} from {}?", folder.display())
}

/// MSG-REMOVED.
pub fn msg_removed(id: &str, folder: &Path) -> String {
    format!("Removed {id} from {}.", folder.display())
}

/// MSG-REMOVE-ELSEWHERE.
pub fn msg_remove_elsewhere(id: &str, location: &str, path: &Path, folder: &Path) -> String {
    format!(
        "{id} is installed in the {location} folder ({}), not in {}.",
        path.display(),
        folder.display()
    )
}

/// MSG-REMOVE-MISSING.
pub fn msg_remove_missing(id: &str) -> String {
    format!("{id} is not installed.")
}

/// MSG-USE.
pub fn msg_use(id: &str) -> String {
    format!("Dictation now uses {id}.")
}

/// MSG-USE-NOT-SAVED.
pub fn msg_use_not_saved(error: &str) -> String {
    format!("The choice could not be saved: {error}.")
}

/// MSG-VERIFY-FAILED, the closing line of a `verify` that found a bad copy.
pub fn msg_verify_failed(failures: usize) -> String {
    format!(
        "{failures} model file{} failed verification.",
        if failures == 1 { "" } else { "s" }
    )
}

/// Where an install may write: `--system` writes the computer's shared folder, otherwise
/// this account's. With managed `model_downloads: false` the managed folder is the only
/// source, so a copy anywhere else would never be read; the one install allowed then is
/// `--system` on a computer whose shared folder is that managed folder.
#[derive(Debug, PartialEq, Eq)]
pub enum TargetRefusal {
    /// MSG-NO-SHARED-FOLDER.
    NoSharedFolder,
    /// MSG-POLICY-DOWNLOADS: the target is not the managed folder.
    OnlyManaged,
}

pub fn install_target(roots: &Roots, system: bool) -> Result<PathBuf, TargetRefusal> {
    let target = roots
        .writable(system)
        .ok_or(TargetRefusal::NoSharedFolder)?;
    if roots.managed_only && roots.managed.as_deref() != Some(target) {
        return Err(TargetRefusal::OnlyManaged);
    }
    Ok(target.to_path_buf())
}

/// Everything the status shows, read once.
struct Status {
    engine: EngineStatus,
    policy: DictationPolicy,
    language: String,
    account: String,
    host: String,
    models_dir: String,
    inventory: Inventory,
    active: Option<&'static SttCatalogEntry>,
    install: Allowance,
    other_account: Option<String>,
    capture: CaptureStatus,
}

/// The terminal's own microphone (PR-25): whether `biorouter dictate` could record here.
/// Read from the capture child's probe, which lists devices without opening one, so no
/// permission prompt appears; never asked under a policy that turns live capture off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureStatus {
    pub available: bool,
    /// `not_compiled`, `policy`, `no_audio_system`, `no_device` or `probe_failed`.
    pub reason: Option<&'static str>,
    pub backend: Option<String>,
    pub devices: usize,
    /// The system default's name, for the text line.
    pub default_device: Option<String>,
}

impl CaptureStatus {
    fn gather(policy: &DictationPolicy) -> CaptureStatus {
        let off = |reason| CaptureStatus {
            available: false,
            reason: Some(reason),
            backend: None,
            devices: 0,
            default_device: None,
        };
        if !cfg!(feature = "dictation") {
            return off("not_compiled");
        }
        if !policy.enabled.is_allowed() || !policy.capture.is_allowed() {
            return off("policy");
        }
        #[cfg(feature = "dictation")]
        {
            match crate::dictation::capture::probe_blocking() {
                Some(probe) => CaptureStatus::of(&probe),
                None => off("probe_failed"),
            }
        }
        #[cfg(not(feature = "dictation"))]
        {
            off("not_compiled")
        }
    }

    #[cfg(feature = "dictation")]
    fn of(probe: &biorouter_dictation_capture::Probe) -> CaptureStatus {
        let reason = crate::dictation::environment::status_reason(probe);
        CaptureStatus {
            available: reason.is_none(),
            reason,
            backend: probe.backend.clone(),
            devices: probe.devices.len(),
            default_device: probe
                .devices
                .iter()
                .find(|d| d.default)
                .or(probe.devices.first())
                .map(|d| d.name.clone()),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "available": self.available,
            "reason": self.reason,
            "backend": self.backend,
            "devices": self.devices,
        })
    }

    /// The `Microphone:` line.
    fn line(&self) -> String {
        match (self.reason, &self.default_device) {
            (None, Some(device)) => {
                let more = self.devices.saturating_sub(1);
                format!(
                    "{device} ({}){}",
                    self.backend.as_deref().unwrap_or("audio"),
                    match more {
                        0 => String::new(),
                        1 => ", 1 more input".to_string(),
                        n => format!(", {n} more inputs"),
                    }
                )
            }
            (Some("not_compiled"), _) => "not part of this build".to_string(),
            (Some("policy"), _) => "turned off by your organization".to_string(),
            (Some("no_audio_system"), _) => "no audio system here".to_string(),
            (Some("no_device"), _) => "no input device".to_string(),
            _ => "could not be checked".to_string(),
        }
    }
}

impl Status {
    fn gather() -> Status {
        let policy = ManagedPolicy::load().dictation().clone();
        let mut inventory = paths::inventory(&Roots::current(), &CATALOG);
        let class = biorouter::dictation::service::host_class();
        let active = service::active_model(
            &mut inventory,
            &policy,
            service::configured_model().as_deref(),
            class,
        );
        let install = install_allowance(&policy, CATALOG.default_for(class).map(|e| e.id));
        let capture = CaptureStatus::gather(&policy);
        Status {
            capture,
            engine: service::service().status(),
            language: service::configured_language(),
            account: service::account(),
            host: service::host(),
            models_dir: service::models_dir().display().to_string(),
            other_account: other_account_service(),
            policy,
            inventory,
            active,
            install,
        }
    }

    fn to_json(&self) -> Value {
        let p = &self.policy;
        json!({
            "schema": STATUS_SCHEMA,
            "engine": self.engine,
            "models": models_json(&self.inventory, self.active),
            "active_model": self.active.map(|e| e.id),
            "language": self.language,
            "policy": {
                "managed": p.managed,
                "enabled": p.enabled.allowance(),
                "capture": p.capture.allowance(),
                "install": p.model_downloads.allowance(),
                "external_engine": p.external_engine.allowance(),
                "dictionary": p.dictionary.allowance(),
                "agent_writes": p.dictionary_agent_writes.allowance(),
                "allowed_models": p.allowed_models,
            },
            // Whether this computer may install its recommended model now; a terminal
            // process has no install running in the background, so `install_job` is the
            // daemon's to fill (PR-15).
            "install": self.install,
            "install_job": null,
            // The terminal's own microphone, from the capture child's probe.
            "capture": self.capture.to_json(),
            "account": self.account,
            "host": self.host,
            "models_dir": self.models_dir,
        })
    }

    /// The lines under the heading, as `status` and `doctor` print them.
    fn lines(&self) -> Vec<(&'static str, String)> {
        let e = &self.engine;
        let engine = match &e.build {
            Some(build) => format!(
                "whisper.cpp {}, ggml {} ({})",
                build.whisper, build.ggml, build.isa
            ),
            None => MSG_NO_ENGINE.to_string(),
        };
        let processor = if e.cpu_sentences.is_empty() {
            "supported".to_string()
        } else {
            e.cpu_sentences.join(" ")
        };
        let backend = match &e.backend {
            Some(backend) => format!(
                "{backend}, {} thread{}",
                e.threads,
                if e.threads == 1 { "" } else { "s" }
            ),
            None => "none".to_string(),
        };
        let worker = match e.state {
            EngineState::NoEngine => "none".to_string(),
            EngineState::UnsupportedCpu => "not started".to_string(),
            EngineState::Stopped => "stopped".to_string(),
            EngineState::Starting => "starting".to_string(),
            EngineState::Ready => match e.rss_bytes {
                Some(rss) => format!("ready (rss {})", human_bytes(rss)),
                None => "ready".to_string(),
            },
            EngineState::Error => "error".to_string(),
        };
        vec![
            ("Engine:", engine),
            ("Processor:", processor),
            ("Backend:", backend),
            ("Worker:", worker),
            ("Speech models:", models_line(&self.inventory, self.active)),
            (
                "Model folder:",
                format!("{} (account {})", self.models_dir, self.account),
            ),
            ("Policy:", policy_line(&self.policy)),
            ("Microphone:", self.capture.line()),
        ]
    }
}

/// `status.install`: whether this computer may install `id` now. The CPU gate, then the
/// managed policy's own precedence (MSG-POLICY, MSG-POLICY-DOWNLOADS, MSG-POLICY-MODELS).
fn install_allowance(policy: &DictationPolicy, id: Option<&str>) -> Allowance {
    if let Some(refusal) = service::spawn_refusal() {
        return Allowance::refused(refusal.to_string());
    }
    match id {
        Some(id) => policy.install(id),
        None => policy.model_downloads.allowance(),
    }
}

/// The `models` array of the status: every verified copy, where it is, and whether
/// dictation uses it.
fn models_json(inventory: &Inventory, active: Option<&SttCatalogEntry>) -> Value {
    Value::Array(
        inventory
            .found
            .iter()
            .map(|f| {
                json!({
                    "id": f.entry.id,
                    "kind": if f.entry.is_speech() { "speech" } else { "vad" },
                    "location": f.location,
                    "path": f.path.display().to_string(),
                    "bytes": f.entry.size_bytes,
                    "sha256": f.sha256,
                    "verified": true,
                    "superseded": f.superseded,
                    "active": active.is_some_and(|a| a.id == f.entry.id),
                })
            })
            .collect(),
    )
}

fn models_line(inventory: &Inventory, active: Option<&SttCatalogEntry>) -> String {
    let speech: Vec<String> = inventory
        .found
        .iter()
        .filter(|f| f.entry.is_speech())
        .map(|f| {
            let mut words = vec![f.location.as_str().to_string()];
            if active.is_some_and(|a| a.id == f.entry.id) {
                words.push("in use".into());
            }
            if !f
                .entry
                .requires
                .iter()
                .all(|id| inventory.get(id).is_some())
            {
                words.push("voice detector missing".into());
            }
            format!("{} ({})", f.entry.id, words.join(", "))
        })
        .collect();
    if speech.is_empty() {
        "none installed".to_string()
    } else {
        speech.join("; ")
    }
}

/// `not managed`, MSG-POLICY, or the narrowing an administrator set.
fn policy_line(p: &DictationPolicy) -> String {
    if !p.managed {
        return "not managed".to_string();
    }
    if let Some(reason) = p.enabled.reason() {
        return reason.to_string();
    }
    let reasons: Vec<&str> = [p.capture.reason(), p.model_downloads.reason()]
        .into_iter()
        .flatten()
        .collect();
    if reasons.is_empty() {
        "managed by your organization; dictation is allowed".to_string()
    } else {
        reasons.join(" ")
    }
}

/// `devices`: the input devices the terminal's microphone child can use, listed without
/// opening any (so no permission prompt appears), or why there are none here, in the words
/// `biorouter dictate` would use. `BIOROUTER_CLI_DICTATION_DEVICE` picks one by name.
pub async fn handle_devices(format: &str) -> Result<()> {
    let policy = ManagedPolicy::load().dictation().clone();
    for rule in [&policy.enabled, &policy.capture] {
        if let Some(reason) = rule.reason() {
            if format == "json" {
                let out = json!({"schema": STATUS_SCHEMA, "available": false, "reason": "policy", "backend": null, "devices": []});
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            return Err(refuse(reason));
        }
    }
    #[cfg(not(feature = "dictation"))]
    {
        if format == "json" {
            let out = json!({"schema": STATUS_SCHEMA, "available": false, "reason": "not_compiled", "backend": null, "devices": []});
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
        Err(refuse(MSG_NO_ENGINE))
    }
    #[cfg(feature = "dictation")]
    {
        use crate::dictation::environment::{self, CaptureVerdict, Environment};
        let probe = crate::dictation::capture::probe_once().await;
        let status = probe.as_ref().map_or(
            CaptureStatus {
                available: false,
                reason: Some("probe_failed"),
                backend: None,
                devices: 0,
                default_device: None,
            },
            CaptureStatus::of,
        );
        let devices: Vec<Value> = probe
            .iter()
            .flat_map(|p| p.devices.iter())
            .map(|d| json!({"name": d.name, "default": d.default}))
            .collect();
        if format == "json" {
            let out = json!({
                "schema": STATUS_SCHEMA,
                "available": status.available,
                "reason": status.reason,
                "backend": status.backend,
                "devices": devices,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
        println!(
            "  {} {}",
            style("▌").fg(ACCENT),
            style("Microphones").bold()
        );
        match &probe {
            Some(probe) if probe.error.is_none() => {
                for device in &probe.devices {
                    println!(
                        "    {}{}",
                        device.name,
                        if device.default { " (default)" } else { "" }
                    );
                }
                println!(
                    "    {:<15}{}",
                    "Backend:",
                    probe.backend.as_deref().unwrap_or("none")
                );
            }
            Some(probe) => {
                let env = Environment::sample();
                let reason = environment::no_capture_reason(&env, probe)
                    .unwrap_or(environment::NoCaptureReason::NoDevice);
                for line in CaptureVerdict::NoCapture(reason).sentences(&env) {
                    println!("    {line}");
                }
            }
            None => println!("    {}", crate::dictation::messages::MSG_NO_DEVICE),
        }
        Ok(())
    }
}

/// `1.0 GB`, `690 MB`.
fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{} MB", bytes / 1_000_000)
    } else {
        format!("{} KB", bytes.div_ceil(1_000))
    }
}

/// `264,477,561`.
fn grouped(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The schema 1 status object. `biorouter doctor --format json` carries the same object
/// as `dictation`.
pub fn status_json() -> Value {
    Status::gather().to_json()
}

/// The status lines without their heading, for `biorouter doctor`.
pub fn print_status_lines() {
    print_lines(&Status::gather());
}

fn print_lines(status: &Status) {
    for (label, value) in status.lines() {
        println!("    {label:<15}{value}");
    }
    if let Some(sentence) = &status.other_account {
        println!("    {}", style(sentence).yellow());
    }
}

pub async fn handle_status(format: &str, verbose: bool) -> Result<()> {
    let status = Status::gather();
    // Under `enabled: false` nothing is started, not even to say hello.
    let refusal = status.policy.enabled.reason().map(str::to_string);
    let probe = match (verbose, &refusal) {
        (true, None) => Some(service::service().probe_worker().await),
        _ => None,
    };
    if format == "json" {
        let mut out = status.to_json();
        if verbose {
            out["probe"] = probe_json(probe.as_ref(), refusal.as_deref());
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    println!("  {} {}", style("▌").fg(ACCENT), style("Dictation").bold());
    print_lines(&status);
    if verbose {
        println!();
        print_probe(probe.as_ref(), refusal.as_deref());
    }
    Ok(())
}

fn probe_json(probe: Option<&Result<WorkerProbe, DictationError>>, refusal: Option<&str>) -> Value {
    match (probe, refusal) {
        (_, Some(reason)) => {
            json!({"hello": null, "error": reason, "stderr": [], "exit_code": null})
        }
        (Some(Ok(probe)), None) => json!({
            "hello": probe.hello.as_ref().ok(),
            "error": probe.hello.as_ref().err().map(ToString::to_string),
            "stderr": probe.stderr,
            "exit_code": probe.exit_code,
        }),
        (Some(Err(e)), None) => {
            json!({"hello": null, "error": e.to_string(), "stderr": [], "exit_code": null})
        }
        (None, None) => Value::Null,
    }
}

fn print_probe(probe: Option<&Result<WorkerProbe, DictationError>>, refusal: Option<&str>) {
    let worker = |label: &str, value: &str| println!("    {label:<15}{value}");
    match (probe, refusal) {
        (_, Some(reason)) => worker("Worker hello:", reason),
        (Some(Err(e)), None) => worker("Worker hello:", &e.to_string()),
        (Some(Ok(probe)), None) => {
            match &probe.hello {
                Ok(hello) => {
                    let cpu = &hello.cpu;
                    worker(
                        "Worker hello:",
                        &format!(
                            "protocol {}, whisper.cpp {}, ggml {}, {}",
                            hello.protocol, hello.whisper, hello.ggml, hello.isa
                        ),
                    );
                    worker("Backends:", &hello.backends.join(", "));
                    worker(
                        "CPU features:",
                        &format!(
                            "avx2 {}, fma {}, f16c {}, bmi2 {}, dotprod {}",
                            yes_no(cpu.avx2),
                            yes_no(cpu.fma),
                            yes_no(cpu.f16c),
                            yes_no(cpu.bmi2),
                            yes_no(cpu.dotprod)
                        ),
                    );
                    worker("System info:", hello.system_info.trim());
                }
                Err(e) => worker("Worker hello:", &e.to_string()),
            }
            if let Some(code) = probe.exit_code {
                worker("Worker exit:", &code.to_string());
            }
            if probe.hello.is_err() || !probe.stderr.is_empty() {
                if probe.stderr.is_empty() {
                    worker("Worker stderr:", "nothing");
                } else {
                    worker("Worker stderr:", "");
                    for line in &probe.stderr {
                        println!("      {}", style(line).dim());
                    }
                }
            }
        }
        (None, None) => {}
    }
}

fn yes_no(present: bool) -> &'static str {
    if present {
        "yes"
    } else {
        "no"
    }
}

/// Whether this process runs in an agent's shell (DD-20). Any value but an empty one
/// counts: the shell sets `1`, and the stricter reading costs a person nothing.
fn in_agent_shell() -> bool {
    std::env::var_os(AGENT_SHELL_ENV).is_some_and(|value| !value.is_empty())
}

/// A refusal that exits 2 with its sentence alone: nothing was attempted.
fn refuse(sentence: impl AsRef<str>) -> anyhow::Error {
    needs_terminal::require(false, sentence.as_ref())
        .expect_err("require(false, _) always refuses")
        .into()
}

/// The catalog entry for `id`: a retired id names its replacement, and says so.
fn resolve_id(id: &str) -> Result<&'static SttCatalogEntry> {
    match CATALOG.resolve(id) {
        Some((entry, retired)) => {
            if retired {
                eprintln!("{}", msg_retired(id, entry.id));
            }
            Ok(entry)
        }
        None => Err(refuse(
            DictationError::UnknownModel(id.to_string()).to_string(),
        )),
    }
}

/// `list`: every catalog file with its size, languages and suitability, where it is
/// installed, and what sits in the model folders that no catalog file claims.
pub async fn handle_list(format: &str) -> Result<()> {
    let policy = ManagedPolicy::load().dictation().clone();
    let mut inventory = paths::inventory(&Roots::current(), &CATALOG);
    let host = HostFacts::current();
    let active = service::active_model(
        &mut inventory,
        &policy,
        service::configured_model().as_deref(),
        host.class,
    );
    let default = CATALOG.default_for(host.class);
    let rows: Vec<Value> = CATALOG
        .entries
        .iter()
        .map(|entry| {
            let bench = service::current_bench(entry.id);
            let found = inventory.get(entry.id);
            json!({
                "id": entry.id,
                "name": entry.display_name,
                "kind": if entry.is_speech() { "speech" } else { "vad" },
                "bytes": entry.size_bytes,
                "languages": entry.languages.as_str(),
                "suitability": hardware::rate(&CATALOG, entry, &host, bench.as_ref()),
                "recommended": default.is_some_and(|d| d.id == entry.id),
                "installed": found.is_some(),
                "location": found.map(|f| f.location),
                "path": found.map(|f| f.path.display().to_string()),
                "superseded": found.is_some_and(|f| f.superseded),
                "active": active.is_some_and(|a| a.id == entry.id),
                "allowed": policy.model(entry.id).allowed || !entry.is_speech(),
            })
        })
        .collect();
    let strays: Vec<Value> = inventory
        .strays
        .iter()
        .map(|s| {
            json!({
                "path": s.path.display().to_string(),
                "bytes": s.bytes,
                "retired": s.retired,
            })
        })
        .collect();
    if format == "json" {
        let out = json!({"schema": STATUS_SCHEMA, "models": rows, "unrecognized": strays});
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    println!(
        "  {} {}",
        style("▌").fg(ACCENT),
        style("Speech models").bold()
    );
    for (entry, row) in CATALOG.entries.iter().zip(&rows) {
        let mut notes = Vec::new();
        if row["recommended"] == true {
            notes.push("recommended for this computer".to_string());
        }
        match inventory.get(entry.id) {
            Some(found) => notes.push(format!(
                "installed ({}, {})",
                found.location.as_str(),
                found.path.display()
            )),
            None => notes.push("not installed".to_string()),
        }
        if row["active"] == true {
            notes.push("in use".to_string());
        }
        if row["superseded"] == true {
            notes.push("a newer revision exists".to_string());
        }
        if row["allowed"] == false {
            notes.push("not allowed by your organization".to_string());
        }
        let suitability = row["suitability"].as_str().unwrap_or("unknown");
        println!(
            "    {:<29} {:>8}  {:<12} {:<14} {}",
            entry.id,
            human_bytes(entry.size_bytes),
            entry.languages.as_str(),
            if entry.is_speech() {
                suitability
            } else {
                "required"
            },
            notes.join(", ")
        );
    }
    for stray in &inventory.strays {
        println!(
            "    {:<29} {:>8}  {}",
            stray.path.display(),
            human_bytes(stray.bytes),
            if stray.retired {
                "retired model"
            } else {
                "not a Biorouter model file"
            }
        );
    }
    Ok(())
}

/// What `install` was asked.
pub struct InstallArgs {
    pub model: Option<String>,
    pub from: Option<PathBuf>,
    pub system: bool,
    pub yes: bool,
}

/// `install`: the gates of `engine-and-models.md` "The installer, step by step", in
/// order, then the download (or `--from`), then a warm up in the background.
pub async fn handle_install(args: InstallArgs) -> Result<()> {
    if in_agent_shell() {
        return Err(refuse(MSG_INSTALL_SHELL));
    }
    if let Some(refusal) = service::spawn_refusal() {
        return Err(refuse(refusal.to_string()));
    }
    let policy = ManagedPolicy::load().dictation().clone();
    if !policy.enabled.is_allowed() {
        return Err(refuse(
            policy.enabled.allowance().reason.unwrap_or_default(),
        ));
    }
    if args.system && !models::running_as_administrator() {
        return Err(refuse(MSG_SYSTEM_ROOT));
    }
    let roots = Roots::current();
    let target = match install_target(&roots, args.system) {
        Ok(target) => target,
        Err(TargetRefusal::NoSharedFolder) => return Err(refuse(MSG_NO_SHARED_FOLDER)),
        Err(TargetRefusal::OnlyManaged) => {
            return Err(refuse(policy.model_downloads.reason().unwrap_or_default()))
        }
    };
    if args.system {
        if let Some(why) = models::system_folder_problem(&target) {
            return Err(refuse(InstallError::UnsafeFolder(why).to_string()));
        }
    }
    let account = service::account();
    let terminal = needs_terminal::prompt_can_run();
    if let Some(sentence) = other_account_service() {
        eprintln!("{}", style(sentence).yellow());
    }
    let mode = if args.system {
        WriteMode::System
    } else {
        WriteMode::User
    };
    if let Some(from) = &args.from {
        return install_from(&policy, from, &target, mode, terminal, args.yes);
    }

    let host = HostFacts::current();
    let entry = match &args.model {
        Some(id) => resolve_id(id)?,
        None => CATALOG
            .default_for(host.class)
            .ok_or_else(|| refuse(MSG_NO_RECOMMENDED))?,
    };
    if !entry.is_speech() {
        return Err(refuse(msg_detector_only(entry.id)));
    }
    let allowance = policy.install(entry.id);
    if !allowance.allowed {
        return Err(refuse(allowance.reason.unwrap_or_default()));
    }
    // A file verified in any folder serves this account; `--system` fills the shared
    // folder itself, whatever this account holds. An earlier revision serves until the
    // current one replaces it, so it counts as missing here (DD-23).
    let present = |file: &'static SttCatalogEntry| {
        if args.system {
            models::verify_writable(&target, file).is_current()
        } else {
            paths::find(&roots, file).is_some_and(|found| !found.superseded)
        }
    };
    let files: Vec<&'static SttCatalogEntry> = CATALOG
        .with_requirements(entry)
        .into_iter()
        .filter(|file| !present(file))
        .collect();
    if files.is_empty() {
        match paths::find(&roots, entry).filter(|_| !args.system) {
            Some(found) => println!(
                "{}",
                msg_already_found(entry.id, found.location.as_str(), &found.path)
            ),
            None => println!("{}", msg_already_in(entry.id, &target)),
        }
        return Ok(());
    }
    let bench = service::current_bench(entry.id);
    let warning = match hardware::rate(&CATALOG, entry, &host, bench.as_ref()) {
        Suitability::TooSlow => Some(match &bench {
            Some(record) => msg_turbo_cpu(
                record.elapsed_ms as f64 / 1000.0,
                record.audio_ms as f64 / 1000.0,
            ),
            None => msg_too_slow(entry.id, host.physical_cores.unwrap_or(0)),
        }),
        Suitability::BelowMinimum => Some(msg_below_minimum(entry.id, entry.min_total_ram_gib)),
        _ => None,
    };

    println!(
        "  {} {}",
        style("▌").fg(ACCENT),
        style("Install a speech model").bold()
    );
    for file in &files {
        println!(
            "    {:<29} {} B  sha256 {}",
            file.id,
            grouped(file.size_bytes),
            file.sha256
        );
    }
    println!("    {:<29} {MODEL_SOURCE}", "Source:");
    println!(
        "    {:<29} {} (account {account})",
        "Folder:",
        target.display()
    );
    if !terminal && !args.yes {
        return Err(refuse(msg_confirm_terminal("install", entry.id)));
    }
    if let Some(warning) = &warning {
        if !args.yes && !cliclack::confirm(warning).initial_value(false).interact()? {
            return Ok(());
        }
        if args.yes {
            eprintln!("{warning} (--yes)");
        }
    }
    if !args.yes
        && warning.is_none()
        && !cliclack::confirm(msg_confirm_install(entry.id))
            .initial_value(true)
            .interact()?
    {
        return Ok(());
    }

    let (tx, rx) = tokio::sync::watch::channel(Progress {
        id: String::new(),
        phase: Phase::Connecting,
        downloaded: 0,
        total: 0,
    });
    let mut options = InstallOptions::new(Network::production(), mode);
    options.progress = Some(tx);
    let report = tokio::spawn(report_progress(rx, std::io::stderr().is_terminal()));
    let started = std::time::Instant::now();
    // Ctrl-C ends the install at once with MSG-INSTALL-CANCELLED, wherever it waits (a
    // connection, a backoff, another process's lock). Dropping the install releases the
    // lock and keeps the partial, so the next install resumes from it.
    let interrupted = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let result = tokio::select! {
        result = models::install_files(&target, &files, &options) => result,
        () = interrupted => Err(InstallError::Cancelled),
    };
    drop(options);
    let _ = report.await;
    match result {
        Ok(outcomes) => {
            for outcome in &outcomes {
                println!("{}", msg_installed(outcome.id, &outcome.path));
            }
            tracing::info!(
                model = entry.id,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "dictation model installed"
            );
            // Only the process that placed the speech model warms it: a second install
            // that waited on the first one's lock, or one that fetched only a missing
            // voice detector, would start a second load of a model already prepared.
            if outcomes.iter().any(|o| o.id == entry.id && o.downloaded) {
                start_warm_up(entry, args.system);
            }
            Ok(())
        }
        Err(InstallError::Offline) => Err(anyhow!(msg_offline(&target))),
        Err(error) => Err(anyhow!(error.to_string())),
    }
}

/// `install --from <file|dir>` (DD-24).
fn install_from(
    policy: &DictationPolicy,
    from: &Path,
    target: &Path,
    mode: WriteMode,
    terminal: bool,
    yes: bool,
) -> Result<()> {
    if !terminal && !yes {
        return Err(refuse(msg_confirm_terminal(
            "install",
            &format!("--from {}", from.display()),
        )));
    }
    if !yes
        && !cliclack::confirm(msg_import_confirm(from, target))
            .initial_value(true)
            .interact()?
    {
        return Ok(());
    }
    let allow = |entry: &SttCatalogEntry| !entry.is_speech() || policy.model(entry.id).allowed;
    let space: models::SpaceProbe =
        std::sync::Arc::new(|path: &Path| models::available_space(path));
    // A read error names the file; a write error is MSG-DISK on a full volume, MSG-HASH
    // when the copy changed, and otherwise says the model could not be written.
    let report = models::import_from(&CATALOG, target, from, mode, &allow, &space)
        .map_err(|e| anyhow!(e.to_string()))?;
    for (id, path) in &report.installed {
        println!("{}", msg_installed(id, path));
    }
    for (id, path) in &report.refused {
        eprintln!(
            "{}",
            msg_import_refused(path, id, &policy.model(id).reason.unwrap_or_default())
        );
    }
    for id in &report.already {
        println!("{}", msg_already_in(id, target));
    }
    for path in &report.unrecognized {
        println!("{}", msg_import_unknown(path));
    }
    if report.installed.is_empty() && report.already.is_empty() && report.refused.is_empty() {
        return Err(anyhow!(msg_import_none(from)));
    }
    if let Some((id, _)) = report
        .installed
        .iter()
        .find(|(id, _)| CATALOG.entry(id).is_some_and(SttCatalogEntry::is_speech))
    {
        if let Some(entry) = CATALOG.entry(id) {
            start_warm_up(entry, mode == WriteMode::System);
        }
    }
    Ok(())
}

/// Shows an install's progress: a bar on a terminal, a line per phase otherwise (the
/// `PullReport` idiom of `biorouter models local pull`).
async fn report_progress(mut rx: tokio::sync::watch::Receiver<Progress>, terminal: bool) {
    let mut bar: Option<(String, cliclack::ProgressBar)> = None;
    let mut last = (String::new(), Phase::Connecting);
    while rx.changed().await.is_ok() {
        let progress = rx.borrow_and_update().clone();
        let phase_changed = last != (progress.id.clone(), progress.phase);
        if progress.phase == Phase::Waiting && phase_changed {
            eprintln!("{}", msg_waiting_lock(&progress.id));
        }
        if terminal && progress.phase == Phase::Downloading {
            if bar.as_ref().is_none_or(|(id, _)| *id != progress.id) {
                if let Some((_, old)) = bar.take() {
                    old.stop("");
                }
                let new = cliclack::progress_bar(progress.total).with_download_template();
                new.start(format!("Downloading {}", progress.id));
                bar = Some((progress.id.clone(), new));
            }
            if let Some((_, bar)) = &bar {
                bar.set_position(progress.downloaded);
            }
        } else if !terminal && phase_changed && progress.phase == Phase::Downloading {
            eprintln!("Downloading {}…", progress.id);
        }
        if progress.phase == Phase::Verifying && phase_changed {
            if let Some((_, done)) = bar.take() {
                done.stop(format!("Downloaded {}", progress.id));
            }
            if !terminal {
                eprintln!("Checking {} against its pin…", progress.id);
            }
        }
        last = (progress.id.clone(), progress.phase);
    }
    if let Some((_, bar)) = bar.take() {
        bar.stop("");
    }
}

/// Starts `biorouter dictation warm-up <id>` in the background and returns at once
/// (PR-11 latency): on Metal the first load after an install compiles the model's
/// shaders, 3.0 to 4.2 s [M], and the per user Metal cache keeps them for every later
/// process. On a CPU host there is nothing to compile, so nothing starts. After a
/// `--system` install, or any install run as root or Administrator, nothing starts
/// either: the cache belongs to the account that compiles, so a warm up run as root would
/// prepare nothing for the people who dictate (MSG-WARM-UP-SYSTEM says so instead).
fn start_warm_up(entry: &SttCatalogEntry, system: bool) {
    if !entry.is_speech() || service::default_backend() != "metal" {
        return;
    }
    if system || models::running_as_administrator() {
        println!("{}", msg_warm_up_system(entry.id));
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut command = std::process::Command::new(exe);
    command
        .args(["dictation", "warm-up", entry.id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    if command.spawn().is_ok() {
        println!("{}", msg_warming(entry.id));
    }
}

/// The hidden `warm-up` command: one load and one decode, then the worker stops. Prints
/// its timings as one JSON line, which `install` discards.
pub async fn handle_warm_up(id: &str) -> Result<()> {
    let entry = resolve_id(id)?;
    let result = service::service().warm_up(entry.id).await;
    service::service().unload().await;
    let warm_up = result.map_err(|e| anyhow!(e.to_string()))?;
    println!("{}", serde_json::to_string(&warm_up)?);
    Ok(())
}

/// `remove`: deletes this account's copy (or the system's with `--system`); removing
/// the last speech model in that folder removes its voice detector too. The detector
/// stays while a speech model in that folder is there or is being installed by another
/// process, and `remove` given the detector's own id refuses then (MSG-DETECTOR-NEEDED).
/// Never refused by policy (DD-23). It refuses while another process installs the same
/// model, whose lock it takes first. This process runs no worker, so there is nothing of
/// its own to unload; the daemon's remove route (PR-15) unloads its worker before it
/// deletes.
pub async fn handle_remove(id: &str, system: bool, yes: bool) -> Result<()> {
    if in_agent_shell() {
        return Err(refuse(MSG_CHANGE_SHELL));
    }
    let entry = resolve_id(id)?;
    if system && !models::running_as_administrator() {
        return Err(refuse(MSG_SYSTEM_ROOT_REMOVE));
    }
    let roots = Roots::current();
    let target = roots
        .writable(system)
        .ok_or_else(|| refuse(MSG_NO_SHARED_FOLDER))?
        .to_path_buf();
    if system {
        if let Some(why) = models::system_folder_problem(&target) {
            return Err(refuse(InstallError::UnsafeFolder(why).to_string()));
        }
    }
    let path = models::model_path(&target, entry);
    if !path.exists() {
        return Err(match paths::find(&roots, entry) {
            Some(found) => anyhow!(msg_remove_elsewhere(
                entry.id,
                found.location.as_str(),
                &found.path,
                &target
            )),
            None => anyhow!(msg_remove_missing(entry.id)),
        });
    }
    // A speech model in this folder that is there, or that another process is installing
    // now: such an install planned its files while the detector was there and does not
    // fetch it again, so taking the detector would leave the new model unable to load.
    let needs_detector = |vad: &SttCatalogEntry| {
        CATALOG.requiring(vad.id).find(|other| {
            other.id != entry.id
                && (models::model_path(&target, other).exists()
                    || models::install_running(&target, other))
        })
    };
    if !entry.is_speech() {
        if let Some(model) = needs_detector(entry) {
            return Err(refuse(msg_detector_needed(entry.id, model.id)));
        }
    }
    if !needs_terminal::prompt_can_run() && !yes {
        return Err(refuse(msg_confirm_terminal("remove", entry.id)));
    }
    if !yes
        && !cliclack::confirm(msg_remove_confirm(entry.id, &target))
            .initial_value(true)
            .interact()?
    {
        return Ok(());
    }
    let mut removed = vec![entry.id];
    models::remove(&target, entry).map_err(|e| anyhow!(e.to_string()))?;
    if entry.is_speech() {
        for vad in entry.requires.iter().filter_map(|id| CATALOG.entry(id)) {
            if needs_detector(vad).is_none()
                && models::remove(&target, vad).map_err(|e| anyhow!(e.to_string()))?
            {
                removed.push(vad.id);
            }
        }
    }
    for id in removed {
        println!("{}", msg_removed(id, &target));
    }
    Ok(())
}

/// `use`: writes `BIOROUTER_DICTATION_MODEL` after the same checks a load makes.
pub async fn handle_use(id: &str) -> Result<()> {
    if in_agent_shell() {
        return Err(refuse(MSG_CHANGE_SHELL));
    }
    let entry = resolve_id(id)?;
    if !entry.is_speech() {
        return Err(refuse(
            DictationError::UnknownModel(id.to_string()).to_string(),
        ));
    }
    service::check_language(entry, &service::configured_language())
        .map_err(|e| refuse(e.to_string()))?;
    if let Err(error) = service::service().resolve(Some(entry.id)).await {
        return Err(refuse(error.to_string()));
    }
    biorouter::config::Config::global()
        .set_param(service::MODEL_KEY, entry.id)
        .map_err(|e| anyhow!(msg_use_not_saved(&e.to_string())))?;
    println!("{}", msg_use(entry.id));
    Ok(())
}

/// `urls`: what a person downloads on another computer for an air gapped one (DD-24).
pub fn handle_urls(id: Option<&str>) -> Result<()> {
    let entries: Vec<&'static SttCatalogEntry> = match id {
        Some(id) => vec![resolve_id(id)?],
        None => match CATALOG.default_for(service::host_class()) {
            Some(entry) => vec![entry],
            None => CATALOG.speech_models().collect(),
        },
    };
    let mut printed: Vec<&str> = Vec::new();
    for entry in entries {
        // The model first, then what it requires, as `engine-and-models.md` prints them.
        let mut files = CATALOG.with_requirements(entry);
        files.rotate_right(1);
        for file in files {
            if printed.contains(&file.id) {
                continue;
            }
            printed.push(file.id);
            println!(
                "{}  {} B  sha256 {}",
                file.id,
                grouped(file.size_bytes),
                file.sha256
            );
            for url in file.urls {
                println!("  {url}");
            }
        }
    }
    Ok(())
}

/// `verify`: hashes every copy in every folder again. A failing copy in this account's
/// folder is deleted; the others are reported and left alone. Exit 1 on any failure.
pub async fn handle_verify() -> Result<()> {
    let roots = Roots::current();
    let results = tokio::task::spawn_blocking(move || {
        let mut lines = Vec::new();
        for (location, root) in roots.ordered() {
            for entry in CATALOG.entries {
                let verdicts = if location == Location::User {
                    vec![models::verify_writable_fully(root, entry)]
                } else {
                    vec![
                        models::verify_readonly_fully(&models::model_path(root, entry), entry),
                        models::verify_readonly_fully(&models::flat_path(root, entry), entry),
                    ]
                };
                for verdict in verdicts {
                    lines.push((location, entry.id, verdict));
                }
            }
        }
        lines
    })
    .await?;
    let mut failures = 0;
    let mut checked = 0;
    for (location, id, verdict) in results {
        match verdict {
            Verdict::Missing => {}
            Verdict::Verified {
                path, superseded, ..
            } => {
                checked += 1;
                println!(
                    "ok      {id} ({}, {}){}",
                    location.as_str(),
                    path.display(),
                    if superseded {
                        ", an earlier revision"
                    } else {
                        ""
                    }
                );
            }
            Verdict::Mismatch { path, deleted } => {
                failures += 1;
                println!(
                    "FAILED  {id} ({}, {}): the file does not match its pin{}",
                    location.as_str(),
                    path.display(),
                    if deleted { ", so it was deleted" } else { "" }
                );
            }
            Verdict::Unreadable { path, error } => {
                failures += 1;
                println!(
                    "FAILED  {id} ({}, {}): {error}",
                    location.as_str(),
                    path.display()
                );
            }
        }
    }
    if failures > 0 {
        return Err(anyhow!(msg_verify_failed(failures)));
    }
    if checked == 0 {
        println!("{MSG_NOTHING_INSTALLED}");
    }
    Ok(())
}

/// `bench`: the real time factor of the model in use on this computer (DD-22).
pub async fn handle_bench(format: &str) -> Result<()> {
    if let Some(refusal) = service::spawn_refusal() {
        return Err(refuse(refusal.to_string()));
    }
    let result = service::service().bench(None).await;
    service::service().unload().await;
    let (model, record) = result.map_err(|e| match e {
        DictationError::NoModel { .. } | DictationError::Policy(_) => refuse(e.to_string()),
        other => anyhow!(other.to_string()),
    })?;
    let host = HostFacts::current();
    let suitability = CATALOG
        .entry(&model)
        .map(|entry| hardware::rate(&CATALOG, entry, &host, Some(&record)));
    if format == "json" {
        let out = json!({
            "model": model,
            "rtf": record.rtf,
            "elapsed_ms": record.elapsed_ms,
            "audio_ms": record.audio_ms,
            "threads": record.threads,
            "backend": record.backend,
            "suitability": suitability,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    println!(
        "{model} on {}, {} threads: {:.1} s of speech in {:.2} s (real time factor {:.2}, {}).",
        record.backend,
        record.threads,
        record.audio_ms as f64 / 1000.0,
        record.elapsed_ms as f64 / 1000.0,
        record.rtf,
        suitability.map_or("unknown", Suitability::as_str)
    );
    Ok(())
}

/// MSG-OTHER-ACCOUNT: a `biorouterd` that another account runs, found by a scan of
/// `/proc` (Linux only; a `/proc` mounted with `hidepid` hides it). Models installed
/// under this account are not visible to that daemon.
fn other_account_service() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        let owner = std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .find_map(|item| {
                let name = item.file_name();
                let pid = name.to_str()?;
                if !pid.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let comm = std::fs::read_to_string(item.path().join("comm")).ok()?;
                if comm.trim() != "biorouterd" {
                    return None;
                }
                let uid = item.metadata().ok()?.uid();
                (uid != me).then_some(uid)
            })?;
        let service_user = user_name(owner).unwrap_or_else(|| owner.to_string());
        let user = service::account();
        Some(format!(
            "The Biorouter service on this computer runs as `{service_user}`; models you install as `{user}` are not visible to it. Run: `sudo -u {service_user} -H biorouter dictation install`"
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn user_name(uid: u32) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let id: u32 = fields.nth(1)?.parse().ok()?;
        (id == uid).then(|| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::managed::Rule;

    /// The keys and types of schema 1 (section 14.2 of the M1 design), as
    /// `scripts/check-dictation-status-json.py` checks them.
    #[test]
    fn the_status_is_schema_1() {
        let json = status_json();
        assert_eq!(json["schema"], 1);
        let engine = &json["engine"];
        for key in [
            "compiled",
            "state",
            "backend",
            "device",
            "threads",
            "missing_cpu_features",
            "rss_bytes",
            "build",
        ] {
            assert!(
                engine.get(key).is_some(),
                "engine.{key} is missing: {engine}"
            );
        }
        assert!(engine.get("cpu_sentences").is_none());
        assert_eq!(json["models"], json!([]));
        assert_eq!(json["active_model"], Value::Null);
        assert!(json["install"]["allowed"].is_boolean());
        assert!(json["install"].get("reason").is_some());
        assert_eq!(json["install_job"], Value::Null);
        assert!(json["language"].is_string());
        for key in [
            "enabled",
            "capture",
            "install",
            "external_engine",
            "dictionary",
            "agent_writes",
        ] {
            let allowance = &json["policy"][key];
            assert!(
                allowance["allowed"].is_boolean(),
                "policy.{key}: {allowance}"
            );
            assert!(
                allowance.get("reason").is_some(),
                "policy.{key}: {allowance}"
            );
        }
        assert!(json["policy"]["managed"].is_boolean());
        assert!(json["policy"].get("allowed_models").is_some());
        // The terminal's microphone, as this machine's probe found it (PR-25): available
        // with no reason, or not with one of the known reasons.
        let capture = &json["capture"];
        let available = capture["available"].as_bool().expect("capture.available");
        match capture["reason"].as_str() {
            None => assert!(available, "{capture}"),
            Some(reason) => {
                assert!(!available, "{capture}");
                assert!(
                    [
                        "not_compiled",
                        "policy",
                        "no_audio_system",
                        "no_device",
                        "probe_failed"
                    ]
                    .contains(&reason),
                    "{capture}"
                );
            }
        }
        for key in ["account", "host", "models_dir"] {
            assert!(json[key].is_string(), "{key}");
        }
        assert!(json["models_dir"]
            .as_str()
            .is_some_and(|dir| dir.ends_with("models")));
    }

    #[test]
    fn the_policy_line_names_what_is_narrowed() {
        assert_eq!(policy_line(&DictationPolicy::unmanaged()), "not managed");
        assert_eq!(
            policy_line(&DictationPolicy::off()),
            biorouter::managed::MANAGED_DICTATION_OFF
        );
        let capture_off = DictationPolicy {
            managed: true,
            capture: Rule::Forbidden(biorouter::managed::MANAGED_DICTATION_CAPTURE_OFF.into()),
            ..DictationPolicy::unmanaged()
        };
        assert_eq!(
            policy_line(&capture_off),
            biorouter::managed::MANAGED_DICTATION_CAPTURE_OFF
        );
        let allowed = DictationPolicy {
            managed: true,
            ..DictationPolicy::unmanaged()
        };
        assert_eq!(
            policy_line(&allowed),
            "managed by your organization; dictation is allowed"
        );
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(human_bytes(1_010_000_000), "1.0 GB");
        assert_eq!(human_bytes(690_000_000), "690 MB");
        assert_eq!(human_bytes(885_098), "886 KB");
        assert_eq!(grouped(264_477_561), "264,477,561");
        assert_eq!(grouped(885_098), "885,098");
        assert_eq!(grouped(12), "12");
    }

    /// MSG-OFFLINE, MSG-TURBO-CPU and MSG-INSTALL-SHELL, word for word.
    #[test]
    fn the_install_sentences_are_the_catalogs() {
        assert_eq!(
            msg_offline(Path::new("/models")),
            "This computer cannot reach Biorouter's model repository. On a computer that can, run `biorouter dictation urls` and download the files it lists, copy them here, then run `biorouter dictation install --from <file>` (or put them in /models and choose Rescan)."
        );
        assert_eq!(
            msg_turbo_cpu(4.5, 5.5),
            "This model takes about 4.5 s to transcribe 5.5 s of speech on this computer. Install it anyway?"
        );
        assert_eq!(
            MSG_INSTALL_SHELL,
            "Install the speech model yourself: run this in a terminal, or use Settings > Models > Speech models."
        );
    }

    /// With downloads off, only the managed folder is read, so the one install allowed is
    /// `--system` on a computer whose shared folder is that managed folder (the plan's
    /// "`model_downloads: false` refuses while `--from` into `models_dir` works").
    #[test]
    fn with_downloads_off_only_the_managed_folder_takes_an_install() {
        let shared = PathBuf::from("/srv/models");
        let off = DictationPolicy {
            managed: true,
            models_dir: Some(shared.clone()),
            model_downloads: Rule::Forbidden("downloads off".into()),
            ..DictationPolicy::unmanaged()
        };
        let user = PathBuf::from("/home/ana/models");
        let roots = Roots::from_parts(&off, None, user.clone(), Some(shared.clone()));
        assert_eq!(install_target(&roots, true), Ok(shared.clone()));
        assert_eq!(
            install_target(&roots, false),
            Err(TargetRefusal::OnlyManaged)
        );
        let elsewhere = Roots::from_parts(&off, None, user.clone(), Some("/var/lib/m".into()));
        assert_eq!(
            install_target(&elsewhere, true),
            Err(TargetRefusal::OnlyManaged)
        );
        let none = Roots::from_parts(&DictationPolicy::unmanaged(), None, user.clone(), None);
        assert_eq!(
            install_target(&none, true),
            Err(TargetRefusal::NoSharedFolder)
        );
        assert_eq!(install_target(&none, false), Ok(user));
    }

    /// The model commands' sentences, word for word as the message catalog has them.
    #[test]
    fn the_model_command_sentences_are_the_catalogs() {
        let p = Path::new("/m/x.bin");
        let folder = Path::new("/m");
        let cases: Vec<(String, &str)> = vec![
            (MSG_SYSTEM_ROOT.into(), "Installing into the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation install --system`, or leave out --system to install for your account."),
            (MSG_SYSTEM_ROOT_REMOVE.into(), "Removing a model from the computer's shared folder needs root or Administrator. Run `sudo biorouter dictation remove --system`."),
            (MSG_NO_SHARED_FOLDER.into(), "This computer has no shared model folder."),
            (MSG_NO_RECOMMENDED.into(), "This computer has no recommended speech model."),
            (MSG_NOTHING_INSTALLED.into(), "No speech model is installed."),
            (MSG_CHANGE_SHELL.into(), "Change speech models yourself: run this in a terminal, or use Settings > Models > Speech models."),
            (msg_detector_only("v"), "v is installed with every speech model; install a speech model instead."),
            (msg_detector_needed("v", "m"), "m needs v; it is removed with the last speech model that needs it."),
            (msg_retired("old", "new"), "old is retired; Biorouter uses new in its place."),
            (msg_already_found("m", "user", p), "m is already installed (user, /m/x.bin)."),
            (msg_already_in("m", folder), "m is already installed in /m."),
            (msg_installed("m", p), "Installed m (/m/x.bin)."),
            (msg_confirm_install("m"), "Download and install m?"),
            (msg_confirm_terminal("install", "m"), "Installing a speech model needs your confirmation at a terminal. To install without one, run: biorouter dictation install --yes m"),
            (msg_confirm_terminal("remove", "m"), "Removing a speech model needs your confirmation at a terminal. To remove it without one, run: biorouter dictation remove --yes m"),
            (msg_too_slow("m", 4), "m is too slow for dictation on a computer with 4 cores; after you install it, `biorouter dictation bench` measures it. Install it anyway?"),
            (msg_below_minimum("m", 8), "m needs at least 8 GB of memory, and this computer has less. Install it anyway?"),
            (msg_warming("m"), "Preparing m for its first use in the background."),
            (msg_warm_up_system("m"), "Each account's first dictation prepares m for that account. To prepare it now, run `biorouter dictation warm-up m` as that account."),
            (msg_import_confirm(p, folder), "Install the speech model files in /m/x.bin into /m?"),
            (msg_import_refused(p, "m", "no."), "/m/x.bin is m, which was not installed: no."),
            (msg_import_unknown(p), "/m/x.bin is not a Biorouter model file, so it was not installed."),
            (msg_import_none(folder), "No file in /m matches a Biorouter model."),
            (msg_waiting_lock("m"), "Another Biorouter process is installing m; waiting for it to finish."),
            (msg_remove_confirm("m", folder), "Remove m from /m?"),
            (msg_removed("m", folder), "Removed m from /m."),
            (msg_remove_elsewhere("m", "system", p, folder), "m is installed in the system folder (/m/x.bin), not in /m."),
            (msg_remove_missing("m"), "m is not installed."),
            (msg_use("m"), "Dictation now uses m."),
            (msg_use_not_saved("disk full"), "The choice could not be saved: disk full."),
            (msg_verify_failed(1), "1 model file failed verification."),
            (msg_verify_failed(2), "2 model files failed verification."),
        ];
        for (sentence, catalog) in cases {
            assert_eq!(sentence, catalog);
        }
    }

    /// The installer's own sentences, as the CLI shows them (MSG-HTTP to MSG-REMOVE-FAILED).
    #[test]
    fn the_installer_sentences_are_the_catalogs() {
        use biorouter::dictation::models::{ImportError, RemoveError};
        let io = || std::io::Error::other("no space");
        let cases: Vec<(String, &str)> = vec![
            (InstallError::Http { status: 451 }.to_string(), "The model repository answered HTTP 451."),
            (InstallError::Tls { host: "github.com".into() }.to_string(), "The secure connection to github.com could not be verified, so nothing was downloaded."),
            (InstallError::Redirect("more than 5 redirects".into()).to_string(), "The download was refused: more than 5 redirects."),
            (InstallError::KeptFailing.to_string(), "The download kept failing, so it stopped. Run the install again to continue where it stopped."),
            (InstallError::UnsafeFolder("/var/lib/biorouter is not owned by root".into()).to_string(), "Nothing was installed in the shared model folder, because /var/lib/biorouter is not owned by root. Make each folder on that path a real folder that only root or an Administrator can change, then try again."),
            (InstallError::Cancelled.to_string(), "The install was cancelled."),
            (InstallError::Io(io()).to_string(), "The model could not be written: no space."),
            (InstallError::Internal("a catalog URL does not parse".into()).to_string(), "The install failed: a catalog URL does not parse."),
            (ImportError::Read { path: "/m/x.bin".into(), error: io() }.to_string(), "/m/x.bin could not be read: no space."),
            (RemoveError::Busy { id: "m".into() }.to_string(), "m is being installed by another Biorouter process; try again when it finishes."),
            (RemoveError::Io { id: "m".into(), error: io() }.to_string(), "m could not be removed: no space."),
        ];
        for (sentence, catalog) in cases {
            assert_eq!(sentence, catalog);
        }
    }

    /// A refusal reaches `main` as `NeedsTerminal`, so it exits 2 with its sentence alone.
    #[test]
    fn refusals_exit_2() {
        let error = refuse("no");
        assert!(error
            .downcast_ref::<crate::commands::needs_terminal::NeedsTerminal>()
            .is_some());
        assert_eq!(error.to_string(), "no");
    }

    #[test]
    fn every_line_has_a_label_that_fits() {
        let lines = Status::gather().lines();
        assert_eq!(lines.len(), 8);
        assert!(lines.iter().all(|(label, _)| label.len() < 15));
    }

    #[test]
    fn a_refused_probe_spawns_nothing_and_says_why() {
        let json = probe_json(None, Some(biorouter::managed::MANAGED_DICTATION_OFF));
        assert_eq!(json["error"], biorouter::managed::MANAGED_DICTATION_OFF);
        assert_eq!(json["hello"], Value::Null);
    }
}
