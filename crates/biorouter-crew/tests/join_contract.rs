//! Broker contract for joining a workspace by invitation and device code, slice S3a of
//! `docs/research/biorouter-crew/naming-design.md` ("Broker protocol (S3a)", "The device code",
//! "Security analysis" and "Tests").
//!
//! Most of this file needs the `join-by-name` feature:
//!
//! ```text
//! cargo test -p biorouter-crew --features test-seams,join-by-name --test join_contract
//! ```
//!
//! **Always compiled:** the downgrade replay. A journal in which a join was added and then
//! removed (one top-level `Set [pending_joins]`, one `Remove [pending_joins]`) replays and keeps
//! working on a broker built **without** the feature; running this file without
//! `join-by-name` is that build. The feature build pins that the real broker writes exactly
//! those two shapes, so the two runs together cover the downgrade.
//!
//! **The previous release's binary.** No released Biorouter carries the Crew broker yet, so the
//! "previous binary" is the broker as it was before this campaign's naming work (`76b88555`,
//! which predates `pending_joins`, `Workspace.name` and `Device.added_via`). The reviewer's run,
//! on Linux as an ordinary user with `/etc/machine-id`:
//!
//! 1. Build the old broker from `git archive 76b88555 crates/biorouter-crew` in a scratch
//!    workspace (`cargo build --bin biorouter-crew`), and this tree's broker with
//!    `--features join-by-name`.
//! 2. With the new binary: `start --state-dir S --bootstrap-key K`, bootstrap the host, invite an
//!    account by `@username`, cancel it (or let it join), then `stop`.
//! 3. With the old binary: `start --state-dir S --bootstrap-key K` must print `running` (the
//!    journal replays; unknown fields are ignored because `State` has no
//!    `deny_unknown_fields`), a signed `workspace.snapshot` and a mutation must succeed, then
//!    `stop`.
//! 4. With the new binary again: `start` must replay the journal the old binary extended.
//!
//! A join still **pending** when a host downgrades is ignored by the old broker and comes back,
//! unchanged, after an upgrade: every claim then re-checks expiry, the account name and the
//! principal generation, so it can only fail closed.
//!
//! Tests that sign `hello` or run a real broker need Linux (a machine identity and
//! `SO_PEERCRED`); the rest run on every Unix.
#![cfg(unix)]

mod support;

use biorouter_crew::PendingJoin;
use serde_json::{json, Value};
use support::*;

/// A journal with a pending join added and then removed replays on a broker built without
/// `join-by-name`, and the broker keeps committing and replaying afterwards.
#[test]
fn a_journal_with_a_join_added_and_removed_replays_without_the_feature() {
    let ws = Workspace::new("join-downgrade");
    let host = ws.host.principal_id.clone();
    let join = PendingJoin {
        join_id: "0a".repeat(16),
        uid: 71_001,
        username: "bob".into(),
        full_name: Some("Bob Lee".into()),
        inviter_id: host.clone(),
        existing_principal_id: None,
        generation: vec![],
        approved_code: Some("7QK2M9XA3JTPWZ4D".into()),
        created_at: now(),
        expires_at: now() + 86_400,
    };
    let mut ws = ws.edit_journal(|journal| {
        journal.append(
            &host,
            "enrollment.invite",
            vec![set(&["pending_joins"], json!({ "71001": join }))],
        );
        journal.append(&host, "enrollment.cancel", vec![remove(&["pending_joins"])]);
    });
    let snapshot = ws.host_snapshot();
    assert!(snapshot
        .get("pending_joins")
        .is_none_or(|joins| joins == &json!([])));
    ws.host_team("Lab");
    let journal = String::from_utf8(ws.journal_bytes()).unwrap();
    let last = journal.lines().last().unwrap();
    assert!(!last.contains("pending_joins"), "{last}");
    let mut ws = ws.reopen();
    ws.host_team("Methods");
}

