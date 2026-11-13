//! **Find words in my recent chats**: the chat extractor run once, on demand, over the
//! person's recent chats (owner, 2026-11-13).
//!
//! The extractor otherwise sees only messages typed after the person turned suggestions on.
//! This reads the person's own recent chats once, with every rule the extractor applies to
//! a live message, and queues what it finds as suggestions the person keeps or dismisses:
//!
//! - only chats that may learn: the person's own (`SessionType::User`), outside Crew, and
//!   never an app's chat or one an ACP client (stdio or `--ws`) opened;
//! - only chats whose tier is Public. A chat's stored tier ratchets to Private at its first
//!   turn on a private model, and stays Public when a public model (a decision model
//!   included, DR-33) makes it Public, so the stored tier is the chat's tier;
//! - only messages the person wrote and can see ([`super::extractor::person_wrote`]),
//!   through the extractor's own candidate rules ([`super::extractor::candidates`]).
//!
//! Candidates are counted across the chats read, in memory, and only a word found in at
//! least two of them is written, once, at the end, so a scan neither writes a row per chat
//! nor fills the store's suggestion slots with words seen once. Nothing leaves the
//! computer and no model is called. It reads the newest [`SCAN_MAX_CHATS`] chats changed
//! within [`SCAN_DAYS`] days, and stops when asked, keeping what it found so far.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use crate::security::dictionary_store::store;
use crate::session::{Session, SessionManager, SessionType};

/// The most chats one scan reads, newest first.
pub const SCAN_MAX_CHATS: usize = 100;
/// How far back a scan reads, by the day a chat last changed.
pub const SCAN_DAYS: i64 = 30;
/// The most distinct words one chat contributes, so a chat of pasted text does not crowd
/// out the rest.
pub const SCAN_MAX_WORDS_PER_CHAT: usize = 50;
/// The name an ACP client's chat starts with ([`ACP_SESSION_NAME`] in the ACP server).
/// A chat an ACP client opened before [`mark_not_learnable`] existed carries no mark, so
/// one that still has this name is skipped too.
pub const ACP_SESSION_NAME: &str = "ACP Session";

/// Where a scan stands. Serialized as snake case for the routes and the CLI's `--json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanState {
    /// No scan has run in this process.
    Idle,
    Running,
    /// It read every chat it meant to.
    Finished,
    /// The person stopped it; what it found until then was kept.
    Stopped,
    /// It could not read the chats or write the dictionary.
    Failed,
}

/// A scan's progress: what the page shows while it runs and when it ends.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScanProgress {
    pub state: ScanState,
    /// Chats read so far.
    pub chats_read: u32,
    /// Chats this scan means to read.
    pub chats_total: u32,
    /// Words found so far in enough chats to be suggested. While it runs this counts what
    /// it has gathered; once it ends, the suggestions it added for review.
    pub words_found: u32,
}

impl ScanProgress {
    pub const IDLE: Self = Self {
        state: ScanState::Idle,
        chats_read: 0,
        chats_total: 0,
        words_found: 0,
    };
}

/// Why a scan did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanRefusal {
    /// Suggestions are off: the person turns them on first.
    Off,
    /// Managed policy turns the dictionary, or its automatic writes, off.
    Policy,
    /// The Dictionary store cannot be read.
    Unreadable,
}

impl ScanRefusal {
    /// The sentence a surface shows.
    pub fn message(self) -> &'static str {
        match self {
            Self::Off => MSG_SCAN_OFF,
            Self::Policy => MSG_SCAN_POLICY,
            Self::Unreadable => MSG_SCAN_UNREADABLE,
        }
    }
}

/// MSG-SCAN-OFF.
pub const MSG_SCAN_OFF: &str =
    "Turn on Suggest words from my messages first, then find words in your recent chats.";
/// MSG-SCAN-POLICY.
pub const MSG_SCAN_POLICY: &str =
    "Your organization turned off adding words to the dictionary automatically.";
