//! Joining a workspace by invitation and device code (S3a), compiled only with the
//! `join-by-name` feature.
//!
//! See `docs/research/biorouter-crew/naming-design.md`, "Broker protocol (S3a)", "The device
//! code" and "Security analysis". The host invites an account by `@username`; the joiner's own
//! desktop computes a device code from its saved key and the workspace key it pinned; the host
//! pastes that code to approve; and `auth.join` binds a key **only** when its device code equals
//! the approved one. A process running as the joiner's UID in the bridge path (M2) can relay,
//! drop, reorder and substitute frames, but it cannot choose what the joiner's screen shows, so
//! to be bound it needs a key whose 80-bit code equals the joiner's: a second preimage.
//!
//! `broker.rs` calls exactly these functions, all behind `#[cfg(feature = "join-by-name")]`:
//!
//! - [`pre_auth`]: in `Broker::process`, before `authenticate_actor`, for the pre-authentication
//!   methods `enrollment.pending` and `auth.join`. `None` falls through to authentication.
//! - [`mutate`]: first in `Broker::mutate` (inside `apply_mutation`, on a clone of the state,
//!   after the idempotency lookup and before the commit), for `enrollment.invite` when its
//!   params carry `username`, `enrollment.approve` and `enrollment.cancel`. The result is
//!   cached in `dedupe`, so it never carries a code, key, UID or join ID.
//! - [`project_for_manager`]: the host's `workspace.snapshot` only, as `pending_joins`.
//! - [`on_principal_revoked`]: `enrollment.revoke`, with the revoked principal's UID and
//!   username.
//! - [`prune_expired`]: at the start of every mutation (`apply_mutation`, `auth.bootstrap`,
//!   `auth.enroll`), so expired joins never count against the cap.
//! - [`on_legacy_enrolled`]: after a successful legacy `auth.enroll` for `uid`.
//! - `hello` advertises `join_by_name_v1` whenever this module is compiled.
//!
//! Account lookups go through `broker.directory` (the [`Directory`] seam), never through libc
//! directly, and only after `broker.manager(..)` has admitted the caller. `enrollment.pending`
//! makes none; `auth.join` makes exactly one, the fresh `by_uid` of check 2.

use super::*;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// At most this many joins may be pending at once. Expired joins are pruned before every
/// mutation, so they never count.
const JOIN_CAP: usize = 100;
/// How long a successful canonicalization of a typed name is reused (SR12: NSS may block on
/// LDAP, and every request runs under the broker's one lock).
const LOOKUP_CACHE_SECS: u64 = 30;
/// At most this many cached canonicalizations; the host's own typing is the only source.
const LOOKUP_CACHE_CAP: usize = 64;
/// How a device bound by `auth.join` is labeled in `actor.devices`.
const ADDED_VIA: &str = "invitation_code";

const NOT_INVITED: &str =
    "not_invited: This account has no invitation to join this workspace. Ask the host to invite you.";
const JOIN_EXPIRED: &str =
    "join_expired: This invitation expired. Ask the host to invite you again.";
const JOIN_CHANGED: &str =
    "join_changed: The host sent a new invitation. Check your join status and try again.";
const ACCOUNT_CHANGED: &str = "account_changed: This server account changed since the host invited it. Ask the host to invite you again.";
const MEMBERSHIP_CHANGED: &str = "account_changed: Membership for this account changed since the host invited it. Ask the host to invite you again.";
const CODE_MISMATCH: &str = "code_mismatch: The host hasn't let this device in. Send the host the code shown on your screen.";
const NAME_INVALID: &str = "name_invalid: Type the account's name on the server, with no spaces, slashes, colons or invisible characters.";
const NAME_DIGITS: &str =
    "name_invalid: Type the account's name on the server, not its numeric user ID.";
const NAME_UNUSABLE: &str = "identity_unavailable: This account's name on the server can't be used to join by invitation. Invite it with an enrollment token instead.";
const AGENT_REFUSED: &str = "forbidden: only a person can invite or admit people";
const MIXED_FORMS: &str = "invalid_params: invite by username, or by uid and public_key, not both";
const JOIN_QUOTA: &str = "quota_exceeded: 100 people are already waiting to join. Cancel an invitation or wait for one to expire.";

