//! The person tools' machinery that needs no daemon: the action table, parameter checks,
//! `describe`, refusal words, the public narrowing of a snapshot and the context diff. What
//! needs a daemon (approvals, consent, the routes' own functions) is in the
//! `crew_tools_*` test binaries, which install the host in a process of their own.

use super::catalog::{self, Kind, ACTIONS};
use super::*;
use biorouter::crew::tool_host::{PersonTool, CHANGE_ACTIONS, READ_ACTIONS};

/// Every action of both tools has exactly one entry, in the tools' own order, so a call can
/// never name an action the host does not know.
#[test]
fn the_table_names_every_action_once() {
    assert!(catalog::matches_the_tools());
    assert_eq!(ACTIONS.len(), READ_ACTIONS.len() + CHANGE_ACTIONS.len());
    for action in ACTIONS {
        assert_eq!(
            catalog::find(action.tool, action.name).map(|found| found.name),
            Some(action.name)
        );
    }
}

/// Every action AO-12 lists runs in a chat: none answers that it is not available yet.
#[test]
fn every_action_runs() {
    let missing: Vec<&str> = ACTIONS
        .iter()
        .filter(|action| action.run.is_none())
        .map(|action| action.name)
        .collect();
    assert!(missing.is_empty(), "no runner for {missing:?}");
}

/// `describe` answers every action with a schema whose example passes the action's own check.
#[test]
fn every_action_is_described_with_an_example_that_fits_it() {
    for action in ACTIONS {
        let described = catalog::describe(action);
        assert_eq!(described["action"], action.name);
        assert_eq!(described["tool"], action.tool.name());
        assert_eq!(described["params"]["type"], "object");
        assert_eq!(described["params"]["additionalProperties"], false);
        assert_eq!(described["available"], action.run.is_some());
        let example = &described["example"]["params"];
        assert!(example.is_object(), "{}: {}", action.name, action.example);
        catalog::check_params(action, example)
            .unwrap_or_else(|error| panic!("{}'s example: {error}", action.name));
        for param in action.params {
            assert!(!param.about.is_empty(), "{}.{}", action.name, param.name);
            assert!(
                !param.about.contains(['\u{2014}', '\u{2013}']),
                "{}.{}: no dashes in user-facing words",
                action.name,
                param.name
            );
        }
        assert!(
            !action.summary.contains(['\u{2014}', '\u{2013}']),
            "{}",
            action.name
        );
    }
}

#[test]
fn parameters_are_held_to_the_table() {
    let setup = catalog::find(PersonTool::Change, "setup").unwrap();
    assert!(catalog::check_params(setup, &json!({"server": "a@b"})).is_ok());
    let unknown = catalog::check_params(setup, &json!({"sever": "a@b"})).unwrap_err();
    assert!(unknown.contains("takes no parameter sever"), "{unknown}");
    assert!(unknown.contains("describe"), "{unknown}");
    let wrong = catalog::check_params(setup, &json!({"keep_older": "yes"})).unwrap_err();
    assert!(wrong.contains("true or false"), "{wrong}");
    let rename = catalog::find(PersonTool::Change, "workspace_rename").unwrap();
    let missing = catalog::check_params(rename, &json!({})).unwrap_err();
    assert!(missing.contains("needs name"), "{missing}");
    assert!(catalog::check_params(rename, &json!({"name": null})).is_err());
    let access = catalog::find(PersonTool::Change, "connections_set_access").unwrap();
    assert!(catalog::check_params(access, &json!({"access": "root"})).is_err());
    assert!(catalog::check_params(access, &json!({"access": "full"})).is_ok());
    assert!(catalog::check_params(access, &json!([])).is_err());
    let start = catalog::find(PersonTool::Change, "setup").unwrap();
    assert!(catalog::check_params(start, &json!({"start_workspaces": ["a", 1]})).is_err());
    assert!(matches!(
        start
            .params
            .iter()
            .find(|param| param.name == "start_workspaces")
            .unwrap()
            .kind,
        Kind::Texts
    ));
}

/// A refusal carries its code and, where the person has to act, the step (AO-12, "Signing in
/// stays the person's"). No step ever asks for a secret in the chat.
#[test]
fn a_refusal_names_its_code_and_the_person_s_step() {
    let text = refusal_text(
        Some("crew_ssh_auth_required"),
        "Signing in to leo needs a password.",
        Some("lab"),
    );
    assert!(text.contains("(code: crew_ssh_auth_required)"), "{text}");
    assert!(
        text.contains("biorouter crew auth --connection lab"),
        "{text}"
    );
    for code in [
        "crew_ssh_host_key_unknown",
        "crew_ssh_host_key_changed",
        "crew_credential_store_unavailable",
        "crew_device_key_missing",
        "crew_device_not_recognized",
    ] {
        let step = next_step(code, None).expect(code);
        assert!(!step.to_lowercase().contains("tell me"), "{step}");
    }
    assert!(next_step("crew_ssh_host_key_unknown", None)
        .unwrap()
        .contains("never accepts"));
    assert!(next_step("crew_credential_store_unavailable", None)
        .unwrap()
        .contains("biorouter crew credentials unlock"));
    assert_eq!(refusal_text(None, "Plain. ", None), "Plain.");
}