#[cfg(feature = "join-by-name")]
mod join {
    use super::*;
    use biorouter_crew::invitation::{self, WorkspaceInvitation};
    use biorouter_crew::{
        device_code, format_device_code, signing_payload, Account, Broker, Connection, DeviceAuth,
        Directory, Mode, Request, Response,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    pub(super) const BOB: u32 = 71_001;
    pub(super) const CAROL: u32 = 71_002;
    pub(super) const DAVE: u32 = 71_003;

    static FRAME: AtomicUsize = AtomicUsize::new(0);

    fn frame_id(prefix: &str) -> String {
        format!("{prefix}-{}", FRAME.fetch_add(1, Ordering::SeqCst))
    }

    // -----------------------------------------------------------------------------------------
    // Local helpers
    // -----------------------------------------------------------------------------------------

    /// The workspace public key `W`, derived from the signing key the first journal record
    /// holds. It is the key `hello` returns (checked on Linux, where `hello` can sign).
    pub(super) fn workspace_key(ws: &Workspace) -> [u8; 32] {
        let journal = String::from_utf8(ws.journal_bytes()).unwrap();
        let first: Value = serde_json::from_str(journal.lines().next().unwrap()).unwrap();
        let secret = first["patches"][0]["value"]["workspace_signing_key"]
            .as_str()
            .expect("the initial record holds the workspace identity");
        let secret: [u8; 32] = hex::decode(secret).unwrap().try_into().unwrap();
        SigningKey::from_bytes(&secret).verifying_key().to_bytes()
    }

    /// The pinned workspace key, as the joiner's daemon gets it: from the host's invitation line,
    /// round-tripped through the real codec.
    fn pinned_from_invitation(ws: &Workspace) -> [u8; 32] {
        let line = invitation::encode(&WorkspaceInvitation {
            workspace_id: ws.broker.workspace().id.clone(),
            workspace_public_key: hex::encode(workspace_key(ws)),
            socket_path: "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock".into(),
            owner_uid: host_uid().max(1),
            workspace_name: Some("lab".into()),
            host_username: Some("alice".into()),
            host_display_name: Some("Alice Chen".into()),
            mode: Some(Mode::Private),
            institution_id: Some("ucsf".into()),
            ssh_host: Some("hpc.example.edu".into()),
            ssh_port: None,
            proxy_jump: None,
            invitee_username: Some("bob".into()),
        })
        .unwrap();
        let message = format!("Join lab on Crew.\nPaste this whole message.\n{line}");
        let parsed = invitation::parse(&message).unwrap().invitation;
        hex::decode(parsed.workspace_public_key)
            .unwrap()
            .try_into()
            .unwrap()
    }

    /// A joiner's desktop: its own device key, and the workspace ID and key it pinned from the
    /// invitation. The code it shows is computed here, from nothing the broker returns.
    pub(super) struct Desktop {
        pub member: Member,
        workspace_id: String,
        pinned: [u8; 32],
    }

    impl Desktop {
        pub fn new(ws: &Workspace, uid: u32, seed: u8) -> Self {
            Self {
                member: Member::new(uid, key(seed)),
                workspace_id: ws.broker.workspace().id.clone(),
                pinned: pinned_from_invitation(ws),
            }
        }
        /// The 16-character code this desktop's screen shows.
        pub fn code(&self) -> String {
            device_code(
                &self.workspace_id,
                &self.pinned,
                &self.member.key.verifying_key().to_bytes(),
            )
        }
        /// The unsigned `enrollment.pending` frame.
        pub fn status(&mut self, broker: &mut Broker) -> Value {
            let uid = self.member.uid;
            ok(broker.handle(uid, &mut self.member.connection, pending_frame()))
        }
        /// Challenge, sign and send `auth.join` over this desktop's own connection.
        pub fn claim(&mut self, broker: &mut Broker, join_id: &str) -> Response {
            let params = json!({"public_key": key_hex(&self.member.key), "join_id": join_id});
            signed_raw(broker, &mut self.member, "auth.join", params)
        }
    }

    pub(super) fn pending_frame() -> Request {
        request(&frame_id("pending"), "enrollment.pending", json!({}))
    }

    /// `enrollment.pending` for `uid` over a fresh connection.
    pub(super) fn status_of(ws: &mut Workspace, uid: u32) -> Value {
        ok(ws
            .broker
            .handle(uid, &mut Connection::new(), pending_frame()))
    }

    pub(super) fn join_id_of(ws: &mut Workspace, uid: u32) -> String {
        status_of(ws, uid)["join_id"]
            .as_str()
            .expect("an invited UID has a join ID")
            .to_owned()
    }

    pub(super) fn invite(ws: &mut Workspace, username: &str) -> Response {
        ws.host_call("enrollment.invite", json!({ "username": username }))
    }

    pub(super) fn approve(ws: &mut Workspace, username: &str, code: &str) -> Response {
        ws.host_call(
            "enrollment.approve",
            json!({"username": username, "code": code}),
        )
    }

    /// The host's `pending_joins` projection.
    pub(super) fn host_joins(ws: &mut Workspace) -> Vec<Value> {
        ws.host_snapshot()["pending_joins"]
            .as_array()
            .expect("the host snapshot lists pending joins")
            .clone()
    }

    /// Sign `method(params)` with `signer` over the nonce `nonce` for `uid`.
    pub(super) fn signed_frame(
        ws: &Workspace,
        uid: u32,
        signer: &SigningKey,
        nonce: &str,
        method: &str,
        params: Value,
    ) -> Request {
        let signature = signer.sign(&signing_payload(
            &ws.broker.workspace().id,
            uid,
            nonce,
            method,
            &params,
        ));
        let mut frame = request(&frame_id("signed"), method, params);
        frame.auth = Some(DeviceAuth {
            device_id: device_id(signer),
            nonce: nonce.into(),
            signature: hex::encode(signature.to_bytes()),
        });
        frame
    }

    /// Ask for a challenge for `device` on `connection`; returns the nonce.
    pub(super) fn challenge(
        broker: &mut Broker,
        uid: u32,
        connection: &mut Connection,
        device: &SigningKey,
    ) -> String {
        let frame = request(
            &frame_id("challenge"),
            "auth.challenge",
            json!({"device_id": device_id(device)}),
        );
        ok(broker.handle(uid, connection, frame))["nonce"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// A claim for `join_id` by `signer`, running as `uid`, on `connection`.
    pub(super) fn claim_on(
        ws: &mut Workspace,
        uid: u32,
        connection: &mut Connection,
        signer: &SigningKey,
        join_id: &str,
    ) -> Response {
        let nonce = challenge(&mut ws.broker, uid, connection, signer);
        let params = json!({"public_key": key_hex(signer), "join_id": join_id});
        let frame = signed_frame(ws, uid, signer, &nonce, "auth.join", params);
        ws.broker.handle(uid, connection, frame)
    }

    /// Whether `signer` is a device of the workspace: a signed read with it authenticates.
    pub(super) fn is_device(ws: &mut Workspace, uid: u32, signer: &SigningKey) -> bool {
        let mut member = Member::new(uid, signer.clone());
        signed(&mut ws.broker, &mut member, "workspace.snapshot", json!({}))
            .error
            .is_none()
    }

    /// Every record of the journal, parsed.
    pub(super) fn records(ws: &Workspace) -> Vec<Value> {
        String::from_utf8(ws.journal_bytes())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Every spelling of a code a leak could take.
    pub(super) fn spellings(code: &str) -> [String; 3] {
        [
            code.to_owned(),
            format_device_code(code),
            code.to_ascii_lowercase(),
        ]
    }

    /// Bob invited by the host: `(desktop, join_id)`.
    pub(super) fn invited_bob(ws: &mut Workspace, seed: u8) -> (Desktop, String) {
        ws.directory.set(BOB, "bob", Some("Bob Lee"));
        ok(invite(ws, "@bob"));
        let desktop = Desktop::new(ws, BOB, seed);
        let join_id = join_id_of(ws, BOB);
        (desktop, join_id)
    }

    // -----------------------------------------------------------------------------------------
    // An NSS stand-in with aliases and case-insensitive lookups
    // -----------------------------------------------------------------------------------------

    /// An account database that, like some NSS back ends, resolves aliases (a second name on
    /// the same UID) and, optionally, looks names up without regard to case.
    #[derive(Clone, Default)]
    struct Nss {
        accounts: Arc<Mutex<BTreeMap<u32, Account>>>,
        aliases: Arc<Mutex<BTreeMap<String, u32>>>,
        case_insensitive: bool,
        calls: Arc<AtomicUsize>,
    }

    impl Nss {
        fn set(&self, uid: u32, name: &str) {
            let account = Account {
                uid,
                name: name.into(),
                full_name: None,
            };
            self.accounts.lock().unwrap().insert(uid, account);
        }
        fn alias(&self, alias: &str, uid: u32) {
            self.aliases.lock().unwrap().insert(alias.into(), uid);
        }
    }

    impl Directory for Nss {
        fn by_uid(&self, uid: u32) -> anyhow::Result<Account> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.accounts
                .lock()
                .unwrap()
                .get(&uid)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("identity_unavailable"))
        }
        fn by_name(&self, name: &str) -> anyhow::Result<Account> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(uid) = self.aliases.lock().unwrap().get(name) {
                return Ok(Account {
                    uid: *uid,
                    name: name.into(),
                    full_name: None,
                });
            }
            self.accounts
                .lock()
                .unwrap()
                .values()
                .find(|account| {
                    account.name == name
                        || (self.case_insensitive && account.name.eq_ignore_ascii_case(name))
                })
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("identity_unavailable"))
        }
    }

