//! Contract for the shared naming library: name keys and validators (S1a, S2a, S2b), the
//! device code and the workspace invitation codec (S3a), and the `hello` signing payloads.
//!
//! Pure functions only: no broker, socket or account lookup, so this runs on every platform.
//! See `docs/research/biorouter-crew/naming-design.md`.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use biorouter_crew::invitation::{
    self, InvitationError, InvitationField, InvitationSource, WorkspaceInvitation,
};
use biorouter_crew::names::{self, NameKind, NameProblem};
use biorouter_crew::{
    canonical_channel_name, device_code, device_code_from_hex, device_code_matches,
    format_device_code, hello_v1_payload, is_uuid_shaped, name_key, normalize_device_code,
    restriction_level_ok, sanitize_display_name, skeleton_key, validate_display_name,
    validate_display_name_for, validate_team_name, validate_workspace_name, DeviceCodeError,
    HelloV2, Mode, PendingJoin, Workspace,
};
use ed25519_dalek::{Signer, SigningKey, Verifier};
use serde_json::{json, Value};

fn problem<T: std::fmt::Debug>(result: Result<T, names::NameError>) -> NameProblem {
    result.expect_err("the name should be refused").problem
}

#[test]
fn equivalent_spellings_share_one_key() {
    let variants = [
        "Analysis Lab",
        "analysis-lab",
        "ANALYSIS_LAB",
        "Analysis.Lab",
        "  Analysis   Lab  ",
        "Analysis\tLab",
        "Analysis -_. Lab",
        "\u{FF21}\u{FF4E}\u{FF41}\u{FF4C}\u{FF59}\u{FF53}\u{FF49}\u{FF53} \u{FF2C}\u{FF41}\u{FF42}",
        "\u{FF21}\u{FF4E}\u{FF41}\u{FF4C}\u{FF59}\u{FF53}\u{FF49}\u{FF53}\u{3000}\u{FF2C}\u{FF41}\u{FF42}",
        "Analysis Lab\u{FE0F}",
        "Analysis\u{200B} Lab",
    ];
    for variant in variants {
        assert_eq!(name_key(variant), "analysis-lab", "{variant:?}");
        assert!(names::names_collide(variant, "Analysis Lab"), "{variant:?}");
    }
    // The skeleton maps capital I to lowercase l, so the skeleton key alone is not
    // case-insensitive for I; the two keys together are.
    assert_ne!(skeleton_key("ANALYSIS LAB"), skeleton_key("analysis lab"));
    assert!(names::names_collide("ANALYSIS LAB", "analysis lab"));
    assert!(names::names_collide("ANAIYSIS LAB", "ANALYSIS LAB"));
    assert!(names::names_collide("anaIysis", "analysis"));
    assert!(!names::names_collide("Analysis Lab", "Synthesis Lab"));
    for variant in [
        "Lab",
        "lab",
        "LAB",
        "\u{FF2C}\u{FF41}\u{FF42}",
        "Lab\u{FE0F}",
        "-lab-",
    ] {
        assert_eq!(name_key(variant), "lab", "{variant:?}");
    }
    // Rust's default lowercase mapping, not full case folding: accepted by the design.
    assert_ne!(name_key("Straße"), name_key("Strasse"));
    assert_ne!(name_key("Analysis Lab"), name_key("AnalysisLab"));
    assert_eq!(names::clean("  Bob \u{2003}\t Lee \n"), "Bob Lee");
    assert_eq!(
        names::strip_ignorable("a\u{200B}b\u{FE0F}c\u{E0041}"),
        "abc"
    );
    assert_eq!(names::team_handle("Analysis Lab"), "analysis-lab");
}

