//! #549: a headless run whose session store runs out of space keeps going.
//!
//! Every turn-path store write propagates with `?`, so one `SQLITE_FULL` used
//! to end `biorouter run` with exit 78 (`session_store_failure`) and no answer.
//! In BioRouterBench it was the per-request usage write that failed, after the
//! model's own files filled the volume the store shared with them.
//!
//! **Opt-in, per chat, headless only.** Nothing changes for a chat that did not
//! ask: [`SessionManager::defer_writes_when_full`] registers one chat of one
//! store, and only `biorouter run` without `--interactive` calls it, for its own
//! chat. The desktop, the server, interactive sessions, `biorouter bench` and
//! every subagent (a chat of its own) keep the strict store, so a stored row
//! still means a durable row wherever a client relies on stored ids.
//!
//! **What waits.** For a registered chat, a message write
//! ([`SessionManager::add_message`], [`SessionManager::add_message_adopting_uid`])
//! or a usage-ledger write ([`SessionManager::apply_usage_event`]) that fails
//! because the store is out of space
//! ([`crate::agents::turn_abort::is_storage_exhausted`]) is kept in memory and
//! reported as written. Every other error stays an error, so a constraint
//! violation, a closed pool or a schema fault still ends the turn with exit 78.
//!
//! **Order.** The waiting rows are a FIFO queue. While anything waits, every
//! later write of that chat first tries to save the queue, front first, and
//! joins its back if the queue is still not empty, so a later row is never
//! stored ahead of an earlier one: a `--resume` never finds a tool request
//! without its response. A message waits with a stable uid (one is minted for
//! a message without one) and a usage row with its event key, so a retry is an
//! exact replay that the store recognizes instead of a duplicate.
//!
//! **Reads see the waiting rows.** [`SessionManager::get_session`] and
//! [`SessionManager::snapshot_for_rewrite`] append them to the stored
//! conversation, so the opening prompt reaches the model even when its write
//! waited; `total_tokens` is the newest waiting request's gauge. A
//! whole-history rewrite (compaction, truncation) first saves the queue and is
//! refused, with the out-of-space error, while rows still wait.
//!
//! **Bounds and spend.** At most [`MAX_PENDING_ROWS`] rows and
//! [`MAX_PENDING_BYTES`] bytes wait; past either, the write fails as it always
//! did. A usage row waits only when no usage limit applies to the chat: an
//! unrecorded request would hide spend from a per-chat or monthly limit, so
//! with a limit in force the run stops (exit 78) instead of spending past it.
//! The per-reply budget is kept in memory and is unaffected.
//!
//! What a waiting row is NOT given is a `MessagesPersisted` frame: the agent
//! names only rows the store holds (`persisted_event`, [`is_pending`]).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Result;

use super::{SessionManager, SessionStorage, UsageLedgerEntry};
use crate::agents::turn_abort::is_storage_exhausted;
use crate::conversation::message::{new_message_id, Message};
use crate::conversation::Conversation;

/// The most rows that may wait for space before a write fails as before.
pub const MAX_PENDING_ROWS: usize = 4096;

/// The most serialized bytes that may wait for space before a write fails as
/// before.
pub const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;

/// One row waiting for space.
#[derive(Clone)]
enum Pending {
    Message(Message),
    Usage(UsageLedgerEntry),
}

#[derive(Default)]
struct Queue {
    rows: VecDeque<(Pending, usize)>,
    bytes: usize,
}

/// The waiting rows of one registered chat.
pub(super) struct DeferredWrites {
    session_id: String,
    /// The rows, oldest first. A short critical section only, never held
    /// across an await, so a read can overlay them while a write is saving.
    queue: Mutex<Queue>,
    /// Serializes this chat's writes, so a queue that is being saved cannot
    /// be passed by a write that started later.
    writes: tokio::sync::Mutex<()>,
    degraded: AtomicBool,
    warned: AtomicBool,
    deferred_rows: AtomicU64,
    max_rows: usize,
    max_bytes: usize,
}

/// What a registered chat's store went through, for the run's final output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeferredWritesStatus {
    /// The store ran out of space at least once during the run.
    pub degraded: bool,
    /// Rows still waiting for space.
    pub unsaved_rows: usize,
    /// Rows that had to wait, saved since or not.
    pub deferred_rows: u64,
}

type Key = (PathBuf, String);

static REGISTRY: LazyLock<Mutex<HashMap<Key, Arc<DeferredWrites>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
/// How many chats are registered, so an unregistered process pays one atomic
/// load per write.
static REGISTERED: AtomicUsize = AtomicUsize::new(0);

fn registry() -> std::sync::MutexGuard<'static, HashMap<Key, Arc<DeferredWrites>>> {
    REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The deferral registered for `session_id` of the store under `session_dir`.
pub(super) fn lookup(session_dir: &Path, session_id: &str) -> Option<Arc<DeferredWrites>> {
    if REGISTERED.load(Ordering::Acquire) == 0 {
        return None;
    }
    registry()
        .get(&(session_dir.to_path_buf(), session_id.to_string()))
        .cloned()
}

