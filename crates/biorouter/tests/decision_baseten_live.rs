//! Live Baseten decision models: REAL, BILLED calls to
//! `POST https://inference.baseten.co/v1/systemone` through the adapter the
//! product uses, plus one read of the published Baseten feed. Every test is
//! `#[ignore]`d, so CI, which never passes `--ignored`, builds this binary and
//! runs nothing in it.
//!
//! Run it from a shell that holds a Baseten key in `BASETEN_API_KEY`:
//!
//! ```text
//! cargo test -p biorouter --test decision_baseten_live -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! One thread keeps the run under the 15 requests a minute of a Basic
//! account. A test that needs the key and finds none prints a note and
//! returns, so a run without the key proves nothing about Baseten; read the
//! output. `BASETEN_DECISION_HOST` points the run at another host.
//!
//! The process runs in a fresh sandbox (a temporary `BIOROUTER_PATH_ROOT`, the
//! keyring off), so the key comes from the environment and nothing reads
//! `~/.config/biorouter` or the Keychain. Every note is synthetic.

biorouter::keep_this_test_binary_off_the_os_credential_store!();

use std::sync::Arc;

use async_trait::async_trait;
use biorouter::decision::service::{context_margin, estimate_tokens};
use biorouter::decision::{
    Decision, DecisionContext, DecisionError, DecisionKind, DecisionModel, DecisionModelRef,
    DecisionModelSource, DecisionRequest, DecisionResult, DecisionService, DecisionSession,
    DecisionUseGate, ProbabilitySource, Question, Threshold,
};
use biorouter::privacy::ProviderTier;
use biorouter::providers::decision::catalog::{
    baseten_entries_from, baseten_entry, BASETEN_DECISION_CONTEXT, BASETEN_DECISION_MODELS,
    DECISION_MODEL_PREFIX,
};
use biorouter::providers::decision::system_one::DEFAULT_BASETEN_DECISION_HOST;
use biorouter::providers::decision::{ConfiguredDecisionModels, SystemOneAdapter};
use biorouter::providers::live_catalog::{LiveCatalog, BASETEN_CATALOG_URL};
use indexmap::IndexMap;
use serde_json::Value;

#[ctor::ctor]
fn sandbox() {
    let root = tempfile::TempDir::new().expect("a sandbox");
    for sub in ["config", "data", "state"] {
        std::fs::create_dir_all(root.path().join(sub)).expect("a sandbox tree");
    }
    std::fs::write(root.path().join("config/config.yaml"), "{}\n").expect("a config file");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

const MS_NOTE: &str = "Neuro f/u. 34F, RRMS dx 2019 on ocrelizumab. Exam stable, EDSS 2.0.";
const ORTHO_NOTE: &str = "Ortho f/u. Knee pain improving after PT. No neuro complaints.";
const SENTINEL: &str = "SYNTHETIC_LIVE_SENTINEL_7F3A";

/// The key, or `None` after saying the test did not run.
fn key(test: &str) -> Option<String> {
    match std::env::var("BASETEN_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => {
            eprintln!(
                "SKIPPED {test}: BASETEN_API_KEY is not set, so no Baseten call was made and \
                 nothing about Baseten was checked."
            );
            None
        }
    }
}

struct Allow;

#[async_trait]
impl DecisionUseGate for Allow {
    async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
        Ok(())
    }
}

/// The product's own path: config, the catalog and the adapter.
fn service() -> DecisionService {
    DecisionService::new(Arc::new(ConfiguredDecisionModels))
        .with_use_gate(Arc::new(Allow))
        .with_default_model(None)
}

fn model(alias: &str) -> DecisionModelRef {
    DecisionModelRef::new("baseten_decision", alias)
}

const COURSES: [&str; 4] = ["rrms", "spms", "ppms", "none"];
const LEVELS: [&str; 3] = ["Poor", "Fair", "Good"];

fn has_ms() -> (String, Question) {
    (
        "has_ms".to_string(),
        Question {
            instructions: "Does this note state the patient has multiple sclerosis?".into(),
            kind: DecisionKind::Binary {},
            threshold: Some(Threshold::Yes(0.9)),
        },
    )
}