    /// A bootstrapped broker over `nss`, hosted by `@alice`.
    fn broker_over(root: &TempRoot, nss: &Nss) -> (Broker, Member) {
        nss.set(host_uid(), "alice");
        let mut host = Member::new(host_uid(), key(7));
        let mut broker =
            Broker::open_with_directory(root.path(), &key_hex(&host.key), Box::new(nss.clone()))
                .unwrap();
        let params = json!({"public_key": key_hex(&host.key)});
        let result = ok(signed_raw(&mut broker, &mut host, "auth.bootstrap", params));
        host.principal_id = result["principal"]["id"].as_str().unwrap().to_owned();
        (broker, host)
    }

    #[test]
    fn canonicalization_refuses_aliases_case_differences_digits_and_unknown_names() {
        let root = TempRoot::new("join-canonical");
        let nss = Nss {
            case_insensitive: true,
            ..Nss::default()
        };
        let (mut broker, mut host) = broker_over(&root, &nss);
        nss.set(BOB, "bob");
        nss.alias("bobby", BOB);
        let mut attempt = |username: &str| {
            signed(
                &mut broker,
                &mut host,
                "enrollment.invite",
                json!({ "username": username }),
            )
        };

        let (code, message) = refused(attempt("@bobby"));
        assert_eq!(code, "identity_ambiguous");
        assert!(
            message.contains("@bobby") && message.contains("Invite @bob"),
            "{message}"
        );

        let (code, message) = refused(attempt("Bob"));
        assert_eq!(code, "identity_ambiguous");
        assert!(
            message.contains("@bob."),
            "the canonical spelling: {message}"
        );

        for digits in ["71001", "@1001", "0"] {
            let (code, message) = refused(attempt(digits));
            assert_eq!(code, "name_invalid", "{digits}");
            assert!(message.contains("numeric user ID"), "{message}");
        }
        for invalid in [
            "",
            "@",
            "bo b",
            "bob/x",
            "bob:x",
            "bo\u{7}b",
            "bob\u{202e}",
            "bob\u{200b}",
        ] {
            let (code, message) = refused(attempt(invalid));
            assert_eq!(code, "name_invalid", "{invalid:?}");
            assert!(
                !message.contains("bo"),
                "an invalid name is never echoed: {message}"
            );
        }
        let (code, _) = refused(attempt("@nobody"));
        assert_eq!(code, "unknown_account");

        // NSS may hand back a canonical name the character rules refuse; it is refused before
        // it could be echoed.
        nss.set(DAVE, "dave\u{202e}");
        nss.alias("dave", DAVE);
        let (code, message) = refused(attempt("dave"));
        assert_eq!(code, "identity_unavailable");
        assert!(!message.contains('\u{202e}'), "{message}");

        let invited = ok(attempt("@bob"));
        assert_eq!(invited["username"], "bob");
        assert_eq!(invited["add_device"], false);
    }

