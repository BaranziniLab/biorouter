//! `biorouter dictation history` (DD-96; `docs/dictation/surfaces.md`, "Dictation
//! history", "In the terminal"): the text of the person's dictations at a terminal. Every
//! command works offline, on the store under its lock, so a desktop app and a terminal on
//! one account share one history; no daemon is needed.
//!
//! Who may do what:
//!
//! | Situation | List and show | Delete and clear | Make public | Make private | Raise a setting | Lower a setting |
//! |---|---|---|---|---|---|---|
//! | An interactive terminal, `BIOROUTER_TERMINAL` unset | every entry | yes | after the typed word | yes | after a question | yes |
//! | No terminal (a pipe, cron, `ssh host cmd`) | public entries, with no count of the rest | refused (exit 2) | refused | refused | refused | yes |
//! | An agent's shell (`BIOROUTER_TERMINAL=1`) | public entries, with no count of the rest | refused (exit 2) | refused | refused | refused | yes |
//! | Managed `history: false` | as above | yes | refused (exit 2) | refused | refused | yes |
//!
//! Raising is turning **Dictation history** on or **Save new dictations as public** on;
//! lowering is turning either off, which never needs anyone, because it only narrows. An
//! id the caller may not list answers exactly as a missing one (MSG-HISTORY-NOT-FOUND, exit
//! 1). Every rule here is accident prevention, as for the dictionary, not a boundary: a
//! shell can unset the variable or read the file.
//!
//! The decisions live in [`run`], which takes the terminal facts, the policy, the store, a
//! [`Prompter`] and the output streams as arguments, so the tests below drive every path
//! without a terminal; [`handle`] gathers the real ones. The history never feeds
//! recognition (P-18): nothing here reaches the dictionary.

use std::collections::HashSet;
use std::io::{self, Write};

use anyhow::Result;
use biorouter::dictation::history::store::{msg_history_file, MSG_HISTORY_NOT_FOUND};
use biorouter::dictation::history::{
    self, store as leaf, EntryTier, HistoryEntry, HistoryError, HistoryStore, SettingsChange,
    Surface, TimeWindow,
};
use biorouter::managed::{ManagedPolicy, Rule};
use biorouter::security::dictionary_store::store::Reach;
use chrono::Local;
use serde_json::{json, Map, Value};

use crate::commands::needs_terminal;

/// Set in an agent's shell; changing the history refuses there (P-17).
pub const AGENT_SHELL_ENV: &str = "BIOROUTER_TERMINAL";

/// Changing the history in an agent's shell (exit 2).
pub const MSG_HISTORY_AGENT: &str = "Your dictation history is yours to change: run this in your own terminal, or use the History tab of the Dictionary page.";
/// Changing the history without a terminal (exit 2).
pub const MSG_HISTORY_TTY: &str = "Changing your dictation history needs you at an interactive terminal. Turning it off, or turning off public by default, works anywhere.";
/// MSG-HISTORY-PUBLICIZE-WARN.
pub const MSG_HISTORY_PUBLICIZE_WARN: &str =
    "Public dictations may be read by chats on public models.";
/// MSG-HISTORY-PUBLICIZE-WARN-TERMS.
pub const MSG_HISTORY_PUBLICIZE_WARN_TERMS: &str = "This dictation used your private dictionary terms, so it may contain them. Public dictations may be read by chats on public models.";
/// MSG-HISTORY-PUBLIC-CONFIRM.
pub const MSG_HISTORY_PUBLIC_CONFIRM: &str = "New dictations will be public unless they go to a private chat or use your private terms. Public dictations may be read by chats on public models.";
/// MSG-HISTORY-EMPTY.
pub const MSG_HISTORY_EMPTY: &str =
    "Text you dictate shows here, so you can copy it again or add its words to your dictionary.";
/// MSG-HISTORY-OFF.
pub const MSG_HISTORY_OFF: &str = "History is off, so new dictations are not saved.";
/// MSG-HISTORY-NO-MATCH.
pub const MSG_HISTORY_NO_MATCH: &str = "No dictation contains these words.";
/// What `clear` says when nothing is listed.
pub const MSG_NOTHING_TO_CLEAR: &str = "There are no dictations to delete.";
const MSG_NOTHING_CHANGED: &str = "Nothing was changed.";
/// The word `publicize` asks the person to type.
const PUBLIC_WORD: &str = "public";

/// The most entries one listing prints.
pub const MAX_LIMIT: usize = leaf::MAX_ENTRIES;
/// What a listing prints without `--limit`.
pub const DEFAULT_LIMIT: usize = 50;
/// The longest `--search`, in characters, as the HTTP listing's `q`.
pub const MAX_SEARCH_CHARS: usize = 200;
/// How much of an entry's text a text listing shows on its line.
const LINE_CHARS: usize = 160;

/// "1 dictation" or "<n> dictations".
fn n_dictations(count: usize) -> String {
    if count == 1 {
        "1 dictation".to_string()
    } else {
        format!("{count} dictations")
    }
}

/// What `clear` says before it asks for `delete <n>`, from MSG-HISTORY-CLEAR.
pub fn msg_clear(count: usize, folder: &std::path::Path) -> String {
    format!(
        "This deletes {} from {}. It cannot be undone.",
        n_dictations(count),
        folder.display()
    )
}