#[test]
fn display_names_refuse_every_invisible_reserved_or_oversized_form() {
    let refused: [(&str, NameProblem); 17] = [
        ("Bob\u{202E}eeL", NameProblem::InvisibleCharacter),
        ("\u{2066}Bob", NameProblem::InvisibleCharacter),
        ("Bob\u{200B}Lee", NameProblem::InvisibleCharacter),
        ("Bob\u{200D}Lee", NameProblem::InvisibleCharacter),
        ("\u{FEFF}Bob", NameProblem::InvisibleCharacter),
        ("\u{FE0F}", NameProblem::InvisibleCharacter),
        ("\u{3164}", NameProblem::InvisibleCharacter),
        ("Bob\u{7}", NameProblem::InvisibleCharacter),
        ("Bob\u{E000}", NameProblem::InvisibleCharacter),
        ("Bob@lab", NameProblem::ReservedCharacter),
        ("Bob\u{FF20}lab", NameProblem::ReservedCharacter),
        ("Bob\u{FE6B}lab", NameProblem::ReservedCharacter),
        ("Bob #1", NameProblem::ReservedCharacter),
        ("Bob \u{FF03}1", NameProblem::ReservedCharacter),
        ("", NameProblem::Empty),
        (" \t\n", NameProblem::Empty),
        ("...", NameProblem::NoLetterOrDigit),
    ];
    for (name, expected) in refused {
        assert_eq!(problem(validate_display_name(name)), expected, "{name:?}");
        assert!(!names::display_name_valid(name), "{name:?}");
    }
    assert_eq!(
        problem(validate_display_name(&"a".repeat(65))),
        NameProblem::TooLong
    );
    // 41 CJK characters are 123 bytes: under the scalar limit, over the byte limit.
    assert_eq!(
        problem(validate_display_name(&"李".repeat(41))),
        NameProblem::TooLong
    );

    for accepted in [
        "李明 Li Ming",
        "Bob Lee",
        "Zoë O'Brien",
        "Bob 🧬",
        "María-José",
    ] {
        assert_eq!(validate_display_name(accepted).as_deref(), Ok(accepted));
    }
    assert_eq!(
        validate_display_name(&"a".repeat(64)).map(|n| n.len()),
        Ok(64)
    );
    assert_eq!(
        validate_display_name("  Bob\u{2003}\u{2028}Lee  ").as_deref(),
        Ok("Bob Lee"),
        "White_Space is collapsed by clean() before the rules run"
    );
    let error = validate_display_name("Bob\u{202E}").unwrap_err();
    assert_eq!(error.kind, NameKind::DisplayName);
    assert_eq!(error.code(), "name_invalid");
    assert!(error
        .wire()
        .starts_with("name_invalid: Display name can't contain"));
    assert!(
        !error.to_string().contains('\u{202E}'),
        "never echoes the input"
    );
}

#[test]
fn a_display_name_may_not_be_another_persons_username() {
    let others = ["alice", "carol", "rn"];
    for claimed in ["alice", "Alice", "ALICE", "\u{FF41}lice", "a l i c e", "m"] {
        let refused = validate_display_name_for(claimed, "bob", others);
        if claimed == "a l i c e" {
            // Spaces are separators, so this keys as a-l-i-c-e, a different name.
            assert!(refused.is_ok());
            continue;
        }
        assert_eq!(problem(refused), NameProblem::ClaimsUsername, "{claimed:?}");
    }
    assert_eq!(
        validate_display_name_for("bob", "bob", others).as_deref(),
        Ok("bob")
    );
    assert_eq!(
        validate_display_name_for("Bob", "bob", ["bob", "alice"]).as_deref(),
        Ok("Bob"),
        "a person's own username is allowed even when it is in the list"
    );
    assert_eq!(
        validate_display_name_for("Bob Lee", "bob", others).as_deref(),
        Ok("Bob Lee")
    );
    let error = validate_display_name_for("alice", "bob", others).unwrap_err();
    assert!(!error.to_string().contains("alice"));
}

#[test]
fn legacy_nicknames_sanitize_or_fall_back_to_the_username() {
    let others = ["alice"];
    assert_eq!(
        sanitize_display_name("\u{202E}Bob Lee", "bob", others),
        "Bob Lee"
    );
    assert_eq!(
        sanitize_display_name("Bob\u{200B}\u{FE0F}", "bob", others),
        "Bob"
    );
    assert_eq!(sanitize_display_name("@admin #1", "bob", others), "admin 1");
    assert_eq!(sanitize_display_name("\u{FF20}alice", "bob", others), "bob");
    assert_eq!(sanitize_display_name("\u{200B}", "bob", others), "bob");
    assert_eq!(sanitize_display_name("", "bob", others), "bob");
    assert_eq!(sanitize_display_name(&"x".repeat(65), "bob", others), "bob");
    assert_eq!(sanitize_display_name("Alice", "bob", others), "bob");
    assert_eq!(sanitize_display_name("Bob Lee", "bob", others), "Bob Lee");
    assert_eq!(
        names::sanitize_team_name(" \u{200B} "),
        names::UNTITLED_TEAM
    );
    assert_eq!(names::sanitize_team_name("Lab\u{202E}  One"), "Lab One");
    assert_eq!(
        names::sanitize_channel_name("\u{FEFF}"),
        names::UNTITLED_CHANNEL
    );
    assert_eq!(names::sanitize_channel_name("gen\u{200B}eral"), "general");
}