/// In-memory join state that is never journaled: per-join mismatched attempts, the last
/// refusal shown by `enrollment.pending`, and the short-lived canonicalization cache. A
/// restart forgets all of it, which only drops warnings and cached lookups.
#[derive(Default)]
pub(super) struct Runtime {
    /// Claims refused with `code_mismatch`, per join ID; shown to the host as "a device with
    /// a different code tried to join".
    mismatched_attempts: BTreeMap<String, u32>,
    /// Per join ID, the approval a `code_mismatch` refusal was made against. The joiner's
    /// `enrollment.pending` reports `last_refusal` only while the join still carries that same
    /// approval, so a host who approves (or re-approves) a new code clears it without a write.
    last_refusal: BTreeMap<String, Option<String>>,
    /// Typed name to canonical account, with the time it was resolved.
    canonical: BTreeMap<String, (Account, u64)>,
}

impl Runtime {
    /// Drop entries for joins that no longer exist (cancelled, replaced, joined or purged).
    fn forget_stale(&mut self, s: &State) {
        let live: BTreeSet<&str> = s
            .pending_joins
            .values()
            .map(|join| join.join_id.as_str())
            .collect();
        self.mismatched_attempts
            .retain(|join_id, _| live.contains(join_id.as_str()));
        self.last_refusal
            .retain(|join_id, _| live.contains(join_id.as_str()));
    }
    fn record_mismatch(&mut self, join: &PendingJoin) {
        let count = self
            .mismatched_attempts
            .entry(join.join_id.clone())
            .or_default();
        *count = count.saturating_add(1);
        self.last_refusal
            .insert(join.join_id.clone(), join.approved_code.clone());
    }
    /// `code_mismatch` while the join still carries the approval a claim was refused against.
    /// A claim refused before the host approved anything warns the host (it counts as a
    /// mismatched attempt) but is not reported to the joiner, whose host has entered no code.
    fn last_refusal(&self, join: &PendingJoin) -> Option<&'static str> {
        (join.approved_code.is_some()
            && self.last_refusal.get(&join.join_id) == Some(&join.approved_code))
        .then_some("code_mismatch")
    }
    fn mismatched_attempts(&self, join: &PendingJoin) -> u32 {
        self.mismatched_attempts
            .get(&join.join_id)
            .copied()
            .unwrap_or(0)
    }
    fn cached(&mut self, typed: &str, now: u64) -> Option<Account> {
        self.canonical
            .retain(|_, (_, at)| now.saturating_sub(*at) < LOOKUP_CACHE_SECS);
        self.canonical
            .get(typed)
            .map(|(account, _)| account.clone())
    }
    fn remember(&mut self, typed: &str, account: &Account, now: u64) {
        if self.canonical.len() >= LOOKUP_CACHE_CAP {
            self.canonical.clear();
        }
        self.canonical
            .insert(typed.to_owned(), (account.clone(), now));
    }
}

/// `enrollment.pending` and `auth.join`, before authentication. Any other method: `None`.
pub(super) fn pre_auth(
    broker: &mut Broker,
    uid: u32,
    conn: &mut Connection,
    req: &Request,
) -> Option<Result<Value>> {
    match req.method.as_str() {
        "enrollment.pending" => {
            broker.join_runtime.forget_stale(&broker.state);
            Some(Ok(pending(broker, uid)))
        }
        "auth.join" => {
            broker.join_runtime.forget_stale(&broker.state);
            Some(claim(broker, uid, conn, req))
        }
        _ => None,
    }
}

/// `enrollment.pending`: the kernel UID's own join, built from state alone (no account lookup,
/// no write). A UID with no join gets exactly `{"invited": false}`.
fn pending(broker: &Broker, uid: u32) -> Value {
    let s = &broker.state;
    let Some(join) = s
        .pending_joins
        .get(&uid.to_string())
        .filter(|join| join.uid == uid)
    else {
        return json!({ "invited": false });
    };
    let index = PeopleIndex::new(s);
    let inviter = s.principals.get(&join.inviter_id).map(|inviter| {
        json!({"username": inviter.username, "display_name": index.display_name(inviter)})
    });
    let mut status = json!({
        "invited": true,
        "join_id": join.join_id,
        "workspace_name": s.workspace.name,
        "inviter": inviter,
        "add_device": join.existing_principal_id.is_some(),
        "approved": join.approved_code.is_some(),
        "expires_at": join.expires_at,
        "expired": join.is_expired(now()),
    });
    if let Some(refusal) = broker.join_runtime.last_refusal(join) {
        status["last_refusal"] = json!(refusal);
    }
    status
}

