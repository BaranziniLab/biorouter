//! The person tools' machinery that needs no daemon: the action table, parameter checks,
//! `describe`, refusal words, the public narrowing of a snapshot and the context diff. What
//! needs a daemon (approvals, consent, the routes' own functions) is in the
//! `crew_tools_*` test binaries, which install the host in a process of their own.

use super::catalog::{self, Kind, ACTIONS};
use super::*;
use biorouter::crew::tool_host::{PersonTool, CHANGE_ACTIONS, READ_ACTIONS};
use biorouter::crew::SetupWorkspace;

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

/// A public call reads only channels the snapshot lists that are not protected: a channel
/// given by an ID beyond a list cut short is withheld, since whether it is protected can't be
/// read there. A private call's reach is the person's.
#[test]
fn a_public_call_reads_only_channels_the_snapshot_lists() {
    let snapshot = json!({
        "channels": [{"id": "c1", "name": "general"}, {"id": "c2", "name": "patients"}],
        "joinable_channels": [
            {"id": "c3", "classification": "public_safe"},
            {"id": "c4", "classification": "restricted"},
        ],
    });
    let scope = ContentScope {
        public_only: true,
        protected_channels: ["c2".to_owned()].into(),
        listed_channels: listed_channels(&snapshot),
        ..ContentScope::default()
    };
    assert!(scope.channel_allowed("c1"));
    assert!(!scope.channel_allowed("c2"), "protected");
    assert!(
        scope.channel_allowed("c3"),
        "a public-safe channel they may join"
    );
    assert!(
        !scope.channel_allowed("c4"),
        "a restricted one they may join"
    );
    assert!(
        !scope.channel_allowed("c9"),
        "a channel the snapshot does not list"
    );
    assert!(ContentScope::default().channel_allowed("c9"));
}

/// An approval allowed and then stopped says the chat was stopped, never that the approval was
/// allowed and nothing changed.
#[test]
fn an_allowed_approval_that_was_stopped_says_so() {
    use biorouter::permission::Permission;
    let stopped = not_approved(&UserActionOutcome::Approved {
        permission: Permission::AllowOnce,
    });
    assert_eq!(stopped.to_string(), STOPPED_BEFORE_IT_RAN);
    assert!(!stopped.to_string().contains("was allowed"));
    let declined = not_approved(&UserActionOutcome::Denied {
        permission: Permission::DenyOnce,
    });
    assert_eq!(
        declined.to_string(),
        "The person declined. Nothing was changed."
    );
    let expired = not_approved(&UserActionOutcome::TimedOut);
    assert!(
        expired
            .to_string()
            .starts_with("Nothing was changed: the approval expired"),
        "{expired}"
    );
}

/// A refusal that names things from a workspace carries the read's scope, so the host raises
/// the chat before it returns; one that names nothing carries none.
#[test]
fn a_refusal_that_names_things_carries_the_read_s_scope() {
    let scope = ContentScope {
        raise: true,
        institutions: ["ucsf".to_owned()].into(),
        ..ContentScope::default()
    };
    let named = naming(said(Some("unknown_name"), "Did you mean #methods?"), &scope);
    let found = named.downcast_ref::<Said>().expect("still a Said");
    assert_eq!(found.code, Some("unknown_name"));
    assert!(found.names_from.as_ref().is_some_and(|scope| scope.raise));
    assert!(said(None, "plain")
        .downcast_ref::<Said>()
        .is_some_and(|said| said.names_from.is_none()));
    // Anything else is passed on as it is.
    let other = naming(anyhow::anyhow!("broker"), &scope);
    assert!(other.downcast_ref::<Said>().is_none());
}

fn resolved(id: &str, label: &str) -> Resolved {
    Resolved {
        id: id.to_owned(),
        label: label.to_owned(),
        username: None,
    }
}

