//! BR-71 §4.3: the daemon→GUI workspace command channel. One bridge per GUI
//! WINDOW (Agent Drafter's `UiBridge` is per app session — same anatomy, one
//! level up): generation-guarded attach/detach, a pending map for blocking
//! round trips, cancel_all on disconnect, and the window's last layout echo.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct WorkspaceBridge {
    inner: Arc<BridgeInner>,
}

struct BridgeInner {
    /// The live connection **and the generation that owns it, under one lock**.
    ///
    /// Only the connection owning the current generation may tear it down
    /// (control.rs `UiBridge::detach` rationale). Keeping the generation beside
    /// the sender — rather than in a separate atomic, as `UiBridge` does — is
    /// what makes `detach`'s compare-and-clear a single critical section. With
    /// the two split, a `detach` that had passed the check could be descheduled
    /// and then null a *newer* connection installed in the meantime — severing
    /// a live window. The window is narrow (0/4000 unaided) but structural:
    /// widening it with a 5 ms sleep between the check and the lock reproduced
    /// it 200/200, and the fused form 0/200 under the same injection.
    conn: Mutex<Option<(u64, mpsc::UnboundedSender<Value>)>>,
    /// Mints generations. It never decides ownership — `conn` does.
    generation: AtomicU64,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    last_echo: Mutex<Option<Value>>,
    last_attach: Mutex<Option<Instant>>,
    request_seq: AtomicU64,
}

/// How a [`WorkspaceBridge::emit_and_wait_cancellable`] wait ended without an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserWait {
    /// The window has no live connection.
    NotAttached,
    /// The window disconnected or reloaded while the request was parked.
    Disconnected,
    /// No answer in time; `request_id` names the operation to abandon.
    Timeout { request_id: String },
    /// The caller cancelled; `request_id` names the operation to abandon.
    Cancelled { request_id: String },
}

/// Opaque proof of which connection generation a socket owns.
#[derive(Debug, Clone, Copy)]
pub struct ConnToken(u64);