/// `auth.join`: checks 1 to 7 of the design, in order. Any refusal writes nothing; a
/// `code_mismatch` only updates the in-memory warning state.
fn claim(broker: &mut Broker, uid: u32, conn: &mut Connection, req: &Request) -> Result<Value> {
    ensure!(
        req.credential.is_none(),
        "invalid_request: mixed authentication"
    );
    let now = now();
    let public_key = text(&req.params, "public_key")?;
    let join_id = text(&req.params, "join_id")?;
    // 1. A live join for this kernel UID, with this join ID.
    let join = claimable_join(&broker.state, uid, join_id, now)?.clone();
    // 2. The account still has the name the host invited (renames, recycled UIDs).
    ensure!(
        broker.directory.by_uid(uid)?.name == join.username,
        ACCOUNT_CHANGED
    );
    // 3. The principals holding this UID or name are the ones there were at invite time.
    check_generation(&broker.state, &join)?;
    // 4. Possession of the key, over a single-use, socket-bound, 60-second nonce.
    broker.verify(uid, conn, req, public_key)?;
    let key: [u8; 32] = hex::decode(public_key)?
        .try_into()
        .map_err(|_| anyhow!("unauthorized: invalid key"))?;
    let device_id = digest(&key);
    // 5. SR11: a key that is already a device is never bound again.
    Broker::ensure_new_device(&broker.state, &device_id)?;
    // 6. The host approved exactly this key's device code.
    if !code_matches(&broker.state, &join, &key)? {
        broker.join_runtime.record_mismatch(&join);
        bail!(CODE_MISMATCH);
    }
    // 7. D3 still holds.
    Broker::ensure_username_free(&broker.state, uid, &join.username)?;
    bind(broker, &join, public_key, device_id, now)
}

fn claimable_join<'a>(s: &'a State, uid: u32, join_id: &str, now: u64) -> Result<&'a PendingJoin> {
    let join = s
        .pending_joins
        .get(&uid.to_string())
        .filter(|join| join.uid == uid)
        .ok_or_else(|| anyhow!(NOT_INVITED))?;
    ensure!(!join.is_expired(now), JOIN_EXPIRED);
    ensure!(join.join_id == join_id, JOIN_CHANGED);
    Ok(join)
}

/// The IDs of every principal, active or former, that holds `uid` or a name colliding with
/// `username`, in ID order. A re-enrollment mints a new principal, so any enrollment of this
/// account between the invite and the claim changes it.
fn generation(s: &State, uid: u32, username: &str) -> Vec<String> {
    s.principals
        .values()
        .filter(|p| p.uid == uid || names_collide(&p.username, username))
        .map(|p| p.id.clone())
        .collect()
}

fn check_generation(s: &State, join: &PendingJoin) -> Result<()> {
    let mut expected = join.generation.clone();
    expected.sort();
    expected.dedup();
    ensure!(
        generation(s, join.uid, &join.username) == expected,
        MEMBERSHIP_CHANGED
    );
    let unchanged = match &join.existing_principal_id {
        Some(existing) => s
            .principals
            .get(existing)
            .is_some_and(|p| p.active && p.uid == join.uid && p.username == join.username),
        None => !s
            .principals
            .values()
            .any(|p| p.active && (p.uid == join.uid || names_collide(&p.username, &join.username))),
    };
    ensure!(unchanged, MEMBERSHIP_CHANGED);
    Ok(())
}

