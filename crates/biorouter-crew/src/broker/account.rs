//! Account ownership (AO-1 to AO-5 of `docs/crew/design/account-ownership.md`), compiled only
//! with the `join-by-name` feature, beside `join.rs`, whose checks its invited branch reuses.
//!
//! The server account is the person: a process running as an account may add a device for that
//! account by itself (`auth.account`), with no invitation code and no other device. Every
//! request is still authenticated twice, by the kernel UID and by a device signature; what
//! changes is only how a device key is admitted. `broker.rs` calls exactly these functions:
//!
//! - [`pre_auth`]: in `Broker::process`, after the deleted-workspace gate and before
//!   `authenticate_actor`, for the pre-authentication methods `auth.account` and
//!   `account.status`. `None` falls through to authentication.
//! - [`revoke_device`]: `device.revoke` in `Broker::mutate`, a person's signed mutation.
//! - [`note_connection`] and [`connection_closed`]: `Broker::handle` and
//!   `Broker::connection_closed`, so `account.status` can tell the host how many connections
//!   other accounts hold (`others_connected`).
//! - `hello` advertises `account_identity_v1` whenever this module is compiled.
//!
//! `auth.account` decides its branch from state alone, so a caller the workspace knows nothing
//! about costs no account lookup, and makes exactly one lookup (`by_uid`) once the key is
//! proved. `account.status` makes none.

use super::*;

/// How a device bound by `auth.account` is labeled in `actor.devices`.
const ADDED_VIA: &str = "account";

/// The account the broker could not look up, or one whose name the invitation rules refuse.
const ACCOUNT_UNAVAILABLE: &str = "identity_unavailable: This server can't look up your account. Try again, or ask the server's administrator.";
const NAME_UNUSABLE: &str =
    "identity_unavailable: This account's name on the server can't be used with Crew.";
/// `replaces_device_id` names a device of another principal.
const REPLACES_ANOTHER: &str =
    "forbidden: The device to replace belongs to someone else. Nothing was changed.";
/// `device.revoke` names no device, or more than one, of the caller's own.
const NO_SUCH_DEVICE: &str =
    "invalid_params: None of your devices has that fingerprint. Check it in Keys and try again.";
const BAD_FINGERPRINT: &str =
    "invalid_params: fingerprint must be the 16 hex digits of a device fingerprint, as Keys shows it";
const AGENT_REFUSED: &str = "forbidden: only a person can revoke a device";

/// In-memory state of the account path, never journaled: a restart forgets it, which only
/// resets a quota allowance and the connection counts (every connection is new then too).
#[derive(Default)]
pub(super) struct Runtime {
    /// Per principal, when `auth.account` last committed past the member's journal share.
    over_share: BTreeMap<String, u64>,
    /// Open connections per UID that have made at least one request.
    connections: BTreeMap<u32, usize>,
}

impl Runtime {
    /// Whether `principal` may commit one `auth.account` past their journal share at `now`.
    fn may_pass_share(&self, principal: &str, now: u64) -> bool {
        self.over_share
            .get(principal)
            .is_none_or(|at| now.saturating_sub(*at) >= DAY_SECS)
    }
}

/// `auth.account` and `account.status`, before authentication. Any other method: `None`.
pub(super) fn pre_auth(
    broker: &mut Broker,
    uid: u32,
    conn: &mut Connection,
    req: &Request,
) -> Option<Result<Value>> {
    match req.method.as_str() {
        "auth.account" => Some(claim(broker, uid, conn, req)),
        "account.status" => Some(Ok(status(broker, uid))),
        _ => None,
    }
}

/// `connection` made a request as `uid`: count it once, for the host's `others_connected`.
pub(super) fn note_connection(broker: &mut Broker, uid: u32, connection: &mut Connection) {
    if connection.counted_uid.is_none() {
        connection.counted_uid = Some(uid);
        *broker
            .join_runtime
            .account
            .connections
            .entry(uid)
            .or_default() += 1;
    }
}

/// `connection` has closed: it no longer counts.
pub(super) fn connection_closed(broker: &mut Broker, connection: &mut Connection) {
    let Some(uid) = connection.counted_uid.take() else {
        return;
    };
    let connections = &mut broker.join_runtime.account.connections;
    if let Some(count) = connections.get_mut(&uid) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            connections.remove(&uid);
        }
    }
}

