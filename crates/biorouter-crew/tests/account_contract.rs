//! Broker contract for account ownership: `auth.account`, `account.status`, `device.revoke`,
//! the `account_identity_v1` capability and `hello`'s `version` (AO-1 to AO-5 of
//! `docs/crew/design/account-ownership.md`).
//!
//! The account path is compiled with the `join-by-name` feature, so most of this file needs it:
//!
//! ```text
//! cargo test -p biorouter-crew --test account_contract
//! cargo test -p biorouter-crew --no-default-features --test account_contract
//! ```
//!
//! **Always compiled:** the downgrade replay. A journal holding every record shape the account
//! path writes (an empty bootstrap key, the host's first device, an invited member's join, a
//! replaced device, a followed host rename and a revoked device) replays and keeps working on
//! a broker built **without** the feature, which stands in for a broker that predates the
//! account path: the `--no-default-features` run is that build. The feature build pins that
//! the real broker writes exactly those shapes, so the two runs together cover the downgrade.
//!
//! **The previous broker, for the reviewer.** The released v1.94.1 broker predates the account
//! path. The run, on any Unix (it uses the account-directory seam, so no second account):
//!
//! 1. Extract `git archive v1.94.1 Cargo.toml crates/biorouter-crew` into a scratch folder, give
//!    its workspace version a pre-release suffix so the two packages differ, and link it beside
//!    this tree's crate in one small harness (`crew_old`, `crew_new`, both with `test-seams`).
//! 2. Alternate the two over one state directory: new (create with no bootstrap key, the host's
//!    first device by `auth.account`, a policy, a team, invite `@bob`), old (replay; the host's
//!    account-added device signs a snapshot; `auth.bootstrap` stays shut; a team), new (`@bob`
//!    joins with no code, his laptop replaces his desktop with `replaces_device_id`, the host
//!    account is renamed and followed, a second host device is revoked by fingerprint), old
//!    (replay; the renamed host and Bob's laptop authenticate, the replaced and revoked devices
//!    are `unknown device`, `auth.account` and `account.status` are refused as unknown, two
//!    mutations), new (replay; a retried key commits nothing; `account.status`), old (replay).
//! 3. Every step must succeed.
//!
//! Measured 2026-11-05 on macOS against this tree: every step passed (14 journal records).
//!
//! Tests that sign `hello` need Linux (a machine identity); the rest run on every Unix.
#![cfg(unix)]

mod support;

use serde_json::{json, Value};
use support::*;

const BOB: u32 = 71_001;

/// A device as the account path stores it.
fn account_device(principal_id: &str, key: &ed25519_dalek::SigningKey) -> Value {
    json!({
        "principal_id": principal_id,
        "public_key": key_hex(key),
        "added_at": now(),
        "added_via": "account",
    })
}

/// A journal holding each record the account path writes replays on a broker built without
/// it, which then keeps authenticating the devices those records added and keeps committing.
#[test]
fn a_journal_holding_account_records_replays_without_the_feature() {
    let ws = Workspace::unbootstrapped("account-downgrade");
    let host = uuid();
    let bob = uuid();
    let host_second = key(41);
    let bob_desktop = key(42);
    let bob_laptop = key(43);
    let (host_device, second_device) = (device_id(&key(7)), device_id(&host_second));
    let (desktop_device, laptop_device) = (device_id(&bob_desktop), device_id(&bob_laptop));
    let join = json!({
        "join_id": "0b".repeat(16), "uid": BOB, "username": "bob", "full_name": null,
        "inviter_id": host, "existing_principal_id": null, "generation": [],
        "approved_code": null, "created_at": now(), "expires_at": now() + 86_400,
    });
    let dedupe_key = format!("{host}:human:revoke-1");
    let mut ws = ws.edit_journal(|journal| {
        // A workspace created without `--bootstrap-key` (AO-7).
        journal.append(
            "system",
            "workspace.initialize",
            vec![set(&["bootstrap_key"], json!(""))],
        );
        // The host's first device (AO-2, host's first device).
        journal.append(
            &host,
            "auth.account",
            vec![
                set(
                    &["principals", host.as_str()],
                    legacy_principal(&host, host_uid(), "alice", "alice", true),
                ),
                set(
                    &["devices", host_device.as_str()],
                    account_device(&host, &key(7)),
                ),
            ],
        );
        // An invited member joins with no code.
        journal.append(
            &host,
            "enrollment.invite",
            vec![set(&["pending_joins"], json!({ "71001": join }))],
        );
        journal.append(
            &bob,
            "auth.account",
            vec![
                set(
                    &["principals", bob.as_str()],
                    legacy_principal(&bob, BOB, "bob", "bob", true),
                ),
                set(
                    &["devices", desktop_device.as_str()],
                    account_device(&bob, &bob_desktop),
                ),
                remove(&["pending_joins"]),
            ],
        );
        // A device replaced in place.
        journal.append(
            &bob,
            "auth.account",
            vec![
                remove(&["devices", desktop_device.as_str()]),
                set(
                    &["devices", laptop_device.as_str()],
                    account_device(&bob, &bob_laptop),
                ),
            ],
        );
        // The host's account renamed and followed, with a second device.
        journal.append(
            &host,
            "auth.account",
            vec![
                set(&["principals", host.as_str(), "username"], json!("alicia")),
                set(&["principals", host.as_str(), "nickname"], json!("alicia")),
                set(
                    &["devices", second_device.as_str()],
                    account_device(&host, &host_second),
                ),
            ],
        );
        // That second device revoked by fingerprint.
        journal.append(
            &host,
            "device.revoke",
            vec![
                remove(&["devices", second_device.as_str()]),
                set(
                    &["dedupe", dedupe_key.as_str()],
                    json!({"digest": "0".repeat(64), "result": {"revoked": true}, "at": now()}),
                ),
            ],
        );
    });
    ws.directory.set(host_uid(), "alicia", None);
    ws.directory.set(BOB, "bob", None);
    ws.host.principal_id = host.clone();
    let snapshot = ws.host_snapshot();
    assert_eq!(snapshot["actor"]["username"], "alicia");
    assert_eq!(snapshot["workspace"]["host_principal_id"], json!(host));
    let mut laptop = Member::new(BOB, bob_laptop);
    laptop.principal_id = bob.clone();
    assert_eq!(ws.snapshot(&mut laptop)["actor"]["id"], json!(bob));
    // The replaced and the revoked devices stay gone.
    let mut desktop = Member::new(BOB, bob_desktop);
    let (_, message) = refused(ws.call(&mut desktop, "workspace.snapshot", json!({})));
    assert_eq!(message, "unauthorized: unknown device");
    ws.host_team("Lab");
    let mut ws = ws.reopen();
    ws.host_team("Methods");
    assert_eq!(ws.broker.state_json()["bootstrap_key"], "");
}