#[test]
fn team_names_follow_the_team_rules() {
    for accepted in [
        "Analysis Lab",
        "Lab 分析",
        "Bob's Lab (R&D) +1",
        "李明 实验室",
        "Équipe Génomique",
        "한국어 Lab",
        "日本語 ラボ",
        "Lab_2.0",
        "विश्लेषण प्रयोगशाला",
        "テクノロジー",
        "दुःख",
        "ห้องแล็บ",
        "تحليل",
    ] {
        assert_eq!(validate_team_name(accepted).as_deref(), Ok(accepted));
    }
    // Arabic harakat are Script=Inherited marks that never compose: refused in team names.
    assert_eq!(
        problem(validate_team_name("تَحليل")),
        NameProblem::UnattachedMark
    );
    assert_eq!(
        validate_team_name("  Analysis   Lab ").as_deref(),
        Ok("Analysis Lab")
    );
    let refused: [(&str, NameProblem); 14] = [
        ("", NameProblem::Empty),
        ("Lab\u{FE0F}", NameProblem::InvisibleCharacter),
        ("Lab\u{202E}", NameProblem::InvisibleCharacter),
        ("Lab@home", NameProblem::ReservedCharacter),
        ("Lab #1", NameProblem::ReservedCharacter),
        ("A/B", NameProblem::ReservedCharacter),
        ("Lab: one", NameProblem::ReservedCharacter),
        ("Lab \u{FF03}1", NameProblem::ReservedCharacter),
        ("Lab 🧬", NameProblem::DisallowedCharacter),
        ("Lab!", NameProblem::DisallowedCharacter),
        ("\u{FF2C}\u{FF41}\u{FF42}", NameProblem::DisallowedCharacter),
        ("Laq\u{301}b", NameProblem::UnattachedMark),
        ("\u{0410}nalysis", NameProblem::MixedScripts),
        ("- _ .", NameProblem::NoLetterOrDigit),
    ];
    for (name, expected) in refused {
        assert_eq!(problem(validate_team_name(name)), expected, "{name:?}");
    }
    assert_eq!(
        validate_team_name("Cafe\u{301}").as_deref(),
        Ok("Café"),
        "a combining accent that composes under NFC is stored composed"
    );
    assert_eq!(
        problem(validate_team_name(&"a".repeat(65))),
        NameProblem::TooLong
    );
    for id in [
        "550e8400-e29b-41d4-a716-446655440000",
        "550E8400 E29B 41D4 A716 446655440000",
        "550e8400e29b41d4a716446655440000",
        &"ab".repeat(32),
    ] {
        assert_eq!(
            problem(validate_team_name(id)),
            NameProblem::LooksLikeId,
            "{id:?}"
        );
    }
    let error = validate_team_name("Lab 🧬").unwrap_err();
    assert_eq!(error.kind, NameKind::Team);
    assert!(error.to_string().starts_with("Team name can use letters"));
}

#[test]
fn channel_names_are_canonicalized_then_validated() {
    let canonical = [
        ("Data Analysis", "data-analysis"),
        ("data_analysis", "data_analysis"),
        ("#methods", "methods"),
        ("  Methods.Notes  ", "methods-notes"),
        ("a -- b", "a-b"),
        (
            "\u{FF2D}\u{FF25}\u{FF34}\u{FF28}\u{FF2F}\u{FF24}\u{FF33}",
            "methods",
        ),
        ("Général", "général"),
        ("general", "general"),
        ("分析", "分析"),
        ("lab-2024", "lab-2024"),
        ("-leading-", "leading"),
        ("विश्लेषण", "विश्लेषण"),
        // Letters whose confusable skeleton is `/` or `:` are letters, not selectors.
        ("テクノロジー", "テクノロジー"),
        ("दुःख", "दुःख"),
    ];
    for (raw, slug) in canonical {
        assert_eq!(canonical_channel_name(raw).as_deref(), Ok(slug), "{raw:?}");
        assert_eq!(
            canonical_channel_name(slug).as_deref(),
            Ok(slug),
            "canonicalization is idempotent for {raw:?}"
        );
    }
    assert_eq!(
        name_key("data_analysis"),
        name_key(&canonical_channel_name("Data Analysis").unwrap()),
        "data_analysis collides with data-analysis by key"
    );
    let refused: [(&str, NameProblem); 13] = [
        ("", NameProblem::Empty),
        ("#", NameProblem::Empty),
        (" - . ", NameProblem::Empty),
        ("x\u{200B}y", NameProblem::InvisibleCharacter),
        ("@team", NameProblem::ReservedCharacter),
        ("team/methods", NameProblem::ReservedCharacter),
        ("a:b", NameProblem::ReservedCharacter),
        ("##methods", NameProblem::ReservedCharacter),
        ("chat 💬", NameProblem::DisallowedCharacter),
        ("a+b", NameProblem::DisallowedCharacter),
        ("_notes", NameProblem::MustStartWithLetterOrDigit),
        ("q\u{301}", NameProblem::UnattachedMark),
        ("\u{0410}nalysis", NameProblem::MixedScripts),
    ];
    for (raw, expected) in refused {
        assert_eq!(problem(canonical_channel_name(raw)), expected, "{raw:?}");
    }
    assert_eq!(
        problem(canonical_channel_name(&"a".repeat(81))),
        NameProblem::TooLong
    );
    assert_eq!(
        canonical_channel_name(&"a".repeat(80)).map(|s| s.len()),
        Ok(80)
    );
    assert_eq!(
        problem(canonical_channel_name(
            "550e8400-e29b-41d4-a716-446655440000"
        )),
        NameProblem::LooksLikeId
    );
    assert_eq!(names::RESERVED_CHANNEL_NAME, "general");
}