    #[test]
    fn an_invite_lookup_is_reused_for_thirty_seconds() {
        let mut ws = Workspace::new("join-cache");
        ws.directory.set(BOB, "bob", None);
        let before = ws.directory.calls();
        ok(invite(&mut ws, "bob"));
        let first = ws.directory.calls() - before;
        let before = ws.directory.calls();
        ok(invite(&mut ws, "bob"));
        let second = ws.directory.calls() - before;
        assert_eq!(first, second + 2, "by_name and by_uid are not repeated");
    }

    #[test]
    fn a_non_managers_invite_is_refused_before_any_account_lookup() {
        let mut ws = Workspace::new("join-non-manager");
        let mut carol = ws.enroll(CAROL, "carol", 12);
        ws.directory.set(BOB, "bob", None);
        // The baseline: one authenticated request that makes no lookup of its own.
        let before = ws.directory.calls();
        ws.call_ok(&mut carol, "profile.update", json!({"nickname": null}));
        let authentication = ws.directory.calls() - before;
        for (method, params) in [
            ("enrollment.invite", json!({"username": "bob"})),
            (
                "enrollment.invite",
                json!({"username": "@nobody", "add_device": true}),
            ),
            (
                "enrollment.approve",
                json!({"username": "bob", "code": "7QK2M9XA3JTPWZ4D"}),
            ),
            ("enrollment.cancel", json!({"username": "bob"})),
        ] {
            let before = ws.directory.calls();
            let (code, _) = refused(ws.call(&mut carol, method, params));
            assert_eq!(code, "forbidden", "{method}");
            assert_eq!(
                ws.directory.calls() - before,
                authentication,
                "{method} made an account lookup before the manager check"
            );
        }
        assert!(status_of(&mut ws, BOB)["invited"] == false);
    }

    // -----------------------------------------------------------------------------------------
    // enrollment.pending and auth.join
    // -----------------------------------------------------------------------------------------

    #[test]
    fn an_uninvited_uid_learns_only_that_it_is_not_invited_and_its_claim_writes_nothing() {
        let mut ws = Workspace::new("join-uninvited");
        ws.directory.set(BOB, "bob", None);
        ws.directory.set(CAROL, "carol", None);
        ok(invite(&mut ws, "bob"));
        let journal = ws.journal_bytes();
        let calls = ws.directory.calls();
        assert_eq!(status_of(&mut ws, CAROL), json!({"invited": false}));
        assert_eq!(
            ws.directory.calls(),
            calls,
            "enrollment.pending makes no lookup"
        );

        let join_id = join_id_of(&mut ws, BOB);
        let mut carol = Desktop::new(&ws, CAROL, 21);
        let (code, _) = refused(carol.claim(&mut ws.broker, &join_id));
        assert_eq!(code, "not_invited");
        assert_eq!(
            ws.directory.calls(),
            calls,
            "check 1 refuses before any lookup"
        );
        assert_eq!(
            ws.journal_bytes(),
            journal,
            "a refused claim writes nothing"
        );
        assert!(!is_device(&mut ws, CAROL, &carol.member.key));
        assert_eq!(host_joins(&mut ws)[0]["mismatched_attempts"], 0);
    }

