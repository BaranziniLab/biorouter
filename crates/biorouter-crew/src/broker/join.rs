//! Joining a workspace by invitation and device code (S3a), compiled only with the
//! `join-by-name` feature.
//!
//! **This file is the hook contract, not the join.** `broker.rs` calls exactly these
//! functions, all behind `#[cfg(feature = "join-by-name")]`; the join logic itself (the new
//! form of `enrollment.invite`, `enrollment.approve`, `enrollment.cancel`,
//! `enrollment.pending` and `auth.join`) belongs to the S3a package and its mandatory
//! adversarial review (`docs/research/biorouter-crew/naming-design.md`, "Broker protocol
//! (S3a)"). Until it lands, every join method **fails closed** with `unsupported`, and the
//! lifecycle hooks keep `pending_joins` consistent so a journal written by a later build stays
//! correct here.
//!
//! The call sites, for the implementer:
//!
//! - [`pre_auth`]: in `Broker::process`, before `authenticate_actor`, for the pre-authentication
//!   methods `enrollment.pending` and `auth.join`. `None` falls through to authentication.
//! - [`mutate`]: first in `Broker::mutate` (inside `apply_mutation`, on a clone of the state,
//!   after the idempotency lookup and before the commit), for `enrollment.invite` when its
//!   params carry `username`, `enrollment.approve` and `enrollment.cancel`. The result is
//!   cached in `dedupe`, so it must never carry a code, key, UID or join ID.
//! - [`project_for_manager`]: the host's `workspace.snapshot` only, as `pending_joins`.
//! - [`on_principal_revoked`]: `enrollment.revoke`, with the revoked principal's UID and
//!   username.
//! - [`prune_expired`]: at the start of every mutation (`apply_mutation`, `auth.bootstrap`,
//!   `auth.enroll`), so expired joins never count against a cap.
//! - [`on_legacy_enrolled`]: after a successful legacy `auth.enroll` for `uid`.
//! - `hello` advertises `join_by_name_v1` whenever this module is compiled.
//!
//! Account lookups go through `broker.directory` (the [`Directory`] seam), never through libc
//! directly, and only after `broker.manager(..)` has admitted the caller.

use super::*;

/// In-memory join state that is never journaled: per-join mismatched attempts and the last
/// refusal shown by `enrollment.pending`, and the short-lived account lookup cache.
#[derive(Default)]
pub(super) struct Runtime {
    /// Claims refused with `code_mismatch`, per join ID; shown to the host as "a device with
    /// a different code tried to join".
    mismatched_attempts: BTreeMap<String, u32>,
}

fn unavailable() -> anyhow::Error {
    anyhow!("unsupported: joining by invitation is not available in this broker; use an enrollment token")
}

/// `enrollment.pending` and `auth.join`, before authentication. Any other method: `None`.
pub(super) fn pre_auth(
    _broker: &mut Broker,
    _uid: u32,
    _conn: &mut Connection,
    req: &Request,
) -> Option<Result<Value>> {
    matches!(req.method.as_str(), "enrollment.pending" | "auth.join").then(|| Err(unavailable()))
}

/// The new form of `enrollment.invite` (params carry `username`), `enrollment.approve` and
/// `enrollment.cancel`. Any other request, including the legacy `enrollment.invite {uid,
/// public_key}`: `None`.
pub(super) fn mutate(
    _broker: &mut Broker,
    _s: &mut State,
    _actor: &Actor,
    req: &Request,
) -> Option<Result<Value>> {
    let join_method = match req.method.as_str() {
        "enrollment.invite" => req.params.get("username").is_some(),
        "enrollment.approve" | "enrollment.cancel" => true,
        _ => false,
    };
    join_method.then(|| Err(unavailable()))
}

/// The host's view of pending joins: `[{username, full_name, add_device, approved,
/// created_at, expires_at, mismatched_attempts}]`. Never a code, key, UID or join ID.
pub(super) fn project_for_manager(broker: &Broker, s: &State) -> Value {
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
                    "mismatched_attempts": broker
                        .join_runtime
                        .mismatched_attempts
                        .get(&join.join_id)
                        .copied()
                        .unwrap_or(0),
                })
            })
            .collect(),
    )
}

/// `enrollment.revoke` purges pending joins for the revoked principal's UID and username.
pub(super) fn on_principal_revoked(s: &mut State, uid: u32, username: &str) {
    s.pending_joins
        .retain(|_, join| join.uid != uid && join.username != username);
}

/// Drop joins that expired at or before `now`.
pub(super) fn prune_expired(s: &mut State, now: u64) {
    s.pending_joins.retain(|_, join| !join.is_expired(now));
}

/// Completing the legacy token path for `uid` removes its pending join.
pub(super) fn on_legacy_enrolled(s: &mut State, uid: u32) {
    s.pending_joins.remove(&uid.to_string());
}
