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
//!   final. A network failure is tried again [`KeepaliveTiming::retry_delays`] times, with
//!   growing gaps, and then every [`KeepaliveTiming::late_retry_every`] for up to
//!   [`KeepaliveTiming::late_retry_for`] (Q3-11: a network that came back after the quick
//!   retries left the connection down until someone pressed Connect). The connection shows
//!   the real reason all the while.
//!
//! **The retries are armed whoever finds the drop (Q4-01).** The schedule
//! ([`CrewManager::schedule_redials`]) used to be armed only by the keepalive's own re-dial, so a
//! drop that a request found first (Crew's own view, reading the connection while the network
//! was down) retired the bridge, ended the keepalive with it, and left nothing to dial again
//! when the network came back. Now the same schedule is armed, with the same rules, by each of
//! the three ways a drop is found: a heartbeat or an ended bridge the keepalive notices, a
//! request that finds its bridge gone before writing (or whose bridge breaks while carrying it:
//! the request is never sent again, only the connection is dialled), and a person's Connect that
//! fails for a network reason. It is armed under the connection's lifecycle guard, in the same
//! hold as the failure it follows, so a Disconnect can only come after it and always disarms it.
//! Each arming replaces the last one's token, so there is never more than one schedule, and it
//! is never armed for a device whose membership ended ([`MEMBERSHIP_ENDED`]) until a dial
//! succeeds again.
//!
//! **An ended bridge is noticed within [`KeepaliveTiming::ended_check`] (Q4-08).** Between
//! ticks the keepalive looks, every few seconds, at whether its bridge's `ssh` has exited. That
//! is local process state, nothing is sent, and the heartbeat and idle rules are unchanged; it
//! only means a dead bridge reads offline (and is dialled again) in seconds rather than at the
//! next 30 s tick.
//!
//! Only a bridge that is **gone** is dialled again. A heartbeat the bridge carried and whose
//! answer was refused (a different node, a `hello` that lost the v2 signature it had, one that
//! does not verify, the broker refusing it) says nothing about the network: a re-dial would only
//! take a second answer from the same place, and `connect` accepts a v1-only `hello`, so a
//! downgrade the refresh path refuses would be adopted without a word. That refusal is final
//! instead: the bridge is retired with it, and it is left showing.
//!
//! Nothing here re-sends a request whose outcome is unknown: a heartbeat is only ever a
//! `hello`, and a request re-dials only when nothing has been written to the old bridge.
//!
//! **A membership that ended stops everything (Q3-12).** A heartbeat is a keyless `hello`, so
//! it cannot see that the workspace revoked this computer: a revoked device's bridge used to be
//! heartbeated and re-dialled indefinitely, and read `connected`, while every read was refused.
//! Now a person-signed request the workspace refuses because it no longer knows this device
//! (`unauthorized: unknown device`, a device whose account is gone, `principal_revoked`) is
//! identity-final, like a workspace that no longer verifies: the idle re-dial is disarmed, the
//! bridge is retired (so its keepalive ends), and the connection reads disconnected with
//! [`MEMBERSHIP_ENDED`] beside its `last_error`. Nothing dials it again by itself; a person's
//! Connect still may, and verifies from scratch. Only a device the workspace accepted in this
//! process can have a membership that *ended*: one it never knew is still joining (the join
//! page keeps its bridge up while the host approves), and its refusals change nothing here. So
//! that a revocation is noticed without waiting for someone to use Crew, every successful
//! keepalive re-dial of such a device is followed by one person-signed read.

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
    /// Once `retry_delays` are spent on network failures, how often to try again (Q3-11).
    pub late_retry_every: Duration,
    /// How long those later tries go on; zero for none.
    pub late_retry_for: Duration,
    /// How often, between ticks, the keepalive checks whether its bridge's `ssh` has exited:
    /// local process state only, never a request (Q4-08).
    pub ended_check: Duration,
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
            late_retry_every: Duration::from_secs(5 * 60),
            late_retry_for: Duration::from_secs(60 * 60),
            ended_check: Duration::from_secs(5),
        }
    }
}

impl KeepaliveTiming {
    /// Every gap a re-dial that keeps failing for a network reason waits before its next try:
    /// the quick retries, then the later ones.
    pub(super) fn redial_gaps(&self) -> impl Iterator<Item = Duration> {
        let late = if self.late_retry_every.is_zero() {
            0
        } else {
            (self.late_retry_for.as_nanos() / self.late_retry_every.as_nanos()) as usize
        };
        self.retry_delays
            .into_iter()
            .chain(std::iter::repeat_n(self.late_retry_every, late))
    }