/// MSG-SCAN-UNREADABLE.
pub const MSG_SCAN_UNREADABLE: &str =
    "Biorouter could not read your dictionary, so it did not look for words.";

/// The per-session record that keeps a chat out of every scan: written when an ACP client
/// (stdio or `--ws`) opens or loads a chat, because such a client can deliver other
/// people's words as user messages (DD-57).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NotLearnable {
    pub reason: NotLearnableReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotLearnableReason {
    Acp,
}

impl crate::session::extension_data::ExtensionState for NotLearnable {
    const EXTENSION_NAME: &'static str = "dictionary_not_learnable";
    const VERSION: &'static str = "v0";
}

/// Keep `session_id` out of every scan. Best effort: a chat this cannot mark is still left
/// out by its name until it is renamed, and the live extractor never reads an ACP chat.
pub async fn mark_not_learnable(
    sessions: &SessionManager,
    session_id: &str,
    reason: NotLearnableReason,
) {
    use crate::session::extension_data::ExtensionState;
    let Ok(value) = (NotLearnable { reason }).to_value() else {
        return;
    };
    if let Err(error) = sessions
        .update_extension_state(
            session_id,
            NotLearnable::EXTENSION_NAME,
            NotLearnable::VERSION,
            move |_| Ok(value),
        )
        .await
    {
        tracing::debug!("could not keep chat {session_id} out of word scans: {error}");
    }
}

/// Whether a stored chat may be scanned, from its row alone: the person's own kind, an
/// app's or an ACP client's chat never, a Public tier only, and changed within
/// [`SCAN_DAYS`]. The Crew scope is asked separately, because it needs the Crew registry.
pub fn row_may_be_scanned(session: &Session, now: chrono::DateTime<chrono::Utc>) -> bool {
    use crate::session::extension_data::ExtensionState;
    session.session_type == SessionType::User
        && !session.name.starts_with("app:")
        && session.name != ACP_SESSION_NAME
        && NotLearnable::from_extension_data(&session.extension_data).is_none()
        && session.privacy_tier == crate::privacy::SessionClassification::Public
        && session.updated_at >= now - chrono::Duration::days(SCAN_DAYS)
}

/// One word across the chats read: its first spelling, and the chats it was found in,
/// newest first.
struct Gathered {
    written: String,
    chats: Vec<String>,
}

/// The words a scan has gathered, by fold key.
#[derive(Default)]
struct Harvest {
    words: BTreeMap<String, Gathered>,
}

impl Harvest {
    /// Count every candidate in the person's own messages of one chat, once per chat.
    fn read_chat(&mut self, session: &Session) {
        let Some(conversation) = session.conversation.as_ref() else {
            return;
        };
        let mut here: HashSet<String> = HashSet::new();
        for message in conversation.messages() {
            if !super::extractor::person_wrote(message) {
                continue;
            }
            let text = message.as_concat_text();
            for word in super::extractor::candidates(&text) {
                if here.len() >= SCAN_MAX_WORDS_PER_CHAT {
                    return;
                }
                let fold = store::fold_key(&word);
                if !here.insert(fold.clone()) {
                    continue;
                }
                let gathered = self.words.entry(fold).or_insert_with(|| Gathered {
                    written: word,
                    chats: Vec::new(),
                });
                gathered.chats.push(session.id.clone());
            }
        }
    }

    /// Words found in at least `required` chats that the store does not already hold or
    /// refuse.
    fn ready(&self, view: &store::schema::StoreFile, required: usize) -> u32 {
        self.words
            .iter()
            .filter(|(fold, word)| {
                word.chats.len() >= required
                    && !view.tombstoned(fold, store::EntryTier::Public)
                    && !view
                        .entries
                        .iter()
                        .any(|row| row.tier == store::EntryTier::Public && &row.fold_key() == *fold)
            })
            .count() as u32
    }

