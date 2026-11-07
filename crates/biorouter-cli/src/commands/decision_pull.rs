//! `biorouter models decision pull [MODEL]`: install a decision model into the
//! Ollama on this computer.
//!
//! A private chat may be read only by a decision model that runs on this
//! computer, so a computer with none gives a private chat no monitors, gates
//! or decisions. This downloads one of the models Biorouter offers
//! (`tev1:0.8b` unless another is named) into the Ollama `OLLAMA_HOST` names
//! and calls it installed only once Ollama answers decisions with it.
//!
//! It runs in this process, with no daemon: the catalog reads Ollama's
//! `/api/tags` again on every listing, so a running daemon lists the model at
//! once. The refusals are the install route's own, because both ask
//! `install::preflight` first: a model Biorouter does not offer, an
//! `OLLAMA_HOST` that does not resolve, an Ollama on another computer (asked
//! nothing) and one that does not answer.
//!
//! How it reports: on a terminal, a spinner with the bytes downloaded; off
//! one, plain lines, the start and each of Ollama's steps on stderr and the
//! result on stdout, where a script reads results. A failure is the error
//! `main` prints, once, in Ollama's words when they are Ollama's.

use std::collections::HashSet;
use std::io::Write;

use anyhow::{anyhow, Result};
use biorouter::decision::DecisionModelEntry;
use biorouter::privacy::ProviderTier;
use biorouter::providers::decision::install::{self, PullProgress, PullStage};
use biorouter::providers::decision::DEFAULT_INSTALL_MODEL;
use console::style;
use url::Url;

use super::decide::terminal_text;

/// The model `pull` installs: the one named, else the default.
pub fn requested_model(model: Option<&str>) -> String {
    model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(DEFAULT_INSTALL_MODEL)
        .to_string()
}

/// `412 MB`, `1.5 GB`: decimal units, one decimal place below ten.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// What `pull` prints once the model answers decisions.
pub fn installed_line(entry: &DecisionModelEntry) -> String {
    let name = terminal_text(&entry.model);
    if entry.local && entry.tier == ProviderTier::Private {
        format!(
            "Installed {name}: Private, runs on this computer. Private chats can now use it for \
             monitors and decisions."
        )
    } else {
        format!("Installed {name}.")
    }
}

/// How `pull` reports, as `models local pull` does: a spinner on a terminal
/// (drawn on stderr, which draws nothing off one), plain lines anywhere else.
struct PullReport<Out: Write, Err: Write> {
    spinner: Option<cliclack::ProgressBar>,
    out: Out,
    err: Err,
    model: String,
    /// Ollama's steps already printed, so a step reported by many lines (one
    /// layer's progress, layers that interleave) is printed once.
    steps: HashSet<String>,
    /// The spinner's message now, so it is redrawn only when it changes.
    shown: String,
    verifying: bool,
}

impl PullReport<std::io::Stdout, std::io::Stderr> {
    fn start(model: &str) -> Self {
        use std::io::IsTerminal;
        let spinner = std::io::stderr().is_terminal().then(cliclack::spinner);
        Self::started(spinner, std::io::stdout(), std::io::stderr(), model)
    }
}

impl<Out: Write, Err: Write> PullReport<Out, Err> {
    fn started(
        spinner: Option<cliclack::ProgressBar>,
        out: Out,
        mut err: Err,
        model: &str,
    ) -> Self {
        let line = format!("Downloading {model} from Ollama's library…");
        match &spinner {
            Some(spinner) => spinner.start(&line),
            None => {
                let _ = writeln!(err, "{line}");
            }
        }
        Self {
            spinner,
            out,
            err,
            model: model.to_string(),
            steps: HashSet::new(),
            shown: line,
            verifying: false,
        }
    }

    fn progress(&mut self, progress: &PullProgress) {
        match progress.stage {
            PullStage::Downloading => {
                if self.spinner.is_some() {
                    if progress.total_bytes > 0 {
                        let line = format!(
                            "Downloading {}… {} of {}",
                            self.model,
                            human_bytes(progress.completed_bytes),
                            human_bytes(progress.total_bytes)
                        );
                        self.show(line);
                    }
                    return;
                }
                let step = terminal_text(progress.status.trim());
                if !step.is_empty() && self.steps.insert(step.clone()) {
                    let _ = writeln!(self.err, "  {step}");
                }
            }
            PullStage::Verifying => {
                if self.verifying {
                    return;
                }
                self.verifying = true;
                let line = format!(
                    "Checking that Ollama answers decisions with {}…",
                    self.model
                );
                if self.spinner.is_some() {
                    self.show(line);
                } else {
                    let _ = writeln!(
                        self.err,
                        "Downloaded {}. {line}",
                        human_bytes(progress.completed_bytes)
                    );
                }
            }
        }
    }