impl Default for WorkspaceBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BridgeInner {
                conn: Mutex::new(None),
                generation: AtomicU64::new(0),
                pending: Mutex::new(HashMap::new()),
                last_echo: Mutex::new(None),
                last_attach: Mutex::new(None),
                request_seq: AtomicU64::new(1),
            }),
        }
    }

    /// Claim the bridge for a new connection.
    ///
    /// On a GUI reload this is the ONLY hook that runs for the outgoing
    /// connection: its `detach` arrives later with a stale token and the
    /// generation guard turns it into a no-op. So `attach` — not `detach` —
    /// owns unparking the old connection's requests, which are waiting on
    /// `request_id`s the fresh page has never seen. Mirrors `UiBridge::attach`.
    pub fn attach(&self) -> (mpsc::UnboundedReceiver<Value>, ConnToken) {
        // Unpark the outgoing connection's requests BEFORE the new sender is
        // installed, so this can never cancel something the fresh connection
        // parked (mirrors `UiBridge::attach`'s cancel-then-install order).
        self.cancel_all();
        *lock(&self.inner.last_attach) = Some(Instant::now());
        let (tx, rx) = mpsc::unbounded_channel();
        let mut conn = lock(&self.inner.conn);
        // Mint and install in ONE critical section. A concurrent `detach` then
        // sees either the old generation (and is its rightful owner) or the new
        // one (and is a no-op) — never a torn in-between state. It also keeps
        // installed generations monotonic when two sockets attach at once.
        let generation = self.inner.generation.fetch_add(1, Ordering::Relaxed) + 1;
        *conn = Some((generation, tx));
        drop(conn);
        (rx, ConnToken(generation))
    }

    /// No-op unless `token` owns the current connection, so a slow old socket
    /// unwinding cannot sever its replacement.
    pub fn detach(&self, token: ConnToken) {
        let mut conn = lock(&self.inner.conn);
        // Compare AND clear under the one lock. `matches!` ends the borrow of
        // `conn` before the assignment.
        if !matches!(conn.as_ref(), Some((generation, _)) if *generation == token.0) {
            return;
        }
        *conn = None;
        drop(conn);
        self.cancel_all();
    }

    pub fn is_attached(&self) -> bool {
        lock(&self.inner.conn).is_some()
    }

    /// Are these two handles the same GUI window?
    ///
    /// `WorkspaceBridge` is a cheap `Arc` handle that `bridge_for` /
    /// `focused_or_recent` / `bridge_for_session` hand out by cloning, so
    /// pointer identity of the inner state *is* window identity. Used by the
    /// `gui_command_near` fallback to tell "try another window" from "try the
    /// window that just failed again", which for a `wait_result` frame would
    /// mean waiting out a second full timeout on a window already known to be
    /// unresponsive.
    pub fn same_window(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn emit(&self, frame: Value) -> Result<(), String> {
        let guard = lock(&self.inner.conn);
        let (_, tx) = guard.as_ref().ok_or("no GUI window attached")?;
        tx.send(frame)
            .map_err(|_| "GUI window channel closed".to_string())
    }

    /// Emit with a minted `request_id` and park until the renderer's
    /// `workspace_result` resolves it (bounded).
    pub async fn emit_and_wait(
        &self,
        mut frame: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let request_id = format!(
            "wsreq-{}",
            self.inner.request_seq.fetch_add(1, Ordering::Relaxed)
        );
        frame["request_id"] = Value::String(request_id.clone());
        let (tx, rx) = oneshot::channel();
        lock(&self.inner.pending).insert(request_id.clone(), tx);
        if let Err(e) = self.emit(frame) {
            lock(&self.inner.pending).remove(&request_id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err("GUI window disconnected before replying".into()),
            Err(_) => {
                lock(&self.inner.pending).remove(&request_id);
                Err("timed out waiting for the GUI".into())
            }
        }
    }

    /// [`Self::emit_and_wait`] for a Biorouter Browser operation (BB-10): the wait also races
    /// `cancel`, and every way it can end is told apart, because the caller answers each one
    /// differently (a timed out or cancelled operation is abandoned on the window and never
    /// replayed).
    pub async fn emit_and_wait_cancellable(
        &self,
        mut frame: Value,
        timeout: Duration,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Value, BrowserWait> {
        let request_id = format!(
            "wsreq-{}",
            self.inner.request_seq.fetch_add(1, Ordering::Relaxed)
        );
        frame["request_id"] = Value::String(request_id.clone());
        let (tx, rx) = oneshot::channel();
        lock(&self.inner.pending).insert(request_id.clone(), tx);
        if self.emit(frame).is_err() {
            lock(&self.inner.pending).remove(&request_id);
            return Err(BrowserWait::NotAttached);
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                lock(&self.inner.pending).remove(&request_id);
                Err(BrowserWait::Cancelled { request_id })
            }
            result = tokio::time::timeout(timeout, rx) => match result {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) => Err(BrowserWait::Disconnected),
                Err(_) => {
                    lock(&self.inner.pending).remove(&request_id);
                    Err(BrowserWait::Timeout { request_id })
                }
            },
        }
    }

    pub fn resolve(&self, request_id: &str, value: Value) {
        if let Some(tx) = lock(&self.inner.pending).remove(request_id) {
            let _ = tx.send(value);
        }
    }

    /// Park a request under a caller-chosen id, so a test can exercise `resolve`
    /// without driving `emit_and_wait`.
    ///
    /// `routes::workspace`'s socket-dispatch test needs *a parked request whose
    /// id it knows*; `emit_and_wait` mints the id itself and then blocks on it,
    /// which would force that test into a `#[tokio::test]` with a spawned task
    /// holding the parked future — testing this module rather than the frame
    /// dispatch it is about.
    #[cfg(test)]
    pub(crate) fn insert_pending_for_test(&self, request_id: &str, tx: oneshot::Sender<Value>) {
        lock(&self.inner.pending).insert(request_id.to_string(), tx);
    }

    pub fn cancel_all(&self) {
        for (_, tx) in lock(&self.inner.pending).drain() {
            drop(tx); // receivers observe Err and unpark
        }
    }

    pub fn store_echo(&self, echo: Value) {
        *lock(&self.inner.last_echo) = Some(echo);
    }

    pub fn last_echo(&self) -> Option<Value> {
        lock(&self.inner.last_echo).clone()
    }

    fn last_attach(&self) -> Option<Instant> {
        *lock(&self.inner.last_attach)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Registry keyed by window_id; entries retained for the process's life,
/// mirroring `UI_BRIDGES` in `routes/apps.rs`.
static BRIDGES: LazyLock<Mutex<HashMap<String, WorkspaceBridge>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn bridge_for(window_id: &str) -> WorkspaceBridge {
    lock(&BRIDGES)
        .entry(window_id.to_string())
        // `or_default()` rather than `or_insert_with(WorkspaceBridge::new)`:
        // clippy's `unwrap_or_default` fires on the latter and CI runs
        // `clippy -D warnings`. `Default` delegates to `new`, so this is the
        // same constructor.
        .or_default()
        .clone()
}

pub fn any_attached() -> bool {
    lock(&BRIDGES).values().any(WorkspaceBridge::is_attached)
}

/// Multi-window aggregation (§4.3): commands target the focused window (per its
/// echo), else the most recently attached.
pub fn focused_or_recent() -> Option<WorkspaceBridge> {
    let map = lock(&BRIDGES);
    let attached: Vec<_> = map.values().filter(|b| b.is_attached()).cloned().collect();
    drop(map);
    pick_target(attached)
}

/// The window whose echo shows a tab for `session_id`, if one is attached.
///
/// ⚠ **Routing by "who holds the parent" beats routing by "who has focus"**
/// (issue #78). `focused_or_recent` answers a question nobody asked: with
/// several windows open it guesses, and before the renderer reported real focus
/// it could not even discriminate. A spawned tab belongs beside its parent, and
/// the parent's location is a fact already on the wire — the echo carries
/// `window_id` and every tab's `session_id`.
///
/// It is also far more robust to the 300 ms echo debounce than focus is: a
/// user's focus can move between the spawn and the frame, while a parent's tab
/// rarely changes windows.
///
/// `None` when the parent has no tab anywhere, which is a real case (a headless
/// spawn, or a parent tab closed mid-turn). Callers must fall back rather than
/// drop the frame: the fire-and-forget contract says a disconnecting window
/// must never break a spawn.
pub fn bridge_for_session(session_id: &str) -> Option<WorkspaceBridge> {
    let map = lock(&BRIDGES);
    let attached: Vec<_> = map.values().filter(|b| b.is_attached()).cloned().collect();
    drop(map);
    pick_by_session(attached, session_id)
}

/// The lookup RULE of [`bridge_for_session`], split from the registry walk for
/// the same reason [`pick_target`] is: so it can be tested against a supplied
/// candidate list rather than against whatever else the test binary has
/// attached.
pub(crate) fn pick_by_session(
    attached: Vec<WorkspaceBridge>,
    session_id: &str,
) -> Option<WorkspaceBridge> {
    attached.into_iter().find(|bridge| {
        bridge
            .last_echo()
            .and_then(|echo| echo.get("layout").cloned())
            .and_then(|layout| layout.as_array().cloned())
            .is_some_and(|groups| {
                groups.iter().any(|group| {
                    group
                        .get("tabs")
                        .and_then(|tabs| tabs.as_array())
                        .is_some_and(|tabs| {
                            tabs.iter().any(|tab| {
                                tab.get("session_id").and_then(|s| s.as_str()) == Some(session_id)
                            })
                        })
                })
            })
    })
}

/// The routing RULE of `focused_or_recent`, separated from its registry walk so
/// it can be tested against a supplied candidate list.
///
/// This split exists for one reason and it is not tidiness: `BRIDGES` is a
/// process-wide static, so a test that asserts *which* bridge `focused_or_recent`
/// returns is not containment-safe against the other tests in this binary — and a
/// test that only asserts `.is_some()` does not test the rule at all. Keep the
/// two halves apart; `focused_or_recent` must stay a one-liner over this.
pub(crate) fn pick_target(attached: Vec<WorkspaceBridge>) -> Option<WorkspaceBridge> {
    attached
        .iter()
        .find(|b| {
            b.last_echo()
                .and_then(|e| e.get("focused_session").cloned())
                .is_some_and(|f| !f.is_null())
        })
        .cloned()
        .or_else(|| {
            // Rank on `Option<Instant>` directly: `None` (a window registered by
            // `bridge_for` that has never opened a socket) sorts BELOW every real
            // timestamp. Defaulting it to `Instant::now()` instead would make a
            // window that has never connected outrank every window the user
            // actually has open — and would make the key non-deterministic, since
            // it advances each time it is read.
            attached.into_iter().max_by_key(|b| b.last_attach())
        })
}

/// All windows' last echoes, merged — what workspace_list renders as `gui`.
pub fn merged_layout() -> Option<serde_json::Value> {
    let echoes: Vec<Value> = lock(&BRIDGES)
        .values()
        .filter(|b| b.is_attached())
        .filter_map(|b| b.last_echo())
        .collect();
    if echoes.is_empty() {
        None
    } else {
        Some(Value::Array(echoes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stale_detach_cannot_tear_down_a_newer_connection() {
        let bridge = WorkspaceBridge::new();
        let (_rx1, token1) = bridge.attach();
        let (_rx2, _token2) = bridge.attach(); // reconnect claims a new generation
        bridge.detach(token1); // stale detach: must be a no-op
        assert!(bridge.is_attached());
    }

    #[test]
    fn emit_delivers_to_the_current_connection_and_fails_detached() {
        let bridge = WorkspaceBridge::new();
        assert!(bridge.emit(json!({"cmd": "open_tab"})).is_err());

        let (mut rx, token) = bridge.attach();
        bridge
            .emit(json!({"cmd": "open_tab", "session_id": "s1"}))
            .unwrap();
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame["cmd"], "open_tab");

        bridge.detach(token);
        assert!(bridge.emit(json!({"cmd": "notify"})).is_err());
    }

    #[tokio::test]
    async fn round_trip_resolves_and_detach_cancels_parked_requests() {
        let bridge = WorkspaceBridge::new();
        let (mut rx, token) = bridge.attach();

        let waiter = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .emit_and_wait(
                        json!({"cmd": "open_tab", "session_id": "s1"}),
                        std::time::Duration::from_secs(5),
                    )
                    .await
            })
        };
        // The socket loop would read the frame, act, and reply by request_id.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if let Ok(f) = rx.try_recv() {
                    break f;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let request_id = frame["request_id"].as_str().unwrap().to_string();
        bridge.resolve(&request_id, json!({"ok": true}));
        assert_eq!(waiter.await.unwrap().unwrap()["ok"], true);

        // A parked request must not hang forever on disconnect.
        //
        // The timeout here is deliberately far longer than the test can run:
        // with the 5s the plan originally specified, `emit_and_wait`'s OWN
        // timeout returns `Err` and satisfies a bare `is_err()`, so the
        // assertion held even with `cancel_all` deleted from `detach` — the
        // exact regression this test exists to catch. Two things make it
        // discriminating: an unreachable timeout, and asserting on WHICH error.
        let waiter2 = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .emit_and_wait(
                        json!({"cmd": "open_tab"}),
                        std::time::Duration::from_secs(600),
                    )
                    .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        bridge.detach(token); // cancel_all unparks
        let err = tokio::time::timeout(std::time::Duration::from_secs(5), waiter2)
            .await
            .expect("detach must unpark the parked request, not leave it to time out")
            .unwrap()
            .unwrap_err();
        assert!(
            err.contains("disconnected"),
            "must be the disconnect error, not a timeout: {err}"
        );
    }

    /// The GUI reload path, which `detach` alone cannot cover.
    ///
    /// When a window reloads, the new socket's `attach` lands first and the old
    /// socket's `detach` arrives afterwards carrying a stale token — so the
    /// generation guard correctly makes it a no-op, and `cancel_all` never runs
    /// for the old generation at all. Anything parked on the old connection is
    /// then waiting on a `request_id` the fresh browser has never seen and can
    /// never reply to, so it holds the agent's turn until the full timeout.
    /// `attach` must unpark it, exactly as `UiBridge::attach` does.
    #[tokio::test]
    async fn reattach_unparks_the_previous_connections_requests() {
        let bridge = WorkspaceBridge::new();
        let (mut rx, _stale_token) = bridge.attach();

        let waiter = {
            let bridge = bridge.clone();
            tokio::spawn(async move {
                bridge
                    .emit_and_wait(json!({"cmd": "open_tab"}), Duration::from_secs(600))
                    .await
            })
        };
        // Wait until the frame is actually on the wire, so the request is
        // genuinely parked rather than merely spawned.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if rx.try_recv().is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();

        let (_rx2, _fresh_token) = bridge.attach(); // the reload

        let err = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("a reload must unpark the old connection's parked requests")
            .unwrap()
            .unwrap_err();
        assert!(
            err.contains("disconnected"),
            "must be the disconnect error, not a timeout: {err}"
        );
    }

    #[test]
    fn registry_tracks_focus_and_merges_layouts() {
        // BRIDGES is a process-wide static shared with every other test in this
        // binary (now and later). Use unique window ids and CONTAINMENT
        // assertions — never exact global counts — so parallel `cargo test`
        // and future bridge-attaching tests cannot flake this one.
        // (uuid is not a biorouter-server dep — a nanos timestamp is unique
        // enough for two ids minted in one test.)
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let win_a = format!("win-a-{nonce}");
        let win_b = format!("win-b-{nonce}");
        let a = bridge_for(&win_a);
        let b = bridge_for(&win_b);
        let (_ra, ta) = a.attach();
        let (_rb, tb) = b.attach();
        a.store_echo(json!({"window_id": win_a, "focused_session": "s1", "layout": []}));
        b.store_echo(json!({"window_id": win_b, "focused_session": null, "layout": []}));
        assert!(any_attached());
        let merged = merged_layout().unwrap();
        let ids: Vec<String> = merged
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| {
                e.get("window_id")
                    .and_then(|w| w.as_str())
                    .map(String::from)
            })
            .collect();
        assert!(ids.contains(&win_a), "merged layout carries window {win_a}");
        assert!(ids.contains(&win_b), "merged layout carries window {win_b}");

        // A CLOSED window must drop out of the merged layout: `workspace_list`'s
        // `gui` block is a claim about what the user can see right now, and a
        // stale echo from a window that is gone is a lie the model will act on.
        // (`last_echo` is retained deliberately — the registry never forgets a
        // window — so only the `is_attached()` filter separates the two cases,
        // and nothing else in this suite exercises it.)
        b.detach(tb);
        let merged_after = merged_layout().unwrap();
        let ids_after: Vec<String> = merged_after
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| {
                e.get("window_id")
                    .and_then(|w| w.as_str())
                    .map(String::from)
            })
            .collect();
        assert!(ids_after.contains(&win_a), "the live window is still there");
        assert!(
            !ids_after.contains(&win_b),
            "a detached window's stale echo must not be reported as GUI state: {ids_after:?}"
        );

        // Clean up so this test leaves no attached bridges for others to see.
        a.detach(ta);
    }

    /// §4.3's routing rule, tested as a RULE: *commands target the focused
    /// window (per its echo), else the most recently attached.*
    ///
    /// ⚠ This replaces `assert!(focused_or_recent().is_some())`, which was true
    /// for **every** implementation that returned any attached bridge — including
    /// one that ignores `focused_session` entirely, which in a two-window session
    /// sends every `workspace_open` to the wrong window. Do not simplify it back:
    /// `is_some()` on a function whose whole job is *which one* asserts nothing
    /// about the choice.
    ///
    /// It runs against `pick_target`, a locally-supplied candidate list, rather
    /// than `focused_or_recent()`'s global `BRIDGES` walk, for the reason the
    /// test above documents: another test in this binary may hold an attached,
    /// focused bridge, and a global assertion about *which* bridge wins is not
    /// containment-safe. `focused_or_recent()` is `pick_target(attached())` and
    /// nothing else, so the rule is fully covered here.
    #[test]
    fn the_command_target_prefers_a_focused_window_over_a_more_recent_one() {
        let focused = WorkspaceBridge::new();
        let (_r1, _t1) = focused.attach();
        focused.store_echo(json!({"window_id": "w-focused", "focused_session": "s1"}));
        // `last_attach` is an Instant; sleep so the ordering is a fact, not a
        // tie broken by clock resolution.
        std::thread::sleep(Duration::from_millis(5));
        let recent = WorkspaceBridge::new();
        let (_r2, _t2) = recent.attach(); // attaches LATER than the focused one
        recent.store_echo(json!({"window_id": "w-recent", "focused_session": null}));

        // Rule 1 — focus beats recency. The candidate order is deliberately
        // "recent first": a `pick_target` that returns `candidates[0]` fails here.
        let picked = pick_target(vec![recent.clone(), focused.clone()]).expect("a target");
        assert_eq!(
            picked.last_echo().unwrap()["window_id"],
            "w-focused",
            "the window whose echo names a focused session wins"
        );

        // Rule 2 — with nobody focused, the most recently attached wins. SAME
        // candidate order, so a `pick_target` that returns `candidates.last()`
        // (which would have passed Rule 1) fails here.
        focused.store_echo(json!({"window_id": "w-focused", "focused_session": null}));
        let picked = pick_target(vec![recent.clone(), focused.clone()]).expect("a target");
        assert_eq!(
            picked.last_echo().unwrap()["window_id"],
            "w-recent",
            "with no focus anywhere, the newest connection is the target"
        );

        // Rule 3 — no candidates is None, not a panic: that is the headless
        // degradation path every workspace tool branches on.
        assert!(pick_target(vec![]).is_none());
    }

    /// ⚠ **The case the fixture above never produced, and the one that was
    /// actually broken** (issue #78).
    ///
    /// That test gives one window a focused session and the other `null`. The
    /// renderer never produced that state: it filled `focused_session` from the
    /// window's ACTIVE TAB with no OS-focus input at all, so every window
    /// holding a chat reported non-null, permanently. Rule 1 was therefore
    /// being asked a question that never arose, while the question that did
    /// arise — two windows both claiming focus — fell through to
    /// `HashMap::values()` order and picked arbitrarily.
    ///
    /// The renderer now reports focus for real, so at most one window claims it
    /// and this asserts the tie is gone. It is pinned here rather than left to
    /// the renderer alone because `pick_target` is where the consequence lands:
    /// if the echo semantics ever regress, this is the test that says which
    /// half broke.
    /// Routing by the parent's location, which is what #78 actually needs.
    ///
    /// Deliberately set up so a focus-based answer would be WRONG: the window
    /// holding the parent is not the focused one, and it attached first. Both
    /// of `pick_target`'s rules would pick the other window, so this passes
    /// only if the lookup really is reading the layout.
    #[test]
    fn the_window_holding_the_parent_is_found_regardless_of_focus_or_recency() {
        let holder = WorkspaceBridge::new();
        let (_r1, _t1) = holder.attach();
        holder.store_echo(json!({
            "window_id": "w-holder",
            "focused_session": null,
            "layout": [{ "tabs": [{ "session_id": "parent-1" }, { "session_id": "other" }] }],
        }));
        std::thread::sleep(Duration::from_millis(5));
        let decoy = WorkspaceBridge::new();
        let (_r2, _t2) = decoy.attach(); // focused AND more recent
        decoy.store_echo(json!({
            "window_id": "w-decoy",
            "focused_session": "s9",
            "layout": [{ "tabs": [{ "session_id": "s9" }] }],
        }));

        let picked = pick_by_session(vec![decoy.clone(), holder.clone()], "parent-1")
            .expect("the parent's window");
        assert_eq!(
            picked.last_echo().unwrap()["window_id"],
            "w-holder",
            "the tab belongs beside its parent, not beside the focused window"
        );

        // A parent with no tab anywhere is a real case (headless spawn, or the
        // tab closed mid-turn). It must answer None so the caller can fall back
        // rather than drop the frame.
        assert!(pick_by_session(vec![decoy, holder], "nowhere").is_none());
    }

    #[test]
    fn two_windows_both_claiming_focus_is_the_state_that_must_not_arise() {
        let a = WorkspaceBridge::new();
        let (_r1, _t1) = a.attach();
        a.store_echo(json!({"window_id": "w-a", "focused_session": "s1"}));
        std::thread::sleep(Duration::from_millis(5));
        let b = WorkspaceBridge::new();
        let (_r2, _t2) = b.attach();
        b.store_echo(json!({"window_id": "w-b", "focused_session": "s2"}));

        // Both claim focus. `pick_target` can only return SOMETHING, and which
        // one is genuinely arbitrary — that is the defect, and it is why the
        // fix has to be upstream in what the renderer reports rather than in a
        // cleverer tie-break here.
        let picked = pick_target(vec![a.clone(), b.clone()]).expect("a target");
        let id = picked.last_echo().unwrap()["window_id"].clone();
        assert!(id == "w-a" || id == "w-b", "got {id}");

        // With the renderer honest, the blurred window reports null and the
        // focused one wins unambiguously, regardless of candidate order or
        // which attached more recently.
        a.store_echo(json!({"window_id": "w-a", "focused_session": null}));
        let picked = pick_target(vec![a.clone(), b.clone()]).expect("a target");
        assert_eq!(
            picked.last_echo().unwrap()["window_id"],
            "w-b",
            "exactly one window claims focus, so it is the target"
        );
        let picked = pick_target(vec![b.clone(), a.clone()]).expect("a target");
        assert_eq!(
            picked.last_echo().unwrap()["window_id"],
            "w-b",
            "and the answer does not depend on candidate order"
        );
    }

    /// A registered-but-never-connected window must never win the recency
    /// fallback.
    ///
    /// `bridge_for` mints an entry on first sight, so a window that has been
    /// named but has never opened a socket sits in `BRIDGES` with
    /// `last_attach == None`. Ranking that as "now" would make it beat every
    /// window the user actually has open, and would make the ranking key
    /// non-deterministic (it moves every time it is read). `focused_or_recent`
    /// pre-filters on `is_attached`, but `pick_target` is called directly with
    /// a supplied candidate list, so the rule has to hold here too.
    #[test]
    fn a_window_that_never_attached_never_wins_on_recency() {
        let live = WorkspaceBridge::new();
        let (_r, _t) = live.attach();
        live.store_echo(json!({"window_id": "w-live", "focused_session": null}));

        let never = WorkspaceBridge::new();
        never.store_echo(json!({"window_id": "w-never", "focused_session": null}));

        let picked = pick_target(vec![never.clone(), live.clone()]).expect("a target");
        assert_eq!(
            picked.last_echo().unwrap()["window_id"],
            "w-live",
            "a window that never attached must sort oldest, not newest"
        );
    }
}
