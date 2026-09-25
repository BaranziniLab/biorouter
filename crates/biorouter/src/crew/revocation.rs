//! F3: a revocation the workspace has not confirmed is asked again by the daemon itself.
//!
//! A revoke stops the grant on this device first and saves that ([`CrewManager::revoke_scope`]),
//! then asks the workspace to `run.revoke` the run. When the workspace cannot be reached, the
//! grant is recorded as stopped here but unconfirmed ([`Revocation::Unconfirmed`]): nothing on
//! this device can use it, but until the workspace revokes the run it still honors it until the
//! run expires. Before this, only a person pressing Retry ever asked again, and "Reconnect to
//! confirm" was not true: reconnecting confirmed nothing.
//!
//! Now the daemon asks again by itself, with no person involved:
//!
//! - **whenever the connection comes back** — a person's Connect, a keepalive re-dial, a request
//!   that re-dials a bridge found gone: every one of them ends in `connect_locked`, which arms
//!   the retries for that connection;
//! - **and, while it stays up, with growing gaps** ([`KeepaliveTiming::revocation_retry_first`],
//!   doubling up to [`KeepaliveTiming::revocation_retry_max`]) when an attempt fails for a reason
//!   a retry can fix, until the workspace confirms every unconfirmed stop on the connection.
//!
//! A pass stops when the connection is down (the next connect arms a new one), when nothing is
//! left unconfirmed, or when a newer pass replaces it; there is never more than one per
//! connection. A run the workspace *answered* and refused to revoke (it does not know it, or it
//! is not this account's) is not asked again in that pass: the workspace has spoken, and asking
//! every few minutes would only repeat its answer. It stays unconfirmed, is shown so, and is
//! asked again at the next reconnect. Revoking only takes authority away and is idempotent at the
//! workspace, so an attempt whose answer was lost is safe to repeat. Only a stop recorded as
//! unconfirmed is ever sent; a live grant, a confirmed stop or one the workspace ended itself
//! (D-1) never is.
//!
//! ⚠ **Grant and revocation state; needs human review.** A chat's grant is stored under its
//! session id, one per id, so an unconfirmed stop could be lost without anyone confirming it:
//! granting the same chat again replaced it, and a deleted chat's grant was pruned once another
//! chat held its id. Either way the only record that made the daemon ask again was gone, the
//! old run stayed live at the workspace until it lapsed, and the grants list stopped showing
//! it. Such a stop is now moved to [`Registry::replaced`] instead ([`Registry::keep_replaced`]):
//! asked about by the same retries, listed as `replaced_grants`, carried across processes like
//! any stop, and forgotten only once settled ([`Registry::forget_settled_replaced`]). It is never
//! authority — nothing asks it what a chat may do.

use super::{CrewManager, Registry, ReplacedGrant, Revocation, Scope};
use std::collections::HashSet;
use std::sync::Weak;
use std::time::Duration;

/// How long an earlier grant is kept past its run's own end (`expires_at`), confirmed or not:
/// by then the workspace no longer honors the run whatever it was told, and a task's ledger has
/// long had its chance to follow the confirmation (F3).
const REPLACED_KEPT_PAST_END: u64 = 7 * 24 * 60 * 60;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl Registry {
    /// Keep `previous`, the grant `session` held until it was replaced or pruned just now, when
    /// it is a stop the workspace has not confirmed; anything else of it goes, as it always did.
    /// Answers whether it was kept, so the caller can ask the workspace again.
    pub(super) fn keep_replaced(&mut self, session: &str, previous: Scope) -> bool {
        self.forget_settled_replaced(unix_now());
        let unconfirmed = previous.expired && previous.revocation == Some(Revocation::Unconfirmed);
        let held = self
            .scopes
            .get(session)
            .is_some_and(|scope| scope.run_id == previous.run_id);
        if !unconfirmed || held {
            return false;
        }
        if !self
            .replaced
            .iter()
            .any(|kept| kept.scope.run_id == previous.run_id)
        {
            self.replaced.push(ReplacedGrant {
                session_id: session.to_owned(),
                scope: previous,
            });
        }
        true
    }

    /// Record the workspace's confirmation of revoking `run_id` on the earlier grant kept for
    /// it under `session`, if there is one.
    pub(super) fn confirm_replaced(&mut self, session: &str, run_id: &str) {
        for kept in self
            .replaced
            .iter_mut()
            .filter(|kept| kept.session_id == session && kept.scope.run_id == run_id)
        {
            kept.scope.revocation = Some(Revocation::Confirmed);
        }
        self.forget_settled_replaced(unix_now());
    }

    /// Forget the earlier grants that are settled: every one [`REPLACED_KEPT_PAST_END`] past
    /// its run's own end, and a confirmed one whose end was never recorded. An unconfirmed one
    /// whose end is unknown is kept until the workspace confirms it.
    pub(super) fn forget_settled_replaced(&mut self, now: u64) {
        self.replaced.retain(|kept| match kept.scope.expires_at {
            Some(end) => end.saturating_add(REPLACED_KEPT_PAST_END) > now,
            None => kept.scope.revocation != Some(Revocation::Confirmed),
        });
    }
}

/// What one pass over a connection's unconfirmed stops came to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RetryPass {
    /// Nothing is left to ask: every stop is confirmed, or the workspace refused the rest.
    Settled,
    /// The connection is down; the next connect asks again.
    Disconnected,
    /// Some stops are still unconfirmed for a reason a later try may fix.
    Pending,
}