/// Whether the join's approved code is the device code of `key` under this workspace's own
/// key. An unapproved join matches nothing.
fn code_matches(s: &State, join: &PendingJoin, key: &[u8; 32]) -> Result<bool> {
    let Some(approved) = &join.approved_code else {
        return Ok(false);
    };
    let secret: [u8; 32] = hex::decode(&s.workspace_signing_key)?
        .try_into()
        .map_err(|_| anyhow!("storage_corrupt: workspace identity"))?;
    let workspace_key = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
    Ok(device_code_matches(
        approved,
        &s.workspace.id,
        &workspace_key,
        key,
    ))
}

/// Bind the key: a new principal (nickname = username, D2) or the existing one for an added
/// device, the device labeled `invitation_code`, the join and any legacy enrollment for the
/// UID removed, committed as `uid:<n>`.
fn bind(
    broker: &mut Broker,
    join: &PendingJoin,
    public_key: &str,
    device_id: String,
    now: u64,
) -> Result<Value> {
    let mut state = broker.state.clone();
    prune_expired(&mut state, now);
    let principal = match &join.existing_principal_id {
        Some(existing) => state
            .principals
            .get(existing)
            .cloned()
            .ok_or_else(|| anyhow!(MEMBERSHIP_CHANGED))?,
        None => {
            let principal = Principal {
                id: id(),
                uid: join.uid,
                username: join.username.clone(),
                nickname: join.username.clone(),
                avatar: None,
                active: true,
            };
            state
                .principals
                .insert(principal.id.clone(), principal.clone());
            principal
        }
    };
    state.devices.insert(
        device_id.clone(),
        Device {
            principal_id: principal.id.clone(),
            public_key: public_key.into(),
            added_at: Some(now),
            added_via: Some(ADDED_VIA.into()),
        },
    );
    state.pending_joins.remove(&join.uid.to_string());
    state
        .enrollments
        .retain(|_, enrollment| enrollment.uid != join.uid);
    broker.commit(state, &format!("uid:{}", join.uid), "auth.join")?;
    broker.join_runtime.forget_stale(&broker.state);
    let display_name = PeopleIndex::new(&broker.state).display_name(&principal);
    Ok(json!({
        "principal": {"username": principal.username, "display_name": display_name},
        "device_id": device_id,
        "workspace": broker.state.workspace,
    }))
}

/// The new form of `enrollment.invite` (params carry `username`), `enrollment.approve` and
/// `enrollment.cancel`. Any other request, including the legacy `enrollment.invite {uid,
/// public_key}`: `None`.
pub(super) fn mutate(
    broker: &mut Broker,
    s: &mut State,
    actor: &Actor,
    req: &Request,
) -> Option<Result<Value>> {
    let result = match req.method.as_str() {
        "enrollment.invite" if req.params.get("username").is_some() => {
            invite(broker, s, actor, req)
        }
        "enrollment.approve" => approve(broker, s, actor, req),
        "enrollment.cancel" => cancel(broker, s, actor, req),
        _ => return None,
    };
    Some(result)
}

/// Only the host's own signed device: never an agent's grant, and checked before any account
/// lookup.
fn human_manager(broker: &Broker, s: &State, actor: &Actor) -> Result<()> {
    broker.manager(s, &actor.id)?;
    ensure!(actor.run.is_none(), AGENT_REFUSED);
    Ok(())
}

/// `username` with one leading `@` removed.
fn username_param(params: &Value) -> Result<&str> {
    let name = params
        .get("username")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("invalid_params: username must be a string"))?;
    Ok(name.strip_prefix('@').unwrap_or(name))
}

/// An optional boolean parameter; absent or `null` is `false`.
fn flag(params: &Value, key: &str) -> Result<bool> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("invalid_params: {key} must be true or false"),
    }
}

/// The character rules for a name typed after `@`, and again for the name NSS returns (SR9):
/// [`valid_username`] (1-256 bytes; no NUL, control character, whitespace, `/` or `:`; not all
/// digits), and no character that renders as nothing or reorders text (categories Cf, Co, Cs,
/// Cn, Zl, Zp, or `Default_Ignorable_Code_Point`), so `@bob` and a lookalike with an invisible
/// character can never both be shown.
fn check_username_characters(name: &str) -> Result<()> {
    ensure!(
        name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()),
        NAME_DIGITS
    );
    let invisible = name.chars().any(|c| {
        is_default_ignorable(c)
            || matches!(
                c.general_category(),
                GeneralCategory::Format
                    | GeneralCategory::PrivateUse
                    | GeneralCategory::Surrogate
                    | GeneralCategory::Unassigned
                    | GeneralCategory::LineSeparator
                    | GeneralCategory::ParagraphSeparator
            )
    });
    ensure!(valid_username(name) && !invisible, NAME_INVALID);
    Ok(())
}