#[cfg(feature = "join-by-name")]
mod account {
    use super::*;
    use biorouter_crew::{Broker, Connection, Quotas, Request, MAX_DEVICES_PER_PRINCIPAL};

    const CAROL: u32 = 71_002;
    const DAVE: u32 = 71_003;
    const STRANGER: u32 = 71_050;
    const LOOKALIKE: u32 = 71_009;

    /// A workspace named `baranzini-lab`, so refusals name it.
    fn named(label: &str) -> Workspace {
        let mut ws = Workspace::new(label);
        ws.host_ok("workspace.rename", json!({"name": "baranzini-lab"}));
        ws
    }

    /// `member`'s own devices, as its snapshot lists them.
    fn devices(ws: &mut Workspace, member: &mut Member) -> Vec<Value> {
        ws.snapshot(member)["actor"]["devices"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn assert_unknown_device(ws: &mut Workspace, member: &mut Member) {
        let (_, message) = refused(ws.call(member, "workspace.snapshot", json!({})));
        assert_eq!(message, "unauthorized: unknown device");
    }

    // ── The host ──────────────────────────────────────────────────────────────────────────

    /// AO-1: a host who lost every computer signs in as the host account from a new one and is
    /// the host again, with no invitation, code or other device.
    #[test]
    fn the_host_gets_a_fresh_key_after_losing_every_device() {
        let mut ws = named("account-host-lost");
        let lost = device_id(&ws.host.key);
        let mut fresh = Member::new(host_uid(), key(50));
        let answer = ws.account_ok(&mut fresh, json!({"replaces_device_id": lost}));
        assert_eq!(answer["role"], "host");
        assert_eq!(answer["admitted"], "device");
        assert_eq!(answer["principal"]["id"], json!(ws.host.principal_id));
        assert_eq!(answer["principal"]["username"], "alice");
        assert_eq!(answer["device_id"], json!(device_id(&fresh.key)));
        assert_eq!(answer["workspace"]["name"], "baranzini-lab");
        assert!(answer.get("evicted_device").is_none());
        // The lost key is gone, and the new one carries the host's authority.
        let mut lost_host = Member::new(host_uid(), key(7));
        assert_unknown_device(&mut ws, &mut lost_host);
        ws.call_ok(&mut fresh, "workspace.rename", json!({"name": "lab"}));
        let listed = devices(&mut ws, &mut fresh);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["added_via"], "account");
        assert_eq!(listed[0]["this_device"], true);
    }

    /// The host's first device comes from `auth.account` when the workspace was created
    /// without a bootstrap key, and `auth.bootstrap` never admits an empty stored key (AO-7).
    #[test]
    fn the_hosts_first_device_needs_no_bootstrap_key() {
        let mut ws = Workspace::without_bootstrap_key("account-host-first");
        assert_eq!(ws.broker.state_json()["bootstrap_key"], "");
        let mut host = Member::new(host_uid(), key(7));
        let params = json!({"public_key": key_hex(&host.key)});
        let (code, message) = refused(signed_raw(
            &mut ws.broker,
            &mut host,
            "auth.bootstrap",
            params,
        ));
        assert_eq!(code, "forbidden");
        assert_eq!(
            message,
            "forbidden: bootstrap key/identity mismatch or already enrolled"
        );
        let answer = ws.account_ok(&mut host, json!({}));
        assert_eq!(answer["role"], "host");
        assert_eq!(answer["admitted"], "host");
        assert_eq!(answer["principal"]["username"], "alice");
        let record = ws.last_record();
        assert_eq!(record["operation"], "auth.account");
        assert_eq!(record["actor"], answer["principal"]["id"]);
        ws.host = host;
        let snapshot = ws.host_snapshot();
        assert_eq!(
            snapshot["workspace"]["host_principal_id"],
            answer["principal"]["id"]
        );
        ws.host_ok(
            "policy.set",
            json!({"mode": "private", "institution_id": "ucsf"}),
        );
        // Once the host has a principal, `auth.bootstrap` stays shut whatever the key.
        let mut other = Member::new(host_uid(), key(51));
        let params = json!({"public_key": key_hex(&other.key)});
        let (code, _) = refused(signed_raw(
            &mut ws.broker,
            &mut other,
            "auth.bootstrap",
            params,
        ));
        assert_eq!(code, "forbidden");
    }

    /// A workspace that does have a bootstrap key still lets the host account add its first
    /// device by account, and the bootstrap key is then shut (devices are no longer empty).
    #[test]
    fn the_hosts_first_device_by_account_beside_a_bootstrap_key() {
        let mut ws = Workspace::unbootstrapped("account-host-first-keyed");
        let mut host = Member::new(host_uid(), key(52));
        assert_eq!(ws.account_ok(&mut host, json!({}))["admitted"], "host");
        let mut bootstrap = Member::new(host_uid(), key(7));
        let params = json!({"public_key": key_hex(&bootstrap.key)});
        let (code, _) = refused(signed_raw(
            &mut ws.broker,
            &mut bootstrap,
            "auth.bootstrap",
            params,
        ));
        assert_eq!(code, "forbidden");
    }

    /// A renamed host account is followed: the host principal takes the new name in the same
    /// record, and the host's older devices authenticate again.
    #[test]
    fn the_renamed_host_is_followed() {
        let mut ws = named("account-host-renamed");
        ws.directory.set(host_uid(), "alicia", Some("Alice Chen"));
        let (_, message) = refused(ws.host_call("workspace.snapshot", json!({})));
        assert_eq!(message, "unauthorized: account enrollment changed");
        let mut fresh = Member::new(host_uid(), key(53));
        let answer = ws.account_ok(&mut fresh, json!({}));
        assert_eq!(answer["principal"]["username"], "alicia");
        assert_eq!(answer["principal"]["display_name"], "alicia");
        assert_eq!(answer["principal"]["id"], json!(ws.host.principal_id));
        let snapshot = ws.host_snapshot();
        assert_eq!(snapshot["actor"]["username"], "alicia");
        assert_eq!(snapshot["actor"]["nickname"], "alicia");
        // A retry of the same key after the next rename commits only the rename.
        ws.directory.set(host_uid(), "alix", None);
        let records = ws.journal_records();
        let answer = ws.account_ok(&mut fresh, json!({}));
        assert_eq!(answer["admitted"], "device");
        assert_eq!(answer["principal"]["username"], "alix");
        assert_eq!(ws.journal_records(), records + 1);
        assert!(!ws
            .last_record()
            .to_string()
            .contains(&device_id(&fresh.key)));
    }

    /// D3 on the account path: a host rename that would collide with an active member's name is
    /// refused, and nothing is written.
    #[test]
    fn a_host_rename_onto_a_members_name_is_refused() {
        let mut ws = named("account-host-rename-d3");
        ws.enroll(BOB, "bob", 11);
        ws.directory.set(host_uid(), "Bob", None);
        let records = ws.journal_records();
        let mut fresh = Member::new(host_uid(), key(54));
        let (code, message) = refused(ws.account(&mut fresh, json!({})));
        assert_eq!(code, "identity_conflict", "{message}");
        assert_eq!(ws.journal_records(), records);
    }

    // ── A member ──────────────────────────────────────────────────────────────────────────

    /// A member adds a second computer by signing in as their account: no invitation, no code,
    /// nobody's approval. The record is the member's own.
    #[test]
    fn a_member_adds_a_second_device_without_approval() {
        let mut ws = named("account-member-second");
        let mut desktop = ws.enroll(BOB, "bob", 11);
        let mut laptop = Member::new(BOB, key(60));
        let answer = ws.account_ok(&mut laptop, json!({}));
        assert_eq!(answer["role"], "member");
        assert_eq!(answer["admitted"], "device");
        assert_eq!(answer["principal"]["id"], json!(desktop.principal_id));
        let record = ws.last_record();
        assert_eq!(record["actor"], json!(desktop.principal_id));
        assert_eq!(record["operation"], "auth.account");
        let listed = devices(&mut ws, &mut laptop);
        assert_eq!(listed.len(), 2);
        let mine: Vec<&Value> = listed.iter().filter(|d| d["this_device"] == true).collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0]["fingerprint"], json!(fingerprint(&laptop.key)));
        assert_eq!(mine[0]["added_via"], "account");
        let from_desktop = devices(&mut ws, &mut desktop);
        let mine: Vec<&Value> = from_desktop
            .iter()
            .filter(|d| d["this_device"] == true)
            .collect();
        assert_eq!(mine[0]["fingerprint"], json!(fingerprint(&desktop.key)));
    }

    /// The same key sent again is a retry: the same principal, `admitted: "device"`, and no
    /// record. A key bound to someone else is `device_conflict`.
    #[test]
    fn a_retry_commits_nothing_and_another_principals_key_conflicts() {
        let mut ws = named("account-retry");
        let bob = ws.enroll(BOB, "bob", 11);
        let mut laptop = Member::new(BOB, key(61));
        ws.account_ok(&mut laptop, json!({}));
        let records = ws.journal_records();
        let again = ws.account_ok(&mut laptop, json!({}));
        assert_eq!(again["admitted"], "device");
        assert_eq!(again["principal"]["id"], json!(bob.principal_id));
        assert_eq!(ws.journal_records(), records);
        ws.enroll(CAROL, "carol", 12);
        let records = ws.journal_records();
        let mut carol_with_bobs_key = Member::new(CAROL, key(61));
        let (code, _) = refused(ws.account(&mut carol_with_bobs_key, json!({})));
        assert_eq!(code, "device_conflict");
        assert_eq!(ws.journal_records(), records);
    }

    /// `replaces_device_id`: one of the principal's own devices goes in the same record; an
    /// unknown one is ignored; another principal's is refused `forbidden` and nothing is written.
    #[test]
    fn replaces_device_id_removes_own_ignores_unknown_and_refuses_anothers() {
        let mut ws = named("account-replace");
        let desktop = ws.enroll(BOB, "bob", 11);
        let mut first = Member::new(BOB, key(62));
        ws.account_ok(
            &mut first,
            json!({"replaces_device_id": device_id(&desktop.key)}),
        );
        let mut old = Member::new(BOB, key(11));
        assert_unknown_device(&mut ws, &mut old);
        assert_eq!(devices(&mut ws, &mut first).len(), 1);
        let mut second = Member::new(BOB, key(63));
        ws.account_ok(&mut second, json!({"replaces_device_id": "ab".repeat(32)}));
        assert_eq!(devices(&mut ws, &mut second).len(), 2);
        let records = ws.journal_records();
        let mut third = Member::new(BOB, key(64));
        let (code, message) = refused(ws.account(
            &mut third,
            json!({"replaces_device_id": device_id(&ws.host.key)}),
        ));
        assert_eq!(code, "forbidden");
        assert_eq!(
            message,
            "forbidden: The device to replace belongs to someone else. Nothing was changed."
        );
        assert_eq!(ws.journal_records(), records);
        ws.host_snapshot();
    }

    /// At most 32 devices: the 33rd evicts the oldest (one without `added_at` first), in the
    /// same record, and says which. Nobody is refused for having too many computers.
    #[test]
    fn the_cap_evicts_the_oldest_device() {
        let mut ws = named("account-cap");
        let bob = ws.enroll(BOB, "bob", 11);
        let bob_id = bob.principal_id.clone();
        let first = device_id(&key(11));
        // Bob's first device predates `added_at`.
        let mut ws = ws.edit_journal(|journal| {
            journal.append(
                "system",
                "test.legacy",
                vec![set(
                    &["devices", first.as_str()],
                    legacy_device(&bob_id, &key(11)),
                )],
            );
        });
        let mut members = Vec::new();
        for seed in 100..(100 + MAX_DEVICES_PER_PRINCIPAL as u8 - 1) {
            let mut member = Member::new(BOB, key(seed));
            let answer = ws.account_ok(&mut member, json!({}));
            assert!(answer.get("evicted_device").is_none());
            members.push(member);
        }
        let count = |ws: &Workspace| {
            ws.broker.state_json()["devices"]
                .as_object()
                .unwrap()
                .values()
                .filter(|device| device["principal_id"] == json!(bob_id))
                .count()
        };
        assert_eq!(count(&ws), MAX_DEVICES_PER_PRINCIPAL);
        let mut over = Member::new(BOB, key(200));
        let answer = ws.account_ok(&mut over, json!({}));
        assert_eq!(
            answer["evicted_device"],
            json!({"fingerprint": fingerprint(&key(11)), "added_at": null})
        );
        assert_eq!(count(&ws), MAX_DEVICES_PER_PRINCIPAL);
        let mut legacy = Member::new(BOB, key(11));
        assert_unknown_device(&mut ws, &mut legacy);
        // Next, the oldest `added_at`, the lowest device ID among equals.
        let state = ws.broker.state_json();
        let mut own: Vec<(u64, String)> = state["devices"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, device)| device["principal_id"] == json!(bob_id))
            .map(|(id, device)| (device["added_at"].as_u64().unwrap(), id.clone()))
            .collect();
        own.sort();
        let (added_at, oldest) = own[0].clone();
        let mut next = Member::new(BOB, key(201));
        let answer = ws.account_ok(&mut next, json!({}));
        assert_eq!(
            answer["evicted_device"],
            json!({
                "fingerprint": biorouter_crew::invitation::grouped_fingerprint(&oldest),
                "added_at": added_at,
            })
        );
        assert_eq!(count(&ws), MAX_DEVICES_PER_PRINCIPAL);
        let record = ws.last_record();
        let removed: Vec<&Value> = record["patches"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|patch| patch["op"] == "remove")
            .collect();
        assert_eq!(removed.len(), 1, "{record}");
        assert_eq!(removed[0]["path"], json!(["devices", oldest]));
    }

    // ── An invited account ────────────────────────────────────────────────────────────────

    /// An account the host invited by name joins by signing in as it: no device code. The join
    /// and any legacy enrollment for the UID go in the same record, written as the new principal.
    #[test]
    fn an_invited_account_joins_with_no_code_in_a_managed_workspace() {
        let mut ws = named("account-invited");
        ws.directory.set(BOB, "bob", Some("Bob Lee"));
        ws.invite_by_name("bob");
        assert!(ws.broker.state_json().get("pending_joins").is_some());
        let mut bob = Member::new(BOB, key(11));
        let answer = ws.account_ok(&mut bob, json!({}));
        assert_eq!(answer["admitted"], "invitation");
        assert_eq!(answer["role"], "member");
        assert_eq!(answer["principal"]["username"], "bob");
        let state = ws.broker.state_json();
        assert!(state.get("pending_joins").is_none());
        let record = ws.last_record();
        assert_eq!(record["actor"], json!(bob.principal_id));
        assert_eq!(record["operation"], "auth.account");
        assert_eq!(ws.snapshot(&mut bob)["actor"]["username"], "bob");
        // A managed workspace adds the new member nowhere.
        let (team, _) = ws.host_team("Lab");
        let teams = ws.snapshot(&mut bob)["teams"].as_array().unwrap().len();
        assert_eq!(teams, 0, "{team}");
    }

    /// In an open workspace the new member lands in every team and unarchived channel, and a
    /// member's invitation admits too.
    #[test]
    fn an_invited_account_joins_with_no_code_in_an_open_workspace() {
        let mut ws = named("account-invited-open");
        let (team, general) = ws.host_team("Lab");
        ws.host_ok("workspace.set_governance", json!({"governance": "open"}));
        ws.directory.set(BOB, "bob", None);
        ws.invite_by_name("bob");
        let mut bob = Member::new(BOB, key(11));
        ws.account_ok(&mut bob, json!({}));
        let snapshot = ws.snapshot(&mut bob);
        assert!(snapshot["teams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == json!(team)));
        assert!(snapshot["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == json!(general)));
        ws.directory.set(CAROL, "carol", None);
        ws.call_ok(&mut bob, "enrollment.invite", json!({"username": "carol"}));
        let mut carol = Member::new(CAROL, key(12));
        assert_eq!(
            ws.account_ok(&mut carol, json!({}))["admitted"],
            "invitation"
        );
    }

    /// The invited branch keeps `auth.join`'s checks: a renamed account, a changed generation
    /// (here a lookalike who became a member, which D3 would refuse) and an inviter who lost
    /// the authority are refused, and nothing is written.
    #[test]
    fn the_invited_branch_keeps_the_join_checks() {
        let mut ws = named("account-invited-checks");
        ws.directory.set(BOB, "bob", None);
        ws.invite_by_name("bob");
        // Renamed before joining.
        ws.directory.set(BOB, "robert", None);
        let records = ws.journal_records();
        let mut bob = Member::new(BOB, key(11));
        let (code, message) = refused(ws.account(&mut bob, json!({})));
        assert_eq!(code, "account_changed", "{message}");
        assert_eq!(ws.journal_records(), records);
        // A lookalike enrolled by token after the invitation.
        ws.directory.set(BOB, "bob", None);
        ws.enroll(LOOKALIKE, "Bob", 30);
        let records = ws.journal_records();
        let (code, _) = refused(ws.account(&mut bob, json!({})));
        assert_eq!(code, "account_changed");
        assert_eq!(ws.journal_records(), records);
        // An open member's invitation ends when the workspace is managed again.
        let mut ws = named("account-invited-inviter");
        ws.host_ok("workspace.set_governance", json!({"governance": "open"}));
        let mut carol = ws.enroll(CAROL, "carol", 12);
        ws.directory.set(DAVE, "dave", None);
        ws.call_ok(&mut carol, "enrollment.invite", json!({"username": "dave"}));
        ws.host_ok("workspace.set_governance", json!({"governance": "managed"}));
        let mut dave = Member::new(DAVE, key(13));
        let (code, _) = refused(ws.account(&mut dave, json!({})));
        assert_eq!(code, "not_member");
    }

    /// An expired invitation admits nobody.
    #[test]
    fn an_expired_invitation_is_not_an_admission() {
        let mut ws = named("account-invited-expired");
        ws.directory.set(BOB, "bob", None);
        ws.invite_by_name("bob");
        let mut ws = ws.edit_journal(|journal| {
            journal.append(
                "system",
                "test.expire",
                vec![set(
                    &["pending_joins", "71001", "expires_at"],
                    json!(now() - 1),
                )],
            );
        });
        let mut bob = Member::new(BOB, key(11));
        let (code, _) = refused(ws.account(&mut bob, json!({})));
        assert_eq!(code, "not_member");
        let status = ws.status(BOB);
        assert_eq!(status["role"], "invited");
        assert_eq!(status["expired"], true);
    }

    // ── Refusals ──────────────────────────────────────────────────────────────────────────

    /// A stranger, a removed member and a renamed member are refused in AO-2's words, and
    /// nothing is written. A stranger costs no account lookup: the branch comes from state.
    #[test]
    fn strangers_removed_and_renamed_members_are_refused() {
        let mut ws = named("account-refusals");
        let records = ws.journal_records();
        let lookups = ws.directory.calls();
        let mut stranger = Member::new(STRANGER, key(70));
        let (code, message) = refused(ws.account(&mut stranger, json!({})));
        assert_eq!(code, "not_member");
        assert_eq!(
            message,
            "not_member: This account is not a member of baranzini-lab. Ask a member to invite it."
        );
        assert_eq!(ws.directory.calls(), lookups, "a stranger costs no lookup");
        assert_eq!(ws.journal_records(), records);

        let bob = ws.enroll(BOB, "bob", 11);
        ws.offboard(&bob.principal_id);
        let records = ws.journal_records();
        let lookups = ws.directory.calls();
        let mut back = Member::new(BOB, key(71));
        let (code, message) = refused(ws.account(&mut back, json!({})));
        assert_eq!(code, "removed");
        assert_eq!(
            message,
            "removed: You were removed from baranzini-lab. Ask a member to invite you again."
        );
        assert_eq!(ws.directory.calls(), lookups);
        assert_eq!(ws.journal_records(), records);

        let carol = ws.enroll(CAROL, "carol", 12);
        ws.directory.set(CAROL, "caroline", None);
        let records = ws.journal_records();
        let mut renamed = Member::new(CAROL, key(72));
        let (code, message) = refused(ws.account(&mut renamed, json!({})));
        assert_eq!(code, "account_changed");
        assert_eq!(
            message,
            "account_changed: This server account is no longer named @carol, the name it joined with. Ask the host to remove @carol and invite you again."
        );
        assert_eq!(ws.journal_records(), records);
        assert!(!carol.principal_id.is_empty());
    }

    /// Another UID cannot add a key to someone else's principal: its own UID decides the
    /// branch, and a stranger's UID is not a member whatever key it signs with.
    #[test]
    fn another_uid_cannot_add_a_device_for_someone_else() {
        let mut ws = named("account-other-uid");
        let bob = ws.enroll(BOB, "bob", 11);
        let mut posing = Member::new(STRANGER, key(73));
        let (code, _) = refused(ws.account(
            &mut posing,
            json!({"replaces_device_id": device_id(&bob.key)}),
        ));
        assert_eq!(code, "not_member");
        // A member signing with a key bound to the host cannot take it over either.
        let mut with_hosts_key = Member::new(BOB, key(7));
        let (code, _) = refused(ws.account(&mut with_hosts_key, json!({})));
        assert_eq!(code, "device_conflict");
    }

    /// State first: a member with a bad signature, and a request without one, are refused
    /// before any account lookup.
    #[test]
    fn a_bad_signature_is_refused_before_any_lookup() {
        let mut ws = named("account-signature");
        ws.enroll(BOB, "bob", 11);
        let lookups = ws.directory.calls();
        let mut bob = Member::new(BOB, key(74));
        // Signed by another key than the one it names.
        let (code, _) = refused(signed_raw(
            &mut ws.broker,
            &mut bob,
            "auth.account",
            json!({"public_key": key_hex(&key(75))}),
        ));
        assert_eq!(code, "unauthorized");
        let (code, _) = refused(ws.broker.handle(
            BOB,
            &mut Connection::new(),
            request(
                "unsigned",
                "auth.account",
                json!({"public_key": key_hex(&key(74))}),
            ),
        ));
        assert_eq!(code, "unauthorized");
        assert_eq!(ws.directory.calls(), lookups);
    }

    /// A worker credential never reaches the account path.
    #[test]
    fn a_worker_credential_is_refused() {
        let mut ws = named("account-worker");
        let response = ws.broker.handle(
            host_uid(),
            &mut Connection::new(),
            Request {
                version: 1,
                id: "worker".into(),
                method: "auth.account".into(),
                params: json!({"public_key": key_hex(&key(76))}),
                auth: None,
                credential: Some("credential".into()),
            },
        );
        let (code, message) = refused(response);
        assert_eq!(code, "invalid_request");
        assert_eq!(message, "invalid_request: mixed authentication");
    }

    // ── Quotas ────────────────────────────────────────────────────────────────────────────

    /// A member past their journal share can still add one computer a day, so they can get
    /// reading back; a second within the day is refused. `device.revoke` is never refused for
    /// the share.
    #[test]
    fn the_journal_share_admits_one_account_record_a_day_past_it() {
        let mut ws = named("account-share");
        let mut bob = ws.enroll(BOB, "bob", 11);
        ws.broker.set_quotas(Quotas {
            member_journal_bytes: 1,
            ..Quotas::STANDARD
        });
        let (code, _) = refused(ws.call(&mut bob, "profile.update", json!({"nickname": "Bobby"})));
        assert_eq!(code, "quota_exceeded");
        let mut laptop = Member::new(BOB, key(80));
        ws.account_ok(&mut laptop, json!({}));
        let mut tablet = Member::new(BOB, key(81));
        let (code, message) = refused(ws.account(&mut tablet, json!({})));
        assert_eq!(code, "quota_exceeded", "{message}");
        let revoked = ws.call_ok(
            &mut laptop,
            "device.revoke",
            json!({"fingerprint": fingerprint(&bob.key)}),
        );
        assert_eq!(revoked["revoked"], true);
        // The host answers to no share.
        let mut host_laptop = Member::new(host_uid(), key(82));
        ws.account_ok(&mut host_laptop, json!({}));
        let mut host_tablet = Member::new(host_uid(), key(83));
        ws.account_ok(&mut host_tablet, json!({}));
    }

    // ── account.status ────────────────────────────────────────────────────────────────────

    /// `account.status` for every role, from state alone and only about the caller.
    #[test]
    fn account_status_answers_each_role_from_state_alone() {
        let mut ws = Workspace::unbootstrapped("account-status");
        let lookups = ws.directory.calls();
        let host = ws.status(host_uid());
        assert_eq!(host["role"], "host");
        assert_eq!(host["devices"], 0);
        assert_eq!(host["others_connected"], 0);
        assert!(host.get("username").is_none());
        assert!(host.get("host_username").is_none());
        assert_eq!(host["workspace_name"], Value::Null);
        assert_eq!(
            ws.directory.calls(),
            lookups,
            "account.status makes no lookup"
        );
        let mut host_member = Member::new(host_uid(), key(7));
        ws.account_ok(&mut host_member, json!({}));
        ws.host = host_member;
        ws.host_ok("workspace.rename", json!({"name": "baranzini-lab"}));

        let host = ws.status(host_uid());
        assert_eq!(host["role"], "host");
        assert_eq!(host["username"], "alice");
        assert_eq!(host["host_username"], "alice");
        assert_eq!(host["devices"], 1);
        assert_eq!(host["workspace_name"], "baranzini-lab");

        let none = ws.status(STRANGER);
        assert_eq!(
            none,
            json!({"role": "none", "workspace_name": "baranzini-lab", "host_username": "alice"})
        );

        ws.directory.set(BOB, "bob", None);
        ws.invite_by_name("bob");
        let lookups = ws.directory.calls();
        let invited = ws.status(BOB);
        assert_eq!(invited["role"], "invited");
        assert_eq!(invited["username"], "bob");
        assert_eq!(
            invited["invited_by"],
            json!({"username": "alice", "display_name": "alice"})
        );
        assert_eq!(invited["expired"], false);
        assert!(invited["expires_at"].as_u64().unwrap() > now());
        assert!(invited.get("devices").is_none());
        assert!(invited.get("others_connected").is_none());

        let mut bob = Member::new(BOB, key(11));
        ws.account_ok(&mut bob, json!({}));
        let lookups_after_join = ws.directory.calls();
        let member = ws.status(BOB);
        assert_eq!(
            member,
            json!({
                "role": "member", "workspace_name": "baranzini-lab", "host_username": "alice",
                "username": "bob", "devices": 1,
            })
        );
        ws.offboard(&bob.principal_id);
        let lookups_after_offboard = ws.directory.calls();
        let removed = ws.status(BOB);
        assert_eq!(removed["role"], "removed");
        assert_eq!(removed["username"], "bob");
        assert!(removed.get("devices").is_none());
        assert_eq!(ws.directory.calls(), lookups_after_offboard);
        assert!(
            lookups_after_join > lookups,
            "the join itself looked the account up once"
        );
    }

    /// The host's `others_connected` counts live connections from other UIDs, and goes down as
    /// they close. Nobody else is told.
    #[test]
    fn the_host_is_told_how_many_other_connections_are_open() {
        let mut ws = Workspace::new("account-others");
        assert_eq!(ws.status(host_uid())["others_connected"], 0);
        let mut bob = ws.enroll(BOB, "bob", 11);
        // Bob's enrollment connection is still open.
        assert_eq!(ws.status(host_uid())["others_connected"], 1);
        let mut second = Connection::new();
        ok(account_status(&mut ws.broker, BOB, &mut second));
        let mut carols = Connection::new();
        ok(account_status(&mut ws.broker, CAROL, &mut carols));
        assert_eq!(ws.status(host_uid())["others_connected"], 3);
        // The host's own connections never count.
        let mut host_extra = Connection::new();
        ok(account_status(&mut ws.broker, host_uid(), &mut host_extra));
        assert_eq!(ws.status(host_uid())["others_connected"], 3);
        assert!(ws.status(BOB).get("others_connected").is_none());
        ws.broker.connection_closed(&mut bob.connection);
        ws.broker.connection_closed(&mut second);
        ws.broker.connection_closed(&mut carols);
        assert_eq!(ws.status(host_uid())["others_connected"], 0);
        // Closing a connection twice changes nothing.
        ws.broker.connection_closed(&mut second);
        assert_eq!(ws.status(host_uid())["others_connected"], 0);
        ws.broker.connection_closed(&mut host_extra);
    }

    // ── device.revoke ─────────────────────────────────────────────────────────────────────

    /// A person revokes one of their own devices by the fingerprint the snapshot shows, the one
    /// signing the request included; a fingerprint that is not one of theirs is `invalid_params`.
    #[test]
    fn a_person_revokes_their_own_devices_by_fingerprint() {
        let mut ws = named("account-revoke");
        let mut desktop = ws.enroll(BOB, "bob", 11);
        let mut laptop = Member::new(BOB, key(90));
        ws.account_ok(&mut laptop, json!({}));
        for wrong in [
            json!({"fingerprint": fingerprint(&ws.host.key)}),
            json!({"fingerprint": device_id(&desktop.key)}),
            json!({"fingerprint": "not a fingerprint"}),
            json!({"fingerprint": 7}),
            json!({}),
        ] {
            let (code, message) = refused(ws.call(&mut laptop, "device.revoke", wrong.clone()));
            assert_eq!(code, "invalid_params", "{wrong}: {message}");
        }
        // Lower case without spaces names the same device.
        let typed = fingerprint(&desktop.key).replace(' ', "").to_lowercase();
        let answer = ws.call_ok(&mut laptop, "device.revoke", json!({"fingerprint": typed}));
        assert_eq!(
            answer,
            json!({"revoked": true, "fingerprint": fingerprint(&desktop.key), "this_device": false})
        );
        assert_unknown_device(&mut ws, &mut desktop);
        let record = ws.last_record();
        assert_eq!(record["operation"], "device.revoke");
        assert_eq!(record["actor"], json!(desktop.principal_id));
        // Its own device: the answer says so, and the key stops working.
        let params = json!({"fingerprint": fingerprint(&laptop.key)});
        let answer = ws.call_ok(&mut laptop, "device.revoke", params);
        assert_eq!(answer["this_device"], true);
        assert_unknown_device(&mut ws, &mut laptop);
        // The person is still a member, with no device: the account adds one again.
        assert_eq!(ws.status(BOB)["role"], "member");
        assert_eq!(ws.status(BOB)["devices"], 0);
        let mut again = Member::new(BOB, key(91));
        assert_eq!(ws.account_ok(&mut again, json!({}))["admitted"], "device");
    }

    /// `device.revoke` is a person's: an agent's grant cannot reach it.
    #[test]
    fn an_agent_cannot_revoke_a_device() {
        let mut ws = named("account-revoke-agent");
        let mut bob = ws.enroll(BOB, "bob", 11);
        let (_, general) = ws.create_team(&mut bob, "Lab");
        let epoch = ws.broker.workspace().policy_epoch;
        let credential = ws.call_ok(
            &mut bob,
            "run.create",
            json!({
                "channel_id": general, "source_channels": [],
                "provider_policy_id": "private", "personal_mode": "private",
                "public_provider": false, "expires_in": 600,
                "expected_workspace_policy_epoch": epoch,
                "expected_protected_context": true,
                "workspace_institution_id": "ucsf", "connection_institution_id": "ucsf",
                "provider_affiliation": {"kind": "local"},
            }),
        )["credential"]
            .as_str()
            .unwrap()
            .to_owned();
        let response = ws.broker.handle(
            BOB,
            &mut Connection::new(),
            Request {
                version: 1,
                id: "agent-revoke".into(),
                method: "device.revoke".into(),
                params: json!({"fingerprint": fingerprint(&bob.key), "idempotency_key": "k"}),
                auth: None,
                credential: Some(credential),
            },
        );
        let (_, message) = refused(response);
        assert_eq!(message, "forbidden: worker operation unavailable");
        ws.snapshot(&mut bob);
    }

    // ── The capability and the version ────────────────────────────────────────────────────

    /// `account_identity_v1` is advertised last, right after `join_by_name_v1`.
    #[test]
    fn the_capability_comes_last() {
        let capabilities = Broker::capabilities();
        let n = capabilities.len();
        assert_eq!(
            &capabilities[n - 2..],
            &["join_by_name_v1", "account_identity_v1"]
        );
    }

    /// `hello` signs the capability inside `signature_v2`, and carries `version` outside both
    /// signed payloads. Needs a machine identity (Linux).
    #[cfg(target_os = "linux")]
    #[test]
    fn hello_signs_the_capability_and_not_the_version() {
        use biorouter_crew::{hello_v1_payload, HelloV2, Mode};
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        if !std::path::Path::new("/etc/machine-id").exists() {
            eprintln!("skipped: hello needs /etc/machine-id");
            return;
        }
        let mut ws = Workspace::new("account-hello");
        let hello = ok(ws.broker.handle(
            BOB,
            &mut Connection::new(),
            request("hello", "hello", json!({"challenge_nonce": "n-1"})),
        ));
        assert_eq!(hello["version"], env!("CARGO_PKG_VERSION"));
        let capabilities: Vec<&str> = hello["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap())
            .collect();
        assert_eq!(capabilities.last(), Some(&"account_identity_v1"));
        let key_hex = hello["workspace_public_key"].as_str().unwrap();
        let key =
            VerifyingKey::from_bytes(&hex::decode(key_hex).unwrap().try_into().unwrap()).unwrap();
        let signature = |field: &str| {
            Signature::from_slice(&hex::decode(hello[field].as_str().unwrap()).unwrap()).unwrap()
        };
        let workspace_id = hello["workspace_id"].as_str().unwrap();
        let node_id = hello["node_id"].as_str().unwrap();
        let host = hello["host_uid"].as_u64().unwrap() as u32;
        key.verify(
            &hello_v1_payload(workspace_id, host, "n-1", key_hex, node_id),
            &signature("signature"),
        )
        .expect("v1 does not cover the version");
        let v2 = HelloV2 {
            workspace_id,
            host_uid: host,
            challenge_nonce: "n-1",
            workspace_public_key: key_hex,
            node_id,
            mode: &Mode::Private,
            institution_id: Some("ucsf"),
            policy_epoch: hello["policy_epoch"].as_u64().unwrap(),
            name: None,
            capabilities: &capabilities,
        };
        key.verify(&v2.signing_payload(), &signature("signature_v2"))
            .expect("v2 covers the capabilities and not the version");
        let stripped: Vec<&str> = capabilities[..capabilities.len() - 1].to_vec();
        assert!(key
            .verify(
                &HelloV2 {
                    capabilities: &stripped,
                    ..v2
                }
                .signing_payload(),
                &signature("signature_v2"),
            )
            .is_err());
        ws.host_snapshot();
    }

    // ── The journal ───────────────────────────────────────────────────────────────────────

    /// The real broker writes the shapes the downgrade test replays: `set` and `remove` patches
    /// on principals, devices, pending joins and idempotency results, and a device carrying only
    /// the four fields every broker knows.
    #[test]
    fn the_account_path_writes_only_shapes_an_older_broker_replays() {
        let mut ws = Workspace::without_bootstrap_key("account-shapes");
        let mut host = Member::new(host_uid(), key(7));
        ws.account_ok(&mut host, json!({}));
        ws.host = host;
        ws.directory.set(BOB, "bob", None);
        ws.invite_by_name("bob");
        let mut desktop = Member::new(BOB, key(11));
        ws.account_ok(&mut desktop, json!({}));
        let mut laptop = Member::new(BOB, key(12));
        ws.account_ok(
            &mut laptop,
            json!({"replaces_device_id": device_id(&desktop.key)}),
        );
        ws.directory.set(host_uid(), "alicia", None);
        let mut second = Member::new(host_uid(), key(13));
        ws.account_ok(&mut second, json!({}));
        let params = json!({"fingerprint": fingerprint(&second.key)});
        ws.call_ok(&mut second, "device.revoke", params);
        let journal = String::from_utf8(ws.journal_bytes()).unwrap();
        let records: Vec<Value> = journal
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|record: &Value| {
                matches!(
                    record["operation"].as_str(),
                    Some("auth.account" | "device.revoke")
                )
            })
            .collect();
        assert_eq!(records.len(), 5);
        for record in &records {
            assert_ne!(record["actor"], "system");
            assert!(!record["actor"].as_str().unwrap().starts_with("uid:"));
            for patch in record["patches"].as_array().unwrap() {
                let op = patch["op"].as_str().unwrap();
                let path: Vec<&str> = patch["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap())
                    .collect();
                assert!(matches!(op, "set" | "remove"), "{patch}");
                assert!(
                    matches!(
                        path[0],
                        "principals" | "devices" | "pending_joins" | "dedupe" | "sequence"
                    ),
                    "{patch}"
                );
                if path[0] == "devices" && op == "set" {
                    let device = patch["value"].as_object().unwrap();
                    let mut fields: Vec<&str> = device.keys().map(String::as_str).collect();
                    fields.sort();
                    assert_eq!(
                        fields,
                        ["added_at", "added_via", "principal_id", "public_key"]
                    );
                    assert_eq!(device["added_via"], "account");
                }
            }
        }
        // And it all replays.
        let mut ws = ws.reopen();
        ws.host_snapshot();
        assert_eq!(ws.snapshot(&mut laptop)["actor"]["username"], "bob");
    }
}