#[test]
fn workspace_names_are_plain_ascii_slugs() {
    let forty = "a".repeat(40);
    for accepted in ["lab", "a", "0", "lab-2", "hpc-ucsf-01", forty.as_str()] {
        assert_eq!(validate_workspace_name(accepted), Ok(()), "{accepted:?}");
        assert!(names::workspace_name_valid(accepted));
    }
    let refused: [(&str, NameProblem); 10] = [
        ("", NameProblem::Empty),
        ("Lab", NameProblem::DisallowedCharacter),
        ("-lab", NameProblem::DisallowedCharacter),
        ("lab-", NameProblem::DisallowedCharacter),
        ("la b", NameProblem::DisallowedCharacter),
        ("lab_1", NameProblem::DisallowedCharacter),
        ("läb", NameProblem::DisallowedCharacter),
        (" lab", NameProblem::DisallowedCharacter),
        (
            "550e8400-e29b-41d4-a716-446655440000",
            NameProblem::LooksLikeId,
        ),
        ("550e8400e29b41d4a716446655440000", NameProblem::LooksLikeId),
    ];
    for (name, expected) in refused {
        assert_eq!(problem(validate_workspace_name(name)), expected, "{name:?}");
    }
    assert_eq!(
        problem(validate_workspace_name(&"a".repeat(41))),
        NameProblem::TooLong
    );
}

#[test]
fn uuid_shaped_text_is_always_an_id() {
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    for shaped in [
        uuid.to_string(),
        uuid.to_uppercase(),
        uuid.replace('-', ""),
        format!("{{{uuid}}}"),
        format!("urn:uuid:{uuid}"),
        "0f".repeat(32),
        "0F".repeat(32),
    ] {
        assert!(is_uuid_shaped(&shaped), "{shaped:?}");
    }
    for name in [
        "general".to_string(),
        "0f".repeat(31) + "0",
        "0f".repeat(32) + "0",
        "g".repeat(64),
        "550e8400-e29b-41d4-a716-44665544000".to_string(),
        String::new(),
    ] {
        assert!(!is_uuid_shaped(&name), "{name:?}");
    }
}

#[test]
fn confusables_and_mixed_scripts() {
    assert_eq!(skeleton_key("anaIysis"), skeleton_key("analysis"));
    assert_ne!(name_key("anaIysis"), name_key("analysis"));
    assert_eq!(skeleton_key("rn"), skeleton_key("m"));
    assert_eq!(skeleton_key("B0B"), skeleton_key("bob"));
    assert_eq!(skeleton_key("\u{0430}nalysis"), skeleton_key("analysis"));
    assert_ne!(skeleton_key("analysis"), skeleton_key("synthesis"));

    for ok in [
        "Analysis",
        "Lab 分析",
        "日本語 テスト",
        "한국어 Lab",
        "Équipe",
        "lab-2",
        "- _ .",
    ] {
        assert!(restriction_level_ok(ok), "{ok:?}");
    }
    for mixed in ["\u{0410}nalysis", "Αλφα Lab", "paypаl"] {
        assert!(!restriction_level_ok(mixed), "{mixed:?}");
    }
}

#[test]
fn usernames_are_checked_for_shape_only() {
    for ok in [
        "bob",
        "Bob",
        "bob.lee",
        "bob@ad.ucsf.edu",
        "b0b",
        "_svc",
        "1bob",
    ] {
        assert!(names::valid_username(ok), "{ok:?}");
    }
    let long = "b".repeat(257);
    for bad in [
        "",
        "1001",
        "bob lee",
        "bob/x",
        "bob:x",
        "bob\u{0}",
        "bob\n",
        long.as_str(),
    ] {
        assert!(!names::valid_username(bad), "{bad:?}");
    }
}

// Independent vectors from Python's hashlib and a hand-written Crockford encoder.
const VECTOR_WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-9f00-0123456789ab";

#[test]
fn device_codes_match_independent_vectors() {
    let counting: [u8; 32] = std::array::from_fn(|i| i as u8);
    let next: [u8; 32] = std::array::from_fn(|i| 32 + i as u8);
    let vectors: [(&str, [u8; 32], [u8; 32], &str); 4] = [
        (VECTOR_WORKSPACE, [0; 32], [0; 32], "R74NBJ2AS6PSV6HT"),
        (VECTOR_WORKSPACE, counting, next, "6EE1QD4GCKF7J51F"),
        (VECTOR_WORKSPACE, [0xff; 32], [0x11; 32], "P9KZS3XCE7M4YEYV"),
        ("", [0; 32], [0; 32], "Y6T469ESGXSC420X"),
    ];
    for (workspace, w, k, expected) in vectors {
        let code = device_code(workspace, &w, &k);
        assert_eq!(code, expected);
        assert_eq!(code.len(), biorouter_crew::DEVICE_CODE_LEN);
        assert!(!code.contains(['I', 'L', 'O', 'U']));
        assert_eq!(
            device_code_from_hex(workspace, &hex::encode(w), &hex::encode(k)).as_deref(),
            Ok(expected)
        );
    }
    // The workspace key is inside the code: a substituted key changes it.
    assert_ne!(
        device_code(VECTOR_WORKSPACE, &[1; 32], &[0; 32]),
        device_code(VECTOR_WORKSPACE, &[0; 32], &[0; 32])
    );
    assert_ne!(
        device_code("another-workspace", &[0; 32], &[0; 32]),
        device_code(VECTOR_WORKSPACE, &[0; 32], &[0; 32])
    );
    assert_eq!(
        device_code_from_hex(VECTOR_WORKSPACE, "zz", &hex::encode([0u8; 32])),
        Err(DeviceCodeError::InvalidKey)
    );
    assert_eq!(
        device_code_from_hex(VECTOR_WORKSPACE, &"00".repeat(31), &hex::encode([0u8; 32])),
        Err(DeviceCodeError::InvalidKey)
    );
}