/// What `clear` and `delete` report.
fn msg_deleted(count: usize) -> String {
    format!("Deleted {}.", n_dictations(count))
}

/// A typed answer that was not the word asked for.
fn msg_typed_mismatch(word: &str) -> String {
    format!("That did not match {word}, so nothing was changed.")
}

/// A `--search` longer than the listing takes (exit 2).
fn msg_search_too_long() -> String {
    format!("Search for at most {MAX_SEARCH_CHARS} characters.")
}

/// A command the history runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    List {
        search: Option<String>,
        limit: usize,
        format: Format,
    },
    Show {
        id: String,
        format: Format,
    },
    Delete {
        id: String,
    },
    Clear,
    Publicize {
        id: String,
    },
    Privatize {
        id: String,
    },
    Settings {
        keep_history: Option<bool>,
        public_by_default: Option<bool>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
}

impl Format {
    pub fn parse(value: Option<&str>) -> Format {
        match value {
            Some("json") => Format::Json,
            _ => Format::Text,
        }
    }
}

/// What a command is, for the authority rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    /// Deleting: allowed under managed `history: false`, as the dictionary's deletes are.
    Delete,
    /// A tier change.
    Tier,
}

impl Action {
    fn kind(&self) -> Kind {
        match self {
            Self::List { .. } | Self::Show { .. } | Self::Settings { .. } => Kind::Read,
            Self::Delete { .. } | Self::Clear => Kind::Delete,
            Self::Publicize { .. } | Self::Privatize { .. } => Kind::Tier,
        }
    }
}

/// Everything a command decides from, sampled once.
#[derive(Debug, Clone)]
pub struct Facts {
    /// `prompt_can_run()`: stdin and stderr are terminals.
    pub interactive: bool,
    /// `BIOROUTER_TERMINAL` is set.
    pub agent_shell: bool,
    /// Managed `history`.
    pub history: Rule,
    /// The account the history belongs to.
    pub account: String,
}

impl Facts {
    /// A person at an interactive terminal lists every entry; anyone else lists public
    /// entries, with no count of the rest.
    fn reach(&self) -> Reach {
        if self.person() {
            Reach::All
        } else {
            Reach::Public
        }
    }

    fn person(&self) -> bool {
        self.interactive && !self.agent_shell
    }

    /// Whether a change only a person may make can run here: the sentence that refuses it
    /// otherwise.
    fn refuse_unless_person(&self) -> Result<(), Failure> {
        if self.agent_shell {
            return Err(Failure::refused(MSG_HISTORY_AGENT));
        }
        if !self.interactive {
            return Err(Failure::refused(MSG_HISTORY_TTY));
        }
        Ok(())
    }

    /// [`Self::refuse_unless_person`], and the managed policy, for a tier change or a raise.
    fn refuse_unless_person_and_policy(&self) -> Result<(), Failure> {
        self.refuse_unless_person()?;
        if let Some(reason) = self.history.reason() {
            return Err(Failure::refused(reason));
        }
        Ok(())
    }
}

/// The questions a command asks at a terminal.
pub trait Prompter {
    /// A yes or no question that defaults to no.
    fn confirm(&mut self, question: &str) -> io::Result<bool>;
    /// Typed text; an empty answer cancels.
    fn phrase(&mut self, prompt: &str) -> io::Result<String>;
}

/// The real questions, with cliclack on stderr.
struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn confirm(&mut self, question: &str) -> io::Result<bool> {
        cliclack::confirm(question).initial_value(false).interact()
    }

    fn phrase(&mut self, prompt: &str) -> io::Result<String> {
        cliclack::input(prompt).required(false).interact()
    }
}

/// The streams a command writes.
pub struct Streams<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

/// How a command ended without doing its work: exit 2 when it declined to start (its
/// sentence alone on stderr), exit 1 when it failed, and exit 0 when the person cancelled a
/// question, which [`run`] reports as "Nothing was changed." on stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub exit: u8,
    pub message: String,
}

impl Failure {
    fn cancelled() -> Self {
        Self {
            exit: 0,
            message: MSG_NOTHING_CHANGED.to_string(),
        }
    }

    fn refused(message: impl Into<String>) -> Self {
        Self {
            exit: needs_terminal::EXIT_CODE,
            message: message.into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            exit: 1,
            message: message.into(),
        }
    }

    fn not_found() -> Self {
        Self::failed(MSG_HISTORY_NOT_FOUND)
    }
}

impl From<HistoryError> for Failure {
    fn from(e: HistoryError) -> Self {
        Self::failed(e.to_string())
    }
}

impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Self {
        // Only a prompt answers `Interrupted` (Escape at a question): nothing was changed,
        // because every question comes before a command's one write.
        if e.kind() == io::ErrorKind::Interrupted {
            Self::cancelled()
        } else {
            Self::failed(e.to_string())
        }
    }
}

type Outcome = std::result::Result<(), Failure>;

fn in_agent_shell() -> bool {
    std::env::var_os(AGENT_SHELL_ENV).is_some_and(|value| !value.is_empty())
}