/// The workspace as a refusal names it.
fn workspace_label(s: &State) -> String {
    s.workspace
        .name
        .clone()
        .unwrap_or_else(|| "this workspace".to_owned())
}

/// The host's active principal.
fn host_principal(s: &State) -> Option<&Principal> {
    s.principals
        .values()
        .find(|p| p.active && p.uid == s.workspace.host_uid)
}

/// `account.status {}`: what the workspace knows about the peer UID, from state alone (no
/// account lookup, no write). It never answers about another account.
fn status(broker: &Broker, uid: u32) -> Value {
    let s = &broker.state;
    let active = s.principals.values().find(|p| p.active && p.uid == uid);
    let join = s
        .pending_joins
        .get(&uid.to_string())
        .filter(|join| join.uid == uid);
    let devices = |principal: Option<&Principal>| {
        principal.map_or(0, |principal| {
            s.devices
                .values()
                .filter(|device| device.principal_id == principal.id)
                .count()
        })
    };
    let mut status = json!({"workspace_name": s.workspace.name});
    if let Some(host) = host_principal(s) {
        status["host_username"] = json!(host.username);
    }
    if uid == s.workspace.host_uid {
        status["role"] = json!("host");
        if let Some(principal) = active {
            status["username"] = json!(principal.username);
        }
        status["devices"] = json!(devices(active));
        let others: usize = broker
            .join_runtime
            .account
            .connections
            .iter()
            .filter(|(other, _)| **other != uid)
            .map(|(_, count)| *count)
            .sum();
        status["others_connected"] = json!(others);
    } else if let Some(principal) = active {
        status["role"] = json!("member");
        status["username"] = json!(principal.username);
        status["devices"] = json!(devices(active));
    } else if let Some(join) = join {
        status["role"] = json!("invited");
        status["username"] = json!(join.username);
        if let Some(inviter) = s.principals.get(&join.inviter_id) {
            let index = PeopleIndex::new(s);
            status["invited_by"] = json!({
                "username": inviter.username,
                "display_name": index.display_name(inviter),
            });
        }
        status["expires_at"] = json!(join.expires_at);
        status["expired"] = json!(join.is_expired(now()));
    } else if let Some(principal) = s.principals.values().find(|p| p.uid == uid) {
        status["role"] = json!("removed");
        status["username"] = json!(principal.username);
    } else {
        status["role"] = json!("none");
    }
    status
}

/// Which way `auth.account` admits the caller, decided from state alone.
enum Branch {
    /// An active principal holds the UID: a member, or the host when the UID is `host_uid`.
    Member(Principal),
    /// The UID is `host_uid` and no principal has ever held it.
    HostFirst,
    /// A live pending join for the UID.
    Invited(PendingJoin),
}

/// AO-2 step 1, in its order. A UID the workspace never knew, or knew and removed, is refused
/// here, before any signature check or account lookup.
fn branch(s: &State, uid: u32, now: u64) -> Result<Branch> {
    if let Some(principal) = s.principals.values().find(|p| p.active && p.uid == uid) {
        return Ok(Branch::Member(principal.clone()));
    }
    if uid == s.workspace.host_uid && !s.principals.values().any(|p| p.uid == uid) {
        return Ok(Branch::HostFirst);
    }
    if let Some(join) = s
        .pending_joins
        .get(&uid.to_string())
        .filter(|join| join.uid == uid && !join.is_expired(now))
    {
        return Ok(Branch::Invited(join.clone()));
    }
    let workspace = workspace_label(s);
    if s.principals.values().any(|p| p.uid == uid) {
        bail!("removed: You were removed from {workspace}. Ask a member to invite you again.");
    }
    // `scripts/check-crew-broker-join.sh` reads this sentence's first words as the marker that
    // a built broker carries the account path; change the script with it.
    bail!("not_member: This account is not a member of {workspace}. Ask a member to invite it.")
}