#[test]
fn device_codes_normalize_what_people_type() {
    let canonical = "7QK2M9XA3JTPWZ4D";
    for typed in [
        "7QK2-M9XA-3JTP-WZ4D",
        "7qk2m9xa3jtpwz4d",
        " 7qk2 m9xa 3jtp wz4d\n",
        "7QK2\u{2010}M9XA\u{2013}3JTP\u{2014}WZ4D",
        "7QK2\u{200B}M9XA-3JTP-WZ4D",
    ] {
        assert_eq!(
            normalize_device_code(typed).as_deref(),
            Ok(canonical),
            "{typed:?}"
        );
    }
    assert_eq!(
        normalize_device_code("IiLlOo00-11110000").as_deref(),
        Ok("1111000011110000")
    );
    assert_eq!(
        normalize_device_code("7QK2-M9XA-3JTP-WZ4U"),
        Err(DeviceCodeError::ContainsU)
    );
    assert_eq!(
        normalize_device_code("7qk2-m9xa-3jtp-wz4u"),
        Err(DeviceCodeError::ContainsU)
    );
    assert_eq!(
        normalize_device_code("7QK2-M9XA-3JTP-WZ4"),
        Err(DeviceCodeError::WrongLength)
    );
    assert_eq!(
        normalize_device_code("7QK2-M9XA-3JTP-WZ4DD"),
        Err(DeviceCodeError::WrongLength)
    );
    assert_eq!(normalize_device_code(""), Err(DeviceCodeError::WrongLength));
    assert_eq!(
        normalize_device_code("7QK2-M9XA-3JTP-WZ4!"),
        Err(DeviceCodeError::InvalidCharacter)
    );
    assert_eq!(
        normalize_device_code("7QK2-M9XA-3JTP-WZ4Ö"),
        Err(DeviceCodeError::InvalidCharacter)
    );
    assert_eq!(
        format_device_code("7qk2m9xa3jtpwz4d"),
        "7QK2-M9XA-3JTP-WZ4D"
    );
    assert_eq!(format_device_code("not a code"), "not a code");

    let code = device_code(VECTOR_WORKSPACE, &[0; 32], &[0; 32]);
    let typed = format_device_code(&code).to_lowercase().replace('1', "l");
    assert!(device_code_matches(
        &typed,
        VECTOR_WORKSPACE,
        &[0; 32],
        &[0; 32]
    ));
    assert!(!device_code_matches(
        &typed,
        VECTOR_WORKSPACE,
        &[0; 32],
        &[2; 32]
    ));
    assert!(!device_code_matches(
        "garbage",
        VECTOR_WORKSPACE,
        &[0; 32],
        &[0; 32]
    ));
    assert!(!device_code_matches(
        "",
        VECTOR_WORKSPACE,
        &[0; 32],
        &[0; 32]
    ));
}

fn workspace_key() -> String {
    hex::encode(SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes())
}

fn sample_invitation() -> WorkspaceInvitation {
    WorkspaceInvitation {
        workspace_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        workspace_public_key: workspace_key(),
        socket_path: format!("/tmp/crew-1000-{}/broker.sock", "0a".repeat(16)),
        owner_uid: 1000,
        workspace_name: Some("lab".into()),
        host_username: Some("alice".into()),
        host_display_name: Some("Alice Chen".into()),
        mode: Some(Mode::Private),
        institution_id: Some("ucsf".into()),
        ssh_host: Some("hpc.ucsf.edu".into()),
        ssh_port: Some(22),
        proxy_jump: Some("gateway.ucsf.edu".into()),
        invitee_username: Some("bob".into()),
    }
}

fn token_for(json: &Value) -> String {
    format!(
        "{}{}",
        invitation::PREFIX,
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(json).unwrap())
    )
}