    fn show(&mut self, line: String) {
        if line != self.shown {
            if let Some(spinner) = &self.spinner {
                spinner.set_message(&line);
            }
            self.shown = line;
        }
    }

    fn done(mut self, entry: &DecisionModelEntry) {
        let line = installed_line(entry);
        match self.spinner.take() {
            Some(spinner) => spinner.stop(style(line).green()),
            None => {
                let _ = writeln!(self.out, "{line}");
            }
        }
    }

    fn failed(mut self, line: &str) {
        if let Some(spinner) = self.spinner.take() {
            spinner.stop(style(line).red());
        }
        let _ = self.err.flush();
    }
}

/// Install `name` into the Ollama at `base`: the install route's refusals
/// first (nothing is sent to an Ollama that is not this computer's, and
/// nothing is downloaded for a model not offered), then the pull, reported
/// through `start`'s report. The entry the model lists under, or the sentence
/// that says why it is not installed.
async fn install_reporting<Out, Err>(
    base: Result<Url, String>,
    name: &str,
    start: impl FnOnce(&str) -> PullReport<Out, Err>,
) -> Result<DecisionModelEntry>
where
    Out: Write,
    Err: Write,
{
    let (model, base) = install::preflight(base, name)
        .await
        .map_err(|refusal| anyhow!("{}", refusal.message))?;
    let mut report = start(model.name);
    let pulled = install::pull(&base, model.name, |progress| report.progress(&progress)).await;
    match pulled {
        Ok(entry) => {
            report.done(&entry);
            Ok(entry)
        }
        Err(error) => {
            let line = error.to_string();
            report.failed(&line);
            Err(anyhow!("{line}"))
        }
    }
}