/// Whether `message` is a row a registered chat holds in memory, not in its
/// store. Such a row must not be named in a `MessagesPersisted` frame.
pub fn is_pending(message: &Message) -> bool {
    if REGISTERED.load(Ordering::Acquire) == 0 {
        return false;
    }
    let Some(id) = message.id.as_deref() else {
        return false;
    };
    let writes: Vec<Arc<DeferredWrites>> = registry().values().cloned().collect();
    writes.iter().any(|writes| writes.holds_message(id))
}

/// Register `session_id` of `storage` ([`SessionManager::defer_writes_when_full`]).
pub(super) fn register(storage: &SessionStorage, session_id: &str) -> DeferredWritesGuard {
    register_bounded(storage, session_id, MAX_PENDING_ROWS, MAX_PENDING_BYTES)
}

/// [`register`] with its own bounds, so a test can reach them.
pub(super) fn register_bounded(
    storage: &SessionStorage,
    session_id: &str,
    max_rows: usize,
    max_bytes: usize,
) -> DeferredWritesGuard {
    let key = (storage.session_dir.clone(), session_id.to_string());
    let writes = Arc::new(DeferredWrites {
        session_id: session_id.to_string(),
        queue: Mutex::new(Queue::default()),
        writes: tokio::sync::Mutex::new(()),
        degraded: AtomicBool::new(false),
        warned: AtomicBool::new(false),
        deferred_rows: AtomicU64::new(0),
        max_rows,
        max_bytes,
    });
    if registry()
        .insert(key.clone(), Arc::clone(&writes))
        .is_none()
    {
        REGISTERED.fetch_add(1, Ordering::AcqRel);
    }
    DeferredWritesGuard { key, writes }
}

/// A chat's registration ([`SessionManager::defer_writes_when_full`]). The chat
/// goes back to the strict store when it drops; rows still waiting then are
/// dropped with it, so read [`Self::status`] after the last [`Self::flush`].
pub struct DeferredWritesGuard {
    key: Key,
    writes: Arc<DeferredWrites>,
}

impl DeferredWritesGuard {
    /// What the chat's store went through so far.
    pub fn status(&self) -> DeferredWritesStatus {
        DeferredWritesStatus {
            degraded: self.writes.degraded.load(Ordering::Acquire),
            unsaved_rows: self.writes.lock_queue().rows.len(),
            deferred_rows: self.writes.deferred_rows.load(Ordering::Acquire),
        }
    }

    /// `true` once, the first time it is asked after the store ran out of
    /// space: when the run prints its warning.
    pub fn take_warning(&self) -> bool {
        self.writes.degraded.load(Ordering::Acquire)
            && !self.writes.warned.swap(true, Ordering::AcqRel)
    }

    /// One more attempt to save the waiting rows, in order. `Ok` whether or
    /// not space came back ([`Self::status`] says); an error other than "out
    /// of space" is returned.
    pub async fn flush(&self, manager: &SessionManager) -> Result<()> {
        let _serial = self.writes.writes.lock().await;
        self.writes.save_queue(&manager.storage).await
    }
}