/// An optional string parameter; absent or `null` is `None`.
fn optional_text<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() && value.len() <= 128 => {
            Ok(Some(value.as_str()))
        }
        Some(_) => bail!("invalid_params: {key} must be a device ID"),
    }
}

/// `auth.account {public_key, replaces_device_id?}`: a computer adds itself for the account it
/// runs as (AO-2). Any refusal writes nothing.
fn claim(broker: &mut Broker, uid: u32, conn: &mut Connection, req: &Request) -> Result<Value> {
    ensure!(
        req.credential.is_none(),
        "invalid_request: mixed authentication"
    );
    let now = now();
    let public_key = text(&req.params, "public_key")?;
    let replaces = optional_text(&req.params, "replaces_device_id")?;
    // 1. The branch, from state alone.
    let branch = branch(&broker.state, uid, now)?;
    // 2. Possession of the key, over a single-use, socket-bound, 60-second nonce.
    broker.verify(uid, conn, req, public_key)?;
    let key: [u8; 32] = hex::decode(public_key)?
        .try_into()
        .map_err(|_| anyhow!("unauthorized: invalid key"))?;
    let device_id = digest(&key);
    // 3. The one account lookup.
    let account = broker
        .directory
        .by_uid(uid)
        .map_err(|_| anyhow!(ACCOUNT_UNAVAILABLE))?;
    ensure!(account.uid == uid, ACCOUNT_UNAVAILABLE);
    join::check_username_characters(&account.name).map_err(|_| anyhow!(NAME_UNUSABLE))?;
    // 4. The branch's own checks.
    let mut state = broker.state.clone();
    join::prune_expired(&mut state, now);
    let host_uid = state.workspace.host_uid;
    let (mut state, principal_id, admitted, changed) =
        admit(broker, state, branch, uid, host_uid, &account)?;
    // 5. The key. One already bound to this principal is a retry: it commits nothing but a
    // host rename this request found or an add-device join it consumed.
    if let Some(device) = state.devices.get(&device_id) {
        ensure!(
            device.principal_id == principal_id,
            "device_conflict: this device key is already enrolled in this workspace; use a new device key"
        );
        if changed {
            commit(broker, state, &principal_id, uid == host_uid, now)?;
        }
        return Ok(answer(broker, &principal_id, &device_id, "device", None));
    }
    if let Some(old) = replaces {
        match state.devices.get(old) {
            Some(device) if device.principal_id == principal_id => {
                state.devices.remove(old);
            }
            Some(_) => bail!(REPLACES_ANOTHER),
            None => {}
        }
    }
    // 6. The cap: the oldest device makes room, so nobody is refused for too many computers.
    let evicted = evict_oldest(&mut state, &principal_id);
    state.devices.insert(
        device_id.clone(),
        Device {
            principal_id: principal_id.clone(),
            public_key: public_key.into(),
            added_at: Some(now),
            added_via: Some(ADDED_VIA.into()),
        },
    );
    // 7. Commit, as the principal.
    commit(broker, state, &principal_id, uid == host_uid, now)?;
    broker.join_runtime.forget_stale(&broker.state);
    Ok(answer(broker, &principal_id, &device_id, admitted, evicted))
}