/// A public call is shown the person's channels that are not protected, the public-safe
/// channels they may join, invitations to teams and to channels it is shown, and references the
/// workspace does not restrict in channels it is shown. What is withheld is counted, and each
/// list's `totals` count is lowered by it, so the resolver reads the narrowed view as whole
/// exactly when the person's own list was whole.
#[test]
fn a_public_call_sees_no_protected_channel() {
    let snapshot = json!({
        "workspace": {"name": "lab", "mode": "public"},
        "channels": [{"id": "c1", "name": "general"}, {"id": "c2", "name": "patients"}],
        "invitations": [
            {"kind": "channel", "target_id": "c2"},
            {"kind": "team", "target_id": "t1"}
        ],
        "protected_channel_ids": ["c2"],
    });
    let scope = ContentScope {
        public_only: true,
        protected_channels: ["c2".to_owned()].into(),
        ..ContentScope::default()
    };
    let visible = public_snapshot(&snapshot, &scope);
    assert_eq!(
        visible["channels"],
        json!([{"id": "c1", "name": "general"}])
    );
    assert_eq!(
        visible["invitations"],
        json!([{"kind": "team", "target_id": "t1"}])
    );
    assert_eq!(
        visible["withheld"],
        json!({"channels": 1, "invitations": 1})
    );
    assert!(visible.get("protected_channel_ids").is_none());
    assert!(visible.get("totals").is_none(), "no count is invented");

    // An open workspace, with counts, what the person may join, an invitation to a channel
    // the person is not in, and references.
    let open = json!({
        "workspace": {"name": "lab", "mode": "public"},
        "channels": [{"id": "c1", "name": "general"}, {"id": "c2", "name": "patients"}],
        "joinable_channels": [
            {"id": "c3", "name": "field-notes", "classification": "public_safe"},
            {"id": "c4", "name": "secret-notes", "classification": "restricted"},
            {"id": "c5", "name": "unsaid"},
        ],
        "invitations": [
            {"kind": "channel", "target_id": "c1"},
            {"kind": "channel", "target_id": "c3"},
            {"kind": "channel", "target_id": "c9", "target_name": "cohort-review"},
            {"kind": "team", "target_id": "t1"},
        ],
        "references": [
            {"id": "r1", "channel_id": "c1", "restricted": false, "label": "plot.png"},
            {"id": "r2", "channel_id": "c1", "restricted": true, "label": "mrns.csv"},
            {"id": "r3", "channel_id": "c2", "restricted": false, "label": "notes.txt"},
            {"id": "r4", "channel_id": "c1", "label": "unmarked.txt"},
        ],
        "protected_channel_ids": ["c2"],
        "totals": {
            "teams": 1, "channels": 2, "joinable_channels": 3, "invitations": 4,
            "references": 4, "runs": 0,
        },
    });
    let visible = public_snapshot(&open, &scope);
    let ids = |key: &str| -> Vec<String> {
        visible[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                entry["id"]
                    .as_str()
                    .or_else(|| entry["target_id"].as_str())
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(ids("channels"), ["c1"]);
    assert_eq!(ids("joinable_channels"), ["c3"]);
    assert_eq!(ids("invitations"), ["c1", "c3", "t1"]);
    assert_eq!(ids("references"), ["r1"]);
    assert_eq!(
        visible["withheld"],
        json!({"channels": 1, "joinable_channels": 2, "invitations": 1, "references": 3})
    );
    assert_eq!(
        visible["totals"],
        json!({
            "teams": 1, "channels": 1, "joinable_channels": 1, "invitations": 3,
            "references": 1, "runs": 0,
        })
    );
    for withheld in [
        "patients",
        "secret-notes",
        "unsaid",
        "cohort-review",
        "mrns",
        "notes.txt",
    ] {
        assert!(
            !visible.to_string().contains(withheld),
            "{withheld}: {visible}"
        );
    }
    // A snapshot that left channels out keeps saying so once narrowed: listed 1 of 2.
    let mut partial = open.clone();
    partial["totals"]["channels"] = json!(3);
    assert_eq!(public_snapshot(&partial, &scope)["totals"]["channels"], 2);
    // A private call's view, totals included, is the snapshot as it came.
    assert_eq!(public_snapshot(&open, &ContentScope::default()), open);
    // A workspace that does not say which channels are protected shows none to a public call.
    let unknown = ContentScope {
        public_only: true,
        all_protected: true,
        ..ContentScope::default()
    };
    assert_eq!(public_snapshot(&snapshot, &unknown)["channels"], json!([]));
    // A private call sees it all.
    assert_eq!(
        public_snapshot(&snapshot, &ContentScope::default()),
        snapshot
    );
}

#[test]
fn the_context_diff_keeps_common_lines_as_context() {
    let ToolPreview::FileEdit {
        lines,
        added,
        removed,
        ..
    } = document_diff("workspace context", "a\nb\nc", "a\nB\nc\nd")
    else {
        panic!("a diff");
    };
    let kinds: Vec<(ToolPreviewLineKind, &str)> = lines
        .iter()
        .map(|line| (line.kind.clone(), line.text.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (ToolPreviewLineKind::Context, "a"),
            (ToolPreviewLineKind::Added, "B"),
            (ToolPreviewLineKind::Removed, "b"),
            (ToolPreviewLineKind::Context, "c"),
            (ToolPreviewLineKind::Added, "d"),
        ]
    );
    assert_eq!((added, removed), (2, 1));
}
