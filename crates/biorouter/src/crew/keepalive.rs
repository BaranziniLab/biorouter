//! D-KEEPALIVE: an idle Crew connection stays up.
//!
//! The broker drops a bridge that sends nothing for 300 s (`serve_client`'s read timeout in
//! `biorouter-crew/src/broker.rs`). Nothing observes while Crew is off screen, so before this a
//! connection that sat idle for five minutes died, the daemon learnt it only when the next
//! request failed, and the person had to press Retry and then Connect (Q2-01, five critics).
//!
//! Two things keep it up, both over what the person already authenticated, and neither skips a
//! gate:
//!
//! - **A heartbeat.** While a bridge is this connection's live one, a task started by the
//!   connect sends `hello` over it whenever it has been idle for [`KeepaliveTiming::idle`], well
//!   inside the broker's 300 s. The answer is verified exactly as a refresh verifies it (the
//!   pinned workspace key and node, over a fresh nonce), so the heartbeat is also a check that
//!   the bridge still reaches the workspace it pinned. It needs no device key and writes nothing
//!   to the workspace.
//! - **A transparent re-dial.** When a heartbeat finds the bridge gone (this computer slept,
//!   the network dropped, the broker restarted), or a request finds it ended or idle past
//!   [`KeepaliveTiming::probe_before_use`] before writing anything, the daemon connects again
//!   the way Connect does: `BatchMode=yes` over the same SSH settings and control socket, and a
//!   freshly verified `hello`. So it never prompts: when SSH needs a password or a code, the
//!   re-dial fails with that classified reason, the connection reads disconnected with it, and
//!   Sign in is the person's to open. A re-dial never follows a person's Disconnect (or an edit
//!   or removal), never runs while a sign-in is pending, and a failure that is not about the
//!   network (sign-in, host key, a missing bridge, a workspace that no longer verifies) is
//!   final. A network failure is tried again at most [`KeepaliveTiming::retry_delays`] times,
//!   with growing gaps, and the connection shows the real reason all the while.
//!
//! Nothing here re-sends a request whose outcome is unknown: a heartbeat is only ever a
//! `hello`, and a request re-dials only when nothing has been written to the old bridge.

use super::{transport, CrewManager, SshFailure, SshFailureKind, WorkspaceIdentityError};
use anyhow::Result;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::Mutex;

/// How the keepalive paces itself. The defaults keep every idle gap under the broker's 300 s:
/// a bridge idle for `idle` is noticed within `tick`, so the longest silence is about 150 s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeepaliveTiming {
    /// How often the keepalive task looks at its bridge.
    pub tick: Duration,
    /// How long a bridge may sit idle before a heartbeat is sent.
    pub idle: Duration,
    /// A request to a bridge idle this long (the heartbeat was not running: this computer
    /// slept) checks it with a heartbeat first, and re-dials if that fails, before writing.
    pub probe_before_use: Duration,
    /// The gaps before each retry of a re-dial that failed for a network reason.
    pub retry_delays: [Duration; 3],
}

impl Default for KeepaliveTiming {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(30),
            idle: Duration::from_secs(120),
            probe_before_use: Duration::from_secs(170),
            retry_delays: [
                Duration::from_secs(20),
                Duration::from_secs(60),
                Duration::from_secs(180),
            ],
        }
    }
}

/// What a re-dial of a dropped bridge came to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Redial {
    /// Connected again over a freshly verified bridge.
    Reconnected,
    /// Someone else already replaced the bridge (a person's Connect): use theirs.
    Replaced,
    /// The connection is no longer this bridge's to repair: it was disconnected (by a person,
    /// an edit or a failure already recorded), or a sign-in is pending.
    NotOurs,
}

/// The last-error text of a bridge the keepalive found gone, until the re-dial says more.
const IDLE_DROPPED: &str = "The connection to this workspace dropped while it was idle.";