/// Run `action` from the real terminal, policy and store.
pub async fn handle(action: Action) -> Result<()> {
    let policy = ManagedPolicy::load();
    let facts = Facts {
        interactive: needs_terminal::prompt_can_run(),
        agent_shell: in_agent_shell(),
        history: policy.dictation().history.clone(),
        account: biorouter::dictation::service::account(),
    };
    let store = history::open_history();
    let (mut stdout, mut stderr) = (io::stdout(), io::stderr());
    let outcome = {
        let mut streams = Streams {
            out: &mut stdout,
            err: &mut stderr,
        };
        run(action, &facts, &store, &mut TerminalPrompter, &mut streams)
    };
    match outcome {
        Ok(()) => Ok(()),
        Err(failure) if failure.exit == needs_terminal::EXIT_CODE => {
            Err(needs_terminal::require(false, &failure.message)
                .expect_err("require(false, _) always refuses")
                .into())
        }
        Err(failure) => {
            let _ = stdout.flush();
            eprintln!("{}", failure.message);
            std::process::exit(i32::from(failure.exit));
        }
    }
}

/// Refuse `action` before it touches the store when this caller may not run it. A
/// settings change is judged in [`settings`], which knows whether it raises.
fn authorize(action: &Action, facts: &Facts) -> Outcome {
    match action.kind() {
        Kind::Read => Ok(()),
        Kind::Delete => facts.refuse_unless_person(),
        Kind::Tier => facts.refuse_unless_person_and_policy(),
    }
}

/// The line every command starts with on stderr: the folder and the account it acts on.
fn say_where(store: &HistoryStore, facts: &Facts, err: &mut dyn Write) -> io::Result<()> {
    writeln!(
        err,
        "Dictation history: {} (account {})",
        store.dir().display(),
        facts.account
    )
}

/// Run one command.
pub fn run(
    action: Action,
    facts: &Facts,
    store: &HistoryStore,
    prompt: &mut dyn Prompter,
    io: &mut Streams<'_>,
) -> Outcome {
    authorize(&action, facts)?;
    say_where(store, facts, io.err)?;
    let reach = facts.reach();
    let outcome = match action {
        Action::List {
            search,
            limit,
            format,
        } => list(store, reach, search.as_deref(), limit, format, io),
        Action::Show { id, format } => show(store, reach, &id, format, io),
        Action::Delete { id } => delete(store, reach, &id, io),
        Action::Clear => clear(store, reach, prompt, io),
        Action::Publicize { id } => change_tier(store, reach, prompt, &id, EntryTier::Public, io),
        Action::Privatize { id } => change_tier(store, reach, prompt, &id, EntryTier::Private, io),
        Action::Settings {
            keep_history,
            public_by_default,
        } => settings(store, facts, prompt, keep_history, public_by_default, io),
    };
    match outcome {
        Err(failure) if failure.exit == 0 => {
            writeln!(io.out, "{}", failure.message)?;
            Ok(())
        }
        other => other,
    }
}

/// The word the History page shows for a surface.
fn surface_word(surface: Surface) -> &'static str {
    match surface {
        Surface::Composer => "Chat",
        Surface::Crew => "Crew",
        Surface::Clipboard => "Clipboard",
        Surface::OtherApp => "Another app",
        Surface::AppPreview => "App preview",
        Surface::Terminal => "Terminal",
    }
}

/// "14 s", or "2 min 5 s", to the nearest second and never under one.
fn length_words(duration_ms: u64) -> String {
    let seconds = (duration_ms.saturating_add(500) / 1_000).max(1);
    if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min {} s", seconds / 60, seconds % 60)
    }
}