    /// How long the keepalive sleeps between looks at its bridge: [`Self::ended_check`], never
    /// longer than a tick, and never zero.
    fn nap(&self) -> Duration {
        let nap = if self.ended_check.is_zero() {
            self.tick
        } else {
            self.ended_check.min(self.tick)
        };
        nap.max(Duration::from_millis(1))
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

/// What one heartbeat found.
#[derive(Debug)]
pub(super) enum Heartbeat {
    /// A `hello` that verified against the pinned workspace and node, over this bridge.
    Alive,
    /// The bridge could not carry the `hello` (it ended, timed out or broke): dialling again
    /// may help.
    Gone(anyhow::Error),
    /// The bridge carried the `hello` and its answer was refused. Final: never re-dialled.
    Refused(anyhow::Error),
}

/// The last-error text of a bridge the keepalive found gone, until the re-dial says more.
const IDLE_DROPPED: &str = "The connection to this workspace dropped while it was idle.";

/// `last_error_code` of a connection whose device the workspace no longer knows (Q3-12): the
/// shared contract `GET /crew/connections` serves and the desktop reads, so it never has to
/// match `last_error`'s words.
pub const MEMBERSHIP_ENDED: &str = "crew_membership_ended";

/// The read a keepalive re-dial sends to learn whether the workspace still knows this device:
/// the cheapest a device can sign (one account lookup), and the one the join status already
/// uses to tell a member from a stranger.
const MEMBERSHIP_PROBE: &str = "profile.suggest";

/// The broker's `{code, message}` for a refusal it answered, from the transport's error.
pub(super) fn broker_refusal(error: &anyhow::Error) -> Option<(String, String)> {
    let text = error.to_string();
    let envelope: serde_json::Value =
        serde_json::from_str(text.strip_prefix("Crew broker refused request: ")?).ok()?;
    Some((
        envelope.get("code")?.as_str()?.to_owned(),
        envelope.get("message")?.as_str()?.to_owned(),
    ))
}

/// Whether the workspace refused a person-signed request because it no longer knows this
/// device or its account (identity-final). `unauthorized` is also the code of a consumed
/// challenge or a missing signature, which a retry fixes, so only the texts that name the
/// device or account count, and `principal_revoked` for a broker that says so outright.
pub(super) fn membership_refused(error: &anyhow::Error) -> bool {
    let Some((code, message)) = broker_refusal(error) else {
        return false;
    };
    code == "principal_revoked"
        || (code == "unauthorized"
            && matches!(
                message.as_str(),
                "unauthorized: unknown device" | "unauthorized"
            ))
}

/// Whether a re-dial that failed with `error` may be tried again later: only a network
/// failure. Anything that needs a person (sign-in, a host key) or that says the workspace is
/// not the one pinned is final, and is left showing.
pub(super) fn worth_retrying(error: &anyhow::Error) -> bool {
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
    /// nothing. Whether a failure is [`Heartbeat::Gone`] or [`Heartbeat::Refused`] is the
    /// bridge's own word: it is gone when it can no longer carry a request, and an answer it
    /// carried that was refused leaves it usable.
    pub(super) async fn heartbeat(
        &self,
        id: &str,
        transport: &Arc<Mutex<transport::Transport>>,
    ) -> Heartbeat {
        let c = match self.connection(id).await {
            Ok(c) => c,
            Err(error) => return Heartbeat::Refused(error),
        };
        let Some(pinned) = c.node_id.clone() else {
            return Heartbeat::Refused(anyhow::anyhow!("Connect to this workspace first."));
        };
        match self.hello_over(id, &c, &pinned, transport).await {
            (Ok(_), true) => Heartbeat::Alive,
            (Ok(_), false) => Heartbeat::Gone(anyhow::anyhow!("Crew SSH transport is unusable")),
            (Err(error), false) => Heartbeat::Gone(error),
            (Err(error), true) => Heartbeat::Refused(error),
        }
    }

    /// A heartbeat over `refused` was answered and the answer refused: retire the bridge with
    /// that reason, left showing, and dial nothing. `false` (and nothing changed) when it is no
    /// longer `id`'s bridge: someone connected, disconnected, edited or removed it meanwhile.
    pub(super) async fn retire_refused(
        &self,
        id: &str,
        refused: &Arc<Mutex<transport::Transport>>,
        error: &anyhow::Error,
    ) -> Result<bool> {
        let _lifecycle = self.connection_guard(id).await?;
        if !self.is_current_transport(id, refused).await {
            return Ok(false);
        }
        tracing::warn!(connection = id, error = %error, "Crew heartbeat answer refused; not dialling again");
        self.retire_locked(id, refused, &error.to_string()).await;
        Ok(true)
    }

    /// `id`'s bridge, checked first when it may have gone while nobody was looking: it has
    /// ended, or it sat idle past [`KeepaliveTiming::probe_before_use`]. A stale bridge gets a
    /// heartbeat; one found gone is dialled again (without a prompt) before the caller writes
    /// anything, so a request never meets a bridge the broker already dropped. One whose
    /// heartbeat answer was refused is retired with that reason, and the request fails with it.
    ///
    /// Every request over a connection's bridge takes it from here, so a bridge that died
    /// while this computer slept is re-dialled whichever request finds it first; and when that
    /// re-dial fails for a network reason, the retries are armed here too (Q4-01, see
    /// [`Self::redial_dropped`]), since the keepalive of the bridge it retired has ended.
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
        if !ended {
            match self.heartbeat(id, &transport).await {
                Heartbeat::Alive => return Ok(transport),
                Heartbeat::Refused(error) => {
                    if self.retire_refused(id, &transport, &error).await? {
                        return Err(error);
                    }
                    // Replaced meanwhile (a person's Connect): use theirs, if there is one.
                    return self.transport(id).await;
                }
                Heartbeat::Gone(error) => {
                    tracing::info!(connection = id, error = %error, "Crew bridge found gone before a request; dialling again");
                }
            }
        }
        self.redial_dropped(id, &transport).await?;
        self.transport(id).await
    }

    /// Dial `id` again in place of `failed`, a bridge found gone, the way Connect does it.
    /// Only while `failed` is still `id`'s bridge and no sign-in is pending; a failure is
    /// recorded as the connection's `last_error` and returned. Whoever found the drop (the
    /// keepalive, or a request), a failure worth retrying arms the retries
    /// ([`Self::schedule_redials`]) before the lifecycle guard is let go, so a Disconnect can
    /// only follow the arming, and disarms it.
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
                tracing::info!(connection = id, error = %error, "Crew bridge dropped and could not be dialled again");
                if worth_retrying(&error) {
                    self.schedule_redials(id);
                }
                Err(error)
            }
        }
    }

    /// A request's bridge broke while carrying it, and the caller (holding the lifecycle
    /// guard) just retired it (Q4-01). The request is never sent again: its outcome is unknown,
    /// and its caller was told so. For a network failure the connection is dialled again later,
    /// on the same schedule a drop the keepalive finds gets.
    pub(super) fn request_bridge_failed(&self, id: &str, error: &anyhow::Error) {
        if !worth_retrying(error) {
            return;
        }
        tracing::info!(connection = id, error = %error, "Crew bridge failed while carrying a request; dialling again later");
        self.schedule_redials(id);
    }

    /// A person's Connect failed with `error`; the caller still holds the lifecycle guard
    /// (Q4-01). For a network failure while no bridge is up, the daemon keeps trying on the same
    /// schedule, so a network that comes back reconnects without a second click. Any other
    /// failure (sign-in, a host key, the workspace's identity, a refusal) is final: it also ends
    /// a schedule a drop armed earlier, because the latest dial is the one that says why.
    pub(super) async fn connect_failed(&self, id: &str, error: &anyhow::Error) {
        if !worth_retrying(error) {
            self.disarm_idle_redial(id);
            return;
        }
        if self.transports.lock().await.contains_key(id) {
            return;
        }
        tracing::info!(connection = id, error = %error, "Crew connect failed for a network reason; trying again by itself");
        self.schedule_redials(id);
    }

    /// Arm the retries of a connection whose bridge is down after a network failure, and start
    /// them (Q4-01): the gaps of [`KeepaliveTiming::redial_gaps`], each try made only while the
    /// schedule is still owed ([`Self::retry_idle_redial`]). The caller holds `id`'s lifecycle
    /// guard. Arming replaces any earlier schedule's token, so that one stops at its next try and
    /// there is never more than one. Nothing is armed for a device the workspace no longer
    /// knows ([`MEMBERSHIP_ENDED`]), or by a manager built without [`CrewManager::shared`].
    pub(super) fn schedule_redials(&self, id: &str) {
        if self.membership_ended(id) {
            tracing::info!(
                connection = id,
                "Not dialling a Crew connection again by itself: this computer is no longer a member"
            );
            return;
        }
        let Some(manager) = self.this.get().cloned() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let token = self.arm_idle_redial(id);
        runtime.spawn(redial_schedule(manager, id.to_owned(), token));
    }

    /// Whether the workspace said this device is no longer a member ([`Self::end_membership`])
    /// and nothing has connected since (a connect clears it).
    fn membership_ended(&self, id: &str) -> bool {
        self.error_codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .is_some_and(|(code, _)| *code == MEMBERSHIP_ENDED)
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

    /// Disarm `id`'s schedule only while it is still the one armed with `token`: a schedule
    /// that ends never removes the one that replaced it.
    fn disarm_idle_redial_if(&self, id: &str, token: u64) {
        let mut armed = self
            .idle_redial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if armed.get(id) == Some(&token) {
            armed.remove(id);
        }
    }

    /// A later try of a re-dial that failed for a network reason, while it is still owed:
    /// `None` when it is not (someone connected or disconnected meanwhile, another finder
    /// armed a newer schedule, the membership ended, or a sign-in is pending).
    async fn retry_idle_redial(&self, id: &str, token: u64) -> Option<Result<()>> {
        let _lifecycle = self.connection_guard(id).await.ok()?;
        if !self.idle_redial_armed(id, token)
            || self.membership_ended(id)
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

    /// After the keepalive found `failed` gone (never one whose answer was refused): dial again
    /// now. A dial that connects is followed by a membership check
    /// ([`Self::probe_membership`]); one that fails for a network reason has armed the later
    /// tries ([`Self::redial_dropped`]).
    async fn recover_dropped(&self, id: &str, failed: &Arc<Mutex<transport::Transport>>) {
        if let Ok(Redial::Reconnected) = self.redial_dropped(id, failed).await {
            self.probe_membership(id).await;
        }
    }

    /// What a person-signed request's answer says about membership. An accepted request
    /// records that the workspace knows this device; a refusal that says it no longer does ends
    /// the connection for good, for a device it accepted before ([`Self::end_membership`]).
    /// The `auth.*` requests (and the join door's) are answered before authentication, so
    /// only their success speaks: this device was just enrolled.
    pub(super) async fn heed_membership(
        &self,
        join_door: bool,
        id: &str,
        method: &str,
        answer: &Result<serde_json::Value>,
        transport: &Arc<Mutex<transport::Transport>>,
    ) {
        match answer {
            Ok(_) => self.note_member(id),
            Err(error)
                if !join_door
                    && !method.starts_with("auth.")
                    && membership_refused(error)
                    && self.was_member(id) =>
            {
                if let Err(ending) = self.end_membership(id, transport).await {
                    tracing::warn!(connection = id, error = %ending, "Couldn't end a Crew connection whose membership ended");
                }
            }
            Err(_) => {}
        }
    }

    /// Record that the workspace accepted a person-signed request from `id`'s device.
    pub(super) fn note_member(&self, id: &str) {
        self.members
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_owned());
    }

    fn was_member(&self, id: &str) -> bool {
        self.members
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(id)
    }

    fn forget_member(&self, id: &str) {
        self.members
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }

    /// The typed code beside `connection`'s `last_error`, while that error is still the one
    /// it was set with (a connect, a later failure or a reload replaces it).
    pub fn last_error_code(&self, connection: &super::Connection) -> Option<&'static str> {
        let codes = self
            .error_codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (code, message) = codes.get(&connection.id)?;
        (connection.status == "disconnected"
            && connection.last_error.as_deref() == Some(message.as_str()))
        .then_some(*code)
    }

    pub(super) fn clear_error_code(&self, id: &str) {
        self.error_codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }

    /// The workspace's name for `id`, as a person would call it: the signed `hello`'s name,
    /// else this device's name for the connection.
    async fn workspace_label(&self, id: &str) -> String {
        if let Some(name) = self
            .broker_hello(id)
            .and_then(|hello| hello.workspace_name)
            .filter(|name| biorouter_crew::workspace_name_valid(name))
        {
            return name;
        }
        self.connection(id)
            .await
            .map(|c| super::plain_label(&c.name))
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "this workspace".to_owned())
    }

    /// The workspace refused `refused`'s device as no longer a member (see the module
    /// documentation). Identity-final: no re-dial is owed any more, and the bridge, while it is
    /// still `id`'s, is retired with a sentence and [`MEMBERSHIP_ENDED`], which ends its
    /// keepalive. `false` (and the bridge, and this device's membership, left alone) when a
    /// person's Connect already replaced it: that bridge answers for itself, still as a member,
    /// so its own refusal ends it too rather than reading as a computer still joining.
    pub(super) async fn end_membership(
        &self,
        id: &str,
        refused: &Arc<Mutex<transport::Transport>>,
    ) -> Result<bool> {
        let _lifecycle = self.connection_guard(id).await?;
        self.disarm_idle_redial(id);
        // A late refusal from a bridge already replaced says nothing about the one that
        // replaced it: that bridge keeps its standing, so its own refusal still ends it.
        if !self.is_current_transport(id, refused).await {
            return Ok(false);
        }
        // A person's Connect verifies from scratch: until the workspace accepts this device
        // again, its refusals are those of a computer still joining.
        self.forget_member(id);
        let message = format!(
            "This computer is no longer a member of {}.",
            self.workspace_label(id).await
        );
        tracing::warn!(
            connection = id,
            "The Crew workspace no longer knows this device; disconnecting and not dialling again"
        );
        self.error_codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_owned(), (MEMBERSHIP_ENDED, message.clone()));
        self.retire_locked(id, refused, &message).await;
        Ok(true)
    }

    /// After a keepalive re-dial: one person-signed read, so that a device the workspace
    /// revoked while this computer slept or was offline is noticed now rather than at the next
    /// request someone makes. Only for a device the workspace accepted in this process; the
    /// read's refusal, if any, is handled where every signed request's is.
    pub(super) async fn probe_membership(&self, id: &str) {
        if !self.was_member(id) {
            return;
        }
        if let Err(error) = self
            .signed_request(id, MEMBERSHIP_PROBE, serde_json::json!({}), None)
            .await
        {
            tracing::debug!(connection = id, error = %error, "Crew membership check after a re-dial failed");
        }
    }
}

