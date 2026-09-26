use super::*;
use serde_json::json;

const ALICE: &str = "0b9d7a52-5f3e-4f65-9d2c-5a1d1f0e0001";
const BOB: &str = "0b9d7a52-5f3e-4f65-9d2c-5a1d1f0e0002";
const CAROL: &str = "0b9d7a52-5f3e-4f65-9d2c-5a1d1f0e0003";
const SPARK: &str = "0b9d7a52-5f3e-4f65-9d2c-5a1d1f0e0004";
const SAMPARK: &str = "0b9d7a52-5f3e-4f65-9d2c-5a1d1f0e0005";
const DAVE: &str = "0b9d7a52-5f3e-4f65-9d2c-5a1d1f0e0006";
const LAB: &str = "7c1e0f8a-4a55-4b8e-8f0e-000000000001";
const IMAGING: &str = "7c1e0f8a-4a55-4b8e-8f0e-000000000002";
const METHODS: &str = "3a6f2c9e-1d4b-4c1a-9e7d-000000000001";
const LAB_GENERAL: &str = "3a6f2c9e-1d4b-4c1a-9e7d-000000000002";
const IMAGING_GENERAL: &str = "3a6f2c9e-1d4b-4c1a-9e7d-000000000003";
const RAW_DATA: &str = "3a6f2c9e-1d4b-4c1a-9e7d-000000000004";
/// Not in any snapshot below.
const HIDDEN_TEAM: &str = "7c1e0f8a-4a55-4b8e-8f0e-00000000ffff";

fn principal(id: &str, username: &str, display_name: &str) -> Value {
    json!({"id": id, "uid": 1000, "username": username, "nickname": display_name,
        "display_name": display_name, "avatar": null, "active": true})
}

/// Alice's snapshot: two teams that both have a `#general`, two people named Sam Park, a
/// person whose display name is their username, and a former member.
fn snapshot() -> Value {
    json!({
        "workspace": {"id": "workspace", "host_uid": 1000, "host_principal_id": ALICE},
        "actor": principal(ALICE, "alice", "Alice Chen"),
        "principals": [
            principal(ALICE, "alice", "Alice Chen"),
            principal(BOB, "bob", "Bob Lee"),
            principal(CAROL, "carol", "carol"),
            principal(SPARK, "spark", "Sam Park"),
            principal(SAMPARK, "sampark", "Sam Park"),
        ],
        "former_principals": [
            {"id": DAVE, "username": "dave", "display_name": "Dave Old", "avatar": null, "active": false},
        ],
        "teams": [
            {"id": LAB, "name": "Analysis Lab", "handle": "analysis-lab", "display_name": "Analysis Lab"},
            {"id": IMAGING, "name": "Imaging Core", "handle": "imaging-core", "display_name": "Imaging Core"},
        ],
        "channels": [
            {"id": METHODS, "team_id": LAB, "name": "methods", "handle": "methods", "archived": false},
            {"id": LAB_GENERAL, "team_id": LAB, "name": "general", "handle": "general", "archived": false},
            {"id": IMAGING_GENERAL, "team_id": IMAGING, "name": "general", "handle": "general", "archived": false},
            {"id": RAW_DATA, "team_id": IMAGING, "name": "raw-data", "handle": "raw-data", "archived": true},
        ],
    })
}

fn selector(kind: Option<SelectorKind>, text: &str) -> SelectorInput {
    SelectorInput {
        kind,
        text: text.into(),
    }
}

fn resolve_one(snapshot: &Value, kind: Option<SelectorKind>, text: &str) -> Resolution {
    let mut results = resolve_all(Some(snapshot), &[], &[selector(kind, text)]);
    assert_eq!(results.len(), 1);
    results.remove(0)
}

fn resolved_id(resolution: &Resolution) -> &str {
    match resolution {
        Resolution::Resolved { id, .. } => id,
        other => panic!("expected a resolution, got {other:?}"),
    }
}