#[test]
fn invitations_round_trip_through_the_line_and_the_message() {
    let original = sample_invitation();
    let line = invitation::encode(&original).unwrap();
    assert!(line.starts_with("brcrew1:"));
    assert!(!line.contains(['=', '+', '/', '\n']));
    let parsed = invitation::parse(&line).unwrap();
    assert_eq!(parsed.source, InvitationSource::Invitation);
    assert_eq!(parsed.invitation, original);

    let message = invitation::message(&original).unwrap();
    assert_eq!(
        message,
        format!(
            "Join lab on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste \
             this whole message.\n{line}"
        )
    );
    let quoted = format!("> Hi Bob,\n> {}\n>\n> Alice", message.replace('\n', "\n> "));
    for pasted in [
        message.clone(),
        quoted,
        format!("<{line}>."),
        format!("  {line}\r\n"),
    ] {
        assert_eq!(
            invitation::parse(&pasted).unwrap().invitation,
            original,
            "{pasted:?}"
        );
    }

    let minimal = WorkspaceInvitation {
        workspace_name: None,
        host_display_name: None,
        mode: Some(Mode::Public),
        institution_id: None,
        ssh_host: None,
        ssh_port: None,
        proxy_jump: None,
        invitee_username: None,
        ..original.clone()
    };
    let parsed = invitation::parse(&invitation::encode(&minimal).unwrap()).unwrap();
    assert_eq!(parsed.invitation, minimal);
    assert!(invitation::message(&minimal)
        .unwrap()
        .starts_with("Join alice's workspace on Crew."));

    // The wire JSON is exactly the design's v1 object.
    let decoded: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(line.trim_start_matches(invitation::PREFIX))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(decoded["v"], 1);
    assert_eq!(decoded["mode"], "private");
    assert_eq!(decoded.as_object().unwrap().len(), 14);

    let fingerprint =
        invitation::workspace_key_fingerprint(&original.workspace_public_key).unwrap();
    assert_eq!(fingerprint.len(), 64);
    let grouped = invitation::grouped_fingerprint(&fingerprint);
    assert_eq!(grouped.len(), 19);
    assert_eq!(
        grouped.replace(' ', ""),
        fingerprint.get(..16).unwrap().to_uppercase()
    );
}

#[test]
fn legacy_status_json_is_accepted() {
    let key = workspace_key();
    let status = json!({
        "pid": 4242,
        "socket": format!("/tmp/crew-1000-{}/broker.sock", "0a".repeat(16)),
        "workspace_id": "550e8400-e29b-41d4-a716-446655440000",
        "host_uid": 1000,
        "protocol": 1,
        "node_id": "ab".repeat(32),
        "workspace_public_key": key,
        "workspace_key_fingerprint": invitation::workspace_key_fingerprint(&key).unwrap(),
    });
    let line = status.to_string();
    let pretty = serde_json::to_string_pretty(&status).unwrap();
    let with_prompt = format!(
        "alice@hpc:~$ ~/.local/bin/biorouter-crew status --state-dir x\n{line}\nalice@hpc:~$"
    );
    for pasted in [line.as_str(), pretty.as_str(), with_prompt.as_str()] {
        let parsed = invitation::parse(pasted).unwrap();
        assert_eq!(parsed.source, InvitationSource::LegacyStatus);
        let expected = WorkspaceInvitation {
            workspace_name: None,
            host_username: None,
            host_display_name: None,
            mode: None,
            institution_id: None,
            ssh_host: None,
            ssh_port: None,
            proxy_jump: None,
            invitee_username: None,
            ..sample_invitation()
        };
        assert_eq!(parsed.invitation, expected);
    }
    let mut wrong_print = status.clone();
    wrong_print["workspace_key_fingerprint"] = json!("00".repeat(32));
    assert_eq!(
        invitation::parse(&wrong_print.to_string()),
        Err(InvitationError::InvalidField(
            InvitationField::WorkspaceKeyFingerprint
        ))
    );
    let mut newer = status.clone();
    newer["protocol"] = json!(2);
    assert_eq!(
        invitation::parse(&newer.to_string()),
        Err(InvitationError::UnsupportedVersion)
    );
    let mut root = status.clone();
    root["host_uid"] = json!(0);
    assert_eq!(
        invitation::parse(&root.to_string()),
        Err(InvitationError::InvalidField(InvitationField::OwnerUid))
    );
}