/// `biorouter models decision pull [MODEL]`.
pub async fn handle_models_decision_pull(model: Option<String>) -> Result<()> {
    let name = requested_model(model.as_deref());
    install_reporting(install::configured_ollama_base(), &name, PullReport::start).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{header, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    const TEV1: &str = "tev1:0.8b";

    /// A fake Ollama on this computer: `/api/version` answers, `/api/pull`
    /// streams `lines`, `/api/show` lists the `decision` capability and the
    /// load probe answers as for a copy Ollama loads. Every request path is
    /// recorded.
    async fn ollama(lines: Vec<Value>) -> (Url, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        let record = |seen: &Arc<Mutex<Vec<String>>>, path: &'static str| {
            let seen = Arc::clone(seen);
            move || {
                seen.lock().unwrap().push(path.to_string());
            }
        };
        let (version, pull, show, probe) = (
            record(&seen, "/api/version"),
            record(&seen, "/api/pull"),
            record(&seen, "/api/show"),
            record(&seen, "/v1/systemone"),
        );
        let app = Router::new()
            .route(
                "/api/version",
                get(move || async move {
                    version();
                    Json(json!({"version": "0.40.15"}))
                }),
            )
            .route(
                "/api/pull",
                post(move || async move {
                    pull();
                    ([(header::CONTENT_TYPE, "application/x-ndjson")], body)
                }),
            )
            .route(
                "/api/show",
                post(move || async move {
                    show();
                    Json(json!({"capabilities": ["decision"]}))
                }),
            )
            .route(
                "/v1/systemone",
                post(move || async move {
                    probe();
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "questions must contain 1\u{2013}64 fields"})),
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (Url::parse(&format!("http://{address}")).unwrap(), seen)
    }

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

    /// A report off a terminal, writing into buffers the test reads back.
    type Buffers = (Arc<Mutex<Vec<u8>>>, Arc<Mutex<Vec<u8>>>);

    #[derive(Clone)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn plain() -> (Buffers, impl FnOnce(&str) -> PullReport<Shared, Shared>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::new(Mutex::new(Vec::new()));
        let (o, e) = (Shared(Arc::clone(&out)), Shared(Arc::clone(&err)));
        ((out, err), move |model: &str| {
            PullReport::started(None, o, e, model)
        })
    }

    fn text(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(buffer.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn models_decision_pull_defaults_to_tev1() {
        assert_eq!(requested_model(None), "tev1:0.8b");
        assert_eq!(requested_model(Some("  ")), "tev1:0.8b");
        assert_eq!(requested_model(Some("clef-flash")), "clef-flash");
        assert_eq!(DEFAULT_INSTALL_MODEL, TEV1);
    }

    #[test]
    fn bytes_read_as_the_desktop_writes_them() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_200), "1.2 kB");
        assert_eq!(human_bytes(412_000_000), "412 MB");
        assert_eq!(human_bytes(1_500_000_000), "1.5 GB");
        assert_eq!(human_bytes(12_000_000_000), "12 GB");
    }

    /// Off a terminal the pull says it started and each of Ollama's steps
    /// once on stderr, and that it is installed on stdout, where a script
    /// reads it.
    #[tokio::test]
    #[serial_test::serial(ollama_discovery)]
    async fn a_pull_off_a_terminal_reports_in_plain_lines() {
        let (base, seen) = ollama(two_layer_pull()).await;
        let ((out, err), start) = plain();
        let entry = install_reporting(Ok(base), TEV1, start)
            .await
            .expect("installed");
        assert_eq!(entry.model, TEV1);
        assert_eq!(
            text(&err),
            "Downloading tev1:0.8b from Ollama's library…\n\
             \x20 pulling manifest\n\
             \x20 pulling aaa\n\
             \x20 pulling bbb\n\
             \x20 verifying sha256 digest\n\
             \x20 writing manifest\n\
             Downloaded 1.2 kB. Checking that Ollama answers decisions with tev1:0.8b…\n"
        );
        assert_eq!(
            text(&out),
            "Installed tev1:0.8b: Private, runs on this computer. Private chats can now use it \
             for monitors and decisions.\n"
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            ["/api/version", "/api/pull", "/api/show", "/v1/systemone"],
            "the place is asked first, then the pull, then the checks"
        );
    }

    /// An `error` line fails the pull with Ollama's own words, and nothing
    /// says it is installed.
    #[tokio::test]
    async fn a_failed_pull_quotes_ollama_and_installs_nothing() {
        let (base, _) = ollama(vec![
            json!({"status": "pulling manifest"}),
            json!({"error": "pull model manifest: file does not exist"}),
        ])
        .await;
        let ((out, err), start) = plain();
        let error = install_reporting(Ok(base), TEV1, start)
            .await
            .expect_err("Ollama refused the pull");
        assert_eq!(
            error.to_string(),
            "Ollama could not download `tev1:0.8b`: pull model manifest: file does not exist."
        );
        assert!(text(&out).is_empty(), "nothing installed: {}", text(&out));
        assert!(text(&err).starts_with("Downloading tev1:0.8b"));
    }

    /// The install route's refusals, asked before anything is downloaded: a
    /// model Biorouter does not offer and an Ollama that is not on this
    /// computer are refused without a request, and nothing is printed as
    /// started.
    #[tokio::test]
    async fn the_routes_refusals_come_first_and_send_nothing() {
        let (local, seen) = ollama(two_layer_pull()).await;

        let ((out, err), start) = plain();
        let error = install_reporting(Ok(local.clone()), "llama3:8b", start)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("`llama3:8b` is not a decision model Biorouter installs."),
            "{error}"
        );
        assert!(text(&out).is_empty() && text(&err).is_empty());
        assert!(seen.lock().unwrap().is_empty(), "nothing was asked");

        let elsewhere = Url::parse("http://10.255.255.1:11434").unwrap();
        let ((out, err), start) = plain();
        let error = install_reporting(Ok(elsewhere), TEV1, start)
            .await
            .unwrap_err();
        assert!(
            error.to_string().starts_with(
                "Biorouter installs decision models only into an Ollama on this computer"
            ),
            "{error}"
        );
        assert!(text(&out).is_empty() && text(&err).is_empty());

        let ((out, _), start) = plain();
        let error = install_reporting(Err("not a URL".to_string()), TEV1, start)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("OLLAMA_HOST could not be read"),
            "{error}"
        );
        assert!(text(&out).is_empty());
    }

    /// The installed line names the tier only for the model this command
    /// installs: one on this computer, Private.
    #[test]
    fn the_installed_line_says_private_only_for_a_local_model() {
        let mut entry = DecisionModelEntry {
            provider: "ollama_decision".into(),
            model: TEV1.into(),
            display_name: TEV1.into(),
            context_tokens: 8_192,
            kinds: Vec::new(),
            tier: ProviderTier::Private,
            affiliation: None,
            adapter: biorouter::decision::DecisionAdapter::SystemOne,
            local: true,
            load_refusal: None,
        };
        assert!(installed_line(&entry).contains("Private, runs on this computer"));
        entry.tier = ProviderTier::Public;
        assert_eq!(installed_line(&entry), "Installed tev1:0.8b.");
    }
}
