//! `biorouter dictation status`: whether this computer can dictate, and why (PR-10).
//!
//! One function builds the status ([`status_json`]), and `biorouter doctor` prints the
//! same object and the same lines, so the two never disagree. Reading the status never
//! starts a worker; `--verbose` does, to show its `hello`, unless the organization's
//! policy turns dictation off. Only `status` exists until the model commands arrive
//! with PR-11 (DD-94: no command appears before it works).

use anyhow::Result;
use biorouter::dictation::service::{self, EngineState, EngineStatus};
use biorouter::dictation::{DictationError, WorkerProbe, MSG_NO_ENGINE};
use biorouter::managed::{DictationPolicy, ManagedPolicy};
use console::{style, Color};
use serde_json::{json, Value};

/// Brand warm tan-brown accent, as in `doctor`.
const ACCENT: Color = Color::Color256(137);

/// The schema of [`status_json`]. Later pull requests add keys; readers ignore keys they
/// do not know.
pub const STATUS_SCHEMA: u32 = 1;

/// Everything the status shows, read once.
struct Status {
    engine: EngineStatus,
    policy: DictationPolicy,
    language: String,
    account: String,
    host: String,
    models_dir: String,
}

impl Status {
    fn gather() -> Status {
        Status {
            engine: service::service().status(),
            policy: ManagedPolicy::load().dictation().clone(),
            language: service::configured_language(),
            account: service::account(),
            host: service::host(),
            models_dir: service::models_dir().display().to_string(),
        }
    }

    fn to_json(&self) -> Value {
        let p = &self.policy;
        json!({
            "schema": STATUS_SCHEMA,
            "engine": self.engine,
            // The catalog and installer arrive with PR-11.
            "models": [],
            "active_model": null,
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
            // The terminal's own microphone arrives with PR-25.
            "capture": {"available": false, "reason": "not_compiled"},
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
            ("Speech models:", "none installed".to_string()),
            (
                "Model folder:",
                format!("{} (account {})", self.models_dir, self.account),
            ),
            ("Policy:", policy_line(&self.policy)),
            ("Microphone:", "not part of this build".to_string()),
        ]
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

/// `1.0 GB`, `690 MB`.
fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
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
        assert_eq!(
            json["capture"],
            json!({"available": false, "reason": "not_compiled"})
        );
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