fn candidates(resolution: &Resolution) -> Vec<String> {
    match resolution {
        Resolution::AmbiguousName { candidates, .. } => candidates.clone(),
        other => panic!("expected an ambiguous name, got {other:?}"),
    }
}

fn assert_unknown(resolution: &Resolution, hint: Option<&str>) {
    match resolution {
        Resolution::UnknownName { did_you_mean, .. } => {
            assert_eq!(did_you_mean.as_deref(), hint, "{resolution:?}")
        }
        other => panic!("expected an unknown name, got {other:?}"),
    }
}

#[test]
fn grammar_reads_sigils_quotes_and_qualified_channels() {
    let snapshot = snapshot();
    use SelectorKind::*;
    for (kind, text, expected) in [
        (None, "@bob", BOB),
        (Some(Person), "@bob", BOB),
        (Some(Person), "bob", BOB),
        (Some(Person), "  @bob  ", BOB),
        (None, "methods", METHODS),
        (None, "#methods", METHODS),
        (Some(Channel), "\"#methods\"", METHODS),
        (Some(Channel), "analysis-lab/methods", METHODS),
        (Some(Channel), "analysis-lab/#methods", METHODS),
        (Some(Channel), "\"Analysis Lab\"/general", LAB_GENERAL),
        (Some(Channel), "imaging-core/general", IMAGING_GENERAL),
        (Some(Team), "analysis-lab", LAB),
        (Some(Team), "\"Analysis Lab\"", LAB),
        (Some(Team), "'Analysis Lab'", LAB),
        (Some(Team), "\u{201C}Analysis Lab\u{201D}", LAB),
        (Some(Team), "ANALYSIS_LAB", LAB),
        (Some(Team), "Analysis.Lab", LAB),
        (Some(Team), "Ａｎａｌｙｓｉｓ Ｌａｂ", LAB),
    ] {
        let resolution = resolve_one(&snapshot, kind, text);
        assert_eq!(resolved_id(&resolution), expected, "{kind:?} {text:?}");
        match &resolution {
            Resolution::Resolved { text: echoed, .. } => assert_eq!(echoed, text),
            _ => unreachable!(),
        }
    }
}

