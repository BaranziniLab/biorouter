//! Gate for the agent-driven-UI example apps in
//! `scripts/agent-drafter-apps/examples/ui/`.
//!
//! These apps are the reference for what "the agent drives the interface" looks
//! like, so they must keep passing the same build harness a model-authored app
//! does. Deterministic: no daemon, no LLM. The live counterpart (which asserts
//! the agent actually emits `ui` command frames) is
//! `ui/desktop/scripts/appcheck/check-ui-app.mjs`.

#[path = "support/path_sandbox.rs"]
mod path_sandbox;

use std::path::{Path, PathBuf};

use biorouter_mcp::agent_drafter::bundle::{self, LintLevel};
use biorouter_mcp::agent_drafter::store::{ArtifactKind, Manifest};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/agent-drafter-apps/examples/ui")
        .canonicalize()
        .expect("examples/ui must exist")
}

fn sdk_template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent_drafter/templates/sdk.ts")
}

fn smoke_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/agent-drafter/app-smoke.mjs")
}

fn example_apps() -> Vec<(String, PathBuf)> {
    let mut apps: Vec<(String, PathBuf)> = std::fs::read_dir(examples_dir())
        .expect("read examples/ui")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    apps.sort_by(|a, b| a.0.cmp(&b.0));
    apps
}

#[test]
fn there_are_example_apps_to_check() {
    let apps = example_apps();
    assert!(
        apps.len() >= 5,
        "expected a fleet of agent-driven-UI examples, found {}",
        apps.len()
    );
}

/// Each example must declare the capability it exists to demonstrate, and its
/// system prompt must tell the agent *when* to reach for the tools — a prompt
/// that never names a `ui_*` tool produces a chat box, not an app.
#[test]
fn every_example_grants_ui_and_its_prompt_directs_the_agent_to_use_it() {
    for (id, dir) in example_apps() {
        let raw = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
        let m: Manifest =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{id}: manifest.json: {e}"));
        assert_eq!(m.id, id, "{id}: manifest id must match its folder");
        assert_eq!(m.kind, ArtifactKind::Agentic, "{id}: must be agentic");

        let agent = m
            .agent
            .as_ref()
            .unwrap_or_else(|| panic!("{id}: no agent block"));
        assert!(
            agent.capabilities.ui.enabled,
            "{id}: ui capability must be on"
        );
        assert!(
            agent.model.is_none(),
            "{id}: examples stay provider-agnostic (pin no model)"
        );

        let prompt = &agent.system_prompt;
        let named: Vec<&str> = [
            "ui_describe",
            "ui_panel",
            "ui_render",
            "ui_chart",
            "ui_graph",
            "ui_highlight",
            "ui_theme",
            "ui_layout",
            "ui_notify",
            "ui_state",
            "ui_ask",
        ]
        .into_iter()
        .filter(|t| prompt.contains(t))
        .collect();
        assert!(
            named.len() >= 2,
            "{id}: system_prompt must direct the agent to at least two ui_* tools, found {named:?}"
        );
        assert!(
            !prompt.contains("```chart") && !prompt.contains("```graph"),
            "{id}: these examples drive the UI with tools, not markdown fences"
        );
    }
}

/// Every `@region:<name>` the prompt targets must actually exist in the markup,
/// or the agent renders into nothing and the SDK shows a "this app does not
/// have that" toast.
#[test]
fn every_region_the_prompt_targets_exists_in_the_markup() {
    for (id, dir) in example_apps() {
        let raw = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
        let m: Manifest = serde_json::from_str(&raw).unwrap();
        let prompt = m.agent.as_ref().unwrap().system_prompt.clone();
        let index = std::fs::read_to_string(dir.join("index.html")).unwrap();

        // Quoted values only, and no byte slicing: `&rest[1..]` after taking the
        // first *char* panics on `data-br-region=é`.
        let declared: Vec<String> = index
            .split("data-br-region=")
            .skip(1)
            .filter_map(|chunk| {
                let mut chars = chunk.chars();
                let quote = chars.next()?;
                if quote != '"' && quote != '\'' {
                    return None;
                }
                chars.as_str().split_once(quote).map(|(n, _)| n.to_string())
            })
            .collect();

        for tail in prompt.split("@region:").skip(1) {
            let name: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !name.is_empty() {
                assert!(
                    declared.contains(&name),
                    "{id}: system_prompt targets @region:{name} but index.html declares {declared:?}"
                );
            }
        }
    }
}

/// The same harness that gates a model-authored app must pass on every example.
#[test]
fn every_example_passes_the_build_harness_with_no_errors() {
    for (id, dir) in example_apps() {
        let findings = bundle::lint_app(&dir);
        let errors: Vec<&str> = findings
            .iter()
            .filter(|f| f.level == LintLevel::Error)
            .map(|f| f.msg.as_str())
            .collect();
        assert!(
            errors.is_empty(),
            "{id}: harness ERRORs block launch/export:\n  - {}",
            errors.join("\n  - ")
        );
    }
}

/// The examples ship only `manifest.json` / `index.html` / `src/main.ts`; the SDK
/// and bundle are supplied at install time. Bundling each against the real SDK
/// proves `main.ts` only calls APIs that exist.
#[test]
fn every_example_bundles_against_the_real_sdk() {
    let Some(_) = bundle::default_sources().first() else {
        return; // no template sources on disk; nothing to prove
    };
    for (id, dir) in example_apps() {
        let staging = std::env::temp_dir().join(format!("br-ui-example-{id}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(staging.join("src")).unwrap();
        std::fs::copy(dir.join("src/main.ts"), staging.join("src/main.ts")).unwrap();
        std::fs::copy(sdk_template(), staging.join("src/sdk.ts")).unwrap();

        let report = bundle::build_app(&staging).unwrap_or_else(|e| panic!("{id}: build: {e}"));
        assert!(
            report.ok,
            "{id}: bundle failed ({}):\n{}",
            report.used, report.log
        );
        let js = std::fs::read_to_string(staging.join("dist/app.js")).unwrap();
        assert!(js.len() > 500, "{id}: suspiciously small bundle");
        let _ = std::fs::remove_dir_all(&staging);
    }
}

/// Run every reference app in Chromium and require its controls, bindings, and
/// accessibility paths to satisfy the same executing harness used by app builds.
#[test]
#[ignore = "needs node + Chromium"]
fn every_example_passes_the_executing_smoke_harness() {
    assert!(smoke_script().is_file(), "app-smoke.mjs must exist");

    for (id, dir) in example_apps() {
        let staging = std::env::temp_dir().join(format!("br-ui-smoke-{id}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(staging.join("src")).unwrap();
        for file in ["index.html", "manifest.json"] {
            std::fs::copy(dir.join(file), staging.join(file)).unwrap();
        }
        std::fs::copy(dir.join("src/main.ts"), staging.join("src/main.ts")).unwrap();
        std::fs::copy(sdk_template(), staging.join("src/sdk.ts")).unwrap();

        let report = bundle::build_app(&staging).unwrap_or_else(|e| panic!("{id}: build: {e}"));
        assert!(report.ok, "{id}: bundle failed:\n{}", report.log);

        let output = std::process::Command::new("node")
            .arg(smoke_script())
            .arg(&staging)
            .arg("--json")
            .output()
            .unwrap_or_else(|e| panic!("{id}: could not launch app-smoke: {e}"));
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&staging);
        assert!(
            output.status.success(),
            "{id}: executing smoke failed ({}):\n{stdout}\n{stderr}",
            output.status
        );
    }
}
