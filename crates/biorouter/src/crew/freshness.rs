//! Every process asks the saved Crew registry as it is now (CROSSCUT-1), and a registry it
//! cannot read restricts the chats it may name, not every chat (DAEMON-6).
//!
//! ⚠ **Grant state; a change here needs human review.** Each process that opens a profile (the
//! desktop's daemon, an in-process `biorouter session`, `biorouter acp`, a `biorouter serve`
//! daemon) loads `connections.json` into its own [`CrewManager`] once, and only the daemon makes
//! grants. Every scope question used to go to that first copy, so a process started before a
//! grant never learned of it: its `workspace_list` listed the Crew chat, its
//! `workspace_read_conversation` read the chat's channel messages into an unscoped chat, and
//! chat recall searched them. Now each scope question first checks whether the file changed
//! since this process last read or wrote it, and reads it again when it did.
//!
//! The file is replaced whole by an atomic rename on every save, so a read never sees half of
//! one; a change is noticed by the file's length, modification time and inode, and confirmed by
//! its digest, the same digest [`CrewManager::try_update_registry`] compares.
//!
//! A registry that does not parse (a newer build's value this build does not know, a hand edit,
//! an agent's overwrite) used to fail [`super::manager`] itself, and with it every chat's model
//! and tool calls, Crew or not. Now the process keeps working and remembers what it could not
//! read ([`Unreadable`]): each chat the file names, and each chat this process holds a grant for,
//! is restricted and authorizes nothing ([`super::Standing::Unreadable`]); a file that names no
//! chat it can read restricts every chat, because none can be told apart. Nothing is written
//! while the file cannot be read, so it is never overwritten with less than it holds.

use super::{carry_process_state, registry_digest, CrewManager, Registry};
use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

/// What every chat the unreadable registry may restrict is told.
pub(super) const REGISTRY_UNREADABLE: &str = "Crew's saved settings on this computer \
(connections.json) can't be read, so this chat's Crew access can't be confirmed. Update \
Biorouter to the version that saved them, or restore the file from a backup.";

/// When the saved registry's file last changed, as far as the file system says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    inode: u64,
}

/// `path`'s stamp, or `None` when there is no file.
pub(super) fn file_stamp(path: &Path) -> std::io::Result<Option<FileStamp>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(FileStamp {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            inode: inode(&metadata),
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn inode(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino()
}

#[cfg(not(unix))]
fn inode(_metadata: &std::fs::Metadata) -> u64 {
    0
}

/// A saved registry this process could not read.
pub(super) struct Unreadable {
    /// The chats the file names as holding a grant, stopped or not; `None` when it names none
    /// this build can read, and so may restrict any chat.
    pub(super) sessions: Option<HashSet<String>>,
}

impl Unreadable {
    /// What `bytes` name, when they cannot be read as a registry.
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self {
            sessions: named_sessions(bytes),
        }
    }
}

/// The session ids an unreadable registry names: the keys of its `scopes`, and each earlier
/// grant's `session_id`. `None` unless every one of them can be read, since a chat it names
/// but this build cannot read must never go unrestricted.
fn named_sessions(bytes: &[u8]) -> Option<HashSet<String>> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let registry = value.as_object()?;
    let mut sessions: HashSet<String> = registry
        .get("scopes")?
        .as_object()?
        .keys()
        .cloned()
        .collect();
    if let Some(replaced) = registry.get("replaced") {
        for kept in replaced.as_array()? {
            sessions.insert(kept.get("session_id")?.as_str()?.to_owned());
        }
    }
    Some(sessions)
}

/// The registry in `path` as a new process loads it: the registry (empty when the file is
/// missing or cannot be read), the digest to compare later saves with (`None` for no file,
/// and for one that cannot be read, so no save ever takes it for this copy), the file's stamp
/// taken before it was read, and what could not be read.
pub(super) fn load(
    path: &Path,
) -> std::io::Result<(
    Registry,
    Option<[u8; 32]>,
    Option<FileStamp>,
    Option<Unreadable>,
)> {
    // Taken first: a file replaced after it is then read again, never missed.
    let stamp = file_stamp(path)?;
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Registry>(&bytes) {
            Ok(registry) => Ok((registry, Some(registry_digest(&bytes)), stamp, None)),
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    %error,
                    "Crew's saved settings can't be read; the chats they may restrict stay \
                     restricted, and nothing is saved over them"
                );
                Ok((
                    Registry::default(),
                    None,
                    stamp,
                    Some(Unreadable::of(&bytes)),
                ))
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((Registry::default(), None, None, None))
        }
        Err(error) => Err(error),
    }
}