/// SR9's canonicalization: the typed name must be the account's canonical name, exactly. An
/// NSS alias and a case-only difference are refused with the canonical spelling, never
/// accepted. Successful results are reused for [`LOOKUP_CACHE_SECS`]; a claim never uses the
/// cache, so a rename in that window still fails closed at `auth.join`.
fn canonical_account(broker: &mut Broker, typed: &str) -> Result<Account> {
    check_username_characters(typed)?;
    let now = now();
    if let Some(account) = broker.join_runtime.cached(typed, now) {
        return Ok(account);
    }
    let by_name = broker.directory.by_name(typed).map_err(|_| {
        anyhow!("unknown_account: There is no account @{typed} on this server. Check the spelling.")
    })?;
    let account = broker.directory.by_uid(by_name.uid).map_err(|_| {
        anyhow!("identity_unavailable: @{typed} can't be matched to one account on this server.")
    })?;
    ensure!(
        account.uid == by_name.uid,
        "identity_unavailable: @{typed} can't be matched to one account on this server."
    );
    check_username_characters(&account.name).map_err(|_| anyhow!(NAME_UNUSABLE))?;
    let canonical = &account.name;
    ensure!(
        by_name.name == *canonical && typed == canonical,
        if typed.eq_ignore_ascii_case(canonical) || typed.to_lowercase() == canonical.to_lowercase()
        {
            format!("identity_ambiguous: This server spells the account @{canonical}. Invite @{canonical}.")
        } else {
            format!("identity_ambiguous: @{typed} is an alias on this server. Invite @{canonical}.")
        }
    );
    broker.join_runtime.remember(typed, &account, now);
    Ok(account)
}

/// Who the join admits: `Some(principal)` to add a device to the account's active principal,
/// `None` for a new person. Refuses the cases that need the host to act first.
fn admission_target(s: &State, account: &Account, add_device: bool) -> Result<Option<String>> {
    match s
        .principals
        .values()
        .find(|p| p.active && p.uid == account.uid)
    {
        Some(member) if member.username == account.name => {
            ensure!(
                add_device,
                "already_member: @{} is already a member. Choose Add device to add another computer for them.",
                member.username
            );
            Ok(Some(member.id.clone()))
        }
        Some(member) => bail!(
            "identity_mismatch: This account joined as @{} and is now @{} on the server. Remove @{} first.",
            member.username,
            account.name,
            member.username
        ),
        None => {
            ensure!(
                !add_device,
                "invalid_params: @{} is not a member yet. Invite them without Add device.",
                account.name
            );
            Ok(None)
        }
    }
}

/// `enrollment.invite {username, add_device?}`: record a pending join for the account the
/// name canonically denotes, replacing any earlier join for its UID with a new join ID.
fn invite(broker: &mut Broker, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
    human_manager(broker, s, actor)?;
    broker.join_runtime.forget_stale(&broker.state);
    let p = &req.params;
    ensure!(
        ["uid", "public_key", "existing_principal_id"]
            .iter()
            .all(|key| p.get(key).is_none()),
        MIXED_FORMS
    );
    let typed = username_param(p)?;
    let add_device = flag(p, "add_device")?;
    let account = canonical_account(broker, typed)?;
    let existing_principal_id = admission_target(s, &account, add_device)?;
    Broker::ensure_username_free(s, account.uid, &account.name)?;
    if let Some(other) = s
        .pending_joins
        .values()
        .find(|join| join.uid != account.uid && names_collide(&join.username, &account.name))
    {
        bail!(
            "identity_conflict: Another account on this server is already invited as @{}. Cancel that invitation first.",
            other.username
        );
    }
    let key = account.uid.to_string();
    ensure!(
        s.pending_joins.contains_key(&key) || s.pending_joins.len() < JOIN_CAP,
        JOIN_QUOTA
    );
    let full_name = account.full_name.as_deref().and_then(|full_name| {
        validate_display_name_for(
            full_name,
            &account.name,
            s.principals.values().map(|p| p.username.as_str()),
        )
        .ok()
    });
    let created_at = now();
    let join = PendingJoin {
        join_id: hex::encode(&Sha256::digest(token().as_bytes())[..16]),
        uid: account.uid,
        username: account.name.clone(),
        full_name,
        inviter_id: actor.id.clone(),
        existing_principal_id,
        generation: generation(s, account.uid, &account.name),
        approved_code: None,
        created_at,
        expires_at: created_at + PENDING_JOIN_LIFETIME_SECS,
    };
    let result = json!({
        "username": join.username,
        "full_name": join.full_name,
        "add_device": join.existing_principal_id.is_some(),
        "expires_at": join.expires_at,
    });
    s.pending_joins.insert(key, join);
    Ok(result)
}

