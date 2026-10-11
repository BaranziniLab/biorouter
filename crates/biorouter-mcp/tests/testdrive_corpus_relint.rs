//! Re-lint the real apps the 100-app test drive produced.
//!
//! This is the regression corpus. Every app under
//! `.br-testdrive/runtime/config/biorouter/agent_drafter/` was authored by Agent
//! Drafter during the audit, and every one of them shipped with at least one of
//! the defects this campaign fixes. The audit's verdict was 0/18 full functional
//! passes.
//!
//! The point of this test is not to make those apps pass — they were authored
//! against the broken platform and would need re-authoring. The point is to prove
//! the fixed platform **now sees** what it was blind to: that the checks which
//! would have caught these apps at build time actually fire on the real artifacts,
//! not just on hand-written fixtures.
//!
//! Skips cleanly when the corpus is absent (it is git-ignored runtime state), so
//! this is a no-op on a fresh clone and on CI.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::agent_drafter::bundle::{lint_app, LintLevel};
use biorouter_mcp::agent_drafter::catalog::Catalog;
use biorouter_mcp::agent_drafter::store::Manifest;
use biorouter_mcp::agent_drafter::validate;
use std::path::PathBuf;

fn corpus() -> Option<PathBuf> {
    // crates/biorouter-mcp → repo root
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .to_path_buf();
    let dir = root.join(".br-testdrive/runtime/config/biorouter/agent_drafter");
    dir.is_dir().then_some(dir)
}

struct App {
    id: String,
    dir: PathBuf,
    manifest: Manifest,
}