    /// Write every word found in at least two chats, those found in the most chats first,
    /// each through the chat extractor's own store operation with up to the most chats a
    /// word can need, oldest first, so the newest chat is the one it says it came from.
    /// The suggestions this listed for review.
    fn write(self, dictionary: &store::DictionaryStore) -> Result<u32, store::StoreError> {
        let mut words: Vec<Gathered> = self
            .words
            .into_values()
            .filter(|word| word.chats.len() >= 2)
            .collect();
        if words.is_empty() {
            return Ok(0);
        }
        words.sort_by(|a, b| {
            b.chats
                .len()
                .cmp(&a.chats.len())
                .then_with(|| a.written.cmp(&b.written))
        });
        let mut listed = 0u32;
        let at = store::schema::now();
        dictionary.write(|file| {
            if !file.settings.learn_from_chats {
                return Ok(());
            }
            let before = listed_chat_suggestions(file);
            let most = store::learning::MAX_REQUIRED_CHATS as usize;
            for word in &words {
                for chat in word.chats.iter().take(most).rev() {
                    let _ = file.observe_auto_suggestion(
                        &word.written,
                        store::EntryTier::Public,
                        chat,
                        None,
                        at,
                    );
                }
            }
            listed = listed_chat_suggestions(file).saturating_sub(before);
            Ok(())
        })?;
        Ok(listed)
    }
}

fn listed_chat_suggestions(file: &store::schema::StoreFile) -> u32 {
    file.entries
        .iter()
        .filter(|row| {
            row.source == store::EntrySource::Auto
                && row.status == store::EntryStatus::Suggested
                && row.seen >= 2
        })
        .count() as u32
}

/// Whether suggestions are on, managed policy allows them, and the store reads.
fn admitted(dictionary: &store::DictionaryStore) -> Result<(), ScanRefusal> {
    let policy = crate::managed::ManagedPolicy::load();
    if !policy.dictation().dictionary.is_allowed()
        || !policy.dictation().dictionary_agent_writes.is_allowed()
    {
        return Err(ScanRefusal::Policy);
    }
    let view = dictionary.view().map_err(|_| ScanRefusal::Unreadable)?;
    if !view.settings().learn_from_chats {
        return Err(ScanRefusal::Off);
    }
    Ok(())
}

/// Whether a scan may start now, without starting one: what the routes answer before they
/// spawn it, so a refusal reaches the person as a refusal rather than as a failed scan.
pub fn preflight() -> Result<(), ScanRefusal> {
    admitted(&crate::security::dictionary_store::open_store())
}

/// Read the person's recent eligible chats and queue what they hold as suggestions.
/// `stop` is checked between chats; `report` hears every change of progress. Runs in the
/// caller's task: the daemon spawns it ([`start`]), the CLI awaits it.
pub async fn scan(
    sessions: &SessionManager,
    stop: &AtomicBool,
    report: &mut (dyn FnMut(&ScanProgress) + Send),
) -> Result<ScanProgress, ScanRefusal> {
    let dictionary = crate::security::dictionary_store::open_store();
    admitted(&dictionary)?;
    let mut progress = ScanProgress {
        state: ScanState::Running,
        ..ScanProgress::IDLE
    };
    report(&progress);
    let failed = |mut progress: ScanProgress| {
        progress.state = ScanState::Failed;
        progress
    };
    let Ok(mut rows) = sessions.list_sessions_by_types(&[SessionType::User]).await else {
        let progress = failed(progress);
        report(&progress);
        return Ok(progress);
    };
    let now = chrono::Utc::now();
    rows.retain(|row| row_may_be_scanned(row, now));
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
    rows.truncate(SCAN_MAX_CHATS);
    progress.chats_total = rows.len() as u32;
    report(&progress);

    let mut harvest = Harvest::default();
    for row in rows {
        if stop.load(Ordering::Acquire) {
            progress.state = ScanState::Stopped;
            break;
        }
        if super::extractor::chat_may_learn(&row).await {
            if let Ok(chat) = sessions.get_session(&row.id, true).await {
                // Read again: the chat may have changed tier since it was listed.
                if row_may_be_scanned(&chat, now) {
                    harvest.read_chat(&chat);
                }
            }
        }
        progress.chats_read += 1;
        if let Ok(view) = dictionary.view() {
            progress.words_found = harvest.ready(view.file(), view_required(&view));
        }
        report(&progress);
    }
    // The setting may have been turned off while the chats were read.
    if admitted(&dictionary).is_err() {
        progress.words_found = 0;
        progress.state = ScanState::Stopped;
        report(&progress);
        return Ok(progress);
    }
    let written = tokio::task::spawn_blocking(move || harvest.write(&dictionary)).await;
    match written {
        Ok(Ok(listed)) => {
            progress.words_found = listed;
            if progress.state == ScanState::Running {
                progress.state = ScanState::Finished;
            }
        }
        _ => progress = failed(progress),
    }
    report(&progress);
    Ok(progress)
}