/// When an entry was ready, in this computer's time.
fn when(entry: &HistoryEntry) -> String {
    entry
        .created_at
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// An entry's one muted line: time, surface, length and tier.
fn detail_line(entry: &HistoryEntry) -> String {
    format!(
        "{} · {} · {} · {}",
        when(entry),
        surface_word(entry.surface),
        length_words(entry.duration_ms),
        entry.tier.as_str()
    )
}

/// The text on one line, at most [`LINE_CHARS`] characters.
fn one_line(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= LINE_CHARS {
        return flat;
    }
    let mut cut: String = flat.chars().take(LINE_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// An entry as `--format json` shows it: the stored entry minus its institutions, its chat
/// and its incarnation; `private_terms` only for a caller who may list private entries, as
/// the HTTP view gives it.
fn entry_view(entry: &HistoryEntry, reach: Reach) -> Value {
    let mut row = Map::new();
    row.insert("id".into(), json!(entry.id));
    row.insert("created_at".into(), json!(entry.created_at.to_rfc3339()));
    row.insert("text".into(), json!(entry.text));
    row.insert("tier".into(), json!(entry.tier.as_str()));
    row.insert("surface".into(), json!(entry.surface.as_str()));
    row.insert("duration_ms".into(), json!(entry.duration_ms));
    row.insert("model".into(), json!(entry.model));
    if reach == Reach::All {
        row.insert("private_terms".into(), json!(entry.private_terms));
    }
    if entry.truncated == Some(true) {
        row.insert("truncated".into(), json!(true));
    }
    Value::Object(row)
}

/// The entry `id` names, if this caller may list it; anything else is not found.
fn listable(store: &HistoryStore, reach: Reach, id: &str) -> Result<HistoryEntry, Failure> {
    if !leaf::is_entry_id(id) {
        return Err(Failure::not_found());
    }
    store
        .get(id)?
        .filter(|entry| reach.lists(entry.tier))
        .ok_or_else(Failure::not_found)
}

/// `history` and `history list`: newest first, the caller's entries only, with no count of
/// the rest.
fn list(
    store: &HistoryStore,
    reach: Reach,
    search: Option<&str>,
    limit: usize,
    format: Format,
    io: &mut Streams<'_>,
) -> Outcome {
    let search = search.map(str::trim).filter(|q| !q.is_empty());
    if search.is_some_and(|q| q.chars().count() > MAX_SEARCH_CHARS) {
        return Err(Failure::refused(msg_search_too_long()));
    }
    let limit = limit.clamp(1, MAX_LIMIT);
    let page = store.search(search, TimeWindow::all(), None, limit, &|entry| {
        reach.lists(entry.tier)
    })?;
    if format == Format::Json {
        let entries: Vec<Value> = page.entries.iter().map(|e| entry_view(e, reach)).collect();
        let body = json!({
            "schema": crate::dictation::SCHEMA,
            "entries": entries,
            "total": page.total,
        });
        writeln!(
            io.out,
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        )?;
        return Ok(());
    }
    if reach == Reach::All && !store.settings().keep_history {
        writeln!(io.err, "{MSG_HISTORY_OFF}")?;
    }
    if page.entries.is_empty() {
        if reach == Reach::All {
            let sentence = if search.is_some() {
                MSG_HISTORY_NO_MATCH
            } else {
                MSG_HISTORY_EMPTY
            };
            writeln!(io.err, "{sentence}")?;
        }
        return Ok(());
    }
    for entry in &page.entries {
        writeln!(io.out, "{}  {}", detail_line(entry), entry.id)?;
        writeln!(io.out, "  {}", one_line(&entry.text))?;
    }
    if page.total > page.entries.len() {
        writeln!(
            io.err,
            "Showing the newest {} of {}. Use --limit to see more.",
            page.entries.len(),
            page.total
        )?;
    }
    Ok(())
}

/// `history show <id>`: the whole text alone on stdout, so it can be piped, and its line
/// on stderr; or the entry as JSON.
fn show(
    store: &HistoryStore,
    reach: Reach,
    id: &str,
    format: Format,
    io: &mut Streams<'_>,
) -> Outcome {
    let entry = listable(store, reach, id)?;
    if format == Format::Json {
        writeln!(
            io.out,
            "{}",
            serde_json::to_string_pretty(&entry_view(&entry, reach)).unwrap_or_default()
        )?;
        return Ok(());
    }
    writeln!(io.err, "{}", detail_line(&entry))?;
    writeln!(io.out, "{}", entry.text)?;
    Ok(())
}

/// `history delete <id>`: one entry, as `dictionary remove` deletes a term.
fn delete(store: &HistoryStore, reach: Reach, id: &str, io: &mut Streams<'_>) -> Outcome {
    let entry = listable(store, reach, id)?;
    let deleted = store.delete(std::slice::from_ref(&entry.id), &|e| reach.lists(e.tier))?;
    if deleted == 0 {
        return Err(Failure::not_found());
    }
    writeln!(io.out, "{}", msg_deleted(deleted))?;
    Ok(())
}

/// `history clear`: the entries a listing shows, after the person types `delete <n>`. The
/// prompt can wait as long as the person takes, so a dictation recorded meanwhile is kept,
/// and the report says how many went.
fn clear(
    store: &HistoryStore,
    reach: Reach,
    prompt: &mut dyn Prompter,
    io: &mut Streams<'_>,
) -> Outcome {
    let page = store.search(None, TimeWindow::all(), None, MAX_LIMIT, &|entry| {
        reach.lists(entry.tier)
    })?;
    let count = page.entries.len();
    let Some(through) = page.entries.iter().map(|e| e.created_at).max() else {
        writeln!(io.out, "{MSG_NOTHING_TO_CLEAR}")?;
        return Ok(());
    };
    writeln!(io.err, "{}", msg_clear(count, store.dir()))?;
    let phrase = format!("delete {count}");
    let typed = prompt.phrase(&format!(
        "Type {phrase} to confirm, or leave blank to cancel"
    ))?;
    if typed.trim() != phrase {
        writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
        return Ok(());
    }
    let seen: HashSet<String> = page.entries.into_iter().map(|e| e.id).collect();
    let deleted = store.clear_through(through, &|e| seen.contains(&e.id) && reach.lists(e.tier))?;
    writeln!(io.out, "{}", msg_deleted(deleted))?;
    Ok(())
}

/// `history publicize <id>` and `privatize <id>`. Making a dictation public shows its
/// warning and then asks the person to type `public`: a blank answer cancels, one that does
/// not match exits 1 and changes nothing. Making one private narrows and asks nothing.
fn change_tier(
    store: &HistoryStore,
    reach: Reach,
    prompt: &mut dyn Prompter,
    id: &str,
    tier: EntryTier,
    io: &mut Streams<'_>,
) -> Outcome {
    let entry = listable(store, reach, id)?;
    let verb = tier.as_str();
    if entry.tier == tier {
        writeln!(io.out, "The dictation is already {verb}.")?;
        return Ok(());
    }
    if tier == EntryTier::Public {
        let warning = if entry.private_terms {
            MSG_HISTORY_PUBLICIZE_WARN_TERMS
        } else {
            MSG_HISTORY_PUBLICIZE_WARN
        };
        writeln!(io.err, "{warning}")?;
        let typed = prompt.phrase(&format!(
            "Type {PUBLIC_WORD} to make it public, or leave blank to cancel"
        ))?;
        if typed.trim().is_empty() {
            writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
            return Ok(());
        }
        if typed.trim() != PUBLIC_WORD {
            return Err(Failure::failed(msg_typed_mismatch(PUBLIC_WORD)));
        }
    }
    store
        .set_tier(&entry.id, tier, &|e| reach.lists(e.tier))?
        .ok_or_else(Failure::not_found)?;
    writeln!(io.out, "The dictation is now {verb}.")?;
    Ok(())
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

/// `history settings`: with no flag, print them. Turning either switch off works anywhere
/// and asks nothing. Turning one on needs a person at an interactive terminal, is refused
/// under managed `history: false`, and asks first (with MSG-HISTORY-PUBLIC-CONFIRM before
/// public by default).
fn settings(
    store: &HistoryStore,
    facts: &Facts,
    prompt: &mut dyn Prompter,
    keep_history: Option<bool>,
    public_by_default: Option<bool>,
    io: &mut Streams<'_>,
) -> Outcome {
    if keep_history.is_some() || public_by_default.is_some() {
        let current = store.read_settings()?;
        let raises_keep = keep_history == Some(true) && !current.keep_history;
        let raises_public = public_by_default == Some(true) && !current.public_by_default;
        if raises_keep || raises_public {
            facts.refuse_unless_person_and_policy()?;
            if raises_public {
                writeln!(io.err, "{MSG_HISTORY_PUBLIC_CONFIRM}")?;
            }
            if !prompt.confirm("Change this setting?")? {
                writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
                return Ok(());
            }
        }
        store.set_settings(SettingsChange {
            keep_history,
            public_by_default,
        })?;
    }
    let settings = store
        .read_settings()
        .map_err(|_| Failure::failed(msg_history_file(&store.settings_path())))?;
    writeln!(io.out, "keep_history: {}", on_off(settings.keep_history))?;
    writeln!(
        io.out,
        "public_by_default: {}",
        on_off(settings.public_by_default)
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::borrow::Cow;
    use std::collections::VecDeque;

    /// Scripted answers, one queue per kind of question.
    #[derive(Default)]
    struct Script {
        confirms: VecDeque<bool>,
        phrases: VecDeque<String>,
        asked: Vec<String>,
    }

    impl Prompter for Script {
        fn confirm(&mut self, question: &str) -> io::Result<bool> {
            self.asked.push(question.to_string());
            Ok(self
                .confirms
                .pop_front()
                .expect("an unexpected yes or no question"))
        }
        fn phrase(&mut self, prompt: &str) -> io::Result<String> {
            self.asked.push(prompt.to_string());
            Ok(self
                .phrases
                .pop_front()
                .expect("an unexpected typed question"))
        }
    }

    fn terminal() -> Facts {
        Facts {
            interactive: true,
            agent_shell: false,
            history: Rule::Allowed,
            account: "person".into(),
        }
    }

    fn piped() -> Facts {
        Facts {
            interactive: false,
            ..terminal()
        }
    }

    fn agent() -> Facts {
        Facts {
            agent_shell: true,
            ..terminal()
        }
    }

    fn managed_off() -> Facts {
        Facts {
            history: Rule::Forbidden(Cow::Borrowed("Your organization turned this off.")),
            ..terminal()
        }
    }

    struct Run {
        result: Outcome,
        out: String,
        err: String,
    }

    fn run_with(store: &HistoryStore, facts: &Facts, script: &mut Script, action: Action) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = {
            let mut streams = Streams {
                out: &mut out,
                err: &mut err,
            };
            run(action, facts, store, script, &mut streams)
        };
        Run {
            result,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    fn go(store: &HistoryStore, facts: &Facts, action: Action) -> Run {
        run_with(store, facts, &mut Script::default(), action)
    }

    fn fresh() -> (tempfile::TempDir, HistoryStore) {
        let root = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(root.path().join("history"));
        (root, store)
    }

    fn add(store: &HistoryStore, text: &str, tier: EntryTier, at: i64) -> String {
        let entry = HistoryEntry {
            id: leaf::new_entry_id(),
            created_at: DateTime::<Utc>::from_timestamp(1_795_707_552 + at, 0).unwrap(),
            text: text.to_string(),
            tier,
            institutions: None,
            surface: Surface::Terminal,
            duration_ms: 14_200,
            model: "whisper-small.en-q8_0".to_string(),
            private_terms: false,
            session_id: None,
            session_incarnation: None,
            truncated: None,
        };
        let id = entry.id.clone();
        store.append(entry).unwrap();
        id
    }

    fn list_action(format: Format) -> Action {
        Action::List {
            search: None,
            limit: DEFAULT_LIMIT,
            format,
        }
    }

    fn texts(store: &HistoryStore, facts: &Facts) -> Vec<String> {
        let run = go(store, facts, list_action(Format::Json));
        assert_eq!(run.result, Ok(()));
        serde_json::from_str::<Value>(&run.out).unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["text"].as_str().unwrap().to_string())
            .collect()
    }

    fn tier_of(store: &HistoryStore, id: &str) -> EntryTier {
        store.get(id).unwrap().unwrap().tier
    }

    /// A person at a terminal lists every entry, newest first; piped or in an agent's shell
    /// only public entries are listed, with no count or sentence about the rest.
    #[test]
    fn a_piped_listing_shows_public_entries_only() {
        let (_root, store) = fresh();
        add(&store, "public one", EntryTier::Public, 0);
        add(&store, "private two", EntryTier::Private, 1);
        add(&store, "public three", EntryTier::Public, 2);
        assert_eq!(
            texts(&store, &terminal()),
            ["public three", "private two", "public one"]
        );
        for facts in [piped(), agent()] {
            assert_eq!(texts(&store, &facts), ["public three", "public one"]);
            let run = go(&store, &facts, list_action(Format::Json));
            let body: Value = serde_json::from_str(&run.out).unwrap();
            assert_eq!(body["total"], 2, "the total counts only what is listed");
            assert!(
                body["entries"][0].get("private_terms").is_none(),
                "private_terms only for a caller who may list private entries"
            );
            let text = go(&store, &facts, list_action(Format::Text));
            assert!(!text.out.contains("private two"));
            assert_eq!(text.out.matches("dch_").count(), 2);
            assert_eq!(
                text.err.lines().count(),
                1,
                "only where it acts: {}",
                text.err
            );
        }
        // An empty or withheld history says nothing to a caller who may not list it all.
        let (_root, only_private) = fresh();
        add(&only_private, "private", EntryTier::Private, 0);
        let run = go(&only_private, &piped(), list_action(Format::Text));
        assert_eq!(run.out, "");
        assert!(!run.err.contains(MSG_HISTORY_EMPTY));
        let run = go(&only_private, &terminal(), list_action(Format::Text));
        assert!(run.out.contains("private"));
    }

    /// `show` prints the whole text alone on stdout; an id the caller may not list answers
    /// exactly as a missing one, and so does anything that is not an entry id.
    #[test]
    fn show_answers_a_withheld_id_as_a_missing_one() {
        let (_root, store) = fresh();
        let private = add(
            &store,
            "EPAS1 rose\nin the second cohort",
            EntryTier::Private,
            0,
        );
        let run = go(
            &store,
            &terminal(),
            Action::Show {
                id: private.clone(),
                format: Format::Text,
            },
        );
        assert_eq!(run.result, Ok(()));
        assert_eq!(run.out, "EPAS1 rose\nin the second cohort\n");
        assert!(run.err.contains("Terminal · 14 s · private"));
        for id in [private.clone(), leaf::new_entry_id(), "EPAS1".to_string()] {
            let run = go(
                &store,
                &piped(),
                Action::Show {
                    id,
                    format: Format::Json,
                },
            );
            assert_eq!(run.result, Err(Failure::failed(MSG_HISTORY_NOT_FOUND)));
            assert_eq!(run.out, "");
        }
    }

    /// Deleting, clearing and every tier change need a person at a terminal; nothing
    /// touches the store otherwise.
    #[test]
    fn changes_need_a_person_at_a_terminal() {
        let (_root, store) = fresh();
        let id = add(&store, "public one", EntryTier::Public, 0);
        let changes = [
            Action::Delete { id: id.clone() },
            Action::Clear,
            Action::Publicize { id: id.clone() },
            Action::Privatize { id: id.clone() },
        ];
        for action in changes {
            let run = go(&store, &piped(), action.clone());
            assert_eq!(
                run.result,
                Err(Failure::refused(MSG_HISTORY_TTY)),
                "{action:?}"
            );
            assert!(run.err.is_empty(), "the sentence alone: {}", run.err);
            let run = go(&store, &agent(), action.clone());
            assert_eq!(
                run.result,
                Err(Failure::refused(MSG_HISTORY_AGENT)),
                "{action:?}"
            );
        }
        assert_eq!(texts(&store, &terminal()), ["public one"]);
        assert_eq!(tier_of(&store, &id), EntryTier::Public);
    }

    /// Raising a setting is refused under `BIOROUTER_TERMINAL=1` and without a terminal;
    /// lowering works anywhere and asks nothing.
    #[test]
    fn raises_are_refused_in_an_agents_shell_and_lowering_always_works() {
        let (_root, store) = fresh();
        let raise_public = Action::Settings {
            keep_history: None,
            public_by_default: Some(true),
        };
        for facts in [agent(), piped()] {
            let run = go(&store, &facts, raise_public.clone());
            assert_eq!(run.result.as_ref().unwrap_err().exit, 2);
            assert!(!store.settings().public_by_default);
        }
        let lower_keep = Action::Settings {
            keep_history: Some(false),
            public_by_default: Some(false),
        };
        let run = go(&store, &agent(), lower_keep);
        assert_eq!(run.result, Ok(()));
        assert_eq!(run.out, "keep_history: off\npublic_by_default: off\n");
        assert!(!store.settings().keep_history);
        let raise_keep = Action::Settings {
            keep_history: Some(true),
            public_by_default: None,
        };
        let run = go(&store, &agent(), raise_keep.clone());
        assert_eq!(run.result, Err(Failure::refused(MSG_HISTORY_AGENT)));
        assert!(!store.settings().keep_history);
        let run = go(&store, &managed_off(), raise_keep.clone());
        assert_eq!(
            run.result,
            Err(Failure::refused("Your organization turned this off."))
        );
        // At a terminal a raise asks first; public by default also shows its warning.
        let mut script = Script {
            confirms: VecDeque::from([false, true]),
            ..Script::default()
        };
        let run = run_with(&store, &terminal(), &mut script, raise_public.clone());
        assert_eq!(run.out, "Nothing was changed.\n");
        assert!(run.err.contains(MSG_HISTORY_PUBLIC_CONFIRM));
        let run = run_with(&store, &terminal(), &mut script, raise_keep);
        assert_eq!(run.result, Ok(()));
        assert!(store.settings().keep_history);
        assert!(!store.settings().public_by_default);
        // Printing them is a read, allowed anywhere.
        let run = go(
            &store,
            &agent(),
            Action::Settings {
                keep_history: None,
                public_by_default: None,
            },
        );
        assert_eq!(run.out, "keep_history: on\npublic_by_default: off\n");
    }

    /// `publicize` shows its warning and then asks for the typed word: blank cancels, a
    /// wrong word exits 1, and only `public` makes it public. `privatize` asks nothing.
    #[test]
    fn publicize_needs_the_typed_confirmation() {
        let (_root, store) = fresh();
        let id = add(&store, "EPAS1", EntryTier::Private, 0);
        let publicize = Action::Publicize { id: id.clone() };
        let mut script = Script {
            phrases: VecDeque::from([String::new(), "yes".to_string(), " public ".to_string()]),
            ..Script::default()
        };
        let run = run_with(&store, &terminal(), &mut script, publicize.clone());
        assert_eq!(run.result, Ok(()));
        assert_eq!(run.out, "Nothing was changed.\n");
        assert!(run.err.contains(MSG_HISTORY_PUBLICIZE_WARN));
        assert_eq!(tier_of(&store, &id), EntryTier::Private);
        let run = run_with(&store, &terminal(), &mut script, publicize.clone());
        assert_eq!(
            run.result,
            Err(Failure::failed(
                "That did not match public, so nothing was changed."
            ))
        );
        assert_eq!(tier_of(&store, &id), EntryTier::Private);
        let run = run_with(&store, &terminal(), &mut script, publicize.clone());
        assert_eq!(run.out, "The dictation is now public.\n");
        assert_eq!(tier_of(&store, &id), EntryTier::Public);
        assert_eq!(
            script.asked,
            vec!["Type public to make it public, or leave blank to cancel"; 3]
        );
        let run = go(&store, &terminal(), Action::Privatize { id: id.clone() });
        assert_eq!(run.out, "The dictation is now private.\n");
        assert_eq!(tier_of(&store, &id), EntryTier::Private);
        // Managed `history: false` refuses either tier change.
        let run = go(&store, &managed_off(), publicize);
        assert_eq!(run.result.unwrap_err().exit, 2);
    }

    /// A dictation that used private terms gets the stronger warning.
    #[test]
    fn publicize_warns_more_when_private_terms_were_used() {
        let (_root, store) = fresh();
        let mut entry = store
            .get(&add(&store, "seed", EntryTier::Private, 0))
            .unwrap()
            .unwrap();
        entry.id = leaf::new_entry_id();
        entry.private_terms = true;
        entry.created_at += chrono::Duration::seconds(1);
        store.append(entry.clone()).unwrap();
        let mut script = Script {
            phrases: VecDeque::from([String::new()]),
            ..Script::default()
        };
        let run = run_with(
            &store,
            &terminal(),
            &mut script,
            Action::Publicize { id: entry.id },
        );
        assert!(run.err.contains(MSG_HISTORY_PUBLICIZE_WARN_TERMS));
    }

    /// `delete` removes one listable entry; `clear` removes what was listed after the typed
    /// count, keeps anything the person did not see, and both work under managed
    /// `history: false`.
    #[test]
    fn delete_and_clear_take_what_the_person_sees() {
        let (_root, store) = fresh();
        let first = add(&store, "one", EntryTier::Private, 0);
        add(&store, "two", EntryTier::Public, 1);
        let run = go(&store, &managed_off(), Action::Delete { id: first.clone() });
        assert_eq!(run.out, "Deleted 1 dictation.\n");
        let run = go(&store, &terminal(), Action::Delete { id: first });
        assert_eq!(run.result, Err(Failure::failed(MSG_HISTORY_NOT_FOUND)));

        add(&store, "three", EntryTier::Private, 2);
        let mut script = Script {
            phrases: VecDeque::from(["delete 1".to_string(), "delete 2".to_string()]),
            ..Script::default()
        };
        let run = run_with(&store, &managed_off(), &mut script, Action::Clear);
        assert_eq!(run.out, "Nothing was changed.\n");
        assert!(run.err.contains(&msg_clear(2, store.dir())));
        assert_eq!(texts(&store, &terminal()).len(), 2);
        let run = run_with(&store, &managed_off(), &mut script, Action::Clear);
        assert_eq!(run.out, "Deleted 2 dictations.\n");
        assert!(texts(&store, &terminal()).is_empty());
        let run = go(&store, &terminal(), Action::Clear);
        assert_eq!(run.out, format!("{MSG_NOTHING_TO_CLEAR}\n"));
    }

    /// A search matches by the dictionary's fold key and says so when nothing matches; the
    /// limit stops the listing and says how many are left, to the person only.
    #[test]
    fn a_listing_searches_and_limits() {
        let (_root, store) = fresh();
        add(&store, "EPAS1 levels rose", EntryTier::Public, 0);
        add(&store, "HIF-2α was stable", EntryTier::Public, 1);
        add(&store, "epas1 again", EntryTier::Public, 2);
        let run = go(
            &store,
            &terminal(),
            Action::List {
                search: Some("EPAS1".into()),
                limit: 1,
                format: Format::Text,
            },
        );
        assert_eq!(run.out.lines().nth(1), Some("  epas1 again"));
        assert!(run.err.contains("Showing the newest 1 of 2."));
        let run = go(
            &store,
            &terminal(),
            Action::List {
                search: Some("PHD2".into()),
                limit: DEFAULT_LIMIT,
                format: Format::Text,
            },
        );
        assert!(run.err.contains(MSG_HISTORY_NO_MATCH));
        let run = go(
            &store,
            &terminal(),
            Action::List {
                search: Some("x".repeat(MAX_SEARCH_CHARS + 1)),
                limit: DEFAULT_LIMIT,
                format: Format::Text,
            },
        );
        assert_eq!(run.result.unwrap_err().exit, 2);
    }

    /// A history file this version cannot read is refused whole, with MSG-HISTORY-FILE,
    /// and left as it was.
    #[test]
    fn an_unreadable_history_is_left_alone() {
        let (_root, store) = fresh();
        std::fs::create_dir_all(store.dir()).unwrap();
        let bytes = b"{\"format\":\"something else\",\"version\":9}\n";
        std::fs::write(store.history_path(), bytes).unwrap();
        let run = go(&store, &terminal(), list_action(Format::Text));
        assert_eq!(
            run.result,
            Err(Failure::failed(msg_history_file(&store.history_path())))
        );
        let mut script = Script::default();
        let run = run_with(&store, &terminal(), &mut script, Action::Clear);
        assert_eq!(run.result.unwrap_err().exit, 1);
        assert_eq!(std::fs::read(store.history_path()).unwrap(), bytes);
    }

    #[test]
    fn the_history_sentences_are_the_catalogs() {
        let folder = std::path::Path::new("/home/a/.local/share/biorouter/dictation/history");
        let cases: Vec<(String, &str)> = vec![
            (MSG_HISTORY_PUBLICIZE_WARN.into(), "Public dictations may be read by chats on public models."),
            (MSG_HISTORY_PUBLICIZE_WARN_TERMS.into(), "This dictation used your private dictionary terms, so it may contain them. Public dictations may be read by chats on public models."),
            (MSG_HISTORY_PUBLIC_CONFIRM.into(), "New dictations will be public unless they go to a private chat or use your private terms. Public dictations may be read by chats on public models."),
            (MSG_HISTORY_EMPTY.into(), "Text you dictate shows here, so you can copy it again or add its words to your dictionary."),
            (MSG_HISTORY_OFF.into(), "History is off, so new dictations are not saved."),
            (MSG_HISTORY_NO_MATCH.into(), "No dictation contains these words."),
            (MSG_HISTORY_NOT_FOUND.into(), "No dictation with that id."),
            (msg_history_file(folder), "The dictation history file at /home/a/.local/share/biorouter/dictation/history could not be read, so nothing was changed. Fix or remove it, then try again."),
            (msg_clear(2, folder), "This deletes 2 dictations from /home/a/.local/share/biorouter/dictation/history. It cannot be undone."),
            (msg_clear(1, folder), "This deletes 1 dictation from /home/a/.local/share/biorouter/dictation/history. It cannot be undone."),
            (msg_deleted(1), "Deleted 1 dictation."),
            (msg_typed_mismatch(PUBLIC_WORD), "That did not match public, so nothing was changed."),
        ];
        for (sentence, catalog) in cases {
            assert_eq!(sentence, catalog);
        }
        for sentence in [MSG_HISTORY_AGENT, MSG_HISTORY_TTY, MSG_NOTHING_TO_CLEAR] {
            assert!(!sentence.contains('\u{2014}') && !sentence.contains('\u{2013}'));
        }
    }

    #[test]
    fn surfaces_read_as_the_history_page_names_them() {
        let words: Vec<&str> = Surface::ALL.into_iter().map(surface_word).collect();
        assert_eq!(
            words,
            [
                "Chat",
                "Crew",
                "Clipboard",
                "Another app",
                "App preview",
                "Terminal"
            ]
        );
        assert_eq!(length_words(14_200), "14 s");
        assert_eq!(length_words(300), "1 s");
        assert_eq!(length_words(125_000), "2 min 5 s");
        assert_eq!(one_line("a\n b  c"), "a b c");
        assert_eq!(
            one_line(&"x".repeat(LINE_CHARS + 5)).chars().count(),
            LINE_CHARS
        );
    }
}
