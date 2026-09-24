//! T-54: joining by invitation refuses the server's own accounts. `root` (UID 0), accounts below
//! the node's `UID_MIN`, the overflow `nobody`, and accounts whose login shell is `nologin` or
//! `false` are never invited, whatever the host types, and nothing is recorded for them.
//!
//! ```text
//! cargo test -p biorouter-crew --features join-by-name --test system_account_contract
//! ```
#![cfg(all(unix, feature = "join-by-name"))]

mod support;

use serde_json::json;
use support::*;

fn system_account(name: &str) -> String {
    format!("name_invalid: @{name} is a system account on this server and can't join a workspace.")
}

fn pending(ws: &mut Workspace) -> serde_json::Value {
    ws.host_snapshot()
        .get("pending_joins")
        .cloned()
        .unwrap_or_else(|| json!([]))
}

#[test]
fn root_nobody_daemons_and_nologin_accounts_are_never_invited() {
    let mut ws = Workspace::new("system-accounts");
    ws.directory.set_with_shell(0, "root", "/bin/bash");
    ws.directory
        .set_with_shell(1, "daemon", "/usr/sbin/nologin");
    ws.directory.set_with_shell(998, "postgres", "/bin/bash");
    ws.directory
        .set_with_shell(65_534, "nobody", "/usr/sbin/nologin");
    ws.directory
        .set_with_shell(74_001, "backup", "/usr/sbin/nologin");
    ws.directory.set_with_shell(74_002, "svc", "/bin/false");
    let before = ws.journal_bytes().len();
    for name in ["root", "daemon", "postgres", "nobody", "backup", "svc"] {
        for params in [
            json!({"username": name}),
            json!({"username": format!("@{name}")}),
        ] {
            let (code, message) = refused(ws.host_call("enrollment.invite", params));
            assert_eq!(code, "name_invalid", "{name}");
            assert_eq!(message, system_account(name));
        }
    }
    assert_eq!(pending(&mut ws), json!([]));
    assert_eq!(ws.journal_bytes().len(), before, "a refusal writes nothing");
}

#[test]
fn an_ordinary_account_is_still_invited_and_uid_min_comes_from_the_node() {
    let mut ws = Workspace::new("system-accounts-people");
    ws.directory.set_with_shell(74_010, "bob", "/bin/bash");
    ws.directory.set(74_011, "carol", None);
    ws.host_ok("enrollment.invite", json!({"username": "bob"}));
    ws.host_ok("enrollment.invite", json!({"username": "carol"}));

    // UID 600 is a person on a node whose login.defs starts people at 500, and a system
    // account on one that starts them at 1000 (the default).
    ws.directory.set_with_shell(600, "olduser", "/bin/bash");
    let (code, _) = refused(ws.host_call("enrollment.invite", json!({"username": "olduser"})));
    assert_eq!(code, "name_invalid");
    ws.directory.set_uid_min(500);
    ws.host_ok("enrollment.invite", json!({"username": "olduser"}));

    let names: Vec<String> = pending(&mut ws)
        .as_array()
        .unwrap()
        .iter()
        .map(|join| join["username"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    for name in ["bob", "carol", "olduser"] {
        assert!(
            names.iter().any(|joined| joined == name),
            "{name}: {names:?}"
        );
    }
}