fn view_required(view: &store::StoreView) -> usize {
    view.file().chat_learning.required_chats() as usize
}

/// The daemon's one scan at a time: its progress, and the flag that stops it.
struct Job {
    progress: Mutex<ScanProgress>,
    stop: Mutex<Arc<AtomicBool>>,
    runs: AtomicU64,
}

static JOB: LazyLock<Job> = LazyLock::new(|| Job {
    progress: Mutex::new(ScanProgress::IDLE),
    stop: Mutex::new(Arc::new(AtomicBool::new(false))),
    runs: AtomicU64::new(0),
});

/// The daemon's scan, as it stands.
pub fn status() -> ScanProgress {
    JOB.progress
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Ask the running scan to stop after the chat it is reading. Its words so far are kept.
pub fn request_stop() -> ScanProgress {
    JOB.stop
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .store(true, Ordering::Release);
    status()
}

/// Start the daemon's scan in the background unless one is running, after checking it
/// may start. `owner` is what the task keeps alive to reach the session store through
/// `sessions` (the daemon's state). The progress then.
pub fn start<S: Send + Sync + 'static>(
    owner: S,
    sessions: fn(&S) -> &SessionManager,
) -> Result<ScanProgress, ScanRefusal> {
    preflight()?;
    let stop = {
        let mut progress = JOB.progress.lock().unwrap_or_else(PoisonError::into_inner);
        if progress.state == ScanState::Running {
            return Ok(progress.clone());
        }
        *progress = ScanProgress {
            state: ScanState::Running,
            ..ScanProgress::IDLE
        };
        let stop = Arc::new(AtomicBool::new(false));
        *JOB.stop.lock().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&stop);
        stop
    };
    let run = JOB.runs.fetch_add(1, Ordering::AcqRel) + 1;
    tokio::spawn(async move {
        let mut report = |progress: &ScanProgress| {
            if JOB.runs.load(Ordering::Acquire) == run {
                *JOB.progress.lock().unwrap_or_else(PoisonError::into_inner) = progress.clone();
            }
        };
        let ended = match scan(sessions(&owner), &stop, &mut report).await {
            Ok(progress) => progress,
            Err(_) => ScanProgress {
                state: ScanState::Failed,
                ..status()
            },
        };
        report(&ended);
    });
    Ok(status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;

    fn chat(id: &str, texts: &[&str]) -> Session {
        let mut session = Session::default();
        session.id = id.to_string();
        let mut conversation = crate::conversation::Conversation::empty();
        for text in texts {
            conversation.push(Message::user().with_text(*text));
            conversation.push(Message::assistant().with_text("PHD2 OMOP fine"));
        }
        session.conversation = Some(conversation);
        session
    }

    #[test]
    fn a_word_counts_once_per_chat_and_only_from_the_persons_messages() {
        let mut harvest = Harvest::default();
        harvest.read_chat(&chat("a", &["EPAS1 and EPAS1", "epas1 again"]));
        harvest.read_chat(&chat("b", &["EPAS1 with HIF2A"]));
        let epas = harvest.words.get(&store::fold_key("EPAS1")).unwrap();
        assert_eq!(epas.chats, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(epas.written, "EPAS1");
        assert!(
            !harvest.words.contains_key(&store::fold_key("PHD2")),
            "the assistant's words are never read"
        );
    }

    #[test]
    fn a_chat_contributes_at_most_its_share_of_words() {
        let many: Vec<String> = (0..200).map(|i| format!("WRD{i}X")).collect();
        let texts: Vec<String> = many.chunks(3).map(|c| c.join(" ")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let mut harvest = Harvest::default();
        harvest.read_chat(&chat("a", &refs));
        assert_eq!(harvest.words.len(), SCAN_MAX_WORDS_PER_CHAT);
    }

    /// A Crew scoped chat is never read, while the same chat without its scope is: the
    /// scan asks the chat extractor's own rule ([`super::super::extractor::chat_may_learn`]).
    #[tokio::test]
    async fn a_crew_scoped_chat_is_never_scanned() {
        use crate::security::dictionary_store::open_store;

        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::TempDir::new().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = SessionManager::new(root.path().join("sessions"));
        let mut chats = Vec::new();
        for name in ["one", "two"] {
            let chat = sessions
                .create_session(root.path().to_path_buf(), name.into(), SessionType::User)
                .await
                .unwrap();
            sessions
                .add_message(&chat.id, &Message::user().with_text("CREWSCAN24"))
                .await
                .unwrap();
            chats.push(chat);
        }
        open_store()
            .write(|file| {
                file.settings.learn_from_chats = true;
                Ok(())
            })
            .unwrap();
        let stop = AtomicBool::new(false);
        let crew = crate::crew::install_test_scope(&chats[0].id, None).await;
        let scoped = scan(&sessions, &stop, &mut |_| {}).await;
        crate::crew::remove_test_scope(&crew, &chats[0].id).await;
        let scoped = scoped.unwrap();
        assert_eq!(scoped.words_found, 0, "a Crew scoped chat was read: {scoped:?}");
        let open = scan(&sessions, &stop, &mut |_| {}).await.unwrap();
        assert_eq!(open.words_found, 1, "control: without the scope it is read");
    }

    #[test]
    fn only_a_persons_own_public_recent_chat_may_be_scanned() {
        let now = chrono::Utc::now();
        let mut row = Session::default();
        row.updated_at = now;
        assert!(row_may_be_scanned(&row, now), "premise: a person's own chat");

        let mut private = row.clone();
        private.privacy_tier = crate::privacy::SessionClassification::Private;
        assert!(!row_may_be_scanned(&private, now));

        for kind in [
            SessionType::SubAgent,
            SessionType::Scheduled,
            SessionType::Hidden,
            SessionType::Terminal,
        ] {
            let mut other = row.clone();
            other.session_type = kind;
            assert!(!row_may_be_scanned(&other, now), "{kind}");
        }

        let mut app = row.clone();
        app.name = "app:notes:client-1".into();
        assert!(!row_may_be_scanned(&app, now));

        let mut acp = row.clone();
        acp.name = ACP_SESSION_NAME.into();
        assert!(!row_may_be_scanned(&acp, now));

        let mut marked = row.clone();
        marked.name = "Renamed by the client".into();
        {
            use crate::session::extension_data::ExtensionState;
            NotLearnable {
                reason: NotLearnableReason::Acp,
            }
            .to_extension_data(&mut marked.extension_data)
            .unwrap();
        }
        assert!(!row_may_be_scanned(&marked, now));

        let mut old = row.clone();
        old.updated_at = now - chrono::Duration::days(SCAN_DAYS + 1);
        assert!(!row_may_be_scanned(&old, now));
    }
}