#[test]
fn a_resolved_person_carries_the_authority_label_and_canonical_username() {
    let resolution = resolve_one(&snapshot(), Some(SelectorKind::Person), "@bob");
    assert_eq!(
        resolution,
        Resolution::Resolved {
            kind: SelectorKind::Person,
            text: "@bob".into(),
            id: BOB.into(),
            label: Some("Bob Lee (@bob)".into()),
            username: Some("bob".into()),
        }
    );
    // The authority form names both even when the display name is the username.
    let carol = resolve_one(&snapshot(), Some(SelectorKind::Person), "@carol");
    match carol {
        Resolution::Resolved { label, .. } => assert_eq!(label.as_deref(), Some("carol (@carol)")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn people_match_the_exact_username_and_a_case_difference_is_only_suggested() {
    let snapshot = snapshot();
    let near = resolve_one(&snapshot, Some(SelectorKind::Person), "@Bob");
    assert_unknown(&near, Some("@bob"));
    let near = resolve_one(&snapshot, None, "@BOB");
    assert_unknown(&near, Some("@bob"));
    // A name-key match is not a username match either.
    assert_unknown(
        &resolve_one(&snapshot, Some(SelectorKind::Person), "@b-o-b"),
        None,
    );
}

#[test]
fn a_display_name_never_selects_a_person() {
    let snapshot = snapshot();
    for text in [
        "Bob Lee",
        "@Bob Lee",
        "\"Bob Lee\"",
        "Alice Chen",
        "Sam Park",
    ] {
        for kind in [Some(SelectorKind::Person), Some(SelectorKind::FormerPerson)] {
            let resolution = resolve_one(&snapshot, kind, text);
            assert_unknown(&resolution, None);
        }
    }
    // Even kind-less, `@` plus a display name is a (missing) username, not the person.
    assert_unknown(&resolve_one(&snapshot, None, "@Alice Chen"), None);
}

#[test]
fn former_members_resolve_only_as_former_people() {
    let snapshot = snapshot();
    let former = resolve_one(&snapshot, Some(SelectorKind::FormerPerson), "@dave");
    match former {
        Resolution::Resolved {
            id,
            label,
            username,
            ..
        } => {
            assert_eq!(id, DAVE);
            assert_eq!(label.as_deref(), Some("Dave Old (@dave) · former member"));
            assert_eq!(username.as_deref(), Some("dave"));
        }
        other => panic!("{other:?}"),
    }
    assert_unknown(
        &resolve_one(&snapshot, Some(SelectorKind::Person), "@dave"),
        None,
    );
    assert_unknown(
        &resolve_one(&snapshot, Some(SelectorKind::FormerPerson), "@bob"),
        None,
    );
}

#[test]
fn general_is_ambiguous_across_teams_and_lists_qualified_labels() {
    let snapshot = snapshot();
    for text in ["general", "#general", "\"#general\""] {
        let resolution = resolve_one(&snapshot, Some(SelectorKind::Channel), text);
        assert_eq!(
            candidates(&resolution),
            vec!["Analysis Lab / #general", "Imaging Core / #general"]
        );
    }
    // A slug shared across teams is qualified in a resolved label too.
    match resolve_one(
        &snapshot,
        Some(SelectorKind::Channel),
        "analysis-lab/general",
    ) {
        Resolution::Resolved { label, .. } => {
            assert_eq!(label.as_deref(), Some("Analysis Lab / #general"))
        }
        other => panic!("{other:?}"),
    }
    match resolve_one(&snapshot, Some(SelectorKind::Channel), "#methods") {
        Resolution::Resolved { label, .. } => assert_eq!(label.as_deref(), Some("#methods")),
        other => panic!("{other:?}"),
    }
    match resolve_one(&snapshot, Some(SelectorKind::Channel), "raw_data") {
        Resolution::Resolved { label, .. } => {
            assert_eq!(label.as_deref(), Some("#raw-data · archived"))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_name_lists_nothing() {
    let snapshot = snapshot();
    for (kind, text) in [
        (Some(SelectorKind::Channel), "#nonexistent"),
        (Some(SelectorKind::Team), "Secret Team"),
        (Some(SelectorKind::Channel), "secret-team/general"),
        (Some(SelectorKind::Person), "@mallory"),
        (Some(SelectorKind::Team), "---"),
    ] {
        let resolution = resolve_one(&snapshot, kind, text);
        assert_unknown(&resolution, None);
        let wire = serde_json::to_value(&resolution).unwrap();
        assert_eq!(wire["status"], "unknown_name");
        assert!(wire.get("candidates").is_none(), "{wire}");
        assert!(wire.get("id").is_none(), "{wire}");
        assert!(wire.get("did_you_mean").is_none(), "{wire}");
    }
}

#[test]
fn candidates_come_only_from_the_callers_snapshot() {
    // Bob's snapshot shows only Imaging Core; Alice's Analysis Lab exists but is not his.
    let mut bobs = snapshot();
    bobs["teams"] = json!([
        {"id": IMAGING, "name": "Imaging Core", "handle": "imaging-core", "display_name": "Imaging Core"},
    ]);
    bobs["channels"] = json!([
        {"id": IMAGING_GENERAL, "team_id": IMAGING, "name": "general", "handle": "general"},
    ]);
    assert_unknown(
        &resolve_one(&bobs, Some(SelectorKind::Team), "analysis-lab"),
        None,
    );
    assert_unknown(
        &resolve_one(&bobs, Some(SelectorKind::Channel), "#methods"),
        None,
    );
    assert_eq!(
        resolved_id(&resolve_one(&bobs, Some(SelectorKind::Channel), "#general")),
        IMAGING_GENERAL
    );
    // Every candidate the full snapshot offers is one of its own labels.
    let offered = candidates(&resolve_one(
        &snapshot(),
        Some(SelectorKind::Channel),
        "general",
    ));
    assert!(offered
        .iter()
        .all(|label| label.ends_with("#general") && !label.contains(LAB_GENERAL)));
}

#[test]
fn uuid_shaped_text_is_always_an_id() {
    let mut snapshot = snapshot();
    // A legacy channel whose NAME is UUID-shaped: the text names the ID, never this channel.
    let legacy_name = "123e4567-e89b-12d3-a456-426614174000";
    snapshot["channels"].as_array_mut().unwrap().push(json!({
        "id": "3a6f2c9e-1d4b-4c1a-9e7d-00000000aaaa", "team_id": LAB,
        "name": legacy_name, "handle": legacy_name,
    }));
    let resolution = resolve_one(&snapshot, Some(SelectorKind::Channel), legacy_name);
    assert_eq!(
        resolution,
        Resolution::Resolved {
            kind: SelectorKind::Channel,
            text: legacy_name.into(),
            id: legacy_name.into(),
            label: None,
            username: None,
        }
    );
    // A visible ID is labelled; an ID the snapshot does not show passes through unlabelled.
    match resolve_one(
        &snapshot,
        Some(SelectorKind::Channel),
        &format!("#{METHODS}"),
    ) {
        Resolution::Resolved { id, label, .. } => {
            assert_eq!(id, METHODS);
            assert_eq!(label.as_deref(), Some("#methods"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        resolve_one(&snapshot, Some(SelectorKind::Team), HIDDEN_TEAM),
        Resolution::Resolved {
            kind: SelectorKind::Team,
            text: HIDDEN_TEAM.into(),
            id: HIDDEN_TEAM.into(),
            label: None,
            username: None,
        }
    );
    // `@` plus a UUID is a person's ID, with its authority label.
    match resolve_one(&snapshot, Some(SelectorKind::Person), &format!("@{BOB}")) {
        Resolution::Resolved { id, username, .. } => {
            assert_eq!(id, BOB);
            assert_eq!(username.as_deref(), Some("bob"));
        }
        other => panic!("{other:?}"),
    }
    // Without a kind, the snapshot says what the ID is; an ID it does not show is unknown.
    match resolve_one(&snapshot, None, IMAGING) {
        Resolution::Resolved { kind, id, .. } => {
            assert_eq!(kind, SelectorKind::Team);
            assert_eq!(id, IMAGING);
        }
        other => panic!("{other:?}"),
    }
    assert_unknown(&resolve_one(&snapshot, None, HIDDEN_TEAM), None);
    let hex = "a".repeat(64);
    assert_eq!(
        resolved_id(&resolve_one(&snapshot, Some(SelectorKind::Channel), &hex)),
        hex
    );
}

#[test]
fn a_handle_the_daemon_disagrees_with_is_ambiguous() {
    let mut snapshot = snapshot();
    // The broker's Unicode tables produced a different handle than the daemon's own key.
    snapshot["teams"][0]["handle"] = json!("analysis-lab-2");
    for text in ["analysis-lab", "analysis-lab-2", "Analysis Lab"] {
        let resolution = resolve_one(&snapshot, Some(SelectorKind::Team), text);
        assert_eq!(candidates(&resolution), vec!["Analysis Lab"], "{text}");
    }
    // A channel qualified by that team is ambiguous for the same reason.
    let resolution = resolve_one(
        &snapshot,
        Some(SelectorKind::Channel),
        "analysis-lab/methods",
    );
    assert_eq!(candidates(&resolution), vec!["Analysis Lab / #methods"]);
    // And a channel whose own handle disagrees.
    snapshot["channels"][0]["handle"] = json!("methods-2");
    let resolution = resolve_one(&snapshot, Some(SelectorKind::Channel), "#methods");
    assert_eq!(candidates(&resolution), vec!["Analysis Lab / #methods"]);
    // An older broker projects no handle: the daemon's own key is used.
    let mut older = self::snapshot();
    for team in older["teams"].as_array_mut().unwrap() {
        team.as_object_mut().unwrap().remove("handle");
    }
    assert_eq!(
        resolved_id(&resolve_one(
            &older,
            Some(SelectorKind::Team),
            "analysis-lab"
        )),
        LAB
    );
}

#[test]
fn legacy_duplicates_are_ambiguous_with_distinct_candidates() {
    let mut snapshot = snapshot();
    let principals = snapshot["principals"].as_array_mut().unwrap();
    let mut stale = principal("0b9d7a52-5f3e-4f65-9d2c-5a1d1f0e0099", "bob", "Bob Lee");
    stale["account_stale"] = json!(true);
    principals.push(stale);
    let resolution = resolve_one(&snapshot, Some(SelectorKind::Person), "@bob");
    assert_eq!(
        candidates(&resolution),
        vec!["Bob Lee (@bob)", "Bob Lee (@bob) · account no longer valid"]
    );
    // Without the host's flag the two lines are identical, so they are numbered.
    snapshot["principals"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["account_stale"] = json!(false);
    let resolution = resolve_one(&snapshot, Some(SelectorKind::Person), "@bob");
    assert_eq!(
        candidates(&resolution),
        vec!["Bob Lee (@bob) (1 of 2)", "Bob Lee (@bob) (2 of 2)"]
    );
    // Teams named `Lab` and `lab` in a legacy journal.
    snapshot["teams"] = json!([
        {"id": LAB, "name": "Lab", "handle": "lab"},
        {"id": IMAGING, "name": "lab", "handle": "lab"},
    ]);
    let resolution = resolve_one(&snapshot, Some(SelectorKind::Team), "LAB");
    assert_eq!(candidates(&resolution), vec!["Lab", "lab"]);
}

#[test]
fn labels_follow_the_display_rule_and_the_collision_rule() {
    let labels = project_labels(&snapshot());
    let label = |id: &str| labels.get(id).cloned().expect("labelled");
    assert_eq!(
        label(BOB),
        PersonLabel {
            full: "Bob Lee (@bob)".into(),
            short: "Bob Lee".into(),
            collides: false,
        }
    );
    assert_eq!(
        label(CAROL),
        PersonLabel {
            full: "@carol".into(),
            short: "@carol".into(),
            collides: false,
        }
    );
    for (id, username) in [(SPARK, "spark"), (SAMPARK, "sampark")] {
        let both = format!("Sam Park (@{username})");
        assert_eq!(
            label(id),
            PersonLabel {
                full: both.clone(),
                short: both,
                collides: true,
            }
        );
    }
    // The former member and the host are in the directory too.
    assert_eq!(label(DAVE).full, "Dave Old (@dave)");
    assert_eq!(label(ALICE).short, "Alice Chen");
    assert_eq!(labels.len(), 6);
}

#[test]
fn labels_collide_on_lookalike_and_equivalent_names() {
    let collide = |first: &str, second: &str| {
        let snapshot = json!({
            "actor": principal(ALICE, "alice", "Alice Chen"),
            "principals": [
                principal(ALICE, "alice", "Alice Chen"),
                principal(BOB, "bob", first),
                principal(CAROL, "carol", second),
            ],
        });
        let labels = project_labels(&snapshot);
        let (bob, carol) = (&labels[BOB], &labels[CAROL]);
        assert_eq!(bob.collides, carol.collides);
        assert!(!labels[ALICE].collides);
        bob.collides
    };
    // Cyrillic `а` in the second name: the confusable skeleton matches.
    assert!(collide("Sam Park", "Sam P\u{0430}rk"));
    // Case, width and separators: the name key matches.
    assert!(collide("Sam Park", "SAM_PARK"));
    assert!(collide("Sam Park", "Ｓａｍ Ｐａｒｋ"));
    // A display name equal to another person's username is shown as the username instead,
    // so it cannot pass for them, and then nothing collides.
    assert!(!collide("Sam Park", "Sam Parker"));
    let snapshot = json!({
        "principals": [principal(BOB, "bob", "alice"), principal(ALICE, "alice", "Alice Chen")],
    });
    assert_eq!(project_labels(&snapshot)[BOB].full, "@bob");
}

#[test]
fn labels_strip_characters_that_could_reorder_the_text() {
    let snapshot = json!({
        "principals": [
            {"id": BOB, "username": "bob", "nickname": "Bob\u{202E} Lee", "active": true},
        ],
    });
    let labels = project_labels(&snapshot);
    assert_eq!(labels[BOB].full, "Bob Lee (@bob)");
    assert!(!labels[BOB].full.contains('\u{202E}'));
}

fn connection(id: &str, name: &str, ssh_target: &str) -> Connection {
    Connection {
        id: id.into(),
        node_id: None,
        name: name.into(),
        ssh_target: ssh_target.into(),
        port: None,
        identity_file: None,
        proxy_jump: None,
        socket_path: "/run/crew.sock".into(),
        owner_uid: 1000,
        workspace_id: "workspace".into(),
        workspace_public_key: "00".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: id.into(),
        mode: biorouter::crew::ClusterMode::Private,
        institution_id: None,
        policy_epoch: 1,
        status: "connected".into(),
        last_error: None,
        device_id: "device".into(),
        public_key: "key".into(),
    }
}

const HPC: &str = "9d4e5a1b-2c3d-4e5f-8a9b-000000000001";
const LAB_SERVER: &str = "9d4e5a1b-2c3d-4e5f-8a9b-000000000002";
const LAB_BACKUP: &str = "9d4e5a1b-2c3d-4e5f-8a9b-000000000003";

fn connections() -> Vec<Connection> {
    vec![
        connection(HPC, "UCSF HPC", "bob@hpc.ucsf.edu"),
        connection(LAB_SERVER, "Lab", "bob@lab.example.org"),
        connection(LAB_BACKUP, "lab", "bob@backup.example.org"),
    ]
}

#[test]
fn connections_resolve_by_name_target_or_id_and_shared_names_are_ambiguous() {
    let saved = connections();
    for text in [
        "UCSF HPC",
        "ucsf hpc",
        "\"UCSF HPC\"",
        "bob@hpc.ucsf.edu",
        HPC,
    ] {
        let resolution = resolve_connection(&saved, text);
        assert_eq!(resolved_id(&resolution), HPC, "{text}");
        match resolution {
            Resolution::Resolved { label, .. } => assert_eq!(label.as_deref(), Some("UCSF HPC")),
            _ => unreachable!(),
        }
    }
    assert_eq!(
        candidates(&resolve_connection(&saved, "LAB")),
        vec!["Lab — bob@lab.example.org", "lab — bob@backup.example.org"]
    );
    match resolve_connection(&saved, "bob@lab.example.org") {
        Resolution::Resolved { id, label, .. } => {
            assert_eq!(id, LAB_SERVER);
            assert_eq!(label.as_deref(), Some("Lab — bob@lab.example.org"));
        }
        other => panic!("{other:?}"),
    }
    // A leading `@` always selects a person; an unknown ID is unknown.
    assert_unknown(&resolve_connection(&saved, "@hpc"), None);
    assert_unknown(&resolve_connection(&saved, HIDDEN_TEAM), None);
    assert_unknown(&resolve_connection(&saved, "HPC"), None);
}

#[test]
fn the_connection_is_named_or_the_only_one_saved() {
    let saved = connections();
    let (named, id) = choose_connection(&saved, Some("UCSF HPC"), true).unwrap();
    assert_eq!(id.as_deref(), Some(HPC));
    assert!(matches!(named, Some(Resolution::Resolved { .. })));
    assert!(matches!(
        choose_connection(&saved, Some("lab"), true),
        Err(ResolveRefusal::Connection(Resolution::AmbiguousName { .. }))
    ));
    assert!(matches!(
        choose_connection(&saved, Some("nowhere"), false),
        Err(ResolveRefusal::Connection(Resolution::UnknownName { .. }))
    ));
    assert!(matches!(
        choose_connection(&saved, None, true),
        Err(ResolveRefusal::ConnectionRequired(_))
    ));
    assert!(matches!(
        choose_connection(&[], None, true),
        Err(ResolveRefusal::ConnectionRequired(_))
    ));
    assert_eq!(
        choose_connection(&saved[..1], None, true).unwrap(),
        (None, Some(HPC.to_owned()))
    );
    // Nothing needs a workspace: nothing is chosen.
    assert_eq!(
        choose_connection(&saved, None, false).unwrap(),
        (None, None)
    );
}

#[test]
fn connection_selectors_need_no_snapshot() {
    let saved = connections();
    let selectors = [selector(Some(SelectorKind::Connection), "UCSF HPC")];
    assert!(!needs_snapshot(&selectors));
    let results = resolve_all(None, &saved, &selectors);
    assert_eq!(resolved_id(&results[0]), HPC);
    assert!(needs_snapshot(&[selector(None, "@bob")]));
    // A workspace selector with no snapshot to consult is unknown, never guessed.
    let results = resolve_all(None, &saved, &[selector(Some(SelectorKind::Team), "lab")]);
    assert_unknown(&results[0], None);
}

#[test]
fn requests_are_bounded_before_anything_is_looked_up() {
    let request = |connection: Option<&str>, selectors: Vec<SelectorInput>| ResolveRequest {
        connection: connection.map(str::to_owned),
        selectors,
    };
    assert!(validate_request(&request(Some("lab"), vec![selector(None, "@bob")])).is_ok());
    for invalid in [
        request(
            None,
            vec![selector(Some(SelectorKind::Attachment), "counts.csv")],
        ),
        request(None, vec![selector(None, "  ")]),
        request(None, vec![selector(None, "\"\"")]),
        request(Some(""), vec![]),
        request(None, vec![selector(None, "#meth\u{7}ods")]),
        request(
            None,
            vec![selector(None, &"a".repeat(MAX_SELECTOR_BYTES + 1))],
        ),
        request(
            None,
            (0..=MAX_SELECTORS)
                .map(|_| selector(None, "@bob"))
                .collect(),
        ),
    ] {
        assert!(matches!(
            validate_request(&invalid),
            Err(ResolveRefusal::Invalid(_))
        ));
    }
    // The wire shape the renderer parses.
    let parsed: ResolveRequest = serde_json::from_value(json!({
        "connection": "lab", "selectors": [{"kind": "former_person", "text": "@dave"}, {"text": "#methods"}]
    }))
    .unwrap();
    assert_eq!(parsed.selectors[0].kind, Some(SelectorKind::FormerPerson));
    assert_eq!(parsed.selectors[1].kind, None);
    assert!(
        serde_json::from_value::<ResolveRequest>(json!({"selectors": [], "extra": 1})).is_err()
    );
}

#[test]
fn resolutions_serialize_with_a_status_tag() {
    let results = resolve_all(
        Some(&snapshot()),
        &[],
        &[
            selector(Some(SelectorKind::Person), "@bob"),
            selector(Some(SelectorKind::Person), "@Bob"),
            selector(Some(SelectorKind::Channel), "general"),
        ],
    );
    let wire = serde_json::to_value(ResolveResponse {
        connection: None,
        results,
    })
    .unwrap();
    assert_eq!(wire["connection"], Value::Null);
    assert_eq!(
        wire["results"][0],
        json!({"status": "resolved", "kind": "person", "text": "@bob", "id": BOB,
            "label": "Bob Lee (@bob)", "username": "bob"})
    );
    assert_eq!(
        wire["results"][1],
        json!({"status": "unknown_name", "kind": "person", "text": "@Bob", "did_you_mean": "@bob"})
    );
    assert_eq!(wire["results"][2]["status"], "ambiguous_name");
    assert_eq!(wire["results"][2]["kind"], "channel");
}