impl CrewManager {
    /// Arm the retries of `id`'s unconfirmed revocations and start them. Arming replaces any
    /// earlier pass's token, so that one stops before its next try. A manager built without
    /// [`CrewManager::shared`], or outside a runtime, starts nothing.
    pub(super) fn schedule_revocation_retries(&self, id: &str) {
        let Some(manager) = self.this.get().cloned() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let token = rand::random::<u64>();
        self.revocation_retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_owned(), token);
        runtime.spawn(revocation_retries(manager, id.to_owned(), token));
    }

    fn revocation_retry_armed(&self, id: &str, token: u64) -> bool {
        self.revocation_retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            == Some(&token)
    }

    fn disarm_revocation_retries_if(&self, id: &str, token: u64) {
        let mut armed = self
            .revocation_retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if armed.get(id) == Some(&token) {
            armed.remove(id);
        }
    }

    /// The grants on `id` stopped here whose revocation the workspace has not confirmed, as
    /// `(session, run_id)`: chats' grants, and the earlier grants kept for their revocation
    /// ([`ReplacedGrant`]). Each run once.
    pub(super) async fn unconfirmed_revocations(&self, id: &str) -> Vec<(String, String)> {
        let unconfirmed = |scope: &Scope| {
            scope.connection_id == id
                && scope.expired
                && scope.revocation == Some(Revocation::Unconfirmed)
        };
        let registry = self.registry.lock().await;
        let mut pending: Vec<(String, String)> = registry
            .scopes
            .iter()
            .filter(|(_, scope)| unconfirmed(scope))
            .map(|(session, scope)| (session.clone(), scope.run_id.clone()))
            .chain(
                registry
                    .replaced
                    .iter()
                    .filter(|kept| unconfirmed(&kept.scope))
                    .map(|kept| (kept.session_id.clone(), kept.scope.run_id.clone())),
            )
            .collect();
        drop(registry);
        pending.sort();
        let mut seen = HashSet::new();
        pending.retain(|(_, run_id)| seen.insert(run_id.clone()));
        pending
    }

    /// One pass: ask the workspace to revoke each unconfirmed stop on `id` that it has not
    /// refused in this round (`refused`), while the connection is up and this pass is still
    /// the armed one.
    pub(super) async fn retry_unconfirmed_revocations(
        &self,
        id: &str,
        token: u64,
        refused: &mut HashSet<String>,
    ) -> RetryPass {
        let mut pending = self.unconfirmed_revocations(id).await;
        pending.retain(|(_, run_id)| !refused.contains(run_id));
        if pending.is_empty() {
            return RetryPass::Settled;
        }
        let mut outcome = RetryPass::Settled;
        for (session, run_id) in pending {
            if !self.revocation_retry_armed(id, token) {
                return RetryPass::Pending;
            }
            // A connection that is down is not dialled from here: bringing it back is the
            // keepalive's or a person's, and that connect arms a pass of its own. (A bridge
            // that is up but stale is checked before use, as for every request.)
            if !self.transports.lock().await.contains_key(id) {
                return RetryPass::Disconnected;
            }
            match self.confirm_revocation(id, &session, &run_id).await {
                Ok(_) => {
                    tracing::info!(
                        connection = id,
                        session,
                        run_id,
                        "The workspace confirmed a Crew revocation the daemon asked for again"
                    );
                }
                Err(error) if super::refused_by_workspace(&error) => {
                    tracing::warn!(
                        connection = id,
                        session,
                        run_id,
                        %error,
                        "The workspace refused to revoke a Crew run; asking again at the next reconnect"
                    );
                    refused.insert(run_id);
                }
                Err(error) => {
                    tracing::debug!(
                        connection = id,
                        session,
                        run_id,
                        %error,
                        "A Crew revocation is still unconfirmed; asking again later"
                    );
                    outcome = RetryPass::Pending;
                }
            }
        }
        outcome
    }
}

/// The retries one arming started with `token`: a pass now, then after each gap (doubling from
/// the first to the largest) while some stop is still unconfirmed for a reason a retry may fix.
/// Ends when the pass settles, when the connection is down, or when a newer arming replaced it;
/// disarms only its own token.
async fn revocation_retries(manager: Weak<CrewManager>, id: String, token: u64) {
    let mut refused = HashSet::new();
    let mut gap: Option<Duration> = None;
    loop {
        if let Some(delay) = gap {
            tokio::time::sleep(delay).await;
        }
        let Some(strong) = manager.upgrade() else {
            return;
        };
        if !strong.revocation_retry_armed(&id, token) {
            return;
        }
        match strong
            .retry_unconfirmed_revocations(&id, token, &mut refused)
            .await
        {
            RetryPass::Pending => {}
            RetryPass::Settled | RetryPass::Disconnected => break,
        }
        let timing = strong.keepalive_timing();
        gap = Some(
            match gap {
                None => timing.revocation_retry_first,
                Some(previous) => previous.saturating_mul(2).min(timing.revocation_retry_max),
            }
            .max(Duration::from_millis(1)),
        );
    }
    if let Some(strong) = manager.upgrade() {
        strong.disarm_revocation_retries_if(&id, token);
    }
}