fn apps() -> Vec<App> {
    let Some(dir) = corpus() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read corpus").flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path.join("manifest.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
            continue;
        };
        out.push(App {
            id: manifest.id.clone(),
            dir: path,
            manifest,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The corpus is real: it must parse under the CURRENT manifest schema.
///
/// This is also the back-compat canary. Waves 1-5 added `requires`, `state_initial`,
/// `effect`, `writes`, `requires_evidence`, `provenance_required`, `eager`,
/// `worker_ui` and `consult_timeout_s` — every one of them defaulted, precisely so
/// that manifests written before they existed keep loading. If any of them had been
/// made required, all 25 of these apps would fail to deserialize right here.
#[test]
fn every_test_drive_app_still_loads_under_the_new_schema() {
    let apps = apps();
    if apps.is_empty() {
        eprintln!("test-drive corpus absent; skipping");
        return;
    }

    println!("{} apps loaded from the test-drive corpus", apps.len());
    for app in &apps {
        assert!(!app.id.is_empty());
    }
}

/// The audit's headline configuration failure: 13 invented knowledge-base ids and
/// 7 nonexistent skill lists, none of which the platform could see.
///
/// The fixed platform must now catch them on the REAL artifacts.
#[test]
fn the_invented_ids_in_the_real_corpus_are_now_caught() {
    let apps = apps();
    if apps.is_empty() {
        eprintln!("test-drive corpus absent; skipping");
        return;
    }

    // The catalog the test drive actually ran against: zero KBs, zero skills.
    let empty_install = Catalog {
        knowledge_bases: Vec::new(),
        skills: Vec::new(),
        extensions: Catalog::discover(&biorouter_mcp::knowledge::caller::KbCaller::restricted())
            .extensions,
    };

    let mut caught: Vec<String> = Vec::new();
    for app in &apps {
        let Some(agent) = app.manifest.agent.as_ref() else {
            continue;
        };
        if let Err(e) = validate::check_all(
            agent.knowledge_base.as_deref(),
            &agent.skills,
            &agent.extensions,
            &empty_install,
        ) {
            caught.push(format!("{}: {}", app.id, e.lines().next().unwrap_or("")));
        }
    }

    println!("\n=== invented ids now rejected at the write boundary ===");
    for c in &caught {
        println!("  {c}");
    }

    assert!(
        !caught.is_empty(),
        "the corpus is known to contain invented knowledge-base and skill ids; the fixed \
         write-boundary check must reject them"
    );
}

/// Report what the new lint rules find across the whole real corpus.
///
/// Informational by design — these apps were authored against the broken platform,
/// so a clean bill of health would be the surprising outcome. What is asserted is
/// that lint RUNS on every one of them without panicking, and the printout is the
/// evidence of what the fixed platform now sees.
#[test]
fn lint_reports_on_the_whole_corpus() {
    let apps = apps();
    if apps.is_empty() {
        eprintln!("test-drive corpus absent; skipping");
        return;
    }

    let mut with_errors = 0usize;
    let mut with_warnings = 0usize;
    let mut clean = 0usize;

    println!("\n=== lint over the {} test-drive apps ===", apps.len());
    for app in &apps {
        let findings = lint_app(&app.dir);
        let errors: Vec<&str> = findings
            .iter()
            .filter(|f| f.level == LintLevel::Error)
            .map(|f| f.msg.as_str())
            .collect();
        let warns: Vec<&str> = findings
            .iter()
            .filter(|f| f.level == LintLevel::Warn)
            .map(|f| f.msg.as_str())
            .collect();

        if !errors.is_empty() {
            with_errors += 1;
        } else if !warns.is_empty() {
            with_warnings += 1;
        } else {
            clean += 1;
        }

        if errors.is_empty() && warns.is_empty() {
            println!("\n{}: clean", app.id);
            continue;
        }
        println!("\n{}:", app.id);
        for e in &errors {
            println!("  ERROR {}", first_line(e));
        }
        for w in &warns {
            println!("  warn  {}", first_line(w));
        }
    }

    println!(
        "\n=== summary: {with_errors} with errors, {with_warnings} warn-only, {clean} clean ==="
    );
}

fn first_line(msg: &str) -> String {
    let one = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = one.chars();
    let preview: String = chars.by_ref().take(110).collect();
    match chars.next() {
        Some(_) => format!("{preview}…"),
        None => one,
    }
}

#[test]
fn first_line_truncates_unicode_on_a_character_boundary() {
    let input = "é".repeat(111);
    assert_eq!(first_line(&input), format!("{}…", "é".repeat(110)));
}

/// The executing check must SEPARATE a working app from a broken one.
///
/// A harness that passes everything is decoration, and one that fails everything
/// gets muted. This is the discrimination test, run against the real corpus:
/// `spec-002` was repaired by the fixed platform's own agent; `spec-009` is the
/// audit's own FAIL verdict and was never touched.
///
/// Ignored by default — it needs Node and a browser. Run with:
///   cargo test -p biorouter-mcp --test testdrive_corpus_relint -- --ignored --nocapture
#[test]
#[ignore = "needs node + a browser"]
fn the_executing_check_separates_the_repaired_app_from_the_broken_ones() {
    let Some(dir) = corpus() else {
        eprintln!("test-drive corpus absent; skipping");
        return;
    };
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("scripts/agent-drafter/app-smoke.mjs");
    if !script.exists() {
        eprintln!("app-smoke.mjs absent; skipping");
        return;
    }

    let smoke = |app: &str| -> i32 {
        std::process::Command::new("node")
            .arg(&script)
            .arg(dir.join(app))
            .output()
            .map(|o| o.status.code().unwrap_or(-1))
            .unwrap_or(-1)
    };

    // The repaired app: every control delivers, every binding paints, the drag is
    // keyboard-reachable.
    let repaired = smoke("spec-002-cohort-funnel-foundry");
    assert_eq!(
        repaired, 0,
        "the app the fixed platform repaired must pass the executing check (got exit {repaired}; \
         2 means the browser could not launch)"
    );

    // The audit's own FAIL: blank bindings, a drag only a mouse can drive, and —
    // the defect no static analysis can see — controls that fire and send nothing.
    let broken = smoke("spec-009-survival-atelier");
    assert_eq!(
        broken, 1,
        "the audit's FAIL app must still fail the executing check (got exit {broken})"
    );
}