/// The pending join for exactly this canonical username (no case fallback, D7).
fn join_for<'a>(s: &'a mut State, username: &str) -> Result<&'a mut PendingJoin> {
    s.pending_joins
        .values_mut()
        .find(|join| join.username == username)
        .ok_or_else(|| {
            anyhow!("not_invited: @{username} has no pending invitation. Invite them first.")
        })
}

/// `enrollment.approve {username, code, replace?}`: record the device code the joiner sent.
/// A different code replaces an earlier approval only when `replace` is true, so a stale or
/// repeated approval never silently displaces the right one.
fn approve(broker: &mut Broker, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
    human_manager(broker, s, actor)?;
    let p = &req.params;
    let username = username_param(p)?;
    let code = normalize_device_code(text(p, "code")?)
        .map_err(|error| anyhow!("{}: {error}", error.code()))?;
    let replace = flag(p, "replace")?;
    let join = join_for(s, username)?;
    match &join.approved_code {
        Some(approved) if *approved == code => {}
        Some(_) if !replace => bail!(
            "already_approved: You already let a device in for @{}. Replace the code only if they sent you a new one.",
            join.username
        ),
        _ => join.approved_code = Some(code),
    }
    Ok(json!({"approved": true, "username": join.username}))
}

/// `enrollment.cancel {username}`: withdraw the pending join.
fn cancel(broker: &mut Broker, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
    human_manager(broker, s, actor)?;
    let username = username_param(&req.params)?.to_owned();
    let uid = join_for(s, &username)?.uid;
    s.pending_joins.remove(&uid.to_string());
    Ok(json!({"cancelled": true}))
}

/// The host's view of pending joins: `[{username, full_name, add_device, approved,
/// created_at, expires_at, expired, mismatched_attempts}]`. Never a code, key, UID or join ID.
pub(super) fn project_for_manager(broker: &Broker, s: &State) -> Value {
    let now = now();
    Value::Array(
        s.pending_joins
            .values()
            .map(|join| {
                json!({
                    "username": join.username,
                    "full_name": join.full_name,
                    "add_device": join.existing_principal_id.is_some(),
                    "approved": join.approved_code.is_some(),
                    "created_at": join.created_at,
                    "expires_at": join.expires_at,
                    "expired": join.is_expired(now),
                    "mismatched_attempts": broker.join_runtime.mismatched_attempts(join),
                })
            })
            .collect(),
    )
}

/// `enrollment.revoke` purges pending joins for the revoked principal's UID and for any name
/// colliding with its username.
pub(super) fn on_principal_revoked(s: &mut State, uid: u32, username: &str) {
    s.pending_joins
        .retain(|_, join| join.uid != uid && !names_collide(&join.username, username));
}

/// Drop joins that expired at or before `now`.
pub(super) fn prune_expired(s: &mut State, now: u64) {
    s.pending_joins.retain(|_, join| !join.is_expired(now));
}

/// Completing the legacy token path for `uid` removes its pending join.
pub(super) fn on_legacy_enrolled(s: &mut State, uid: u32) {
    s.pending_joins.remove(&uid.to_string());
}