impl Drop for DeferredWritesGuard {
    fn drop(&mut self) {
        let mut registry = registry();
        if registry
            .get(&self.key)
            .is_some_and(|registered| Arc::ptr_eq(registered, &self.writes))
        {
            registry.remove(&self.key);
            REGISTERED.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// The error a write gets when no more rows may wait: the same class as the
/// store's own "out of space", so the turn ends as it always did (exit 78).
fn bound_reached(rows: usize, bytes: usize) -> anyhow::Error {
    anyhow::Error::new(sqlx::Error::Io(std::io::Error::new(
        std::io::ErrorKind::StorageFull,
        format!(
            "the session store is out of space and {rows} rows ({bytes} bytes) already wait \
             for it, the most a run holds in memory"
        ),
    )))
}

impl DeferredWrites {
    fn lock_queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn has_pending(&self) -> bool {
        !self.lock_queue().rows.is_empty()
    }

    fn holds_message(&self, id: &str) -> bool {
        self.lock_queue()
            .rows
            .iter()
            .any(|(row, _)| matches!(row, Pending::Message(message) if message.id.as_deref() == Some(id)))
    }

    /// Save the waiting rows, oldest first, until the queue is empty or the
    /// store is still out of space. The caller holds `self.writes`.
    async fn save_queue(&self, storage: &SessionStorage) -> Result<()> {
        loop {
            let front = self.lock_queue().rows.front().map(|(row, _)| row.clone());
            let Some(front) = front else {
                return Ok(());
            };
            let saved = match &front {
                Pending::Message(message) => storage
                    .add_message(&self.session_id, message)
                    .await
                    .map(|_| ()),
                Pending::Usage(entry) => storage.apply_usage_event(entry.clone()).await.map(|_| ()),
            };
            match saved {
                Ok(()) => {
                    let mut queue = self.lock_queue();
                    if let Some((_, bytes)) = queue.rows.pop_front() {
                        queue.bytes = queue.bytes.saturating_sub(bytes);
                    }
                }
                Err(error) if is_storage_exhausted(&error) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }

    /// Put `row` at the back of the queue, or fail with `error` when it would
    /// pass a bound.
    fn wait(&self, row: Pending, error: anyhow::Error) -> Result<()> {
        let bytes = match &row {
            Pending::Message(message) => serde_json::to_vec(message).map_or(0, |json| json.len()),
            Pending::Usage(_) => std::mem::size_of::<UsageLedgerEntry>(),
        };
        let mut queue = self.lock_queue();
        if queue.rows.len() >= self.max_rows || queue.bytes.saturating_add(bytes) > self.max_bytes {
            tracing::warn!(
                session_id = %self.session_id,
                rows = queue.rows.len(),
                bytes = queue.bytes,
                "session store still out of space and the in-memory queue is full; failing the write"
            );
            return Err(error);
        }
        queue.rows.push_back((row, bytes));
        queue.bytes = queue.bytes.saturating_add(bytes);
        drop(queue);
        self.deferred_rows.fetch_add(1, Ordering::AcqRel);
        if !self.degraded.swap(true, Ordering::AcqRel) {
            tracing::warn!(
                session_id = %self.session_id,
                "session store out of space; this headless run keeps its unsaved rows in memory: {error:#}"
            );
        }
        Ok(())
    }

    /// A message write for this chat ([`SessionManager::add_message`]).
    pub(super) async fn add_message(
        &self,
        storage: &SessionStorage,
        message: &Message,
    ) -> Result<String> {
        let _serial = self.writes.lock().await;
        if self.has_pending() {
            self.save_queue(storage).await?;
        }
        let error = if self.has_pending() {
            let queue = self.lock_queue();
            bound_reached(queue.rows.len(), queue.bytes)
        } else {
            match storage.add_message(&self.session_id, message).await {
                Ok(uid) => return Ok(uid),
                Err(error) if is_storage_exhausted(&error) => error,
                Err(error) => return Err(error),
            }
        };
        // Nothing was stored, so the uid can be chosen now; it stays the row's
        // uid when the row is saved later.
        let mut waiting = message.clone();
        let uid = waiting.id.clone().unwrap_or_else(new_message_id);
        waiting.id = Some(uid.clone());
        self.wait(Pending::Message(waiting), error)?;
        Ok(uid)
    }

    /// A usage-ledger write for this chat ([`SessionManager::apply_usage_event`]).
    pub(super) async fn apply_usage(
        &self,
        manager: &SessionManager,
        entry: UsageLedgerEntry,
    ) -> Result<bool> {
        let _serial = self.writes.lock().await;
        let storage = &manager.storage;
        if self.has_pending() {
            self.save_queue(storage).await?;
        }
        let error = if self.has_pending() {
            let queue = self.lock_queue();
            bound_reached(queue.rows.len(), queue.bytes)
        } else {
            match storage.apply_usage_event(entry.clone()).await {
                Ok(applied) => return Ok(applied),
                Err(error) if is_storage_exhausted(&error) => error,
                Err(error) => return Err(error),
            }
        };
        // Spend before survival: unrecorded usage would hide this request from
        // a per-chat or monthly limit, so it waits only where none applies. A
        // check that cannot answer counts as a limit.
        let unlimited = matches!(
            crate::agents::usage_limit::check(manager, &self.session_id).await,
            Ok(check) if check.effective.is_unlimited()
        );
        if !unlimited {
            return Err(error);
        }
        self.wait(Pending::Usage(entry), error)?;
        Ok(true)
    }

    /// Before a whole-history rewrite: save the waiting rows, and refuse the
    /// rewrite while any still wait, since it would replace a history they
    /// are not part of yet.
    pub(super) async fn settle_before_rewrite(&self, storage: &SessionStorage) -> Result<()> {
        let _serial = self.writes.lock().await;
        self.save_queue(storage).await?;
        let queue = self.lock_queue();
        if queue.rows.is_empty() {
            Ok(())
        } else {
            Err(bound_reached(queue.rows.len(), queue.bytes))
        }
    }

    /// The stored view of the chat with the waiting rows on the end, as the
    /// run sees it in memory: messages after the stored conversation, and the
    /// newest waiting request's context gauge.
    pub(super) fn overlay(&self, session: &mut super::Session) {
        let queue = self.lock_queue();
        if queue.rows.is_empty() {
            return;
        }
        let mut waiting = Vec::new();
        for (row, _) in &queue.rows {
            match row {
                Pending::Message(message) => waiting.push(message.clone()),
                Pending::Usage(entry) => {
                    if entry.current_total_tokens.is_some() {
                        session.total_tokens = entry.current_total_tokens;
                        session.input_tokens = entry.current_input_tokens;
                        session.output_tokens = entry.current_output_tokens;
                    }
                }
            }
        }
        drop(queue);
        if waiting.is_empty() {
            return;
        }
        session.message_count += waiting.len();
        if let Some(conversation) = session.conversation.take() {
            let mut messages = conversation.into_messages();
            messages.extend(waiting);
            session.conversation = Some(Conversation::new_unvalidated(messages));
        }
    }
}