fn three_questions() -> IndexMap<String, Question> {
    let descriptions = [
        "Relapsing-remitting",
        "Secondary progressive",
        "Primary progressive",
        "Not stated",
    ];
    IndexMap::from([
        has_ms(),
        (
            "course".to_string(),
            Question {
                instructions: "Which MS course is described?".into(),
                kind: DecisionKind::Choice {
                    options: COURSES
                        .iter()
                        .zip(descriptions)
                        .map(|(name, text)| (name.to_string(), Some(text.to_string())))
                        .collect(),
                },
                threshold: None,
            },
        ),
        (
            "rating".to_string(),
            Question {
                instructions: "How well documented is the exam?".into(),
                kind: DecisionKind::Score {
                    levels: LEVELS.iter().map(|l| l.to_string()).collect(),
                },
                threshold: None,
            },
        ),
    ])
}

fn ask(questions: IndexMap<String, Question>, note: &str) -> DecisionRequest {
    DecisionRequest {
        questions,
        context: DecisionContext::Text(note.into()),
    }
}

/// The sum of a raw answer's own `probabilities`, after checking each is a
/// probability. Asserted on the server's numbers: the service renormalises
/// what it keeps, so a sum taken after that proves nothing.
fn raw_sum(decision: &Decision) -> f64 {
    let reported = decision.raw["probabilities"]
        .as_object()
        .unwrap_or_else(|| {
            panic!(
                "`{}` has no probabilities: {}",
                decision.question, decision.raw
            )
        });
    reported
        .values()
        .map(|p| {
            let p = p.as_f64().expect("a number");
            assert!((0.0..=1.0).contains(&p), "{}", decision.raw);
            p
        })
        .sum()
}