impl CrewManager {
    fn seen_file(&self) -> Option<Option<FileStamp>> {
        *self
            .seen_file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_seen_file(&self, stamp: Option<FileStamp>) {
        *self
            .seen_file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(stamp);
    }

    fn set_unreadable(&self, unreadable: Option<Unreadable>) {
        *self
            .unreadable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = unreadable;
    }

    fn is_unreadable(&self) -> bool {
        self.unreadable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Forget that the saved registry could not be read: a save just read it back whole.
    pub(super) fn registry_readable_again(&self) {
        self.set_unreadable(None);
    }

    /// Read the saved registry again when its file changed since this process last looked,
    /// and make it this process's copy with this process's own state carried in
    /// ([`carry_process_state`]), exactly as a save reads it. A file that is gone is no news:
    /// this process keeps what it holds, and its next save decides, as it always did. A file
    /// that cannot be read leaves this copy as it was and restricts what it may name.
    pub(super) async fn refresh_registry(&self) {
        let path = self.root.join("connections.json");
        let stamp = match file_stamp(&path) {
            Ok(stamp) => stamp,
            Err(error) => {
                tracing::warn!(%error, "couldn't look at Crew's saved settings; using this process's copy");
                return;
            }
        };
        if self.seen_file() == Some(stamp) {
            return;
        }
        let mut registry = self.registry.lock().await;
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.set_seen_file(stamp);
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "couldn't read Crew's saved settings; using this process's copy");
                return;
            }
        };
        let digest = registry_digest(&bytes);
        if Some(digest) == self.saved_digest() && !self.is_unreadable() {
            self.set_seen_file(stamp);
            return;
        }
        match serde_json::from_slice::<Registry>(&bytes) {
            Ok(mut theirs) => {
                carry_process_state(&registry, &mut theirs);
                *registry = theirs;
                self.set_saved_digest(Some(digest));
                self.set_unreadable(None);
            }
            Err(error) => {
                if !self.is_unreadable() {
                    tracing::error!(
                        %error,
                        "Crew's saved settings can't be read; the chats they may restrict stay \
                         restricted, and nothing is saved over them"
                    );
                }
                // No save may take the file for this copy while it cannot be read.
                self.set_saved_digest(None);
                self.set_unreadable(Some(Unreadable::of(&bytes)));
            }
        }
        drop(registry);
        self.set_seen_file(stamp);
    }

    /// Why `session` is restricted while the saved registry cannot be read, or `None` when it
    /// can be, or it names no grant of this chat and this process holds none.
    pub(super) async fn unreadable_restriction(&self, session: &str) -> Option<&'static str> {
        let named = {
            let unreadable = self
                .unreadable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let unreadable = unreadable.as_ref()?;
            unreadable
                .sessions
                .as_ref()
                .is_none_or(|sessions| sessions.contains(session))
        };
        if named {
            return Some(REGISTRY_UNREADABLE);
        }
        let registry = self.registry.lock().await;
        (registry.scopes.contains_key(session)
            || registry
                .replaced
                .iter()
                .any(|kept| kept.session_id == session))
        .then_some(REGISTRY_UNREADABLE)
    }

    /// The chats an unreadable registry names, when it cannot be read: `Some(None)` when it
    /// names none this build can read and so may restrict any chat.
    pub(super) fn unreadable_sessions(&self) -> Option<Option<HashSet<String>>> {
        self.unreadable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|unreadable| unreadable.sessions.clone())
    }

    /// Why saving is refused while the saved registry cannot be read, for the person.
    pub(super) fn unreadable_save_error(error: &serde_json::Error) -> anyhow::Error {
        anyhow::anyhow!(
            "Crew's saved settings on this computer (connections.json) can't be read, so \
             nothing was changed: {error}. Update Biorouter to the version that saved them, \
             or restore the file from a backup."
        )
    }

    /// Every chat saved in the store grants name, for a registry that may restrict any chat.
    pub(super) async fn every_chat(&self) -> Vec<String> {
        #[cfg(test)]
        if let Some(store) = self.test_session_store() {
            return store
                .list_sessions()
                .await
                .map(|sessions| sessions.into_iter().map(|session| session.id).collect())
                .unwrap_or_default();
        }
        crate::session::SessionManager::instance()
            .list_sessions()
            .await
            .map(|sessions| sessions.into_iter().map(|session| session.id).collect())
            .unwrap_or_default()
    }
}
