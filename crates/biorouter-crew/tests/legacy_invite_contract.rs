//! Q2-14: the legacy `enrollment.invite {uid, public_key}` token path refuses the server's own
//! accounts exactly as the invitation by name does (T-54, `system_account_contract.rs`). UID 0,
//! a UID below the node's `UID_MIN`, the overflow UID `nobody` holds and an account whose login
//! shell is `nologin` or `false` are never enrolled, and a refusal records nothing.
//!
//! The check used to live only in the feature-gated `broker/join.rs`, so the token path skipped
//! it in every build: live, a host was refused `@root` by name and, one second later, handed an
//! hour-long enrollment for UID 0 that no host screen listed. This file therefore runs in both
//! builds:
//!
//! ```text
//! cargo test -p biorouter-crew --test legacy_invite_contract
//! cargo test -p biorouter-crew --no-default-features --test legacy_invite_contract
//! ```
#![cfg(unix)]

mod support;

use serde_json::json;
use support::*;

fn system_account(name: &str) -> String {
    format!("name_invalid: @{name} is a system account on this server and can't join a workspace.")
}

/// The number of complete records in the workspace's journal.
fn journal_records(ws: &Workspace) -> usize {
    ws.journal_bytes()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .count()
}

#[test]
fn root_low_uids_nobody_and_nologin_accounts_get_no_legacy_enrollment() {
    let mut ws = Workspace::new("legacy-invite-system-accounts");
    // The host is the test process's own UID; a run as root can't also have a separate `root`
    // account to invite.
    let mut accounts: Vec<(u32, &str)> = vec![
        (1, "daemon"),
        (999, "postgres"),
        (65_534, "nobody"),
        (74_001, "backup"),
        (74_002, "svc"),
    ];
    if host_uid() != 0 {
        ws.directory.set_with_shell(0, "root", "/bin/bash");
        accounts.insert(0, (0, "root"));
    }
    ws.directory
        .set_with_shell(1, "daemon", "/usr/sbin/nologin");
    // Below UID_MIN (1000 unless login.defs says otherwise) with a real login shell.
    ws.directory.set_with_shell(999, "postgres", "/bin/bash");
    ws.directory.set_with_shell(65_534, "nobody", "/bin/sh");
    ws.directory
        .set_with_shell(74_001, "backup", "/usr/sbin/nologin");
    ws.directory.set_with_shell(74_002, "svc", "/bin/false");

    let bytes = ws.journal_bytes().len();
    let records = journal_records(&ws);
    for (seed, (uid, name)) in (40u8..).zip(accounts) {
        let (code, message) = refused(ws.legacy_invite(uid, &key(seed)));
        assert_eq!(code, "name_invalid", "UID {uid}");
        assert_eq!(message, system_account(name), "UID {uid}");
    }
    assert_eq!(journal_records(&ws), records, "a refusal writes no record");
    assert_eq!(ws.journal_bytes().len(), bytes, "a refusal writes nothing");
}

#[test]
fn a_system_account_is_refused_before_its_key_or_existing_principal_is_read() {
    let mut ws = Workspace::new("legacy-invite-order");
    ws.directory
        .set_with_shell(74_020, "www-data", "/usr/sbin/nologin");
    let records = journal_records(&ws);
    // A malformed key and a made-up principal would each be refused for their own reason; the
    // system account is the first answer, so nothing about it is weighed further.
    for params in [
        json!({"uid": 74_020, "public_key": "not-a-key"}),
        json!({"uid": 74_020, "public_key": key_hex(&key(50)), "existing_principal_id": uuid()}),
    ] {
        let (code, message) = refused(ws.host_call("enrollment.invite", params));
        assert_eq!(code, "name_invalid");
        assert_eq!(message, system_account("www-data"));
    }
    assert_eq!(journal_records(&ws), records);
}

#[test]
fn an_ordinary_account_is_still_enrolled_by_token() {
    let mut ws = Workspace::new("legacy-invite-people");
    ws.directory.set_with_shell(74_010, "bob", "/bin/bash");
    let records = journal_records(&ws);
    let mut bob = Member::new(74_010, key(60));
    let invitation = ok(ws.legacy_invite(74_010, &bob.key));
    assert_eq!(invitation["uid"], json!(74_010));
    assert!(
        journal_records(&ws) > records,
        "an accepted invitation is recorded"
    );
    let token = invitation["invitation"].as_str().unwrap().to_owned();
    let enrolled = ok(ws.legacy_enroll(&mut bob, &token));
    assert_eq!(enrolled["principal"]["username"], json!("bob"));

    // An account with no shell recorded (NSS left it empty) is a person, as by name.
    ws.directory.set(74_011, "carol", None);
    ok(ws.legacy_invite(74_011, &key(61)));
}

#[test]
fn uid_min_comes_from_the_node_on_the_token_path_too() {
    let mut ws = Workspace::new("legacy-invite-uid-min");
    ws.directory.set_with_shell(600, "olduser", "/bin/bash");
    let (code, message) = refused(ws.legacy_invite(600, &key(70)));
    assert_eq!(code, "name_invalid");
    assert_eq!(message, system_account("olduser"));
    // A node whose login.defs starts people at 500.
    ws.directory.set_uid_min(500);
    ok(ws.legacy_invite(600, &key(70)));
}