/// What every live answer must be, whatever the model.
fn check_shape(decisions: &[Decision]) {
    let first = decisions.first().expect("decisions");
    for decision in decisions {
        assert_eq!(decision.source, ProbabilitySource::Native, "{decision:?}");
        assert!(decision.calibrated, "{decision:?}");
        assert_eq!(decision.tier, ProviderTier::Public);
        assert_eq!(decision.request_id, first.request_id, "one request");
        assert!(
            decision
                .probabilities
                .values()
                .all(|p| (0.0..=1.0).contains(p)),
            "{decision:?}"
        );
        let kept: f64 = decision.probabilities.values().sum();
        assert!((kept - 1.0).abs() < 1e-6, "{decision:?}");
        assert!(decision.raw["type"].as_str().is_some(), "{}", decision.raw);
    }
    assert!(first.latency_ms > 0, "the latency is recorded");
    assert!(
        first.usage.input_tokens.is_some_and(|n| n > 0),
        "{:?}",
        first.usage
    );
    eprintln!(
        "{}: {} ms, {:?} input tokens, raw answers: {}",
        first.model.model,
        first.latency_ms,
        first.usage.input_tokens,
        decisions
            .iter()
            .map(|d| d.raw.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );
}

/// The choice and score answers name only what was asked, and each raw
/// distribution sums to about 1.
fn check_distributions(decisions: &[Decision]) {
    let by_name = |name: &str| {
        decisions
            .iter()
            .find(|d| d.question == name)
            .unwrap_or_else(|| panic!("no `{name}` decision"))
    };
    let noul = by_name("has_ms").raw["noul"].as_f64().expect("a noul");
    assert!((0.0..=1.0).contains(&noul));
    for name in ["course", "rating"] {
        let sum = raw_sum(by_name(name));
        assert!((sum - 1.0).abs() < 0.01, "`{name}` sums to {sum}");
    }
    let course = by_name("course").raw["probabilities"].as_object().unwrap();
    assert!(
        course.keys().all(|k| COURSES.contains(&k.as_str())),
        "{course:?}"
    );
    let rating = by_name("rating").raw["probabilities"].as_object().unwrap();
    assert!(
        rating.keys().all(|k| {
            k.parse::<usize>().is_ok_and(|i| i < LEVELS.len()) || LEVELS.contains(&k.as_str())
        }),
        "{rating:?}"
    );
}

#[tokio::test]
#[ignore = "live, billed Baseten call; needs BASETEN_API_KEY; run with --ignored --test-threads=1"]
async fn joey_answers_every_kind_with_a_whole_distribution() {
    if key("joey_answers_every_kind_with_a_whole_distribution").is_none() {
        return;
    }
    let joey = model("joey-1");
    let opened = ConfiguredDecisionModels
        .open(&joey)
        .await
        .expect("joey-1 opens");
    assert_eq!(
        opened.caps().context_tokens,
        BASETEN_DECISION_CONTEXT,
        "Baseten's real prompt limit (16,000, measured 2026-11-10; the feed says 16,384)"
    );
    assert_eq!(opened.tier(), ProviderTier::Public);

    let svc = service();
    let mut p_yes = Vec::new();
    for note in [MS_NOTE, ORTHO_NOTE] {
        let decisions = svc
            .decide(None, Some(&joey), &ask(three_questions(), note))
            .await
            .expect("joey-1 answers");
        assert_eq!(
            decisions
                .iter()
                .map(|d| d.question.as_str())
                .collect::<Vec<_>>(),
            ["has_ms", "course", "rating"]
        );
        check_shape(&decisions);
        check_distributions(&decisions);
        p_yes.push(decisions[0].probabilities["yes"]);
    }
    // An ordering, not a threshold, so model quality cannot make it flaky.
    assert!(
        p_yes[0] > p_yes[1],
        "P(yes) MS {} vs ortho {}",
        p_yes[0],
        p_yes[1]
    );
}

#[tokio::test]
#[ignore = "live, billed Baseten call; needs BASETEN_API_KEY; run with --ignored --test-threads=1"]
async fn every_known_model_answers_a_yes_no_question() {
    if key("every_known_model_answers_a_yes_no_question").is_none() {
        return;
    }
    let svc = service();
    for known in BASETEN_DECISION_MODELS {
        let decisions = svc
            .decide(
                None,
                Some(&model(known.name)),
                &ask(IndexMap::from([has_ms()]), MS_NOTE),
            )
            .await
            .unwrap_or_else(|e| panic!("{} answers: {e}", known.name));
        assert_eq!(decisions.len(), 1);
        check_shape(&decisions);
        assert!(
            decisions[0].raw["noul"].as_f64().is_some(),
            "{}",
            decisions[0].raw
        );
    }
}

#[tokio::test]
#[ignore = "live, billed Baseten call; needs BASETEN_API_KEY; run with --ignored --test-threads=1"]
async fn a_state_at_the_estimated_budget_is_accepted() {
    if key("a_state_at_the_estimated_budget_is_accepted").is_none() {
        return;
    }
    let limit = BASETEN_DECISION_CONTEXT;
    let svc = service();
    for (kind, unit) in [
        (
            "prose",
            "Patient reports improving knee pain after physical therapy. ",
        ),
        (
            "labs",
            "Na 138 K 4.1 Cl 101 CO2 24 BUN 14 Cr 0.92 Glu 104 Ca 9.4; ",
        ),
    ] {
        // The most repetitions the size check before sending still accepts.
        let fits = |n: usize| {
            let request = ask(IndexMap::from([has_ms()]), &unit.repeat(n));
            async move { estimate_tokens(&request).await + context_margin(limit) <= limit }
        };
        let (mut low, mut high) = (1usize, 8_000usize);
        while low + 1 < high {
            let mid = (low + high) / 2;
            if fits(mid).await {
                low = mid;
            } else {
                high = mid;
            }
        }
        let request = ask(IndexMap::from([has_ms()]), &unit.repeat(low));
        let estimate = estimate_tokens(&request).await;
        let decisions = svc
            .decide(None, Some(&model("joey-1")), &request)
            .await
            .unwrap_or_else(|e| panic!("a {kind} state at the budget ({estimate}): {e}"));
        eprintln!(
            "{kind}: BioRouter estimated {estimate} tokens, Baseten counted {:?}",
            decisions[0].usage.input_tokens
        );
    }
}

#[tokio::test]
#[ignore = "live, billed Baseten call; needs BASETEN_API_KEY; run with --ignored --test-threads=1"]
async fn an_over_window_state_is_refused_whole_and_not_echoed() {
    let Some(key) = key("an_over_window_state_is_refused_whole_and_not_echoed") else {
        return;
    };
    let host = std::env::var("BASETEN_DECISION_HOST")
        .unwrap_or_else(|_| DEFAULT_BASETEN_DECISION_HOST.to_string());
    let joey = baseten_entry("decision-models/baseten-joey-2b-latest", 16_384);
    let adapter = SystemOneAdapter::baseten(&host, key, joey).expect("an adapter");
    // Straight to the adapter: the service would refuse this before sending.
    let state = format!("{SENTINEL} ") + &"Synthetic clinical note sentence. ".repeat(3_500);
    let err = adapter
        .decide(&ask(IndexMap::from([has_ms()]), &state))
        .await
        .unwrap_err();
    eprintln!("over the window: {err:?}");
    let DecisionError::Backend {
        status, error_type, ..
    } = &err
    else {
        panic!("a backend refusal, not {err:?}");
    };
    assert!(
        matches!(status, Some(400 | 413 | 422)),
        "refused as invalid or too large, not {status:?}"
    );
    assert!(!err.to_string().contains(SENTINEL), "{err}");
    assert!(error_type.as_ref().is_none_or(|t| t.len() <= 64));
}

#[tokio::test]
#[ignore = "live Baseten call; needs no key; run with --ignored --test-threads=1"]
async fn a_wrong_key_is_refused_without_echo() {
    let host = std::env::var("BASETEN_DECISION_HOST")
        .unwrap_or_else(|_| DEFAULT_BASETEN_DECISION_HOST.to_string());
    let bogus = "br-live-test-not-a-key";
    let adapter = SystemOneAdapter::baseten(
        &host,
        bogus.into(),
        baseten_entry("decision-models/baseten-joey-2b-latest", 16_384),
    )
    .expect("an adapter");
    let err = adapter
        .decide(&ask(IndexMap::from([has_ms()]), MS_NOTE))
        .await
        .unwrap_err();
    eprintln!("a wrong key: {err}");
    let DecisionError::Backend { status, .. } = &err else {
        panic!("a backend refusal, not {err:?}");
    };
    assert!(matches!(status, Some(401 | 403)), "{status:?}");
    assert!(err.to_string().contains("refused the credential"), "{err}");
    assert!(!err.to_string().contains(bogus), "{err}");
}

#[tokio::test]
#[ignore = "live read of the published feed; needs no key; run with --ignored"]
async fn the_published_feed_lists_the_three_decision_models() {
    let bytes = reqwest::get(BASETEN_CATALOG_URL)
        .await
        .and_then(|response| response.error_for_status())
        .expect("the feed answers")
        .bytes()
        .await
        .expect("the feed's body");
    let catalog = LiveCatalog::from_bytes(&bytes).expect("the feed parses");
    let entries = baseten_entries_from(Some(&catalog));
    for known in BASETEN_DECISION_MODELS {
        let entry = entries
            .iter()
            .find(|e| e.model == known.slug)
            .unwrap_or_else(|| panic!("the feed lists {}", known.slug));
        assert_eq!(
            entry.context_tokens, BASETEN_DECISION_CONTEXT,
            "{}",
            known.slug
        );
        assert_eq!(entry.display_name, known.name);
    }
    // The chat picker still sees no decision model.
    assert!(catalog
        .advertised()
        .all(|m| !m.id.starts_with(DECISION_MODEL_PREFIX)));
    let listed: Vec<Value> = entries
        .iter()
        .map(|e| serde_json::json!({"model": e.model, "window": e.context_tokens}))
        .collect();
    eprintln!("feed decision models: {listed:?}");
}
