//! Lint catches the interactions that *look* correct and are dead.
//!
//! These are the failure modes no string analysis in the old lint could see,
//! because the code is correct-looking and the failure is behavioural:
//!
//!   * a drag surface that no synthetic or assistive pointer can drive
//!     (HTML5 DnD does not fire `dragstart` for a programmatic pointer move — so
//!     spec-009's core interaction was reachable only by a human with a mouse);
//!   * a bound element with no initial state, which renders blank until the first
//!     *paid* agent turn writes to the document — the reason authors kept a private
//!     local `state` object, which then diverged from the doc the agent reads.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use biorouter_mcp::agent_drafter::bundle::{lint_app, LintLevel};
use std::path::Path;
use tempfile::TempDir;

/// Write a minimal app project and lint it.
fn lint(index: &str, main: &str, manifest: &str) -> Vec<(LintLevel, String)> {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "index.html", index);
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    write(dir.path(), "src/main.ts", main);
    write(dir.path(), "manifest.json", manifest);

    lint_app(dir.path())
        .into_iter()
        .map(|f| (f.level, f.msg))
        .collect()
}

fn write(root: &Path, rel: &str, content: &str) {
    std::fs::write(root.join(rel), content).unwrap();
}

fn agentic_manifest(extra: &str) -> String {
    format!(
        r#"{{
            "id": "app", "title": "App", "kind": "agentic", "entry": "index.html",
            "created_at": 1, "updated_at": 1,
            "agent": {{ "system_prompt": "Drive the page." }}
            {extra}
        }}"#
    )
}

fn errors(findings: &[(LintLevel, String)]) -> Vec<&str> {
    findings
        .iter()
        .filter(|(l, _)| *l == LintLevel::Error)
        .map(|(_, m)| m.as_str())
        .collect()
}

fn warnings(findings: &[(LintLevel, String)]) -> Vec<&str> {
    findings
        .iter()
        .filter(|(l, _)| *l == LintLevel::Warn)
        .map(|(_, m)| m.as_str())
        .collect()
}

/// The spec-009 repro: a stratum builder wired with raw HTML5 drag-and-drop and
/// nothing else. A coordinate drag from an automated pointer fires no `dragstart`,
/// so the whole interaction is dead to everything but a human mouse.
#[test]
fn a_drag_only_surface_is_an_error() {
    let findings = lint(
        r#"<div id="covariates"><div class="chip" draggable="true">Age</div></div>
           <div id="stratum" class="br-dropzone"></div>"#,
        r#"const el = document.getElementById("covariates");
           el.addEventListener("dragstart", (e) => { e.dataTransfer.setData("id", "age"); });"#,
        &agentic_manifest(""),
    );

    let errs = errors(&findings);
    assert!(
        errs.iter().any(|e| e.contains("unreachable by keyboard")),
        "a drag-only surface must fail the build: {findings:?}"
    );
    assert!(
        errs.iter().any(|e| e.contains("br.dnd.catalog")),
        "and the error must name the working alternative: a rule with no alternative \
         just makes the model hand-roll something worse: {findings:?}"
    );
}

/// The primitive is the sanctioned path, so an app that uses it must lint clean.
#[test]
fn the_dnd_primitive_is_accepted() {
    let findings = lint(
        r#"<div id="covariates" data-br-dnd><div data-br-item="age">Age</div></div>
           <div id="stratum" data-br-zone="stratum"></div>"#,
        "br.dnd.catalog({ source: '#covariates', target: '#stratum', signal: 'stratum_changed' });",
        &agentic_manifest(""),
    );

    assert!(
        !errors(&findings).iter().any(|e| e.contains("unreachable")),
        "br.dnd.catalog is the fix, not the failure: {findings:?}"
    );
}

/// A hand-rolled drag that at least ALSO wires a keyboard path is a warning, not a
/// hard failure — the interaction is reachable, just not ideal.
#[test]
fn hand_rolled_drag_with_a_keyboard_fallback_is_only_a_warning() {
    let findings = lint(
        r#"<div id="covariates"><div class="chip" draggable="true">Age</div></div>
           <div id="stratum"></div>"#,
        r#"el.addEventListener("dragstart", onDragStart);
           el.addEventListener("keydown", onKey);"#,
        &agentic_manifest(""),
    );

    assert!(
        !errors(&findings)
            .iter()
            .any(|e| e.contains("unreachable by keyboard")),
        "a keyboard fallback exists, so this is reachable: {findings:?}"
    );
    assert!(
        warnings(&findings)
            .iter()
            .any(|w| w.contains("synthetic or assistive pointer")),
        "but it still will not respond to a synthetic pointer: {findings:?}"
    );
}

/// Bindings with no declared initial state render blank on first load. This is the
/// root of the state-divergence bug, so lint names it directly.
#[test]
fn bindings_without_an_initial_state_are_flagged() {
    let findings = lint(
        r#"<div class="br-card"><span data-br-bind="/power/n">—</span></div>"#,
        r#"br.state.subscribe("/power/n", () => {});"#,
        &agentic_manifest(""),
    );

    let warns = warnings(&findings);
    assert!(
        warns.iter().any(|w| w.contains("state_initial")),
        "a bound element with no initial doc renders blank until a PAID turn: {findings:?}"
    );
    assert!(
        warns.iter().any(|w| w.contains("silently diverges")),
        "and the warning must explain why the obvious workaround is the bug: {findings:?}"
    );
}

/// Declaring the initial document is the fix, and must lint clean.
#[test]
fn bindings_with_a_declared_initial_state_are_clean() {
    let findings = lint(
        r#"<div class="br-card"><span data-br-bind="/power/n">—</span></div>"#,
        r#"br.state.subscribe("/power/n", () => {});"#,
        &agentic_manifest(r#", "surface": { "state_initial": { "power": { "n": 0 } } }"#),
    );

    assert!(
        !warnings(&findings)
            .iter()
            .any(|w| w.contains("state_initial")),
        "the initial doc is declared: {findings:?}"
    );
}

/// Back-compat: an app with no bindings and no drag is untouched by these rules.
#[test]
fn a_plain_app_is_not_newly_flagged() {
    let findings = lint(
        r#"<div class="br-card"><button id="go">Run</button></div>"#,
        "document.getElementById('go').addEventListener('click', () => br.run('go', '#out'));",
        &agentic_manifest(""),
    );

    assert!(
        !warnings(&findings)
            .iter()
            .any(|w| w.contains("state_initial")),
        "no bindings, so no initial-state warning: {findings:?}"
    );
    assert!(
        !errors(&findings).iter().any(|e| e.contains("unreachable")),
        "no drag, so no drag error: {findings:?}"
    );
}