/// The retries one finder armed with `token` (see [`CrewManager::schedule_redials`]): each gap,
/// then one try while the schedule is still owed. Ends at a connect, at a failure that is not
/// about the network, when the schedule is no longer owed (a Disconnect, an edit or removal, a
/// newer schedule, a pending sign-in), or when the gaps run out; and disarms its own token, never
/// a newer one.
async fn redial_schedule(manager: Weak<CrewManager>, id: String, token: u64) {
    let Some(gaps) = manager
        .upgrade()
        .map(|manager| manager.keepalive_timing().redial_gaps().collect::<Vec<_>>())
    else {
        return;
    };
    for delay in gaps {
        tokio::time::sleep(delay).await;
        let Some(manager) = manager.upgrade() else {
            return;
        };
        match manager.retry_idle_redial(&id, token).await {
            None => break,
            Some(Ok(())) => {
                manager.probe_membership(&id).await;
                break;
            }
            Some(Err(error)) if worth_retrying(&error) => continue,
            Some(Err(_)) => break,
        }
    }
    if let Some(manager) = manager.upgrade() {
        manager.disarm_idle_redial_if(&id, token);
    }
}

/// The keepalive of one bridge: heartbeat it while it is idle, and dial again when it is gone.
/// Between ticks it checks, every [`KeepaliveTiming::ended_check`], whether the bridge's `ssh`
/// has exited (Q4-08); idleness is only looked at on a tick. Ends when the bridge is no longer
/// the connection's (a disconnect, a reconnect, a removal); a successful re-dial's own connect
/// starts the next bridge's keepalive.
async fn keepalive(
    manager: Weak<CrewManager>,
    id: String,
    transport: Weak<Mutex<transport::Transport>>,
) {
    let mut since_tick = Duration::ZERO;
    loop {
        let Some(timing) = manager.upgrade().map(|manager| manager.keepalive_timing()) else {
            return;
        };
        let nap = timing.nap();
        tokio::time::sleep(nap).await;
        since_tick += nap;
        let (Some(manager), Some(transport)) = (manager.upgrade(), transport.upgrade()) else {
            return;
        };
        if !manager.is_current_transport(&id, &transport).await {
            return;
        }
        let tick = since_tick >= timing.tick;
        if tick {
            since_tick = Duration::ZERO;
        }
        // A bridge in use is neither ended nor idle; look again next time.
        let (ended, idle) = match transport.try_lock() {
            Ok(mut locked) => (locked.has_ended(), locked.idle_for()),
            Err(_) => continue,
        };
        if !ended && (!tick || idle < timing.idle) {
            continue;
        }
        if ended {
            tracing::info!(connection = %id, "Crew bridge ended; dialling again");
        } else {
            match manager.heartbeat(&id, &transport).await {
                Heartbeat::Alive => continue,
                // Not about the network: final, shown, and never dialled again.
                Heartbeat::Refused(error) => {
                    let _ = manager.retire_refused(&id, &transport, &error).await;
                    return;
                }
                Heartbeat::Gone(error) => {
                    tracing::info!(connection = %id, error = %error, "Crew bridge found gone while idle; dialling again");
                }
            }
        }
        manager.recover_dropped(&id, &transport).await;
        return;
    }
}