/// A direct add refused part way names the person and channels only to a call that may be
/// shown names from the workspace, and then only as content; any other call is told how many.
#[test]
fn a_direct_add_refused_part_way_names_nothing_to_a_call_that_may_not_read() {
    let person = resolved("p-bob", "Bob Lee (@bob)");
    let channels = [
        resolved("c-1", "#methods"),
        resolved("c-2", "#patients"),
        resolved("c-3", "#cohort"),
    ];
    let reason = anyhow::anyhow!("You can't add people to that channel.");
    let withheld = super::channels::stopped_adding(None, &person, &channels, 1, &reason);
    let text = withheld.to_string();
    assert_eq!(
        text,
        "Added them to 1 of the 3 channels, then one was refused: You can't add people to that channel."
    );
    for name in ["Bob", "bob", "#methods", "#patients"] {
        assert!(!text.contains(name), "{name}: {text}");
    }
    let scope = ContentScope {
        raise: true,
        ..ContentScope::default()
    };
    let named = super::channels::stopped_adding(Some(&scope), &person, &channels, 1, &reason);
    assert!(
        named
            .to_string()
            .starts_with("Added Bob Lee (@bob) to #methods, then stopped at #patients"),
        "{named}"
    );
    assert!(named
        .downcast_ref::<Said>()
        .is_some_and(|said| said.names_from.is_some()));
}

/// A grant's labels go to a call only under a scope that may name things from the workspace;
/// to a public call, only for channels it may read.
#[test]
fn a_grant_s_labels_follow_what_the_call_may_read() {
    let labels = json!({
        "you": "Alice Chen (@alice)",
        "workspace": "lab",
        "destination": {"channel_id": "c1", "label": "#general", "team": "Analysis Lab"},
        "sources": [
            {"channel_id": "c1", "label": "#general"},
            {"channel_id": "c2", "label": "#patients"},
        ],
    });
    assert_eq!(super::workspace::grant_labels(&labels, None), Value::Null);
    let private = ContentScope {
        raise: true,
        ..ContentScope::default()
    };
    assert_eq!(
        super::workspace::grant_labels(&labels, Some(&private)),
        labels
    );
    let public = ContentScope {
        public_only: true,
        protected_channels: ["c2".to_owned()].into(),
        listed_channels: ["c1".to_owned(), "c2".to_owned()].into(),
        ..ContentScope::default()
    };
    let kept = super::workspace::grant_labels(&labels, Some(&public));
    assert_eq!(
        kept["sources"],
        json!([{"channel_id": "c1", "label": "#general"}])
    );
    assert!(!kept.to_string().contains("patients"), "{kept}");
    let protected_destination = ContentScope {
        protected_channels: ["c1".to_owned()].into(),
        ..public
    };
    assert_eq!(
        super::workspace::grant_labels(&labels, Some(&protected_destination)),
        Value::Null
    );
}

fn listed_workspace(name: &str, version: &str, binary_current: bool) -> SetupWorkspace {
    use biorouter::crew::{SetupThisComputer, SetupWorkspaceRole, SetupWorkspaceState};
    SetupWorkspace {
        workspace_id: format!("{name}-id"),
        name: Some(name.to_owned()),
        role: SetupWorkspaceRole::Host,
        host_uid: Some(1000),
        host_username: Some("alice".to_owned()),
        socket_path: None,
        workspace_key_fingerprint: Some("AAAA BBBB CCCC DDDD".to_owned()),
        mode: Some(ClusterMode::Private),
        institution_id: Some("ucsf".to_owned()),
        state: SetupWorkspaceState::Running,
        version: Some(version.to_owned()),
        binary_current: Some(binary_current),
        state_dir: Some("~/.local/share/biorouter-crew/lab".to_owned()),
        found_by: None,
        invitation: Some("brcrew1:fixture".to_owned()),
        error: None,
        others_connected: Some(0),
        saved_connection_id: None,
        this_computer: SetupThisComputer::NotSaved,
    }
}