    #[test]
    fn the_invited_status_names_the_inviter_and_the_workspace() {
        let mut ws = Workspace::new("join-status");
        ws.host_ok("workspace.rename", json!({"name": "lab"}));
        ws.host_ok("profile.update", json!({"nickname": "Alice Chen"}));
        let (_, join_id) = invited_bob(&mut ws, 21);
        let calls = ws.directory.calls();
        let status = status_of(&mut ws, BOB);
        assert_eq!(ws.directory.calls(), calls);
        assert_eq!(status["invited"], true);
        assert_eq!(status["join_id"], join_id.as_str());
        assert_eq!(join_id.len(), 32, "128 bits, hex");
        assert_eq!(status["workspace_name"], "lab");
        assert_eq!(
            status["inviter"],
            json!({"username": "alice", "display_name": "Alice Chen"})
        );
        assert_eq!(status["add_device"], false);
        assert_eq!(status["approved"], false);
        assert_eq!(status["expired"], false);
        assert!(status["expires_at"].as_u64().unwrap() >= now() + 86_000);
        assert!(status.get("last_refusal").is_none());
        let keys: Vec<&String> = status.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "add_device",
                "approved",
                "expired",
                "expires_at",
                "invited",
                "inviter",
                "join_id",
                "workspace_name"
            ],
            "no code, key or UID"
        );
    }

    #[test]
    fn the_approved_desktop_code_binds_a_new_principal_named_by_its_username() {
        let mut ws = Workspace::new("join-success");
        let (mut bob, join_id) = invited_bob(&mut ws, 21);
        let code = bob.code();
        assert_eq!(
            ok(approve(
                &mut ws,
                "@bob",
                &format_device_code(&code).to_lowercase()
            )),
            json!({"approved": true, "username": "bob"})
        );
        assert_eq!(bob.status(&mut ws.broker)["approved"], true);
        let calls = ws.directory.calls();
        let joined = ok(bob.claim(&mut ws.broker, &join_id));
        assert_eq!(
            ws.directory.calls(),
            calls + 1,
            "exactly the by_uid of check 2"
        );
        assert_eq!(
            joined["principal"],
            json!({"username": "bob", "display_name": "bob"})
        );
        assert_eq!(joined["device_id"], device_id(&bob.member.key));
        assert_eq!(joined["workspace"]["id"], ws.broker.workspace().id.as_str());

        let record = records(&ws).pop().unwrap();
        assert_eq!(record["operation"], "auth.join");
        assert_eq!(record["actor"], format!("uid:{BOB}"));
        assert!(
            record["patches"]
                .as_array()
                .unwrap()
                .contains(&json!({"op": "remove", "path": ["pending_joins"]})),
            "the last join leaves state as one top-level remove"
        );

        let view = ws.snapshot(&mut bob.member);
        assert_eq!(view["actor"]["username"], "bob");
        assert_eq!(
            view["actor"]["nickname"], "bob",
            "D2: the nickname is the username"
        );
        let devices = view["actor"]["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["added_via"], "invitation_code");
        assert_eq!(status_of(&mut ws, BOB), json!({"invited": false}));
        assert!(host_joins(&mut ws).is_empty());
        // The claim cannot be repeated.
        let (code, _) = refused(bob.claim(&mut ws.broker, &join_id));
        assert_eq!(code, "not_invited");
    }

    #[test]
    fn a_wrong_code_writes_nothing_and_warns_both_sides() {
        let mut ws = Workspace::new("join-wrong-code");
        let (mut bob, join_id) = invited_bob(&mut ws, 21);
        let other = Desktop::new(&ws, BOB, 22);
        ok(approve(&mut ws, "bob", &other.code()));
        let journal = ws.journal_bytes();
        let (code, message) = refused(bob.claim(&mut ws.broker, &join_id));
        assert_eq!(code, "code_mismatch");
        assert!(!message.contains(&bob.code()) && !message.contains(&other.code()));
        assert_eq!(ws.journal_bytes(), journal, "a mismatch writes nothing");
        assert!(!is_device(&mut ws, BOB, &bob.member.key));

        let status = bob.status(&mut ws.broker);
        assert_eq!(status["last_refusal"], "code_mismatch");
        assert_eq!(status["approved"], true);
        assert_eq!(host_joins(&mut ws)[0]["mismatched_attempts"], 1);
        assert_eq!(ws.journal_bytes(), journal, "reads write nothing");

        // Re-approving the right code clears the joiner's refusal (not the host's warning) and
        // lets Bob in.
        let (code, _) = refused(approve(&mut ws, "bob", &bob.code()));
        assert_eq!(code, "already_approved");
        ok(ws.host_call(
            "enrollment.approve",
            json!({"username": "bob", "code": bob.code(), "replace": true}),
        ));
        assert!(bob.status(&mut ws.broker).get("last_refusal").is_none());
        assert_eq!(host_joins(&mut ws)[0]["mismatched_attempts"], 1);
        ok(bob.claim(&mut ws.broker, &join_id));
    }

    #[test]
    fn approve_normalizes_the_code_and_refuses_what_is_not_one() {
        let mut ws = Workspace::new("join-approve");
        let (bob, _) = invited_bob(&mut ws, 21);
        let code = bob.code();
        for invalid in [
            "",
            "7QK2",
            "UUUUUUUUUUUUUUUU",
            "7QK2-M9XA-3JTP-WZ4D-XXXX",
            "7QK2*M9XA3JTPWZ4",
        ] {
            let (error, _) = refused(approve(&mut ws, "bob", invalid));
            assert!(
                matches!(error.as_str(), "device_code_invalid" | "invalid_params"),
                "{invalid}: {error}"
            );
        }
        let spaced = format!(" {} ", format_device_code(&code).replace('-', " "));
        ok(approve(&mut ws, "bob", &spaced));
        // The same code again is idempotent; another code needs `replace`.
        ok(approve(&mut ws, "bob", &code.to_lowercase()));
        let another = Desktop::new(&ws, BOB, 30).code();
        let (error, _) = refused(approve(&mut ws, "bob", &another));
        assert_eq!(error, "already_approved");
        let (error, _) = refused(approve(&mut ws, "carol", &code));
        assert_eq!(error, "not_invited");
        let (error, _) = refused(approve(&mut ws, "Bob", &code));
        assert_eq!(
            error, "not_invited",
            "no case fallback on an authority command"
        );
        assert_eq!(host_joins(&mut ws)[0]["approved"], true);
    }

    // -----------------------------------------------------------------------------------------
    // Accounts and generations
    // -----------------------------------------------------------------------------------------

    /// Invite `@bob` and approve his desktop's code: `(desktop, join_id)`.
    fn approved_bob(ws: &mut Workspace, seed: u8) -> (Desktop, String) {
        let (bob, join_id) = invited_bob(ws, seed);
        ok(approve(ws, "bob", &bob.code()));
        (bob, join_id)
    }

    /// `desktop`'s claim is refused with `expected`, writes nothing and binds nothing.
    fn claim_refused(ws: &mut Workspace, desktop: &mut Desktop, join_id: &str, expected: &str) {
        let journal = ws.journal_bytes();
        let (code, _) = refused(desktop.claim(&mut ws.broker, join_id));
        assert_eq!(code, expected);
        assert_eq!(ws.journal_bytes(), journal);
        let uid = desktop.member.uid;
        assert!(!is_device(ws, uid, &desktop.member.key));
    }

    #[test]
    fn a_renamed_account_is_refused_at_join() {
        let mut ws = Workspace::new("join-renamed");
        let (mut bob, join_id) = approved_bob(&mut ws, 21);
        ws.directory.set(BOB, "robert", None);
        claim_refused(&mut ws, &mut bob, &join_id, "account_changed");
    }

    #[test]
    fn a_recycled_uid_under_a_new_name_is_refused_at_join() {
        let mut ws = Workspace::new("join-recycled");
        let (mut bob, join_id) = approved_bob(&mut ws, 21);
        ws.directory.remove(BOB);
        ws.directory.set(BOB, "mallory", None);
        claim_refused(&mut ws, &mut bob, &join_id, "account_changed");
        // The account vanishing altogether also fails closed.
        ws.directory.remove(BOB);
        let (code, _) = refused(bob.claim(&mut ws.broker, &join_id));
        assert_eq!(code, "identity_unavailable");
    }

    #[test]
    fn a_new_uid_reusing_the_name_is_not_the_invited_account() {
        let mut ws = Workspace::new("join-reused-name");
        let (bob, join_id) = approved_bob(&mut ws, 21);
        ws.directory.remove(BOB);
        ws.directory.set(DAVE, "bob", None);
        // Same key, same code, but the kernel says another UID.
        let mut impostor = Desktop::new(&ws, DAVE, 21);
        assert_eq!(impostor.code(), bob.code());
        claim_refused(&mut ws, &mut impostor, &join_id, "not_invited");
    }

    #[test]
    fn a_changed_principal_generation_is_refused_at_join() {
        let mut ws = Workspace::new("join-generation");
        let (mut bob, join_id) = approved_bob(&mut ws, 21);
        let host = ws.host.principal_id.clone();
        let former = uuid();
        // A principal holding this UID appears after the invite (an out-of-band record).
        let mut ws = ws.edit_journal(|journal| {
            journal.append(
                &host,
                "test.inject",
                vec![set(
                    &["principals", &former],
                    legacy_principal(&former, BOB, "bob", "bob", false),
                )],
            );
        });
        claim_refused(&mut ws, &mut bob, &join_id, "account_changed");
    }

    #[test]
    fn a_member_enrolled_by_token_after_the_invite_is_not_joined_twice() {
        let mut ws = Workspace::new("join-generation-token");
        let (mut bob, join_id) = approved_bob(&mut ws, 21);
        let mut token_bob = Member::new(BOB, key(40));
        let token = ok(ws.legacy_invite(BOB, &token_bob.key))["invitation"]
            .as_str()
            .unwrap()
            .to_owned();
        ok(ws.legacy_enroll(&mut token_bob, &token));
        claim_refused(&mut ws, &mut bob, &join_id, "not_invited");
    }

    #[test]
    fn add_device_requires_the_flag_and_binds_the_existing_principal() {
        let mut ws = Workspace::new("join-add-device");
        let mut bob_laptop = ws.enroll(BOB, "bob", 40);
        let (code, message) = refused(invite(&mut ws, "bob"));
        assert_eq!(code, "already_member");
        assert!(message.contains("Add device"), "{message}");
        let invited = ok(ws.host_call(
            "enrollment.invite",
            json!({"username": "bob", "add_device": true}),
        ));
        assert_eq!(invited["add_device"], true);
        let mut desktop = Desktop::new(&ws, BOB, 41);
        let status = desktop.status(&mut ws.broker);
        assert_eq!(status["add_device"], true);
        assert_eq!(host_joins(&mut ws)[0]["add_device"], true);
        ok(approve(&mut ws, "bob", &desktop.code()));
        let join_id = status["join_id"].as_str().unwrap().to_owned();
        ok(desktop.claim(&mut ws.broker, &join_id));
        let view = ws.snapshot(&mut desktop.member);
        assert_eq!(view["actor"]["id"], bob_laptop.principal_id.as_str());
        assert_eq!(view["actor"]["devices"].as_array().unwrap().len(), 2);
        // The first device still works.
        ws.snapshot(&mut bob_laptop);
        // Adding a device to someone who is not a member is refused.
        ws.directory.set(CAROL, "carol", None);
        let (code, _) = refused(ws.host_call(
            "enrollment.invite",
            json!({"username": "carol", "add_device": true}),
        ));
        assert_eq!(code, "invalid_params");
    }

    #[test]
    fn an_added_device_is_refused_once_its_principal_is_revoked() {
        let mut ws = Workspace::new("join-add-device-revoked");
        let bob_laptop = ws.enroll(BOB, "bob", 40);
        ok(ws.host_call(
            "enrollment.invite",
            json!({"username": "bob", "add_device": true}),
        ));
        let mut desktop = Desktop::new(&ws, BOB, 41);
        let join_id = join_id_of(&mut ws, BOB);
        ok(approve(&mut ws, "bob", &desktop.code()));
        ws.offboard(&bob_laptop.principal_id);
        assert_eq!(
            status_of(&mut ws, BOB),
            json!({"invited": false}),
            "revoke purges"
        );
        claim_refused(&mut ws, &mut desktop, &join_id, "not_invited");
    }

    #[test]
    fn invite_refusals_follow_the_design_order() {
        let mut ws = Workspace::new("join-invite-order");
        // An active principal on the UID whose account was renamed: remove it first.
        let bob = ws.enroll(BOB, "bob", 40);
        ws.directory.set(BOB, "robert", None);
        let (code, message) = refused(invite(&mut ws, "robert"));
        assert_eq!(code, "identity_mismatch");
        assert!(
            message.contains("@bob") && message.contains("@robert"),
            "{message}"
        );
        ws.offboard(&bob.principal_id);

        // D3: an active @carol on one UID blocks inviting a colliding @Carol on another.
        ws.enroll(CAROL, "carol", 42);
        ws.directory.set(DAVE, "Carol", None);
        let (code, _) = refused(invite(&mut ws, "Carol"));
        assert_eq!(code, "identity_conflict");

        // A pending join for a colliding name on another UID blocks a second one.
        ws.directory.set(BOB, "dave", None);
        ok(invite(&mut ws, "dave"));
        ws.directory.set(71_010, "DAVE", None);
        let (code, message) = refused(invite(&mut ws, "DAVE"));
        assert_eq!(code, "identity_conflict");
        assert!(message.contains("Cancel"), "{message}");

        // Mixing the two forms is refused.
        let (code, _) = refused(ws.host_call(
            "enrollment.invite",
            json!({"username": "dave", "uid": BOB, "public_key": key_hex(&key(50))}),
        ));
        assert_eq!(code, "invalid_params");
        let (code, _) = refused(ws.host_call(
            "enrollment.invite",
            json!({"username": "dave", "add_device": "yes"}),
        ));
        assert_eq!(code, "invalid_params");
    }

    // -----------------------------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------------------------

    /// Move Bob's join's expiry into the past with an out-of-band journal record.
    fn expire_bob(ws: Workspace) -> Workspace {
        let host = ws.host.principal_id.clone();
        ws.edit_journal(|journal| {
            journal.append(
                &host,
                "test.expire",
                vec![set(
                    &["pending_joins", &BOB.to_string(), "expires_at"],
                    json!(now() - 1),
                )],
            );
        })
    }

    #[test]
    fn an_expired_join_is_reported_refused_and_pruned() {
        let mut ws = Workspace::new("join-expiry");
        let (mut bob, join_id) = approved_bob(&mut ws, 21);
        let mut ws = expire_bob(ws);
        let status = bob.status(&mut ws.broker);
        assert_eq!(status["invited"], true);
        assert_eq!(status["expired"], true);
        assert_eq!(host_joins(&mut ws)[0]["expired"], true);
        claim_refused(&mut ws, &mut bob, &join_id, "join_expired");
        // The next mutation prunes it, so it no longer counts or answers.
        ws.host_team("Lab");
        assert_eq!(bob.status(&mut ws.broker), json!({"invited": false}));
        assert!(host_joins(&mut ws).is_empty());
        let (code, _) = refused(approve(&mut ws, "bob", &bob.code()));
        assert_eq!(code, "not_invited");
    }

    #[test]
    fn a_cancelled_join_cannot_be_claimed() {
        let mut ws = Workspace::new("join-cancel");
        let (mut bob, join_id) = approved_bob(&mut ws, 21);
        assert_eq!(
            ws.host_ok("enrollment.cancel", json!({"username": "@bob"})),
            json!({"cancelled": true})
        );
        assert_eq!(bob.status(&mut ws.broker), json!({"invited": false}));
        claim_refused(&mut ws, &mut bob, &join_id, "not_invited");
        let (code, _) = refused(ws.host_call("enrollment.cancel", json!({"username": "bob"})));
        assert_eq!(code, "not_invited");
    }

    #[test]
    fn a_reinvite_mints_a_new_join_and_refuses_the_old_one() {
        let mut ws = Workspace::new("join-reinvite");
        let (mut bob, old) = approved_bob(&mut ws, 21);
        ok(invite(&mut ws, "bob"));
        let status = bob.status(&mut ws.broker);
        let new = status["join_id"].as_str().unwrap().to_owned();
        assert_ne!(new, old);
        assert_eq!(status["approved"], false, "a re-invite starts unapproved");
        claim_refused(&mut ws, &mut bob, &old, "join_changed");
        claim_refused(&mut ws, &mut bob, &new, "code_mismatch");
        ok(approve(&mut ws, "bob", &bob.code()));
        ok(bob.claim(&mut ws.broker, &new));
    }

    #[test]
    fn an_approval_survives_a_restart_between_approve_and_join() {
        let mut ws = Workspace::new("join-restart");
        let (mut bob, join_id) = approved_bob(&mut ws, 21);
        let mut ws = ws.reopen();
        assert_eq!(bob.status(&mut ws.broker)["approved"], true);
        ok(bob.claim(&mut ws.broker, &join_id));
        let ws = ws.reopen();
        drop(ws);
    }

    #[test]
    fn revoking_a_principal_purges_joins_for_its_uid_and_username() {
        let mut ws = Workspace::new("join-revoke");
        let carol = ws.enroll(CAROL, "carol", 42);
        ok(ws.host_call(
            "enrollment.invite",
            json!({"username": "carol", "add_device": true}),
        ));
        ws.directory.set(BOB, "bob", None);
        ok(invite(&mut ws, "bob"));
        assert_eq!(host_joins(&mut ws).len(), 2);
        ws.offboard(&carol.principal_id);
        let joins = host_joins(&mut ws);
        assert_eq!(joins.len(), 1);
        assert_eq!(joins[0]["username"], "bob");
        assert_eq!(status_of(&mut ws, CAROL), json!({"invited": false}));
        // A former member may be invited again; the join mints a new principal.
        ok(invite(&mut ws, "carol"));
        let mut desktop = Desktop::new(&ws, CAROL, 43);
        let join_id = join_id_of(&mut ws, CAROL);
        ok(approve(&mut ws, "carol", &desktop.code()));
        let joined = ok(desktop.claim(&mut ws.broker, &join_id));
        assert_eq!(joined["principal"]["username"], "carol");
        let view = ws.snapshot(&mut desktop.member);
        assert_ne!(view["actor"]["id"], carol.principal_id.as_str());
    }

    #[test]
    fn a_key_that_is_already_a_device_is_refused_at_join() {
        let mut ws = Workspace::new("join-key-reuse");
        let carol = ws.enroll(CAROL, "carol", 42);
        ws.directory.set(BOB, "bob", None);
        ok(invite(&mut ws, "bob"));
        let join_id = join_id_of(&mut ws, BOB);
        // Bob's UID presents Carol's key, and the host even approves its code.
        let mut stolen = Desktop::new(&ws, BOB, 42);
        assert_eq!(key_hex(&stolen.member.key), key_hex(&carol.key));
        ok(approve(&mut ws, "bob", &stolen.code()));
        let journal = ws.journal_bytes();
        let (code, _) = refused(stolen.claim(&mut ws.broker, &join_id));
        assert_eq!(code, "device_conflict");
        assert_eq!(ws.journal_bytes(), journal);
        assert_eq!(host_joins(&mut ws)[0]["mismatched_attempts"], 0);
        // And at both legacy binds, which N-BROKER-S1S2 owns; pinned here beside the new one.
        let (code, _) = refused(ws.legacy_invite(BOB, &carol.key));
        assert_eq!(code, "device_conflict");
    }

    #[test]
    fn the_legacy_token_path_still_enrolls_and_clears_the_join() {
        let mut ws = Workspace::new("join-legacy");
        let (_, _) = invited_bob(&mut ws, 21);
        let mut bob = Member::new(BOB, key(44));
        let token = ok(ws.legacy_invite(BOB, &bob.key))["invitation"]
            .as_str()
            .unwrap()
            .to_owned();
        ok(ws.legacy_enroll(&mut bob, &token));
        assert_eq!(status_of(&mut ws, BOB), json!({"invited": false}));
        assert!(host_joins(&mut ws).is_empty());
        let view = ws.snapshot(&mut bob);
        assert_eq!(view["actor"]["devices"][0]["added_via"], "token");
    }

    #[test]
    fn a_join_removes_any_legacy_enrollment_for_the_uid() {
        let mut ws = Workspace::new("join-clears-token");
        let (mut desktop, join_id) = approved_bob(&mut ws, 21);
        let mut token_device = Member::new(BOB, key(45));
        let token = ok(ws.legacy_invite(BOB, &token_device.key))["invitation"]
            .as_str()
            .unwrap()
            .to_owned();
        ok(desktop.claim(&mut ws.broker, &join_id));
        let (code, _) = refused(ws.legacy_enroll(&mut token_device, &token));
        assert_eq!(code, "forbidden", "the token died with the join");
    }

    #[test]
    fn at_most_one_hundred_joins_wait_at_once() {
        let mut ws = Workspace::new("join-cap");
        for n in 0..100u32 {
            ws.directory.set(80_000 + n, &format!("user{n}"), None);
            ok(invite(&mut ws, &format!("user{n}")));
        }
        ws.directory.set(80_100, "user100", None);
        let (code, _) = refused(invite(&mut ws, "user100"));
        assert_eq!(code, "quota_exceeded");
        // Re-inviting someone already waiting replaces their join and does not count.
        ok(invite(&mut ws, "user7"));
        ok(ws.host_call("enrollment.cancel", json!({"username": "user3"})));
        ok(invite(&mut ws, "user100"));
    }
}