/// AO-2 step 4: the branch's own checks, made on `state`. Returns `state` with the caller
/// admitted, the principal's ID, how it was admitted, and whether the record changes anything
/// besides the device (a followed host rename, or an add-device join it consumes).
fn admit(
    broker: &Broker,
    mut state: State,
    branch: Branch,
    uid: u32,
    host_uid: u32,
    account: &Account,
) -> Result<(State, String, &'static str, bool)> {
    let mut changed = false;
    let (principal_id, admitted) = match branch {
        Branch::Member(member) => {
            if member.username != account.name {
                // Only the account the broker runs as is followed through a rename; a member
                // keeps `account_changed`, which guards against a recycled UID.
                ensure!(
                    uid == host_uid,
                    "account_changed: This server account is no longer named @{0}, the name it joined with. Ask the host to remove @{0} and invite you again.",
                    member.username
                );
                Broker::ensure_username_free(&state, uid, &account.name).map_err(|_| {
                    host_rename_conflict(&state, uid, &member.username, &account.name)
                })?;
                let principal = state
                    .principals
                    .get_mut(&member.id)
                    .expect("the branch's principal");
                if principal.nickname == principal.username {
                    principal.nickname = account.name.clone();
                }
                principal.username = account.name.clone();
                changed = true;
            }
            // A member reaches this branch before the invited one, so an add-device join for
            // this principal ends here, in the same record that adds the device.
            changed |= consume_add_device_join(&mut state, uid, &member.id);
            (member.id, "device")
        }
        Branch::HostFirst => {
            Broker::ensure_username_free(&state, uid, &account.name)?;
            let principal = Principal {
                id: id(),
                uid,
                username: account.name.clone(),
                nickname: account.name.clone(),
                avatar: None,
                active: true,
            };
            state
                .principals
                .insert(principal.id.clone(), principal.clone());
            admit_everywhere(&mut state, &principal.id);
            (principal.id, "host")
        }
        Branch::Invited(pending) => {
            // `auth.join`'s checks 2, 3 and 3b, without the code.
            ensure!(account.name == pending.username, join::ACCOUNT_CHANGED);
            join::check_generation(&broker.state, &pending)?;
            ensure!(
                join::inviter_holds_authority(broker, &broker.state, &pending),
                join::INVITER_CHANGED
            );
            Broker::ensure_username_free(&state, uid, &pending.username)?;
            let principal_id = match &pending.existing_principal_id {
                Some(existing) => {
                    ensure!(
                        state
                            .principals
                            .get(existing)
                            .is_some_and(|p| p.active && p.uid == uid),
                        join::MEMBERSHIP_CHANGED
                    );
                    existing.clone()
                }
                None => {
                    let principal = Principal {
                        id: id(),
                        uid,
                        username: pending.username.clone(),
                        nickname: pending.username.clone(),
                        avatar: None,
                        active: true,
                    };
                    state
                        .principals
                        .insert(principal.id.clone(), principal.clone());
                    admit_everywhere(&mut state, &principal.id);
                    principal.id
                }
            };
            state.pending_joins.remove(&uid.to_string());
            state
                .enrollments
                .retain(|_, enrollment| enrollment.uid != uid);
            (principal_id, "invitation")
        }
    };
    Ok((state, principal_id, admitted, changed))
}

/// AO-2 step 4 for a member: an add-device join selecting `principal_id` is removed, with any
/// legacy enrollment for the UID, as the invited branch removes a join it admits. Whether one
/// was.
fn consume_add_device_join(state: &mut State, uid: u32, principal_id: &str) -> bool {
    let key = uid.to_string();
    let selects_this_principal = state.pending_joins.get(&key).is_some_and(|join| {
        join.uid == uid && join.existing_principal_id.as_deref() == Some(principal_id)
    });
    if selects_this_principal {
        state.pending_joins.remove(&key);
        state
            .enrollments
            .retain(|_, enrollment| enrollment.uid != uid);
    }
    selects_this_principal
}

/// D3's refusal of a followed host rename, in words the host can act on. Every device of the
/// host stops authenticating once its account is renamed, so the host can remove the other
/// member only after renaming the account back.
fn host_rename_conflict(state: &State, uid: u32, old: &str, new: &str) -> anyhow::Error {
    let other = state
        .principals
        .values()
        .find(|p| p.active && p.uid != uid && names_collide(&p.username, new))
        .map_or(new, |p| p.username.as_str());
    let workspace = workspace_label(state);
    anyhow!(
        "identity_conflict: Crew can't follow this account's new name, @{new}, because @{other} already uses it in {workspace}. Rename the account back to @{old} on the server and remove @{other} from {workspace} before renaming it again."
    )
}

/// When `principal_id` already holds [`MAX_DEVICES_PER_PRINCIPAL`] devices or more, remove the
/// one added longest ago (a device without `added_at` counts as oldest) and describe it.
fn evict_oldest(state: &mut State, principal_id: &str) -> Option<Value> {
    let own: Vec<(&String, &Device)> = state
        .devices
        .iter()
        .filter(|(_, device)| device.principal_id == principal_id)
        .collect();
    if own.len() < MAX_DEVICES_PER_PRINCIPAL {
        return None;
    }
    let (oldest, device) = own
        .into_iter()
        .min_by_key(|(id, device)| (device.added_at.is_some(), device.added_at, *id))?;
    let (oldest, added_at) = (oldest.clone(), device.added_at);
    state.devices.remove(&oldest);
    Some(json!({
        "fingerprint": crate::invitation::grouped_fingerprint(&oldest),
        "added_at": added_at,
    }))
}