/// Whether a re-dial that failed with `error` may be tried again later: only a network
/// failure. Anything that needs a person (sign-in, a host key) or that says the workspace is
/// not the one pinned is final, and is left showing.
fn worth_retrying(error: &anyhow::Error) -> bool {
    if error
        .chain()
        .any(|cause| cause.is::<WorkspaceIdentityError>())
    {
        return false;
    }
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SshFailure>())
        .is_some_and(|failure| {
            matches!(
                failure.kind,
                SshFailureKind::Unreachable | SshFailureKind::Other
            ) && failure.code != "ssh_sign_in_refused"
        })
}

impl CrewManager {
    pub(super) fn keepalive_timing(&self) -> KeepaliveTiming {
        *self
            .keepalive
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(super) fn set_keepalive_timing(&self, timing: KeepaliveTiming) {
        *self
            .keepalive
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = timing;
    }

    /// Keep `transport`, `id`'s bridge from now on, alive while it stays `id`'s bridge. A
    /// manager built without [`CrewManager::shared`] starts nothing.
    pub(super) fn start_keepalive(&self, id: &str, transport: &Arc<Mutex<transport::Transport>>) {
        let Some(manager) = self.this.get().cloned() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(keepalive(manager, id.to_owned(), Arc::downgrade(transport)));
    }

    /// Whether `transport` is still `id`'s published bridge.
    pub(super) async fn is_current_transport(
        &self,
        id: &str,
        transport: &Arc<Mutex<transport::Transport>>,
    ) -> bool {
        self.transports
            .lock()
            .await
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, transport))
    }

    /// One verified `hello` over `transport` (see [`CrewManager::hello_over`]). Retires
    /// nothing; `Err` means the bridge can't be trusted to carry the next request.
    pub(super) async fn heartbeat(
        &self,
        id: &str,
        transport: &Arc<Mutex<transport::Transport>>,
    ) -> Result<()> {
        let c = self.connection(id).await?;
        let pinned = c
            .node_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Connect to this workspace first."))?;
        let (answer, usable) = self.hello_over(id, &c, &pinned, transport).await;
        answer?;
        anyhow::ensure!(usable, "Crew SSH transport is unusable");
        Ok(())
    }

    /// `id`'s bridge, checked first when it may have gone while nobody was looking: it has
    /// ended, or it sat idle past [`KeepaliveTiming::probe_before_use`]. A stale bridge gets a
    /// heartbeat; one that fails it is dialled again (without a prompt) before the caller
    /// writes anything, so a request never meets a bridge the broker already dropped.
    pub(super) async fn live_transport(
        &self,
        id: &str,
    ) -> Result<Arc<Mutex<transport::Transport>>> {
        let transport = self.transport(id).await?;
        let (ended, idle) = {
            let mut locked = transport.lock().await;
            (locked.has_ended(), locked.idle_for())
        };
        if !ended && idle < self.keepalive_timing().probe_before_use {
            return Ok(transport);
        }
        if !ended && self.heartbeat(id, &transport).await.is_ok() {
            return Ok(transport);
        }
        self.redial_dropped(id, &transport).await?;
        self.transport(id).await
    }

    /// Dial `id` again in place of `failed`, a bridge found gone, the way Connect does it.
    /// Only while `failed` is still `id`'s bridge and no sign-in is pending; a failure is
    /// recorded as the connection's `last_error` and returned.
    pub(super) async fn redial_dropped(
        &self,
        id: &str,
        failed: &Arc<Mutex<transport::Transport>>,
    ) -> Result<Redial> {
        let _lifecycle = self.connection_guard(id).await?;
        match self.transports.lock().await.get(id) {
            Some(current) if Arc::ptr_eq(current, failed) => {}
            Some(_) => return Ok(Redial::Replaced),
            None => return Ok(Redial::NotOurs),
        }
        if super::authentication::ensure_connect_available(id).is_err() {
            self.retire_locked(id, failed, IDLE_DROPPED).await;
            return Ok(Redial::NotOurs);
        }
        match self.connect_locked(id).await {
            Ok(_) => Ok(Redial::Reconnected),
            Err(error) => {
                self.retire_locked(id, failed, &error.to_string()).await;
                Err(error)
            }
        }
    }

    fn arm_idle_redial(&self, id: &str) -> u64 {
        let token = rand::random::<u64>();
        self.idle_redial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_owned(), token);
        token
    }

    fn idle_redial_armed(&self, id: &str, token: u64) -> bool {
        self.idle_redial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            == Some(&token)
    }

    /// No idle re-dial is owed to `id` any more: someone connected or disconnected it.
    pub(super) fn disarm_idle_redial(&self, id: &str) {
        self.idle_redial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }

    /// A later try of a re-dial that failed for a network reason, while it is still owed:
    /// `None` when it is not (someone connected or disconnected meanwhile, or a sign-in is
    /// pending).
    async fn retry_idle_redial(&self, id: &str, token: u64) -> Option<Result<()>> {
        let _lifecycle = self.connection_guard(id).await.ok()?;
        if !self.idle_redial_armed(id, token)
            || self.transports.lock().await.contains_key(id)
            || super::authentication::ensure_connect_available(id).is_err()
        {
            return None;
        }
        match self.connect_locked(id).await {
            Ok(_) => Some(Ok(())),
            Err(error) => {
                let mut registry = self.registry.lock().await;
                if let Some(connection) = registry.connections.iter_mut().find(|c| c.id == id) {
                    connection.status = "disconnected".into();
                    connection.last_error = Some(error.to_string());
                }
                Some(Err(error))
            }
        }
    }

    /// After a heartbeat found `failed` gone: dial again now, and, for a network failure,
    /// a few more times with growing gaps.
    async fn recover_dropped(&self, id: &str, failed: &Arc<Mutex<transport::Transport>>) {
        let first = self.redial_dropped(id, failed).await;
        let error = match first {
            Ok(_) => return,
            Err(error) => error,
        };
        tracing::info!(connection = id, error = %error, "Crew bridge dropped and could not be dialled again");
        if !worth_retrying(&error) {
            return;
        }
        let token = self.arm_idle_redial(id);
        for delay in self.keepalive_timing().retry_delays {
            tokio::time::sleep(delay).await;
            match self.retry_idle_redial(id, token).await {
                None | Some(Ok(())) => return,
                Some(Err(error)) if worth_retrying(&error) => continue,
                Some(Err(_)) => break,
            }
        }
        if self.idle_redial_armed(id, token) {
            self.disarm_idle_redial(id);
        }
    }
}

/// The keepalive of one bridge: heartbeat it while it is idle, and dial again when it is gone.
/// Ends when the bridge is no longer the connection's (a disconnect, a reconnect, a removal);
/// a successful re-dial's own connect starts the next bridge's keepalive.
async fn keepalive(
    manager: Weak<CrewManager>,
    id: String,
    transport: Weak<Mutex<transport::Transport>>,
) {
    loop {
        let Some(timing) = manager.upgrade().map(|manager| manager.keepalive_timing()) else {
            return;
        };
        tokio::time::sleep(timing.tick).await;
        let (Some(manager), Some(transport)) = (manager.upgrade(), transport.upgrade()) else {
            return;
        };
        if !manager.is_current_transport(&id, &transport).await {
            return;
        }
        // A bridge in use is not idle; look again next tick.
        let (ended, idle) = match transport.try_lock() {
            Ok(mut locked) => (locked.has_ended(), locked.idle_for()),
            Err(_) => continue,
        };
        if !ended && idle < timing.idle {
            continue;
        }
        if !ended && manager.heartbeat(&id, &transport).await.is_ok() {
            continue;
        }
        manager.recover_dropped(&id, &transport).await;
        return;
    }
}