#[test]
fn garbage_oversize_and_unknown_versions_are_refused() {
    let base = serde_json::to_value(sample_invitation()).unwrap();
    let mut wire = base.as_object().unwrap().clone();
    wire.insert("v".into(), json!(1));
    let wire = Value::Object(wire);
    assert!(invitation::parse(&token_for(&wire)).is_ok());

    let cases: Vec<(String, InvitationError)> = vec![
        ("hello Bob".into(), InvitationError::NotFound),
        (String::new(), InvitationError::NotFound),
        ("{\"workspace_id\": 1}".into(), InvitationError::NotFound),
        ("brcrew1:".into(), InvitationError::Malformed),
        ("brcrew1:!!!".into(), InvitationError::Malformed),
        ("brcrew1:a".into(), InvitationError::Malformed),
        (
            format!("brcrew1:{}", URL_SAFE_NO_PAD.encode("not json")),
            InvitationError::Malformed,
        ),
        (token_for(&json!([1, 2])), InvitationError::Malformed),
        (
            token_for(&json!({"v": 2, "anything": true})),
            InvitationError::UnsupportedVersion,
        ),
        (token_for(&json!({"v": "1"})), InvitationError::Malformed),
        (
            token_for(&json!({"workspace_id": "x"})),
            InvitationError::Malformed,
        ),
        (
            "x".repeat(invitation::MAX_PASTED_BYTES + 1),
            InvitationError::TooLong,
        ),
        (
            format!("brcrew1:{}", "A".repeat(8000)),
            InvitationError::TooLong,
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(invitation::parse(&text), Err(expected), "{:.80}", text);
    }
    let mut unknown = wire.clone();
    unknown["team_hint"] = json!("analysis-lab");
    assert_eq!(
        invitation::parse(&token_for(&unknown)),
        Err(InvitationError::Malformed)
    );
    let mut padded = wire.clone();
    padded["host_display_name"] = json!("x".repeat(4096));
    assert_eq!(
        invitation::parse(&token_for(&padded)),
        Err(InvitationError::TooLong)
    );
    // The first token is the one parsed, even when a valid one follows it.
    let two = format!("brcrew1:!!! then {}", token_for(&wire));
    assert_eq!(invitation::parse(&two), Err(InvitationError::Malformed));
}

/// One invalid value per rule, each with the field the refusal must name.
fn invalid_field_cases() -> Vec<(&'static str, Value, InvitationField)> {
    use InvitationField as F;
    let key_upper = workspace_key().to_uppercase();
    let not_a_point = (1u8..=255)
        .map(|byte| [byte; 32])
        .find(|bytes| ed25519_dalek::VerifyingKey::from_bytes(bytes).is_err())
        .map(hex::encode)
        .expect("some repeated byte is not a curve point");
    vec![
        ("workspace_id", json!("not-a-uuid"), F::WorkspaceId),
        (
            "workspace_id",
            json!("550E8400-E29B-41D4-A716-446655440000"),
            F::WorkspaceId,
        ),
        (
            "workspace_id",
            json!("550e8400e29b41d4a716446655440000"),
            F::WorkspaceId,
        ),
        (
            "workspace_public_key",
            json!(key_upper),
            F::WorkspacePublicKey,
        ),
        (
            "workspace_public_key",
            json!("ab".repeat(31)),
            F::WorkspacePublicKey,
        ),
        (
            "workspace_public_key",
            json!(not_a_point),
            F::WorkspacePublicKey,
        ),
        ("socket_path", json!("relative/broker.sock"), F::SocketPath),
        (
            "socket_path",
            json!("/var/run/crew/broker.sock"),
            F::SocketPath,
        ),
        (
            "socket_path",
            json!("/tmp/../etc/broker.sock"),
            F::SocketPath,
        ),
        (
            "socket_path",
            json!("/tmp/crew/x/broker.sock"),
            F::SocketPath,
        ),
        (
            "socket_path",
            json!("/tmp/crew dir/broker.sock"),
            F::SocketPath,
        ),
        (
            "socket_path",
            json!(format!("/tmp/{}/broker.sock", "a".repeat(100))),
            F::SocketPath,
        ),
        ("owner_uid", json!(0), F::OwnerUid),
        ("workspace_name", json!("Lab"), F::WorkspaceName),
        ("host_username", json!("1000"), F::HostUsername),
        ("host_username", json!("bob lee"), F::HostUsername),
        (
            "host_display_name",
            json!("Alice\u{202E}"),
            F::HostDisplayName,
        ),
        ("host_display_name", json!(" Alice "), F::HostDisplayName),
        ("institution_id", json!("UCSF"), F::InstitutionId),
        ("ssh_host", json!("alice@hpc.ucsf.edu"), F::SshHost),
        ("ssh_host", json!("-oProxyCommand=x"), F::SshHost),
        ("ssh_host", json!("hpc ucsf"), F::SshHost),
        ("ssh_port", json!(0), F::SshPort),
        ("proxy_jump", json!("gw;rm -rf"), F::ProxyJump),
        ("invitee_username", json!("bob:x"), F::InviteeUsername),
    ]
}

#[test]
fn every_invitation_field_is_validated_like_a_saved_connection() {
    use InvitationField as F;
    for (field, value, expected) in invalid_field_cases() {
        let mut candidate = sample_invitation();
        let mut object = serde_json::to_value(&candidate).unwrap();
        object[field] = value.clone();
        candidate = serde_json::from_value(object.clone()).unwrap();
        assert_eq!(
            invitation::encode(&candidate),
            Err(InvitationError::InvalidField(expected)),
            "encode {field} = {value}"
        );
        object["v"] = json!(1);
        assert_eq!(
            invitation::parse(&token_for(&object)),
            Err(InvitationError::InvalidField(expected)),
            "parse {field} = {value}"
        );
    }
    for (missing, field) in [("host_username", F::HostUsername), ("mode", F::Mode)] {
        let mut object = serde_json::to_value(sample_invitation()).unwrap();
        object[missing] = Value::Null;
        let candidate: WorkspaceInvitation = serde_json::from_value(object).unwrap();
        assert_eq!(
            invitation::encode(&candidate),
            Err(InvitationError::InvalidField(field))
        );
    }
    let error = InvitationError::InvalidField(F::SocketPath);
    assert_eq!(error.code(), "invitation_invalid_field");
    assert!(error.to_string().contains("server socket path"));
}

#[test]
fn hello_payloads_are_stable() {
    let key = workspace_key();
    let node = "cd".repeat(32);
    let v1 = hello_v1_payload("w-1", 1000, "nonce-1", &key, &node);
    assert_eq!(
        String::from_utf8(v1.clone()).unwrap(),
        format!("[\"w-1\",1000,\"nonce-1\",\"{key}\",\"{node}\"]")
    );
    // Byte-identical to the array the broker and daemon sign and verify inline today.
    assert_eq!(
        v1,
        serde_json::to_vec(&json!(["w-1", 1000, "nonce-1", key, node])).unwrap()
    );

    let capabilities = ["human_chat", "signed_devices", "human_names_v1"];
    let hello = HelloV2 {
        workspace_id: "w-1",
        host_uid: 1000,
        challenge_nonce: "nonce-1",
        workspace_public_key: &key,
        node_id: &node,
        mode: &Mode::Private,
        institution_id: Some("ucsf"),
        policy_epoch: 3,
        name: Some("lab"),
        capabilities: &capabilities,
    };
    assert_eq!(
        String::from_utf8(hello.signing_payload()).unwrap(),
        format!(
            "[\"w-1\",1000,\"nonce-1\",\"{key}\",\"{node}\",\"private\",\"ucsf\",3,\"lab\",\
             [\"human_chat\",\"signed_devices\",\"human_names_v1\"]]"
        )
    );
    let legacy = HelloV2 {
        institution_id: None,
        name: None,
        mode: &Mode::Public,
        capabilities: &[],
        ..hello
    };
    assert_eq!(
        String::from_utf8(legacy.signing_payload()).unwrap(),
        format!("[\"w-1\",1000,\"nonce-1\",\"{key}\",\"{node}\",\"public\",null,3,null,[]]")
    );

    // A signature over v2 fails to verify when any covered field changes.
    let signer = SigningKey::from_bytes(&[7; 32]);
    let signature = signer.sign(&hello.signing_payload());
    let verifier = signer.verifying_key();
    assert!(verifier
        .verify(&hello.signing_payload(), &signature)
        .is_ok());
    let fewer = ["human_chat", "signed_devices"];
    let reordered = ["signed_devices", "human_chat", "human_names_v1"];
    let tampered = [
        HelloV2 {
            name: Some("lab2"),
            ..hello
        },
        HelloV2 {
            name: None,
            ..hello
        },
        HelloV2 {
            mode: &Mode::Public,
            ..hello
        },
        HelloV2 {
            institution_id: Some("ucla"),
            ..hello
        },
        HelloV2 {
            policy_epoch: 4,
            ..hello
        },
        HelloV2 {
            capabilities: &fewer,
            ..hello
        },
        HelloV2 {
            capabilities: &reordered,
            ..hello
        },
        HelloV2 {
            host_uid: 1001,
            ..hello
        },
        HelloV2 {
            challenge_nonce: "nonce-2",
            ..hello
        },
    ];
    for changed in tampered {
        assert!(
            verifier
                .verify(&changed.signing_payload(), &signature)
                .is_err(),
            "{changed:?}"
        );
    }
    assert!(verifier.verify(&v1, &signature).is_err());
}

#[test]
fn pending_joins_and_workspace_names_serialize_additively() {
    let join = PendingJoin {
        join_id: "0a".repeat(16),
        uid: 1001,
        username: "bob".into(),
        full_name: Some("Bob Lee".into()),
        inviter_id: "550e8400-e29b-41d4-a716-446655440000".into(),
        existing_principal_id: None,
        generation: vec!["6ba7b810-9dad-11d1-80b4-00c04fd430c8".into()],
        approved_code: None,
        created_at: 100,
        expires_at: 100 + biorouter_crew::PENDING_JOIN_LIFETIME_SECS,
    };
    let value = serde_json::to_value(&join).unwrap();
    assert_eq!(
        serde_json::from_value::<PendingJoin>(value.clone()).unwrap(),
        join
    );
    assert!(!join.is_expired(100));
    assert!(join.is_expired(100 + 86_400));
    let mut without_generation = value;
    without_generation
        .as_object_mut()
        .unwrap()
        .remove("generation");
    assert!(
        serde_json::from_value::<PendingJoin>(without_generation).is_err(),
        "a join without its generation must fail to load, not compare against nothing"
    );

    let legacy = json!({"id": "w", "host_uid": 1000, "institution_id": null, "mode": "private", "policy_epoch": 1});
    let workspace: Workspace = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(workspace.name, None);
    assert_eq!(
        serde_json::to_value(&workspace).unwrap(),
        legacy,
        "an unnamed workspace re-serializes byte for byte"
    );
    let named = Workspace {
        name: Some("lab".into()),
        ..workspace
    };
    assert_eq!(serde_json::to_value(&named).unwrap()["name"], "lab");
}