/// Commit an `auth.account` record as `principal_id`. The host's uses the administrative
/// allowance. A member's uses the state's headroom and counts toward their journal share; once
/// that share is used up, one record per principal per day is still admitted, so a member can
/// always get reading back on a new computer.
fn commit(
    broker: &mut Broker,
    state: State,
    principal_id: &str,
    host: bool,
    now: u64,
) -> Result<()> {
    if host {
        return broker.commit(state, principal_id, "auth.account");
    }
    let allowance = Allowance {
        state_headroom: true,
        journal_headroom: false,
        member: true,
    };
    if !broker
        .join_runtime
        .account
        .may_pass_share(principal_id, now)
    {
        return broker.commit_with(state, principal_id, "auth.account", allowance);
    }
    match broker.commit_with(state.clone(), principal_id, "auth.account", allowance) {
        Err(error) if error.to_string() == MEMBER_JOURNAL_QUOTA => {
            let past_share = Allowance {
                member: false,
                ..allowance
            };
            broker.commit_with(state, principal_id, "auth.account", past_share)?;
            broker
                .join_runtime
                .account
                .over_share
                .insert(principal_id.to_owned(), now);
            Ok(())
        }
        committed => committed,
    }
}

/// `{principal: {id, username, display_name}, device_id, workspace, role, admitted,
/// evicted_device?}`, from the committed state.
fn answer(
    broker: &Broker,
    principal_id: &str,
    device_id: &str,
    admitted: &str,
    evicted: Option<Value>,
) -> Value {
    let s = &broker.state;
    let principal = s.principals.get(principal_id);
    let display_name = principal.map(|principal| PeopleIndex::new(s).display_name(principal));
    let host = principal.is_some_and(|principal| principal.uid == s.workspace.host_uid);
    let mut answer = json!({
        "principal": {
            "id": principal_id,
            "username": principal.map(|principal| principal.username.as_str()),
            "display_name": display_name,
        },
        "device_id": device_id,
        "workspace": s.workspace,
        "role": if host { "host" } else { "member" },
        "admitted": admitted,
    });
    if let Some(evicted) = evicted {
        answer["evicted_device"] = evicted;
    }
    answer
}

/// The grouped fingerprint a person typed or copied: exactly 16 hex digits, spaces allowed
/// between them, any case.
fn fingerprint_param(params: &Value) -> Result<String> {
    let typed = text(params, "fingerprint").map_err(|_| anyhow!(BAD_FINGERPRINT))?;
    let digits = typed.chars().filter(char::is_ascii_hexdigit).count();
    ensure!(
        digits == 16 && typed.chars().all(|c| c.is_ascii_hexdigit() || c == ' '),
        BAD_FINGERPRINT
    );
    Ok(crate::invitation::grouped_fingerprint(typed))
}

/// `device.revoke {fingerprint}`: remove one device of the actor's own principal, the one
/// signing this request included (AO-5). Never through an agent's grant.
pub(super) fn revoke_device(s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
    ensure!(actor.run.is_none(), AGENT_REFUSED);
    let fingerprint = fingerprint_param(&req.params)?;
    let mut matching = s
        .devices
        .iter()
        .filter(|(device_id, device)| {
            device.principal_id == actor.id
                && crate::invitation::grouped_fingerprint(device_id) == fingerprint
        })
        .map(|(device_id, _)| device_id.clone());
    let (Some(device_id), None) = (matching.next(), matching.next()) else {
        bail!(NO_SUCH_DEVICE);
    };
    s.devices.remove(&device_id);
    let this_device = req
        .auth
        .as_ref()
        .is_some_and(|auth| auth.device_id == device_id);
    Ok(json!({"revoked": true, "fingerprint": fingerprint, "this_device": this_device}))
}