/// A setup's card names the workspaces it will restart, and who is connected, even when the
/// listing it reads was taken before the setup installs this app's program: then every hosted
/// workspace running an older Crew is one it restarts, though the listing found each on the
/// program installed at the time.
#[test]
fn a_setup_card_names_what_it_restarts_after_installing() {
    use super::setup::planned_restarts;
    let mut lab = listed_workspace("lab", "1.94.0", true);
    lab.others_connected = Some(3);
    let quiet = listed_workspace("quiet", "1.94.0", true);
    let mut unknown = listed_workspace("old-lab", "1.93.0", true);
    unknown.others_connected = None;
    let newer = listed_workspace("newer", "1.96.0", true);
    let listing = [lab, quiet, unknown, newer];

    let lines = planned_restarts(Some(&listing), Some("1.95.0"), None, &[]);
    assert_eq!(
        lines,
        [
            "It leaves lab running the older Crew: 3 other people are connected.",
            "It restarts quiet, which nobody else is connected to.",
            "It leaves old-lab running the older Crew: its server can't say who is connected.",
        ],
        "{lines:?}"
    );
    let forced = planned_restarts(Some(&listing), Some("1.95.0"), Some(true), &[]);
    assert!(
        forced[0].starts_with("It restarts lab: 3 other people are connected"),
        "{forced:?}"
    );
    assert!(
        forced[2].starts_with("It restarts old-lab. Its server can't say who is connected"),
        "{forced:?}"
    );
    // Nothing installed: only what the listing found on an older program.
    assert!(planned_restarts(Some(&listing), None, None, &[]).is_empty());
    for line in lines.iter().chain(&forced) {
        assert!(!line.contains(['\u{2014}', '\u{2013}']), "{line}");
    }
}

fn saved(name: &str, ssh_target: &str) -> Connection {
    serde_json::from_value(json!({
        "id": format!("{name}-id"), "name": name, "ssh_target": ssh_target, "port": null,
        "identity_file": null, "proxy_jump": null, "socket_path": "/tmp/crew.sock",
        "owner_uid": 1000, "workspace_id": "lab-id", "workspace_public_key": "00",
        "cluster_connection_id": "cluster", "mode": "private", "policy_epoch": 1,
        "status": "connected", "last_error": null, "device_id": "11", "public_key": "22",
    }))
    .expect("a saved connection")
}

/// Opening a workspace this computer already saved under another login says so on its card
/// and names that connection, rather than naming the checked login and its role.
#[test]
fn an_open_through_another_login_s_connection_says_so() {
    use super::setup::open_card;
    let entry = listed_workspace("lab", "1.95.0", true);
    let fresh = open_card(&entry, "wanjun@leo", None, None);
    assert!(fresh
        .lines
        .iter()
        .any(|line| line == "You join it as its host."));

    let other = saved("lab-gb", "gbellucci@leo");
    let reused = open_card(&entry, "wanjun@leo", Some("full"), Some(&other));
    assert!(
        reused.lines.iter().any(|line| line
            == "It uses the connection already saved on this computer as lab-gb, which signs in as gbellucci@leo, not wanjun@leo."),
        "{:?}",
        reused.lines
    );
    assert!(
        !reused
            .lines
            .iter()
            .any(|line| line.starts_with("You join it as")),
        "{:?}",
        reused.lines
    );
    assert_eq!(reused.payload["connection"], "lab-gb");
    assert!(
        reused
            .lines
            .iter()
            .any(|line| line.starts_with("Agents get full access")
                && line.contains("on gbellucci@leo")),
        "agent access is set on the connection the open uses: {:?}",
        reused.lines
    );

    let same = saved("lab", "wanjun@leo");
    let kept = open_card(&entry, "wanjun@leo", None, Some(&same));
    assert!(kept
        .lines
        .iter()
        .any(|line| line == "It uses the connection already saved for it on this computer."));
    assert!(kept
        .lines
        .iter()
        .any(|line| line == "You join it as its host."));
}
