//! `biorouter dictionary` (PR-13, DD-75; `docs/dictation/dictionary.md`, "Dictionary
//! commands in the CLI"): the personal dictionary at a terminal. Every command works
//! offline, on the store under its lock; no daemon is needed.
//!
//! Who may do what ("Terminal only rules"):
//!
//! | Situation | Reads | Writes | Tier change | Private rows |
//! |---|---|---|---|---|
//! | An interactive terminal, `BIOROUTER_TERMINAL` unset | yes | yes | after the typed term | listed and reviewed; exported only with `--include-private`, a private model configured, and MSG-EXPORT-PRIVATE |
//! | No terminal (a pipe, cron, `ssh host cmd`) | public rows | refused (exit 2), except `import` | refused | withheld, with no count |
//! | An agent's shell (`BIOROUTER_TERMINAL=1`) | public rows | refused with MSG-DICTIONARY-AGENT (exit 2), `import` too | refused | withheld, with no count |
//! | Managed `dictionary: false` | yes | refused with MSG-POLICY (exit 2); deleting and `export` allowed | refused | as above |
//!
//! `import` is the one write admitted without a terminal, because the sync path pipes
//! into it (`biorouter dictionary export - | ssh <host> biorouter dictionary import -`);
//! it only merges. Every rule here is accident prevention, as DD-76 is for live capture,
//! not a boundary: a shell can unset the variable or read the file.
//!
//! The decisions live in [`run`], which takes the terminal facts, the policy, the store,
//! a [`Prompter`] and the output streams as arguments, so the tests below drive every
//! interactive path without a terminal; [`handle`] gathers the real ones.
//!
//! Chat word suggestions (owner, 2026-11-13): `suggestions` lists what waits for review (a
//! read); `suggestions keep`, `dismiss` and `scan` are writes. Every **Keep** or **Dismiss**
//! of a chat suggestion, from `suggestions`, `accept`, `dismiss`, `review` or the TUI's
//! review, is counted in the store's `chat_learning`, the person's own measure, which
//! raises the chats a word needs once most are dismissed. `scan` waits on the session
//! store, so [`handle`] runs it through [`run_scan`] rather than [`run`].

use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use biorouter::dictation::suggestion_scan::{self, ScanProgress, ScanRefusal, ScanState};
use biorouter::managed::{ManagedPolicy, Rule};
use biorouter::security::dictionary_store::{self, store};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use store::transfer::{self, ImportFormat, RowOutcome};
use store::{
    AddOutcome, AgentWrites, DictionaryStore, Entry, EntryEdit, EntryId, EntrySource, EntryStatus,
    EntryTier, NewTerm, Reach, StoreError, StoreFile, StoreView, UsageScope,
    UserDictionaryTierChange, ValidationCode,
};
use unicode_width::UnicodeWidthStr;

use crate::commands::needs_terminal;

/// Set in an agent's shell; every dictionary write refuses there (P-7).
pub const AGENT_SHELL_ENV: &str = "BIOROUTER_TERMINAL";

/// MSG-DICTIONARY-AGENT.
pub const MSG_DICTIONARY_AGENT: &str =
    "Changing the dictionary needs the person. The agent's terms go through dictionary__add_terms.";
/// A write other than `import` without a terminal (exit 2).
pub const MSG_DICTIONARY_TTY: &str = "Changing the dictionary needs you at an interactive terminal. To add terms from a file or a pipe, use `biorouter dictionary import`.";
/// `export --include-private` where DD-77 does not admit it (exit 2).
pub const MSG_EXPORT_PRIVATE_REFUSED: &str = "Exporting private terms needs you at your own terminal and a private model configured on this computer. Run the command without --include-private to export public terms.";
/// MSG-PUBLICIZE-WARN, before publicizing a private row that came from a chat
/// ([`Entry::came_from_a_chat`]).
pub const MSG_PUBLICIZE_WARN: &str = "This term came from a private chat. Making it public lets it appear when you dictate into chats on public models.";
/// MSG-PUBLICIZE-WARN-NO-CHAT, before publicizing a private row nothing shows came from a
/// chat: one the person added as private, made private, or imported.
pub const MSG_PUBLICIZE_WARN_NO_CHAT: &str = "This term is private. Making it public lets it appear when you dictate into chats on public models.";
/// MSG-EXPORT-PRIVATE, and its two answers.
pub const MSG_EXPORT_PRIVATE: &str =
    "This file leaves Biorouter. It will include your private terms.";
const EXPORT_WITH_PRIVATE: &str = "Export with private terms";
const EXPORT_PUBLIC_ONLY: &str = "Export public terms only";
/// MSG-EMPTY.
pub const MSG_EMPTY: &str =
    "No terms yet. Add the words dictation should spell your way, such as UCSF or EPAS1.";
const MSG_NOTHING_CHANGED: &str = "Nothing was changed.";

/// MSG-SPOKEN-ONE-TERM (exit 2).
pub const MSG_SPOKEN_ONE_TERM: &str = "Use --spoken with one term at a time.";
/// MSG-IMPORT-FORMAT (exit 2).
pub const MSG_IMPORT_FORMAT: &str = "Name the format with --format txt, csv, tsv or json.";

/// MSG-SAME-LETTERS.
pub fn msg_same_letters(written: &str) -> String {
    format!("A term with the same letters exists: {written}")
}

/// "1 term" or "<n> terms".
fn n_terms(count: usize) -> String {
    if count == 1 {
        "1 term".to_string()
    } else {
        format!("{count} terms")
    }
}

/// MSG-CLEAR: what `clear` says before it asks for `delete <n>`.
pub fn msg_clear(count: usize, folder: &Path) -> String {
    format!("This deletes {} from {}.", n_terms(count), folder.display())
}

/// What `clear` reports, from MSG-DICTIONARY-DONE.
fn msg_deleted(count: usize) -> String {
    format!("Deleted {}.", n_terms(count))
}

/// MSG-NO-TERM.
fn msg_no_term(term: &str, tier: Option<EntryTier>) -> String {
    match tier {
        Some(tier) => format!(
            "There is no {} term {term} in the dictionary.",
            tier.as_str()
        ),
        None => format!("There is no term {term} in the dictionary."),
    }
}

/// MSG-TYPED-MISMATCH.
fn msg_typed_mismatch(term: &str) -> String {
    format!("That did not match {term}, so nothing was changed.")
}

/// The name an import's read errors give the input: the file as named, or standard input.
fn import_source(input: &str) -> &str {
    if input == "-" {
        "Standard input"
    } else {
        input
    }
}

/// MSG-IMPORT-NOT-FOUND.
fn msg_import_not_found(input: &str) -> String {
    format!("There is no file at {input}.")
}

/// MSG-IMPORT-UNREADABLE.
fn msg_import_unreadable(input: &str) -> String {
    format!("{} could not be read.", import_source(input))
}

/// MSG-IMPORT-TOO-LARGE.
fn msg_import_too_large(input: &str) -> String {
    format!(
        "{} is larger than 16 MB; split it and import each part.",
        import_source(input)
    )
}

/// MSG-IMPORT-NOT-TEXT.
fn msg_import_not_text(input: &str) -> String {
    format!("{} is not UTF-8 text.", import_source(input))
}

/// MSG-EXPORT-UNWRITABLE.
fn msg_export_unwritable(path: &str, error: &io::Error) -> String {
    format!("{path} could not be written: {error}.")
}

/// What `suggestions` says at a terminal when nothing waits.
pub const MSG_NO_SUGGESTIONS: &str = "No suggestions to review.";
/// What `suggestions scan` says when it could not read the chats or write the dictionary.
pub const MSG_SCAN_FAILED: &str = "Biorouter could not finish reading your chats.";
/// What `suggestions scan` says after it added words.
pub const MSG_SCAN_REVIEW: &str = "Review them with `biorouter dictionary suggestions`.";
/// Below MSG-SCAN-OFF, which names the Dictionary page's switch: the same switch here.
pub const MSG_SCAN_TURN_ON: &str =
    "To turn it on here, run `biorouter dictionary settings --learn-from-chats on`.";
/// A surface that cannot wait for a scan (a session's `run`) names the command that can.
pub const MSG_SCAN_FROM_THE_COMMAND: &str =
    "Run `biorouter dictionary suggestions scan` to find words in your recent chats.";

/// The offer to turn suggestions off, once most were dismissed: `required` is the chats a
/// word now needs, 3 or 4.
pub fn msg_suggest_fewer(required: u32) -> String {
    format!(
        "You dismissed most suggestions, so Biorouter now suggests fewer: a word has to show up in {required} chats. Turn them off with `biorouter dictionary settings --learn-from-chats off`."
    )
}

/// What `suggestions keep` and `dismiss` say of a word or id that names no waiting
/// suggestion.
fn msg_no_suggestion(named: &str) -> String {
    format!("There is no suggestion {named} to review.")
}

/// "1 chat" or "<n> chats".
fn n_chats(count: u32) -> String {
    if count == 1 {
        "1 chat".to_string()
    } else {
        format!("{count} chats")
    }
}

/// The line a running scan draws on stderr.
pub fn line_scan_progress(progress: &ScanProgress) -> String {
    let chat = (progress.chats_read + 1).min(progress.chats_total);
    let words = if progress.words_found == 1 {
        "1 word".to_string()
    } else {
        format!("{} words", progress.words_found)
    };
    format!(
        "Reading chat {chat} of {}, {words} found so far",
        progress.chats_total
    )
}

/// What a scan that finished or was stopped says on stdout.
pub fn msg_scan_ended(progress: &ScanProgress) -> String {
    let found = match progress.words_found {
        0 => "No new words found.".to_string(),
        1 => "1 new word to review.".to_string(),
        n => format!("{n} new words to review."),
    };
    if progress.state == ScanState::Stopped {
        format!(
            "Stopped after {} of {}. {found}",
            progress.chats_read,
            n_chats(progress.chats_total)
        )
    } else {
        format!("Read {}. {found}", n_chats(progress.chats_total))
    }
}

/// A command the dictionary runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    List {
        format: ListFormat,
        status: StatusFilter,
    },
    Add {
        terms: Vec<String>,
        /// Each value a comma separated list.
        spoken: Vec<String>,
        private: bool,
    },
    Edit {
        term: Option<String>,
        id: Option<String>,
        new_term: Option<String>,
        spoken: Option<Vec<String>>,
    },
    Remove {
        terms: Vec<String>,
        ids: Vec<String>,
    },
    Review,
    Accept {
        ids: Vec<String>,
    },
    Dismiss {
        ids: Vec<String>,
    },
    Import {
        input: String,
        format: Option<String>,
        dry_run: bool,
    },
    Export {
        output: Option<String>,
        include_private: bool,
    },
    Publicize {
        term: Option<String>,
        id: Option<String>,
    },
    Privatize {
        term: Option<String>,
        id: Option<String>,
    },
    Settings {
        agent_writes: Option<AgentWrites>,
        learn_from_chats: Option<bool>,
        suggest_from_chats: Option<bool>,
    },
    Clear {
        suggested_only: bool,
    },
    /// `suggestions`: the suggestions waiting for the person (`text` or `json`).
    Suggestions {
        format: ListFormat,
    },
    /// `suggestions keep`: each a written form or an id.
    KeepSuggestions {
        terms: Vec<String>,
    },
    /// `suggestions dismiss`: each a written form or an id.
    DismissSuggestions {
        terms: Vec<String>,
    },
    /// `suggestions scan`: read the person's recent chats once. Only [`handle`] runs it,
    /// because it waits on the session store; [`run`] names the command instead.
    ScanSuggestions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    Text,
    Json,
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    Active,
    Suggested,
    All,
}

impl StatusFilter {
    fn admits(self, status: EntryStatus) -> bool {
        match self {
            Self::Active => status == EntryStatus::Active,
            Self::Suggested => status == EntryStatus::Suggested,
            Self::All => true,
        }
    }
}

/// What a command is, for the authority rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    Write,
    /// Writes that only delete: allowed under managed `dictionary: false` (DD-90).
    Delete,
    Import,
}

impl Action {
    fn kind(&self) -> Kind {
        match self {
            Self::List { .. } | Self::Export { .. } | Self::Suggestions { .. } => Kind::Read,
            Self::Settings {
                agent_writes: None,
                learn_from_chats: None,
                suggest_from_chats: None,
            } => Kind::Read,
            Self::Remove { .. }
            | Self::Dismiss { .. }
            | Self::DismissSuggestions { .. }
            | Self::Clear { .. } => Kind::Delete,
            Self::Import { .. } => Kind::Import,
            _ => Kind::Write,
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
    /// `caller_capability()` is Private; read only for `export --include-private`.
    pub private_capability: bool,
    /// Managed `dictionary`.
    pub dictionary: Rule,
    /// Managed `dictionary_agent_writes`.
    pub agent_writes: Rule,
    /// The account the store belongs to.
    pub account: String,
}

impl Facts {
    /// A person at an interactive terminal lists every row (DD-37); anyone else lists
    /// public rows, with no count of the rest.
    fn reach(&self) -> Reach {
        if self.interactive && !self.agent_shell {
            Reach::All
        } else {
            Reach::Public
        }
    }
}

/// The questions a command asks at a terminal.
pub trait Prompter {
    /// A yes or no question that defaults to no.
    fn confirm(&mut self, question: &str) -> io::Result<bool>;
    /// Typed text; an empty answer cancels.
    fn phrase(&mut self, prompt: &str) -> io::Result<String>;
    /// One of `options`, or `None` to cancel.
    fn choose(&mut self, question: &str, options: &[String]) -> io::Result<Option<usize>>;
    /// Any of `items`.
    fn multiselect(&mut self, question: &str, items: &[String]) -> io::Result<Vec<usize>>;
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

    fn choose(&mut self, question: &str, options: &[String]) -> io::Result<Option<usize>> {
        let mut select = cliclack::select(question);
        for (i, option) in options.iter().enumerate() {
            select = select.item(Some(i), option, "");
        }
        select = select.item(None, "Cancel", "");
        select.interact()
    }

    fn multiselect(&mut self, question: &str, items: &[String]) -> io::Result<Vec<usize>> {
        let mut select = cliclack::multiselect(question).required(false);
        for (i, item) in items.iter().enumerate() {
            select = select.item(i, item, "");
        }
        select.interact()
    }
}

/// The streams a command reads and writes.
pub struct Streams<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub input: &'a mut dyn Read,
}

/// How a command ended without doing its work: exit 2 when it declined to start (its
/// sentence alone on stderr), exit 1 when it failed, and exit 0 when the person
/// cancelled a question ([`Failure::cancelled`]), which [`run`] reports as "Nothing was
/// changed." on stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub exit: u8,
    pub message: String,
}

impl Failure {
    /// Escape at a question, which cliclack reports as `io::ErrorKind::Interrupted`: the
    /// person declined, as a blank answer or **Cancel** does. Every question comes before
    /// a command's one write, so nothing was changed. (Ctrl-C never gets here: `console`
    /// raises SIGINT, which ends the process before any write.)
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
}

impl From<StoreError> for Failure {
    fn from(e: StoreError) -> Self {
        Self::failed(e.to_string())
    }
}

impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Self {
        // Only a prompt answers `Interrupted`: `write_all` and `read_to_end` retry it,
        // and the import's reads map their errors themselves.
        if e.kind() == io::ErrorKind::Interrupted {
            Self::cancelled()
        } else {
            Self::failed(e.to_string())
        }
    }
}

type Outcome = std::result::Result<(), Failure>;

/// Whether this process runs in an agent's shell. Any value but an empty one counts.
fn in_agent_shell() -> bool {
    std::env::var_os(AGENT_SHELL_ENV).is_some_and(|value| !value.is_empty())
}

pub(crate) fn session_facts() -> Facts {
    let policy = ManagedPolicy::load();
    Facts {
        interactive: needs_terminal::prompt_can_run(),
        agent_shell: in_agent_shell(),
        private_capability: false,
        dictionary: policy.dictation().dictionary.clone(),
        agent_writes: policy.dictation().dictionary_agent_writes.clone(),
        account: biorouter::dictation::service::account(),
    }
}

pub(crate) fn authorize_session_action(action: &Action) -> std::result::Result<(), String> {
    authorize(action, &session_facts()).map_err(|failure| failure.message)
}

/// The core's secret scan, which the store cannot call (`validate.rs`).
fn is_secret(text: &str) -> bool {
    biorouter::guardrails::secret_output::redact_text(text).is_some()
}

/// Run `action` from the real terminal, policy and store.
pub async fn handle(action: Action) -> Result<()> {
    let interactive = needs_terminal::prompt_can_run();
    let agent_shell = in_agent_shell();
    let private_capability = match &action {
        Action::Export {
            include_private: true,
            ..
        } if interactive && !agent_shell => crate::session::privacy::caller_capability()
            .await
            .is_private(),
        _ => false,
    };
    let policy = ManagedPolicy::load();
    let facts = Facts {
        interactive,
        agent_shell,
        private_capability,
        dictionary: policy.dictation().dictionary.clone(),
        agent_writes: policy.dictation().dictionary_agent_writes.clone(),
        account: biorouter::dictation::service::account(),
    };
    let store = dictionary_store::open_store();
    let (mut stdin, mut stdout, mut stderr) = (io::stdin(), io::stdout(), io::stderr());
    let outcome = if action == Action::ScanSuggestions {
        scan_here(&facts, &store, &mut stdout, &mut stderr).await
    } else {
        let mut streams = Streams {
            out: &mut stdout,
            err: &mut stderr,
            input: &mut stdin,
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

/// Refuse `action` before it touches the store when this caller may not run it.
fn authorize(action: &Action, facts: &Facts) -> Outcome {
    if let Action::Export {
        include_private: true,
        ..
    } = action
    {
        if facts.agent_shell || !facts.interactive || !facts.private_capability {
            return Err(Failure::refused(MSG_EXPORT_PRIVATE_REFUSED));
        }
    }
    let kind = action.kind();
    if kind == Kind::Read {
        return Ok(());
    }
    if facts.agent_shell {
        return Err(Failure::refused(MSG_DICTIONARY_AGENT));
    }
    if kind != Kind::Delete {
        if let Some(reason) = facts.dictionary.reason() {
            return Err(Failure::refused(reason));
        }
    }
    if kind != Kind::Import && !facts.interactive {
        return Err(Failure::refused(MSG_DICTIONARY_TTY));
    }
    Ok(())
}

/// The line every command starts with on stderr: the folder and the account it acts on.
fn say_where(store: &DictionaryStore, facts: &Facts, err: &mut dyn Write) -> io::Result<()> {
    writeln!(
        err,
        "Dictionary: {} (account {})",
        store.dir().display(),
        facts.account
    )
}

/// Run one command.
pub fn run(
    action: Action,
    facts: &Facts,
    store: &DictionaryStore,
    prompt: &mut dyn Prompter,
    io: &mut Streams<'_>,
) -> Outcome {
    authorize(&action, facts)?;
    say_where(store, facts, io.err)?;
    let reach = facts.reach();
    let outcome = match action {
        Action::List { format, status } => list(store, reach, format, status, io),
        Action::Add {
            terms,
            spoken,
            private,
        } => add(store, &terms, &spoken, private, io),
        Action::Edit {
            term,
            id,
            new_term,
            spoken,
        } => edit(store, prompt, term, id, new_term, spoken, io),
        Action::Remove { terms, ids } => remove(store, prompt, &terms, &ids, io),
        Action::Review => review(store, prompt, io),
        Action::Accept { ids } => accept_or_dismiss(store, &ids, true, io),
        Action::Dismiss { ids } => accept_or_dismiss(store, &ids, false, io),
        Action::Import {
            input,
            format,
            dry_run,
        } => import(store, reach, &input, format.as_deref(), dry_run, io),
        Action::Export {
            output,
            include_private,
        } => export(store, prompt, output.as_deref(), include_private, io),
        Action::Publicize { term, id } => {
            change_tier(store, prompt, term, id, EntryTier::Public, io)
        }
        Action::Privatize { term, id } => {
            change_tier(store, prompt, term, id, EntryTier::Private, io)
        }
        Action::Settings {
            agent_writes,
            learn_from_chats,
            suggest_from_chats,
        } => settings(
            store,
            facts,
            prompt,
            agent_writes,
            learn_from_chats,
            suggest_from_chats,
            io,
        ),
        Action::Clear { suggested_only } => clear(store, prompt, suggested_only, io),
        Action::Suggestions { format } => suggestions(store, reach, format, io),
        Action::KeepSuggestions { terms } => decide_suggestions(store, &terms, true, io),
        Action::DismissSuggestions { terms } => decide_suggestions(store, &terms, false, io),
        Action::ScanSuggestions => Err(Failure::failed(MSG_SCAN_FROM_THE_COMMAND)),
    };
    match outcome {
        Err(failure) if failure.exit == 0 => {
            writeln!(io.out, "{}", failure.message)?;
            Ok(())
        }
        other => other,
    }
}

/// Rows a listing shows: the caller's tiers, the status asked for, and never an `auto`
/// row seen once ("Auto rows with `seen` below 2 are never listed").
fn listed(view: &StoreView, reach: Reach, status: StatusFilter) -> Vec<&Entry> {
    let mut rows: Vec<&Entry> = view
        .entries()
        .iter()
        .filter(|e| reach.lists(e.tier) && status.admits(e.status))
        .filter(|e| !(e.source == EntrySource::Auto && e.seen < 2))
        .collect();
    rows.sort_by(|a, b| {
        a.written
            .to_lowercase()
            .cmp(&b.written.to_lowercase())
            .then_with(|| a.written.cmp(&b.written))
            .then(a.tier.cmp(&b.tier))
    });
    rows
}

/// `EntryView` ("Routes"): the stored row minus `seen`; usage and `suggested_in` only
/// for a caller with private standing; a private row's institutions with their
/// published names.
fn entry_view(entry: &Entry, view: &StoreView, reach: Reach) -> Value {
    let mut row = Map::new();
    row.insert("id".into(), json!(entry.id));
    row.insert("rev".into(), json!(entry.rev));
    row.insert("written".into(), json!(entry.written));
    row.insert("tier".into(), json!(entry.tier));
    row.insert("status".into(), json!(entry.status));
    row.insert("source".into(), json!(entry.source));
    row.insert("reviewed".into(), json!(entry.reviewed));
    row.insert("spoken_forms".into(), json!(entry.spoken_forms));
    let heard: Vec<Value> = entry
        .misrecognitions
        .iter()
        .map(|m| json!({ "text": m.text, "count": m.count }))
        .collect();
    row.insert("misrecognitions".into(), Value::Array(heard));
    row.insert("kind".into(), json!(entry.kind));
    row.insert("match".into(), json!(entry.match_policy));
    row.insert("bias".into(), json!(entry.bias));
    row.insert("context_cues".into(), json!(entry.context_cues));
    row.insert("note".into(), json!(entry.note));
    row.insert("reason".into(), json!(entry.reason));
    row.insert("provenance".into(), json!(entry.provenance));
    let affiliation = match (&entry.affiliation, entry.tier) {
        (Some(stored), EntryTier::Private) => {
            let institutions: Vec<Value> = stored
                .institutions
                .iter()
                .map(|id| {
                    let institution = biorouter::privacy::affiliation::InstitutionId::new(id);
                    let name =
                        biorouter::privacy::affiliation::institution_display_name(institution);
                    json!({ "id": id, "name": name })
                })
                .collect();
            json!({ "institutions": institutions })
        }
        _ => Value::Null,
    };
    row.insert("affiliation".into(), affiliation);
    row.insert("created_at".into(), json!(entry.created_at));
    row.insert("updated_at".into(), json!(entry.updated_at));
    if reach == Reach::All {
        let usage = view.usage(&entry.id, UsageScope::All);
        row.insert("uses".into(), json!(usage.map_or(0, |u| u.uses)));
        row.insert(
            "last_used_at".into(),
            json!(usage.and_then(|u| u.last_used_at)),
        );
        row.insert(
            "corrected_to".into(),
            json!(usage.map_or(0, |u| u.corrected_to)),
        );
        row.insert("suggested_in".into(), json!(entry.suggested_in));
    }
    Value::Object(row)
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn list(
    store: &DictionaryStore,
    reach: Reach,
    format: ListFormat,
    status: StatusFilter,
    io: &mut Streams<'_>,
) -> Outcome {
    let view = store.view()?;
    let rows = listed(&view, reach, status);
    if format == ListFormat::Json {
        let views: Vec<Value> = rows.iter().map(|e| entry_view(e, &view, reach)).collect();
        writeln!(
            io.out,
            "{}",
            serde_json::to_string_pretty(&Value::Array(views)).unwrap_or_default()
        )?;
        return Ok(());
    }
    let uses = |entry: &Entry| match reach {
        Reach::All => view
            .usage(&entry.id, UsageScope::All)
            .map_or(0, |u| u.uses)
            .to_string(),
        Reach::Public => "-".to_string(),
    };
    let table: Vec<[String; 6]> = rows
        .iter()
        .map(|e| {
            [
                e.written.clone(),
                e.status.as_str().to_string(),
                e.tier.as_str().to_string(),
                e.source.as_str().to_string(),
                uses(e),
                e.id.to_string(),
            ]
        })
        .collect();
    let header = ["written", "status", "tier", "source", "uses", "id"].map(str::to_string);
    if format == ListFormat::Csv {
        for line in std::iter::once(&header).chain(&table) {
            let cells: Vec<String> = line.iter().map(|cell| csv_field(cell)).collect();
            writeln!(io.out, "{}", cells.join(","))?;
        }
        return Ok(());
    }
    if table.is_empty() {
        if reach == Reach::All {
            writeln!(io.err, "{MSG_EMPTY}")?;
        }
        return Ok(());
    }
    let header = header.map(|h| h.to_uppercase());
    let widths: Vec<usize> = (0..6)
        .map(|column| {
            std::iter::once(&header)
                .chain(&table)
                .map(|line| line[column].width())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for line in std::iter::once(&header).chain(&table) {
        let mut text = String::new();
        for (column, cell) in line.iter().enumerate() {
            text.push_str(cell);
            if column + 1 < line.len() {
                text.push_str(&" ".repeat(widths[column] - cell.width() + 2));
            }
        }
        writeln!(io.out, "{text}")?;
    }
    Ok(())
}

/// Split `--spoken` values on commas.
fn spoken_forms(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|form| !form.is_empty())
        .map(str::to_string)
        .collect()
}

/// The validator's code for a term the core's secret scan flags.
fn refuse_secrets<'a>(texts: impl IntoIterator<Item = &'a String>) -> Result<(), ValidationCode> {
    if texts.into_iter().any(|text| is_secret(text)) {
        Err(ValidationCode::SecretShaped)
    } else {
        Ok(())
    }
}

/// `add`: public unless `--private` (O-6: a terminal add starts from no chat), and a
/// private term added here carries no institution (O-36).
fn add(
    store: &DictionaryStore,
    terms: &[String],
    spoken: &[String],
    private: bool,
    io: &mut Streams<'_>,
) -> Outcome {
    if terms.len() > 1 && !spoken.is_empty() {
        return Err(Failure::refused(MSG_SPOKEN_ONE_TERM));
    }
    let tier = if private {
        EntryTier::Private
    } else {
        EntryTier::Public
    };
    let forms = spoken_forms(spoken);
    let mut failures = Vec::new();
    for raw in terms {
        let written = store::validate::normalize(raw);
        let term = NewTerm {
            spoken_forms: forms.clone(),
            ..NewTerm::person(raw.as_str(), tier)
        };
        let outcome = refuse_secrets(std::iter::once(&written).chain(&forms))
            .map_err(StoreError::Invalid)
            .and_then(|()| store.write(|file| file.add(term, Reach::All, store::schema::now())));
        match outcome {
            Ok(AddOutcome::Created { same_letters, .. }) => {
                writeln!(io.out, "Added {written} ({}).", tier.as_str())?;
                if let Some(other) = same_letters {
                    writeln!(io.err, "{}", msg_same_letters(&other))?;
                }
            }
            Ok(AddOutcome::Exists { .. } | AddOutcome::Skipped) => {
                writeln!(
                    io.out,
                    "{written} is already in the dictionary ({}).",
                    tier.as_str()
                )?;
            }
            Err(e @ StoreError::Unreadable { .. }) => return Err(e.into()),
            Err(e) => failures.push(format!("{raw}: {e}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Failure::failed(failures.join("\n")))
    }
}

/// The one row a command names: by `--id`, or by its written form among `rows`
/// (twins ask which). `Ok(None)` when the person cancelled.
fn resolve(
    view: &StoreView,
    prompt: &mut dyn Prompter,
    term: Option<&str>,
    id: Option<&str>,
    tier: Option<EntryTier>,
) -> std::result::Result<Option<Entry>, Failure> {
    if let Some(id) = id {
        let found = EntryId::parse(id)
            .and_then(|id| view.find(&id).cloned())
            .filter(|e| tier.is_none_or(|t| t == e.tier));
        return found
            .map(Some)
            .ok_or_else(|| Failure::failed(store::MSG_NOT_FOUND));
    }
    let term = term.unwrap_or_default();
    let rows: Vec<&Entry> = view
        .file()
        .rows_written(term, Reach::All)
        .into_iter()
        .filter(|e| tier.is_none_or(|t| t == e.tier))
        .collect();
    match rows.as_slice() {
        [] => Err(Failure::failed(msg_no_term(term, tier))),
        [only] => Ok(Some((*only).clone())),
        twins => {
            let options: Vec<String> = twins
                .iter()
                .map(|e| format!("{} ({}, {})", e.written, e.tier.as_str(), e.id))
                .collect();
            let question = format!("{term} is both a public and a private term. Which one?");
            Ok(prompt
                .choose(&question, &options)?
                .and_then(|i| twins.get(i))
                .map(|e| (*e).clone()))
        }
    }
}

fn edit(
    store: &DictionaryStore,
    prompt: &mut dyn Prompter,
    term: Option<String>,
    id: Option<String>,
    new_term: Option<String>,
    spoken: Option<Vec<String>>,
    io: &mut Streams<'_>,
) -> Outcome {
    let view = store.view()?;
    let Some(row) = resolve(&view, prompt, term.as_deref(), id.as_deref(), None)? else {
        writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
        return Ok(());
    };
    let spoken = spoken.as_deref().map(spoken_forms);
    let written = new_term.as_deref().map(store::validate::normalize);
    refuse_secrets(written.iter().chain(spoken.iter().flatten()))
        .map_err(|code| Failure::failed(code.message()))?;
    let change = EntryEdit {
        written: new_term,
        spoken_forms: spoken,
        ..EntryEdit::default()
    };
    store.write(|file| file.edit(&row.id, Some(row.rev), change, store::schema::now()))?;
    writeln!(
        io.out,
        "Changed {}.",
        written.as_deref().unwrap_or(&row.written)
    )?;
    Ok(())
}

fn remove(
    store: &DictionaryStore,
    prompt: &mut dyn Prompter,
    terms: &[String],
    ids: &[String],
    io: &mut Streams<'_>,
) -> Outcome {
    let view = store.view()?;
    let mut doomed: Vec<Entry> = Vec::new();
    let mut failures = Vec::new();
    for term in terms {
        match resolve(&view, prompt, Some(term), None, None) {
            Ok(Some(row)) => doomed.push(row),
            Ok(None) => {}
            // Escape at a twin question cancels the whole command, before its write.
            Err(failure) if failure.exit == 0 => return Err(failure),
            Err(failure) => failures.push(failure.message),
        }
    }
    for id in ids {
        match resolve(&view, prompt, None, Some(id), None) {
            Ok(Some(row)) => doomed.push(row),
            Ok(None) => {}
            Err(failure) => failures.push(format!("{id}: {}", failure.message)),
        }
    }
    let removed = store.write(|file| {
        let now = store::schema::now();
        Ok(doomed
            .iter()
            .filter(|row| file.remove(&row.id, now).is_ok())
            .map(|row| row.written.clone())
            .collect::<Vec<_>>())
    })?;
    if removed.is_empty() && failures.is_empty() {
        // The person chose **Cancel** at every twin question, or another process
        // removed the rows first.
        writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
    }
    for written in removed {
        writeln!(io.out, "Removed {written}.")?;
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Failure::failed(failures.join("\n")))
    }
}

/// A row `review` offers: a suggestion, or an agent row the person has not kept.
fn awaits_review(entry: &Entry) -> bool {
    entry.status == EntryStatus::Suggested
        || (entry.source == EntrySource::Agent && !entry.reviewed)
}

/// Dismiss the rows a review offered, from an earlier read. Each one still a suggestion
/// goes through [`StoreFile::dismiss_suggestions`], so a dismissed chat suggestion counts
/// as a **Dismiss** in the person's own measure; each one still an agent row the person
/// has not kept is deleted with its tombstone. A row another surface accepted or kept
/// meanwhile stays. How many went. The terminal's `review` and the TUI's review share it.
pub(crate) fn dismiss_pending(file: &mut StoreFile, ids: &[EntryId], at: DateTime<Utc>) -> usize {
    let suggestions = file.dismiss_suggestions(ids, at).len();
    suggestions + file.remove_confirmed(ids, awaits_review, at).len()
}

/// The offer to turn suggestions off, when it applies: suggestions are on, most of the
/// person's recent decisions were dismissals, and the person has not chosen to keep them
/// on within the last 20 decisions (as the daemon's `offer_turning_off`).
pub(crate) fn offer_turning_off(file: &StoreFile) -> Option<String> {
    (file.settings.learn_from_chats && file.chat_learning.offers_turning_off())
        .then(|| msg_suggest_fewer(file.chat_learning.required_chats()))
}

/// `review`: suggestions and the agent's unreviewed rows; the chosen ones are accepted
/// or kept, then the person may dismiss the rest. The prompt can wait as long as the
/// person takes, so a row another surface kept or accepted meanwhile is not dismissed.
fn review(store: &DictionaryStore, prompt: &mut dyn Prompter, io: &mut Streams<'_>) -> Outcome {
    let view = store.view()?;
    let pending: Vec<&Entry> = listed(&view, Reach::All, StatusFilter::All)
        .into_iter()
        .filter(|e| awaits_review(e))
        .collect();
    if pending.is_empty() {
        writeln!(io.out, "Nothing to review.")?;
        return Ok(());
    }
    let labels: Vec<String> = pending
        .iter()
        .map(|e| {
            let mut label = format!("{} · {}", e.written, e.source.as_str());
            if e.tier.is_private() {
                label.push_str(" · private");
            }
            if let Some(reason) = &e.reason {
                label.push_str(" · ");
                label.push_str(reason);
            }
            label
        })
        .collect();
    let chosen = prompt.multiselect("Choose the terms to keep", &labels)?;
    let (keep, rest): (Vec<_>, Vec<_>) = pending
        .iter()
        .enumerate()
        .partition(|(i, _)| chosen.contains(i));
    let dismiss = !rest.is_empty() && prompt.confirm("Dismiss the rest?")?;
    let rest: Vec<EntryId> = rest.iter().map(|(_, e)| e.id.clone()).collect();
    let (kept, dismissed, offer) = store.write(|file| {
        let now = store::schema::now();
        let kept = keep
            .iter()
            .filter(|(_, e)| file.accept(&e.id, now).is_ok())
            .count();
        let dismissed = if dismiss {
            dismiss_pending(file, &rest, now)
        } else {
            0
        };
        let offer = offer_turning_off(file).filter(|_| kept + dismissed > 0);
        Ok((kept, dismissed, offer))
    })?;
    writeln!(io.out, "Kept {kept}. Dismissed {dismissed}.")?;
    if let Some(offer) = offer {
        writeln!(io.out, "{offer}")?;
    }
    Ok(())
}

fn accept_or_dismiss(
    store: &DictionaryStore,
    ids: &[String],
    accept: bool,
    io: &mut Streams<'_>,
) -> Outcome {
    let (done, failures) = store.write(|file| {
        let now = store::schema::now();
        let mut done = Vec::new();
        let mut failures = Vec::new();
        for raw in ids {
            let Some(id) = EntryId::parse(raw).filter(|id| {
                file.find(id)
                    .is_some_and(|entry| accept || entry.status == EntryStatus::Suggested)
            }) else {
                failures.push(format!("{raw}: {}", store::MSG_NOT_FOUND));
                continue;
            };
            let written = file
                .find(&id)
                .map(|e| e.written.clone())
                .unwrap_or_default();
            // A dismissed chat suggestion counts as a **Dismiss** in the person's own
            // measure, as an accepted one counts as a **Keep** inside `accept`.
            let result = if accept {
                file.accept(&id, now)
            } else if file
                .dismiss_suggestions(std::slice::from_ref(&id), now)
                .is_empty()
            {
                Err(StoreError::NotFound)
            } else {
                Ok(())
            };
            match result {
                Ok(()) => done.push(written),
                Err(e) => failures.push(format!("{raw}: {e}")),
            }
        }
        Ok((done, failures))
    })?;
    let verb = if accept { "Accepted" } else { "Dismissed" };
    for written in done {
        writeln!(io.out, "{verb} {written}.")?;
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Failure::failed(failures.join("\n")))
    }
}

/// The most an import reads, so a mistaken pipe cannot fill memory.
const MAX_IMPORT_BYTES: u64 = 16 * 1024 * 1024;

fn read_import(input: &str, io: &mut Streams<'_>) -> std::result::Result<String, Failure> {
    let mut bytes = Vec::new();
    let read = if input == "-" {
        io.input.take(MAX_IMPORT_BYTES + 1).read_to_end(&mut bytes)
    } else {
        match std::fs::File::open(input) {
            Ok(file) => file.take(MAX_IMPORT_BYTES + 1).read_to_end(&mut bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Failure::failed(msg_import_not_found(input)))
            }
            Err(e) => Err(e),
        }
    };
    read.map_err(|_| Failure::failed(msg_import_unreadable(input)))?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err(Failure::failed(msg_import_too_large(input)));
    }
    String::from_utf8(bytes).map_err(|_| Failure::failed(msg_import_not_text(input)))
}

/// `import`: merges, never deletes. Rows marked private import private, the rest
/// public. The plan counts only rows the caller can list (oracle rule 3): without a
/// terminal, the rows a file marks private are reported together, never as new or as
/// already present.
fn import(
    store: &DictionaryStore,
    reach: Reach,
    input: &str,
    format: Option<&str>,
    dry_run: bool,
    io: &mut Streams<'_>,
) -> Outcome {
    let content = read_import(input, io)?;
    let format = match format.and_then(ImportFormat::parse) {
        Some(format) => format,
        None if input == "-" && transfer::looks_like_export(&content) => ImportFormat::Json,
        None => match ImportFormat::from_extension(Path::new(input)).filter(|_| input != "-") {
            Some(format) => format,
            None => return Err(Failure::refused(MSG_IMPORT_FORMAT)),
        },
    };
    let rows =
        transfer::parse_import(format, &content).map_err(|e| Failure::failed(e.to_string()))?;
    let now = store::schema::now();
    let plan = if dry_run {
        let view = store.view()?;
        let mut copy = view.file().clone();
        copy.import(rows, EntryTier::Public, &is_secret, now)
    } else {
        store.write(|file| Ok(file.import(rows, EntryTier::Public, &is_secret, now)))?
    };

    let (mut new, mut merged, mut duplicate, mut withheld) = (0, 0, 0, 0);
    let mut invalid: Vec<(usize, &'static str)> = Vec::new();
    for row in &plan.rows {
        let listed = reach.lists(row.tier);
        match &row.outcome {
            RowOutcome::Invalid { code } if listed || *code != "cap" => {
                invalid.push((row.line, *code))
            }
            _ if !listed => withheld += 1,
            RowOutcome::New => new += 1,
            RowOutcome::Merged => merged += 1,
            RowOutcome::Duplicate => duplicate += 1,
            RowOutcome::Invalid { .. } => {}
        }
    }
    let lead = if dry_run { "Dry run" } else { "Imported" };
    writeln!(
        io.out,
        "{lead}: {new} new, {merged} merged, {duplicate} duplicate, {} invalid.",
        invalid.len()
    )?;
    if withheld > 0 {
        writeln!(io.out, "Private terms received: {withheld}.")?;
    }
    for (line, code) in invalid {
        let why = match code {
            "cap" => store::MSG_DICTIONARY_CAP,
            code => ValidationCode::parse(code)
                .map_or(transfer::MSG_IMPORT_ROW_UNREADABLE, ValidationCode::message),
        };
        writeln!(io.out, "line {line}: {code}: {why}")?;
    }
    Ok(())
}

/// `export`: public rows, and private rows only after MSG-EXPORT-PRIVATE.
fn export(
    store: &DictionaryStore,
    prompt: &mut dyn Prompter,
    output: Option<&str>,
    include_private: bool,
    io: &mut Streams<'_>,
) -> Outcome {
    let mut reach = Reach::Public;
    if include_private {
        let options = [
            EXPORT_WITH_PRIVATE.to_string(),
            EXPORT_PUBLIC_ONLY.to_string(),
        ];
        let choice = match prompt.choose(MSG_EXPORT_PRIVATE, &options) {
            // Escape answers as **Cancel** does.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => None,
            other => other?,
        };
        match choice {
            Some(0) => reach = Reach::All,
            Some(_) => {}
            None => {
                writeln!(io.err, "Nothing was exported.")?;
                return Ok(());
            }
        }
    }
    let view = store.view()?;
    let mut text = serde_json::to_string_pretty(&view.file().export(reach, store::schema::now()))
        .unwrap_or_default();
    text.push('\n');
    match output.filter(|o| *o != "-") {
        None => io.out.write_all(text.as_bytes())?,
        Some(path) => {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            if reach == Reach::All {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(path)
                .and_then(|mut file| {
                    // `mode` applies only to a file this call creates. One that was
                    // already there, such as an earlier public export left at 0644, is
                    // narrowed while it is still empty, before a private term is in it.
                    #[cfg(unix)]
                    if reach == Reach::All {
                        use std::os::unix::fs::PermissionsExt;
                        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                    }
                    file.write_all(text.as_bytes())
                })
                .map_err(|e| Failure::failed(msg_export_unwritable(path, &e)))?;
            writeln!(io.err, "Wrote {path}.")?;
        }
    }
    Ok(())
}

/// `publicize` and `privatize`: the person types the term exactly (O-5), and only then
/// is the tier proof built, here and nowhere else in this crate.
fn change_tier(
    store: &DictionaryStore,
    prompt: &mut dyn Prompter,
    term: Option<String>,
    id: Option<String>,
    tier: EntryTier,
    io: &mut Streams<'_>,
) -> Outcome {
    let view = store.view()?;
    let from = if tier.is_private() {
        EntryTier::Public
    } else {
        EntryTier::Private
    };
    let Some(row) = resolve(&view, prompt, term.as_deref(), id.as_deref(), Some(from))? else {
        writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
        return Ok(());
    };
    if tier == EntryTier::Public {
        let warning = if row.came_from_a_chat() {
            MSG_PUBLICIZE_WARN
        } else {
            MSG_PUBLICIZE_WARN_NO_CHAT
        };
        writeln!(io.err, "{warning}")?;
    }
    let verb = if tier.is_private() {
        "private"
    } else {
        "public"
    };
    let typed = prompt.phrase(&format!(
        "Type {} to make it {verb}, or leave blank to cancel",
        row.written
    ))?;
    if typed.trim().is_empty() {
        writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
        return Ok(());
    }
    if typed.trim() != row.written {
        return Err(Failure::failed(msg_typed_mismatch(&row.written)));
    }
    let proof = UserDictionaryTierChange::from_user_action();
    store.write(|file| file.set_tier(&row.id, tier, &proof, store::schema::now()))?;
    writeln!(io.out, "{} is now {verb}.", row.written)?;
    Ok(())
}

fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
    }
}

/// `settings`: with no flag, print them. A flag that raises `agent_writes` or turns
/// `learn_from_chats` on asks first; lowering asks nothing. Managed
/// `dictionary_agent_writes: false` refuses raising either.
fn settings(
    store: &DictionaryStore,
    facts: &Facts,
    prompt: &mut dyn Prompter,
    agent_writes: Option<AgentWrites>,
    learn_from_chats: Option<bool>,
    suggest_from_chats: Option<bool>,
    io: &mut Streams<'_>,
) -> Outcome {
    let current = store.view()?.settings().clone();
    let current_suggest = biorouter::dictation::activation::suggest_from_chats();
    if agent_writes.is_some() || learn_from_chats.is_some() || suggest_from_chats.is_some() {
        let raises = agent_writes.is_some_and(|mode| mode > current.agent_writes)
            || (learn_from_chats == Some(true) && !current.learn_from_chats)
            || (suggest_from_chats == Some(true) && !current_suggest);
        if raises {
            if let Some(reason) = facts.agent_writes.reason() {
                return Err(Failure::refused(reason));
            }
            if !prompt.confirm("Change this setting?")? {
                writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
                return Ok(());
            }
        }
        store.write(|file| {
            if let Some(mode) = agent_writes {
                file.settings.agent_writes = mode;
            }
            if let Some(learn) = learn_from_chats {
                // Turning suggestions on is the person choosing them, so the offer to turn
                // them off waits for more decisions before it is made again, as the
                // daemon's `POST /dictionary/settings` does.
                if learn && !file.settings.learn_from_chats {
                    file.chat_learning.choose_on();
                }
                file.settings.learn_from_chats = learn;
            }
            Ok(())
        })?;
        if let Some(suggest) = suggest_from_chats {
            biorouter::dictation::activation::activate(suggest)
                .map_err(|error| Failure::failed(error.to_string()))?;
        }
    }
    let view = store.view()?;
    let settings = view.settings();
    writeln!(io.out, "agent_writes: {}", settings.agent_writes.as_str())?;
    writeln!(
        io.out,
        "suggest_from_chats: {}",
        on_off(biorouter::dictation::activation::suggest_from_chats())
    )?;
    writeln!(
        io.out,
        "learn_from_chats: {}",
        on_off(settings.learn_from_chats)
    )?;
    let learning = &view.file().chat_learning;
    if learning.decisions() > 0 {
        writeln!(
            io.out,
            "suggestions: kept {} of {} recent; a word needs {} chats",
            learning.recent_kept(),
            learning.recent.len(),
            learning.required_chats()
        )?;
    }
    Ok(())
}

/// `clear`: the rows `list` shows, or only its suggestions, after the person types
/// `delete <n>`. The count and the rows deleted are the ones the person saw: the prompt
/// can wait as long as the person takes, so a row another process adds meanwhile (an
/// agent's term, a suggestion) is kept, a suggestion accepted meanwhile is kept by
/// `--suggested-only`, and the report says how many went.
fn clear(
    store: &DictionaryStore,
    prompt: &mut dyn Prompter,
    suggested_only: bool,
    io: &mut Streams<'_>,
) -> Outcome {
    let filter = if suggested_only {
        StatusFilter::Suggested
    } else {
        StatusFilter::All
    };
    let view = store.view()?;
    let seen: Vec<EntryId> = listed(&view, Reach::All, filter)
        .into_iter()
        .map(|e| e.id.clone())
        .collect();
    let count = seen.len();
    if count == 0 {
        writeln!(io.out, "There are no terms to delete.")?;
        return Ok(());
    }
    writeln!(io.err, "{}", msg_clear(count, store.dir()))?;
    let phrase = format!("delete {count}");
    let typed = prompt.phrase(&format!(
        "Type {phrase} to confirm, or leave blank to cancel"
    ))?;
    if typed.trim() != phrase {
        writeln!(io.out, "{MSG_NOTHING_CHANGED}")?;
        return Ok(());
    }
    let deleted = store.write(|file| {
        let still = |e: &Entry| filter.admits(e.status);
        Ok(file
            .remove_confirmed(&seen, still, store::schema::now())
            .len())
    })?;
    writeln!(io.out, "{}", msg_deleted(deleted))?;
    Ok(())
}

/// Where a row came from, as the Dictionary page labels it, in the lower case a line of
/// text uses.
fn source_label(source: EntrySource) -> &'static str {
    match source {
        EntrySource::User => "added by you",
        EntrySource::Import => "imported",
        EntrySource::Agent => "added by the agent",
        EntrySource::Auto => "found in your chats",
        EntrySource::Correction => "learned from your corrections",
    }
}

/// One line of `suggestions`: the word, where it came from, "private" for a private row,
/// the agent's reason, and the id `keep` and `dismiss` take.
fn suggestion_line(entry: &Entry) -> String {
    let mut line = format!("{} · {}", entry.written, source_label(entry.source));
    if entry.tier.is_private() {
        line.push_str(" · private");
    }
    if let Some(reason) = &entry.reason {
        line.push_str(" · ");
        line.push_str(reason);
    }
    line.push_str(" · ");
    line.push_str(entry.id.as_str());
    line
}

/// `suggestions`: the suggestions waiting for the person, the rows `list --status
/// suggested` shows to this caller; then, while suggestions are held back because most
/// were dismissed, the offer to turn them off. JSON carries the rows and the person's own
/// measure, as the daemon's settings answer does.
fn suggestions(
    store: &DictionaryStore,
    reach: Reach,
    format: ListFormat,
    io: &mut Streams<'_>,
) -> Outcome {
    let view = store.view()?;
    let rows = listed(&view, reach, StatusFilter::Suggested);
    let learning = &view.file().chat_learning;
    let offer = offer_turning_off(view.file());
    if format == ListFormat::Json {
        let rows: Vec<Value> = rows.iter().map(|e| entry_view(e, &view, reach)).collect();
        let answer = json!({
            "suggestions": rows,
            "required_chats": learning.required_chats(),
            "recent_kept": learning.recent_kept(),
            "recent_decisions": learning.recent.len(),
            "offer_turning_off": offer.is_some(),
        });
        writeln!(
            io.out,
            "{}",
            serde_json::to_string_pretty(&answer).unwrap_or_default()
        )?;
        return Ok(());
    }
    for row in &rows {
        writeln!(io.out, "{}", suggestion_line(row))?;
    }
    if rows.is_empty() && reach == Reach::All {
        writeln!(io.err, "{MSG_NO_SUGGESTIONS}")?;
    }
    if let Some(offer) = offer {
        if !rows.is_empty() {
            writeln!(io.out)?;
        }
        writeln!(io.out, "{offer}")?;
    }
    Ok(())
}

/// A suggestion waiting for the person, as a listing shows it: never an `auto` row seen
/// once.
fn waiting_suggestion(entry: &Entry) -> bool {
    entry.status == EntryStatus::Suggested && !(entry.source == EntrySource::Auto && entry.seen < 2)
}

/// The waiting suggestions `named` names: the one with that id, else every one whose
/// written form folds to the same key (a word suggested both public and private names
/// both).
fn suggestions_named(file: &StoreFile, named: &str) -> Vec<EntryId> {
    if let Some(id) = EntryId::parse(named) {
        if file.find(&id).is_some_and(waiting_suggestion) {
            return vec![id];
        }
    }
    let fold = store::fold_key(named);
    file.entries
        .iter()
        .filter(|e| waiting_suggestion(e) && e.fold_key() == fold)
        .map(|e| e.id.clone())
        .collect()
}

/// What `suggestions keep` and `dismiss` did: the rows decided, the sentences for what
/// they could not do, and the offer to turn suggestions off when it now applies.
struct Decided {
    rows: Vec<Entry>,
    failures: Vec<String>,
    offer: Option<String>,
}

/// Keep or dismiss the waiting suggestions `named` names, under the store's lock. Keeping
/// goes through `accept` and dismissing through `dismiss_suggestions`, so each decision on
/// a chat suggestion is counted in the person's own measure.
fn decide_named(file: &mut StoreFile, named: &[String], keep: bool) -> Decided {
    let now = store::schema::now();
    let mut ids: Vec<EntryId> = Vec::new();
    let mut failures = Vec::new();
    for name in named {
        let found = suggestions_named(file, name);
        if found.is_empty() {
            failures.push(msg_no_suggestion(name));
        }
        for id in found {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let rows = if keep {
        let mut kept = Vec::new();
        for id in &ids {
            let Some(row) = file.find(id).cloned() else {
                continue;
            };
            match file.accept(id, now) {
                Ok(()) => kept.push(row),
                Err(e) => failures.push(format!("{}: {e}", row.written)),
            }
        }
        kept
    } else {
        file.dismiss_suggestions(&ids, now)
    };
    let offer = offer_turning_off(file).filter(|_| !rows.is_empty());
    Decided {
        rows,
        failures,
        offer,
    }
}

/// `suggestions keep` and `suggestions dismiss`.
fn decide_suggestions(
    store: &DictionaryStore,
    named: &[String],
    keep: bool,
    io: &mut Streams<'_>,
) -> Outcome {
    let decided = store.write(|file| Ok(decide_named(file, named, keep)))?;
    let verb = if keep { "Kept" } else { "Dismissed" };
    for row in &decided.rows {
        let private = if row.tier.is_private() {
            " (private)"
        } else {
            ""
        };
        writeln!(io.out, "{verb} {}{private}.", row.written)?;
    }
    if let Some(offer) = decided.offer {
        writeln!(io.out, "{offer}")?;
    }
    if decided.failures.is_empty() {
        Ok(())
    } else {
        Err(Failure::failed(decided.failures.join("\n")))
    }
}

/// What runs a scan: [`suggestion_scan::scan`] over this account's session store, or a
/// test's stand in.
pub(crate) trait ChatScanner {
    async fn scan(
        &mut self,
        stop: &AtomicBool,
        report: &mut (dyn FnMut(&ScanProgress) + Send),
    ) -> std::result::Result<ScanProgress, ScanRefusal>;
}

/// The person's own chats, in this account's session store.
struct SessionScanner;

impl ChatScanner for SessionScanner {
    async fn scan(
        &mut self,
        stop: &AtomicBool,
        report: &mut (dyn FnMut(&ScanProgress) + Send),
    ) -> std::result::Result<ScanProgress, ScanRefusal> {
        let sessions = biorouter::session::SessionManager::instance();
        suggestion_scan::scan(&sessions, stop, report).await
    }
}

/// Where a scan writes: its last sentences on `out`, its progress on `err`, in place when
/// `err` is a terminal and otherwise as one line at the end.
pub(crate) struct ScanStreams<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut (dyn Write + Send),
    pub in_place: bool,
}

/// The progress line a running scan draws. It holds only the error stream, which the
/// scan's report may carry across threads.
struct ProgressLine<'a> {
    err: &'a mut (dyn Write + Send),
    in_place: bool,
    last: Option<String>,
}

impl ProgressLine<'_> {
    fn show(&mut self, progress: &ScanProgress) {
        if progress.state != ScanState::Running || progress.chats_total == 0 {
            return;
        }
        let line = line_scan_progress(progress);
        if self.in_place {
            // A progress line that cannot be drawn is not worth stopping the scan for.
            let _ = write!(self.err, "\r\x1b[2K{line}");
            let _ = self.err.flush();
        }
        self.last = Some(line);
    }

    /// Clear the line drawn in place, or write the last one.
    fn finish(self) -> io::Result<()> {
        match self.last {
            Some(_) if self.in_place => {
                write!(self.err, "\r\x1b[2K")?;
                self.err.flush()
            }
            Some(line) => writeln!(self.err, "{line}"),
            None => Ok(()),
        }
    }
}

/// A scan that did not start, or could not: its sentence, exit 1. Below MSG-SCAN-OFF, the
/// command that turns suggestions on here.
fn scan_refused(refusal: ScanRefusal) -> Failure {
    let mut message = refusal.message().to_string();
    if refusal == ScanRefusal::Off {
        message.push('\n');
        message.push_str(MSG_SCAN_TURN_ON);
    }
    Failure::failed(message)
}

/// Whether a scan may start, from the store this command was given: managed policy keeps
/// the agent's automatic writes on, the store reads, and suggestions are on. The scan asks
/// the same of the account's store again before it reads a chat.
fn scan_admitted(facts: &Facts, store: &DictionaryStore) -> Outcome {
    if facts.agent_writes.reason().is_some() {
        return Err(scan_refused(ScanRefusal::Policy));
    }
    let view = store
        .view()
        .map_err(|_| scan_refused(ScanRefusal::Unreadable))?;
    if !view.settings().learn_from_chats {
        return Err(scan_refused(ScanRefusal::Off));
    }
    Ok(())
}

/// `suggestions scan`: a person's write, so it is refused in an agent's shell and without
/// a terminal, as `accept` is. It reads the person's own chats on public models from the
/// last 30 days (at most 100) and writes the words found in enough of them as suggestions.
/// `stop` ends it after the chat it is reading, keeping what it found.
pub(crate) async fn run_scan(
    facts: &Facts,
    store: &DictionaryStore,
    scanner: &mut impl ChatScanner,
    stop: &AtomicBool,
    streams: &mut ScanStreams<'_>,
) -> Outcome {
    authorize(&Action::ScanSuggestions, facts)?;
    say_where(store, facts, &mut *streams.err)?;
    scan_admitted(facts, store)?;
    let mut line = ProgressLine {
        err: &mut *streams.err,
        in_place: streams.in_place,
        last: None,
    };
    let ended = scanner.scan(stop, &mut |p| line.show(p)).await;
    line.finish()?;
    let progress = ended.map_err(scan_refused)?;
    if progress.state == ScanState::Failed {
        return Err(Failure::failed(MSG_SCAN_FAILED));
    }
    writeln!(streams.out, "{}", msg_scan_ended(&progress))?;
    if progress.words_found > 0 {
        writeln!(streams.out, "{MSG_SCAN_REVIEW}")?;
    }
    Ok(())
}

/// Set `stop` at the first Ctrl+C, so the scan stops after the chat it is reading and
/// keeps what it found; a second Ctrl+C ends the process at once.
async fn stop_on_ctrl_c(stop: Arc<AtomicBool>) {
    if tokio::signal::ctrl_c().await.is_err() {
        return;
    }
    stop.store(true, Ordering::Release);
    if tokio::signal::ctrl_c().await.is_ok() {
        std::process::exit(130);
    }
}

/// `suggestions scan` from the real terminal and session store.
async fn scan_here(
    facts: &Facts,
    store: &DictionaryStore,
    out: &mut io::Stdout,
    err: &mut io::Stderr,
) -> Outcome {
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = tokio::spawn(stop_on_ctrl_c(Arc::clone(&stop)));
    let in_place = err.is_terminal();
    let mut streams = ScanStreams { out, err, in_place };
    let outcome = run_scan(facts, store, &mut SessionScanner, &stop, &mut streams).await;
    watcher.abort();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted answers, one queue per kind of question.
    #[derive(Default)]
    struct Script {
        confirms: VecDeque<bool>,
        phrases: VecDeque<String>,
        choices: VecDeque<Option<usize>>,
        selections: VecDeque<Vec<usize>>,
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
        fn choose(&mut self, question: &str, _options: &[String]) -> io::Result<Option<usize>> {
            self.asked.push(question.to_string());
            Ok(self.choices.pop_front().expect("an unexpected choice"))
        }
        fn multiselect(&mut self, question: &str, _items: &[String]) -> io::Result<Vec<usize>> {
            self.asked.push(question.to_string());
            Ok(self
                .selections
                .pop_front()
                .expect("an unexpected selection"))
        }
    }

    fn terminal() -> Facts {
        Facts {
            interactive: true,
            agent_shell: false,
            private_capability: false,
            dictionary: Rule::Allowed,
            agent_writes: Rule::Allowed,
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

    struct Run {
        result: Outcome,
        out: String,
        err: String,
    }

    fn run_with(
        store: &DictionaryStore,
        facts: &Facts,
        script: &mut Script,
        input: &str,
        action: Action,
    ) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut input = input.as_bytes();
        let result = {
            let mut streams = Streams {
                out: &mut out,
                err: &mut err,
                input: &mut input,
            };
            run(action, facts, store, script, &mut streams)
        };
        Run {
            result,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    fn go(store: &DictionaryStore, facts: &Facts, action: Action) -> Run {
        run_with(store, facts, &mut Script::default(), "", action)
    }

    fn fresh() -> (tempfile::TempDir, DictionaryStore) {
        let root = tempfile::tempdir().unwrap();
        let store = DictionaryStore::open(root.path().join("dictionary"));
        (root, store)
    }

    fn add_action(term: &str, private: bool) -> Action {
        Action::Add {
            terms: vec![term.into()],
            spoken: vec![],
            private,
        }
    }

    fn list_json(store: &DictionaryStore, facts: &Facts) -> Vec<Value> {
        let run = go(
            store,
            facts,
            Action::List {
                format: ListFormat::Json,
                status: StatusFilter::All,
            },
        );
        assert_eq!(run.result, Ok(()));
        serde_json::from_str::<Value>(&run.out)
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    }

    fn every_write() -> Vec<Action> {
        vec![
            add_action("EPAS1", false),
            Action::Edit {
                term: Some("EPAS1".into()),
                id: None,
                new_term: Some("EPAS2".into()),
                spoken: None,
            },
            Action::Remove {
                terms: vec!["EPAS1".into()],
                ids: vec![],
            },
            Action::Review,
            Action::Accept { ids: vec![] },
            Action::Dismiss { ids: vec![] },
            Action::Publicize {
                term: Some("EPAS1".into()),
                id: None,
            },
            Action::Privatize {
                term: Some("EPAS1".into()),
                id: None,
            },
            Action::Settings {
                agent_writes: Some(AgentWrites::Suggest),
                learn_from_chats: None,
                suggest_from_chats: None,
            },
            Action::Clear {
                suggested_only: false,
            },
            Action::KeepSuggestions {
                terms: vec!["EPAS1".into()],
            },
            Action::DismissSuggestions {
                terms: vec!["EPAS1".into()],
            },
            Action::ScanSuggestions,
        ]
    }

    fn import_action() -> Action {
        Action::Import {
            input: "-".into(),
            format: Some("txt".into()),
            dry_run: false,
        }
    }

    /// Every write except `import` is refused without a terminal (exit 2), and every
    /// write, `import` included, in an agent's shell; nothing touches the store.
    #[test]
    fn writes_need_a_person_at_a_terminal() {
        let (_root, store) = fresh();
        for action in every_write() {
            let run = go(&store, &piped(), action.clone());
            assert_eq!(
                run.result,
                Err(Failure::refused(MSG_DICTIONARY_TTY)),
                "{action:?}"
            );
            assert!(run.err.is_empty(), "the sentence alone: {}", run.err);
        }
        for action in every_write().into_iter().chain([import_action()]) {
            let run = go(&store, &agent(), action.clone());
            assert_eq!(
                run.result,
                Err(Failure::refused(MSG_DICTIONARY_AGENT)),
                "{action:?}"
            );
        }
        assert!(!store.dir().exists(), "nothing was written");
        let run = run_with(
            &store,
            &piped(),
            &mut Script::default(),
            "UCSF\n",
            import_action(),
        );
        assert_eq!(run.result, Ok(()));
        assert!(run.out.starts_with("Imported: 1 new"), "{}", run.out);
    }

    /// Managed `dictionary: false` refuses writes with its own sentence; deleting and
    /// exporting still work.
    #[test]
    fn the_managed_policy_refuses_writes_but_not_deletes_or_export() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        let sentence =
            "Dictation is turned off on this computer by your organization's managed policy.";
        let managed = Facts {
            dictionary: Rule::Forbidden(sentence.into()),
            ..terminal()
        };
        for action in [add_action("UCSF", false), import_action()] {
            assert_eq!(
                go(&store, &managed, action).result,
                Err(Failure::refused(sentence))
            );
        }
        let export = go(
            &store,
            &managed,
            Action::Export {
                output: None,
                include_private: false,
            },
        );
        assert!(export.out.contains("EPAS1"), "{}", export.out);
        let removed = go(
            &store,
            &managed,
            Action::Remove {
                terms: vec!["EPAS1".into()],
                ids: vec![],
            },
        );
        assert_eq!(removed.result, Ok(()));
        assert!(store.view().unwrap().entries().is_empty());
    }

    #[test]
    fn session_add_review_and_observation_share_managed_and_agent_shell_authority() {
        let add = Action::Add {
            terms: vec!["EPAS1".into()],
            spoken: Vec::new(),
            private: true,
        };
        let review = Action::Review;
        let accept = Action::Accept { ids: Vec::new() };
        let dismiss = Action::Dismiss { ids: Vec::new() };
        let denied = Facts {
            dictionary: Rule::Forbidden("Managed dictionary is off.".into()),
            ..terminal()
        };
        for action in [&add, &review, &accept] {
            assert_eq!(
                authorize(action, &denied),
                Err(Failure::refused("Managed dictionary is off."))
            );
            assert_eq!(
                authorize(action, &agent()),
                Err(Failure::refused(MSG_DICTIONARY_AGENT))
            );
        }
        assert_eq!(
            authorize(&dismiss, &denied),
            Ok(()),
            "deleting a suggestion remains allowed"
        );
        assert_eq!(
            authorize(&dismiss, &agent()),
            Err(Failure::refused(MSG_DICTIONARY_AGENT))
        );
        assert_eq!(
            authorize(&add, &piped()),
            Err(Failure::refused(MSG_DICTIONARY_TTY))
        );
    }

    /// `add --private` writes a private row with `affiliation: null` (O-36); a piped
    /// list withholds it with no count, and the terminal's list shows it, whatever model
    /// is configured.
    #[test]
    fn private_rows_are_listed_only_at_a_terminal() {
        let (_root, store) = fresh();
        assert_eq!(
            go(&store, &terminal(), add_action("EPAS1", false)).result,
            Ok(())
        );
        let private = go(&store, &terminal(), add_action("PHD2", true));
        assert_eq!(private.out.trim(), "Added PHD2 (private).");
        let row = store.view().unwrap().entries()[1].clone();
        assert_eq!((row.tier, row.affiliation), (EntryTier::Private, None));

        let all = list_json(&store, &terminal());
        assert_eq!(all.len(), 2);
        let phd2 = all.iter().find(|r| r["written"] == "PHD2").unwrap();
        assert_eq!(phd2["affiliation"], Value::Null);
        assert_eq!(
            phd2["uses"], 0,
            "usage reaches a caller with private standing"
        );
        assert!(phd2.get("seen").is_none());

        for facts in [piped(), agent()] {
            let public = list_json(&store, &facts);
            assert_eq!(public.len(), 1);
            assert_eq!(public[0]["written"], "EPAS1");
            assert!(public[0].get("uses").is_none() && public[0].get("suggested_in").is_none());
            let text = go(
                &store,
                &facts,
                Action::List {
                    format: ListFormat::Text,
                    status: StatusFilter::All,
                },
            );
            assert!(!text.out.contains("PHD2") && !text.err.contains("PHD2"));
            assert_eq!(
                text.out.lines().count(),
                2,
                "the header and the public row, and no count of private rows: {}",
                text.out
            );
            assert!(!text.out.to_lowercase().contains("withheld"));
            assert_eq!(
                text.err.lines().count(),
                1,
                "the folder line only: {}",
                text.err
            );
        }
    }

    #[test]
    fn a_private_row_lists_its_institutions_with_their_names() {
        let (_root, store) = fresh();
        store
            .write(|file| {
                file.add(
                    NewTerm {
                        affiliation: Some(store::RowAffiliation {
                            institutions: vec!["no-such-institution".into()],
                        }),
                        ..NewTerm::person("VHL", EntryTier::Private)
                    },
                    Reach::All,
                    store::schema::now(),
                )
            })
            .unwrap();
        let rows = list_json(&store, &terminal());
        assert_eq!(
            rows[0]["affiliation"],
            json!({ "institutions": [{ "id": "no-such-institution", "name": null }] })
        );
    }

    #[test]
    fn add_warns_about_the_same_letters_and_refuses_bad_terms() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        let run = go(&store, &terminal(), add_action("E-PAS-1", false));
        assert_eq!(run.result, Ok(()));
        assert!(
            run.err
                .contains("A term with the same letters exists: EPAS1"),
            "{}",
            run.err
        );
        let run = go(
            &store,
            &terminal(),
            Action::Add {
                terms: vec![
                    "https://example.org".into(),
                    "UCSF".into(),
                    "sk-ant-api03-abcdefghijklmnop".into(),
                ],
                spoken: vec![],
                private: false,
            },
        );
        let Err(failure) = run.result else {
            panic!("bad terms were added");
        };
        assert_eq!(failure.exit, 1);
        assert!(failure.message.contains(
            "https://example.org: A dictionary term cannot be a web address, an email address or a file path."
        ));
        assert!(failure
            .message
            .contains("This looks like an identifier or a key"));
        assert!(run.out.contains("Added UCSF (public)."));
        let spoken = go(
            &store,
            &terminal(),
            Action::Add {
                terms: vec!["HIF1A".into()],
                spoken: vec!["hif one a, hif 1 a".into(), "hypoxia factor".into()],
                private: false,
            },
        );
        assert_eq!(spoken.result, Ok(()));
        let hif = store
            .view()
            .unwrap()
            .file()
            .rows_written("HIF1A", Reach::All)[0]
            .clone();
        assert_eq!(
            hif.spoken_forms,
            vec!["hif one a", "hif 1 a", "hypoxia factor"]
        );
    }

    /// `publicize` shows the publicize warning and needs the term typed exactly; a wrong
    /// or blank answer changes nothing. A term the person added as private at a terminal
    /// came from no chat, so the warning claims none.
    #[test]
    fn publicize_needs_the_typed_term() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("PHD2", true));
        let publicize = || Action::Publicize {
            term: Some("PHD2".into()),
            id: None,
        };
        let before = std::fs::read(store.store_path()).unwrap();
        for (typed, exit) in [("", None), ("phd2", Some(1)), ("PHD", Some(1))] {
            let mut script = Script {
                phrases: VecDeque::from([typed.to_string()]),
                ..Script::default()
            };
            let run = run_with(&store, &terminal(), &mut script, "", publicize());
            assert_eq!(run.result.err().map(|f| f.exit), exit, "{typed:?}");
            assert!(run.err.contains(MSG_PUBLICIZE_WARN_NO_CHAT), "{}", run.err);
            assert!(!run.err.contains(MSG_PUBLICIZE_WARN));
            assert_eq!(
                std::fs::read(store.store_path()).unwrap(),
                before,
                "{typed:?}"
            );
        }
        let mut script = Script {
            phrases: VecDeque::from(["PHD2".to_string()]),
            ..Script::default()
        };
        let run = run_with(&store, &terminal(), &mut script, "", publicize());
        assert_eq!(run.result, Ok(()));
        assert_eq!(run.out.trim(), "PHD2 is now public.");
        let row = store.view().unwrap().entries()[0].clone();
        assert_eq!((row.tier, row.affiliation), (EntryTier::Public, None));

        // Privatize asks the same, without the warning, and writes `null`.
        let mut script = Script {
            phrases: VecDeque::from(["PHD2".to_string()]),
            ..Script::default()
        };
        let run = run_with(
            &store,
            &terminal(),
            &mut script,
            "",
            Action::Privatize {
                term: Some("PHD2".into()),
                id: None,
            },
        );
        assert_eq!(run.result, Ok(()));
        assert!(!run.err.contains(MSG_PUBLICIZE_WARN));
        assert!(!run.err.contains(MSG_PUBLICIZE_WARN_NO_CHAT));
        let row = store.view().unwrap().entries()[0].clone();
        assert_eq!((row.tier, row.affiliation), (EntryTier::Private, None));
    }

    /// PR-14's live check: a term added publicly at a terminal and then privatized was
    /// said to come "from a private chat". The warning claims a chat only when the row
    /// shows one: the agent's and the extractor's rows, and a person's row or a learned
    /// correction that names its chat or carries a chat's institutions. A row the person
    /// privatized, or imported, never does.
    #[test]
    fn the_publicize_warning_claims_a_chat_only_when_the_row_shows_one() {
        use store::schema::{Provenance, ProvenanceReason};
        use store::RowAffiliation;

        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", true));
        let warning_for = |edit: &dyn Fn(&mut Entry)| -> String {
            store
                .write(|file| {
                    edit(&mut file.entries[0]);
                    Ok(())
                })
                .unwrap();
            let mut script = Script {
                phrases: VecDeque::from([String::new()]),
                ..Script::default()
            };
            let run = run_with(
                &store,
                &terminal(),
                &mut script,
                "",
                Action::Publicize {
                    term: Some("EPAS1".into()),
                    id: None,
                },
            );
            assert_eq!(run.result, Ok(()), "a blank answer cancels");
            let warnings: Vec<&str> = run
                .err
                .lines()
                .filter(|l| l.contains("Making it public"))
                .collect();
            assert_eq!(warnings.len(), 1, "{}", run.err);
            warnings[0].to_string()
        };
        let reset = |e: &mut Entry| {
            e.source = EntrySource::User;
            e.suggested_in = None;
            e.affiliation = None;
            e.provenance = None;
        };
        let chat = MSG_PUBLICIZE_WARN.to_string();
        let no_chat = MSG_PUBLICIZE_WARN_NO_CHAT.to_string();
        let institutions = || {
            Some(RowAffiliation {
                institutions: vec!["ucsf".into()],
            })
        };
        type Edit = Box<dyn Fn(&mut Entry)>;
        let cases: Vec<(&str, Edit, &String)> = vec![
            (
                "a person's add with no chat",
                Box::new(|_: &mut Entry| {}),
                &no_chat,
            ),
            (
                "the agent's",
                Box::new(|e: &mut Entry| e.source = EntrySource::Agent),
                &chat,
            ),
            (
                "the extractor's",
                Box::new(|e: &mut Entry| e.source = EntrySource::Auto),
                &chat,
            ),
            (
                "a person's add inside a chat that names institutions",
                Box::new(move |e: &mut Entry| e.affiliation = institutions()),
                &chat,
            ),
            (
                "an add whose chat could not be read",
                Box::new(|e: &mut Entry| e.affiliation = Some(RowAffiliation::unknown())),
                &no_chat,
            ),
            (
                "a correction that names its chat",
                Box::new(|e: &mut Entry| {
                    e.source = EntrySource::Correction;
                    e.suggested_in = Some("20261106_1".into());
                }),
                &chat,
            ),
            (
                "an import",
                Box::new(move |e: &mut Entry| {
                    e.source = EntrySource::Import;
                    e.affiliation = institutions();
                }),
                &no_chat,
            ),
            (
                "the agent's row the person made private",
                Box::new(|e: &mut Entry| {
                    e.source = EntrySource::Agent;
                    e.suggested_in = Some("20261106_3".into());
                    e.provenance = Some(Provenance {
                        reason: ProvenanceReason::PrivatizedByUser,
                        at: store::schema::now(),
                    });
                }),
                &no_chat,
            ),
        ];
        for (label, edit, want) in cases {
            let line = warning_for(&|e: &mut Entry| {
                reset(e);
                edit(e);
            });
            assert_eq!(&line, want, "{label}");
        }
    }

    /// A term that names twins asks which, or takes `--id`.
    #[test]
    fn twins_ask_which_one() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        go(&store, &terminal(), add_action("EPAS1", true));
        let private_id = store
            .view()
            .unwrap()
            .entries()
            .iter()
            .find(|e| e.tier.is_private())
            .unwrap()
            .id
            .to_string();
        let remove = Action::Remove {
            terms: vec!["EPAS1".into()],
            ids: vec![],
        };
        let mut cancel = Script {
            choices: VecDeque::from([None]),
            ..Script::default()
        };
        let cancelled = run_with(&store, &terminal(), &mut cancel, "", remove.clone());
        assert_eq!(cancelled.result, Ok(()));
        assert_eq!(cancelled.out, "Nothing was changed.\n");
        assert_eq!(store.view().unwrap().entries().len(), 2);
        let removed = go(
            &store,
            &terminal(),
            Action::Remove {
                terms: vec![],
                ids: vec![private_id],
            },
        );
        assert_eq!(removed.result, Ok(()));
        let left = store.view().unwrap();
        assert_eq!(left.entries().len(), 1);
        assert_eq!(left.entries()[0].tier, EntryTier::Public);
        let missing = go(
            &store,
            &terminal(),
            Action::Remove {
                terms: vec![],
                ids: vec!["dct_00000000-0000-4000-8000-000000000000".into()],
            },
        );
        assert!(missing
            .result
            .unwrap_err()
            .message
            .contains(store::MSG_NOT_FOUND));
    }

    /// `settings` asks before raising and not before lowering; the managed
    /// `dictionary_agent_writes: false` refuses raising.
    #[test]
    fn settings_ask_before_raising_only() {
        let (_root, store) = fresh();
        let set = |mode: Option<AgentWrites>, learn: Option<bool>| Action::Settings {
            agent_writes: mode,
            learn_from_chats: learn,
            suggest_from_chats: None,
        };
        let shown = go(&store, &piped(), set(None, None));
        assert_eq!(
            shown.out,
            "agent_writes: on_request\nsuggest_from_chats: off\nlearn_from_chats: off\n"
        );

        let lowered = go(&store, &terminal(), set(Some(AgentWrites::Suggest), None));
        assert_eq!(lowered.result, Ok(()), "lowering asks nothing");
        assert_eq!(
            store.view().unwrap().settings().agent_writes,
            AgentWrites::Suggest
        );

        let mut no = Script {
            confirms: VecDeque::from([false]),
            ..Script::default()
        };
        run_with(
            &store,
            &terminal(),
            &mut no,
            "",
            set(Some(AgentWrites::Add), None),
        );
        assert_eq!(no.asked, vec!["Change this setting?"]);
        assert_eq!(
            store.view().unwrap().settings().agent_writes,
            AgentWrites::Suggest
        );

        let mut yes = Script {
            confirms: VecDeque::from([true]),
            ..Script::default()
        };
        run_with(&store, &terminal(), &mut yes, "", set(None, Some(true)));
        assert!(store.view().unwrap().settings().learn_from_chats);
        go(&store, &terminal(), set(None, Some(false)));
        assert!(
            !store.view().unwrap().settings().learn_from_chats,
            "off asks nothing"
        );

        let reason = "Your organization's policy keeps the agent out of the dictionary.";
        let managed = Facts {
            agent_writes: Rule::Forbidden(reason.into()),
            ..terminal()
        };
        assert_eq!(
            go(&store, &managed, set(Some(AgentWrites::OnRequest), None)).result,
            Err(Failure::refused(reason))
        );
    }

    /// PR-23: `settings --suggest-from-chats on` asks before it turns the `dictionary`
    /// capability on, and a no changes nothing; turning on what is already on asks
    /// nothing, and `off` asks nothing. Without a terminal and in an agent's shell either
    /// value is refused before any question, and managed `dictionary_agent_writes: false`
    /// refuses turning it on. Both values go through `activation.rs`, which writes this
    /// process's one configuration file, so the test runs in a process of its own: the
    /// test above reads the same switch as off.
    #[test]
    fn settings_ask_before_turning_chat_suggestions_on_and_never_before_turning_them_off() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        use biorouter::dictation::activation::suggest_from_chats;
        let (_root, store) = fresh();
        let set = |suggest: bool| Action::Settings {
            agent_writes: None,
            learn_from_chats: None,
            suggest_from_chats: Some(suggest),
        };
        let printed = |suggest: &str| {
            format!(
                "agent_writes: on_request\nsuggest_from_chats: {suggest}\nlearn_from_chats: off\n"
            )
        };
        assert!(
            !suggest_from_chats(),
            "a fresh configuration has the capability off"
        );

        let mut no = Script {
            confirms: VecDeque::from([false]),
            ..Script::default()
        };
        let declined = run_with(&store, &terminal(), &mut no, "", set(true));
        assert_eq!(declined.result, Ok(()));
        assert_eq!(no.asked, vec!["Change this setting?"]);
        assert_eq!(declined.out, "Nothing was changed.\n");
        assert!(!suggest_from_chats(), "a no leaves the capability off");

        let mut yes = Script {
            confirms: VecDeque::from([true]),
            ..Script::default()
        };
        let raised = run_with(&store, &terminal(), &mut yes, "", set(true));
        assert_eq!(raised.result, Ok(()));
        assert_eq!(yes.asked, vec!["Change this setting?"]);
        assert_eq!(raised.out, printed("on"));
        assert!(suggest_from_chats(), "a yes turns the capability on");

        let mut silent = Script::default();
        let again = run_with(&store, &terminal(), &mut silent, "", set(true));
        assert_eq!(again.result, Ok(()));
        assert!(
            silent.asked.is_empty(),
            "turning on what is already on asks nothing: {:?}",
            silent.asked
        );
        assert_eq!(again.out, printed("on"));

        for (facts, sentence) in [
            (piped(), MSG_DICTIONARY_TTY),
            (agent(), MSG_DICTIONARY_AGENT),
        ] {
            for suggest in [false, true] {
                let mut silent = Script::default();
                let refused = run_with(&store, &facts, &mut silent, "", set(suggest));
                assert_eq!(refused.result, Err(Failure::refused(sentence)), "{suggest}");
                assert!(silent.asked.is_empty(), "{:?}", silent.asked);
            }
        }
        assert!(
            suggest_from_chats(),
            "a refused command leaves the capability as it was"
        );

        let mut silent = Script::default();
        let lowered = run_with(&store, &terminal(), &mut silent, "", set(false));
        assert_eq!(lowered.result, Ok(()));
        assert!(
            silent.asked.is_empty(),
            "off asks nothing: {:?}",
            silent.asked
        );
        assert_eq!(lowered.out, printed("off"));
        assert!(!suggest_from_chats(), "off turns the capability off");

        let reason = "Your organization's policy keeps the agent out of the dictionary.";
        let managed = Facts {
            agent_writes: Rule::Forbidden(reason.into()),
            ..terminal()
        };
        let mut silent = Script::default();
        assert_eq!(
            run_with(&store, &managed, &mut silent, "", set(true)).result,
            Err(Failure::refused(reason))
        );
        assert!(silent.asked.is_empty(), "{:?}", silent.asked);
        assert!(!suggest_from_chats());
    }

    /// PR-23 narrows `dismiss` to rows still suggested, as the route does: a row the
    /// person added, a row the agent added active and unreviewed, an unknown id and text
    /// that is no id each answer MSG-NOT-FOUND, word for word as an unknown id does, and
    /// stay as they were, while the suggestion named in the same call is dismissed.
    #[test]
    fn dismiss_takes_only_suggested_rows_and_answers_any_other_id_as_not_found() {
        let (_root, store) = fresh();
        let created = |outcome: AddOutcome| match outcome {
            AddOutcome::Created { id, .. } => id,
            other => panic!("{other:?}"),
        };
        let (person, agent_row, suggestion) = store
            .write(|file| {
                let now = store::schema::now();
                let person =
                    file.add(NewTerm::person("EPAS1", EntryTier::Public), Reach::All, now)?;
                let agent_row = file.add(
                    NewTerm {
                        source: EntrySource::Agent,
                        ..NewTerm::person("PHD2", EntryTier::Public)
                    },
                    Reach::All,
                    now,
                )?;
                let suggestion = file.add(
                    NewTerm {
                        source: EntrySource::Agent,
                        status: EntryStatus::Suggested,
                        ..NewTerm::person("SPOKE", EntryTier::Public)
                    },
                    Reach::All,
                    now,
                )?;
                Ok((person, agent_row, suggestion))
            })
            .unwrap();
        let (person, agent_row, suggestion) =
            (created(person), created(agent_row), created(suggestion));
        {
            let view = store.view().unwrap();
            let row = |id: &EntryId| view.entries().iter().find(|e| &e.id == id).unwrap();
            assert_eq!(row(&person).status, EntryStatus::Active);
            assert_eq!(row(&agent_row).status, EntryStatus::Active);
            assert!(
                !row(&agent_row).reviewed,
                "a row `review` offers, though it is not a suggestion"
            );
            assert_eq!(row(&suggestion).status, EntryStatus::Suggested);
        }
        let unknown = EntryId::random().to_string();
        let ids = vec![
            person.to_string(),
            agent_row.to_string(),
            unknown.clone(),
            "EPAS1".to_string(),
            suggestion.to_string(),
        ];
        let run = go(&store, &terminal(), Action::Dismiss { ids });
        let not_found = |raw: &str| format!("{raw}: {}", store::MSG_NOT_FOUND);
        assert_eq!(
            run.result,
            Err(Failure::failed(
                [
                    not_found(person.as_str()),
                    not_found(agent_row.as_str()),
                    not_found(&unknown),
                    not_found("EPAS1"),
                ]
                .join("\n")
            ))
        );
        assert_eq!(run.out, "Dismissed SPOKE.\n");

        let view = store.view().unwrap();
        let left: Vec<(&str, EntryStatus, bool)> = view
            .entries()
            .iter()
            .map(|e| (e.written.as_str(), e.status, e.reviewed))
            .collect();
        assert_eq!(
            left,
            [
                ("EPAS1", EntryStatus::Active, true),
                ("PHD2", EntryStatus::Active, false),
            ]
        );
        assert!(view.file().tombstoned("spoke", EntryTier::Public));
        assert!(!view.file().tombstoned("epas1", EntryTier::Public));
        assert!(!view.file().tombstoned("phd2", EntryTier::Public));

        let alone = go(
            &store,
            &terminal(),
            Action::Dismiss {
                ids: vec![person.to_string()],
            },
        );
        assert_eq!(
            alone.result,
            Err(Failure::failed(not_found(person.as_str())))
        );
        assert!(alone.out.is_empty(), "{}", alone.out);
        assert_eq!(store.view().unwrap().entries().len(), 2);
    }

    #[test]
    fn clear_needs_the_typed_count() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        go(&store, &terminal(), add_action("PHD2", true));
        let mut wrong = Script {
            phrases: VecDeque::from(["delete 3".to_string()]),
            ..Script::default()
        };
        let run = run_with(
            &store,
            &terminal(),
            &mut wrong,
            "",
            Action::Clear {
                suggested_only: false,
            },
        );
        assert!(run.err.contains(&msg_clear(2, store.dir())), "{}", run.err);
        assert_eq!(store.view().unwrap().entries().len(), 2);
        let mut right = Script {
            phrases: VecDeque::from(["delete 2".to_string()]),
            ..Script::default()
        };
        let run = run_with(
            &store,
            &terminal(),
            &mut right,
            "",
            Action::Clear {
                suggested_only: false,
            },
        );
        assert_eq!(run.out.trim(), "Deleted 2 terms.");
        assert!(store.view().unwrap().entries().is_empty());
    }

    #[test]
    fn review_keeps_the_chosen_and_dismisses_the_rest() {
        let (_root, store) = fresh();
        store
            .write(|file| {
                for written in ["EPAS1", "SPOKE", "PHD2"] {
                    file.add(
                        NewTerm {
                            source: EntrySource::Agent,
                            status: EntryStatus::Suggested,
                            reason: Some("named in the chat".into()),
                            ..NewTerm::person(written, EntryTier::Public)
                        },
                        Reach::All,
                        store::schema::now(),
                    )?;
                }
                Ok(())
            })
            .unwrap();
        let mut script = Script {
            selections: VecDeque::from([vec![0]]),
            confirms: VecDeque::from([true]),
            ..Script::default()
        };
        let run = run_with(&store, &terminal(), &mut script, "", Action::Review);
        assert_eq!(run.out.trim(), "Kept 1. Dismissed 2.");
        let view = store.view().unwrap();
        assert_eq!(view.entries().len(), 1);
        assert_eq!(view.entries()[0].written, "EPAS1");
        assert_eq!(view.entries()[0].status, EntryStatus::Active);
        assert!(view.entries()[0].reviewed);
        assert!(view.file().tombstoned("spoke", EntryTier::Public));
    }

    /// A person who presses Escape at the first question: cliclack answers
    /// `io::ErrorKind::Interrupted`.
    struct Escape;

    impl Prompter for Escape {
        fn confirm(&mut self, _question: &str) -> io::Result<bool> {
            Err(io::ErrorKind::Interrupted.into())
        }
        fn phrase(&mut self, _prompt: &str) -> io::Result<String> {
            Err(io::ErrorKind::Interrupted.into())
        }
        fn choose(&mut self, _question: &str, _options: &[String]) -> io::Result<Option<usize>> {
            Err(io::ErrorKind::Interrupted.into())
        }
        fn multiselect(&mut self, _question: &str, _items: &[String]) -> io::Result<Vec<usize>> {
            Err(io::ErrorKind::Interrupted.into())
        }
    }

    /// Escape at any question cancels the command as a blank answer or **Cancel** does:
    /// "Nothing was changed." ("Nothing was exported." for `export`), exit 0, and the
    /// store byte identical. It used to print cliclack's bare "operation interrupted"
    /// and exit 1.
    #[test]
    fn escape_at_a_question_changes_nothing() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        go(&store, &terminal(), add_action("EPAS1", true));
        go(&store, &terminal(), add_action("PHD2", true));
        store
            .write(|file| {
                file.add(
                    NewTerm {
                        source: EntrySource::Agent,
                        status: EntryStatus::Suggested,
                        ..NewTerm::person("SPOKE", EntryTier::Public)
                    },
                    Reach::All,
                    store::schema::now(),
                )
            })
            .unwrap();
        let before = std::fs::read(store.store_path()).unwrap();
        let escaped = |action: Action, facts: &Facts| {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let mut input: &[u8] = b"";
            let result = run(
                action,
                facts,
                &store,
                &mut Escape,
                &mut Streams {
                    out: &mut out,
                    err: &mut err,
                    input: &mut input,
                },
            );
            (
                result,
                String::from_utf8(out).unwrap(),
                String::from_utf8(err).unwrap(),
            )
        };
        for action in [
            // EPAS1 names twins, so these two ask which one first.
            Action::Edit {
                term: Some("EPAS1".into()),
                id: None,
                new_term: Some("EPAS2".into()),
                spoken: None,
            },
            Action::Remove {
                terms: vec!["EPAS1".into()],
                ids: vec![],
            },
            Action::Review,
            Action::Publicize {
                term: Some("PHD2".into()),
                id: None,
            },
            Action::Settings {
                agent_writes: Some(AgentWrites::Add),
                learn_from_chats: None,
                suggest_from_chats: None,
            },
            Action::Clear {
                suggested_only: false,
            },
        ] {
            let (result, out, _) = escaped(action.clone(), &terminal());
            assert_eq!(result, Ok(()), "{action:?}");
            assert_eq!(out, "Nothing was changed.\n", "{action:?}");
            assert_eq!(
                std::fs::read(store.store_path()).unwrap(),
                before,
                "{action:?}"
            );
        }
        let (result, out, err) = escaped(
            Action::Export {
                output: None,
                include_private: true,
            },
            &Facts {
                private_capability: true,
                ..terminal()
            },
        );
        assert_eq!(result, Ok(()));
        assert!(out.is_empty(), "nothing exported: {out}");
        assert!(err.contains("Nothing was exported."), "{err}");
    }

    /// Scripted answers, with something another process does while the person is
    /// still answering the first question.
    struct Meanwhile<'a> {
        script: Script,
        meanwhile: Option<Box<dyn FnOnce() + 'a>>,
    }

    impl Meanwhile<'_> {
        fn happen(&mut self) {
            if let Some(change) = self.meanwhile.take() {
                change();
            }
        }
    }

    impl Prompter for Meanwhile<'_> {
        fn confirm(&mut self, question: &str) -> io::Result<bool> {
            self.happen();
            self.script.confirm(question)
        }
        fn phrase(&mut self, prompt: &str) -> io::Result<String> {
            self.happen();
            self.script.phrase(prompt)
        }
        fn choose(&mut self, question: &str, options: &[String]) -> io::Result<Option<usize>> {
            self.script.choose(question, options)
        }
        fn multiselect(&mut self, question: &str, items: &[String]) -> io::Result<Vec<usize>> {
            self.script.multiselect(question, items)
        }
    }

    fn other_process(store: &DictionaryStore) -> DictionaryStore {
        DictionaryStore::open(store.dir())
    }

    /// `clear` deletes the rows the person counted: a term another process adds while
    /// the person types `delete <n>` stays, untombstoned, and an `auto` row seen once,
    /// which no listing shows, is neither counted nor deleted.
    #[test]
    fn clear_deletes_only_the_rows_the_person_counted() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        go(&store, &terminal(), add_action("PHD2", true));
        store
            .write(|file| {
                file.add(
                    NewTerm {
                        source: EntrySource::Auto,
                        status: EntryStatus::Suggested,
                        ..NewTerm::person("SPOKE", EntryTier::Public)
                    },
                    Reach::All,
                    store::schema::now(),
                )
            })
            .unwrap();
        let other = other_process(&store);
        let mut prompter = Meanwhile {
            script: Script {
                phrases: VecDeque::from(["delete 2".to_string()]),
                ..Script::default()
            },
            meanwhile: Some(Box::new(|| {
                other
                    .write(|file| {
                        file.add(
                            NewTerm::person("UCSF", EntryTier::Public),
                            Reach::All,
                            store::schema::now(),
                        )
                    })
                    .unwrap();
            })),
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut input: &[u8] = b"";
        let result = run(
            Action::Clear {
                suggested_only: false,
            },
            &terminal(),
            &store,
            &mut prompter,
            &mut Streams {
                out: &mut out,
                err: &mut err,
                input: &mut input,
            },
        );
        assert_eq!(result, Ok(()));
        assert!(String::from_utf8(err)
            .unwrap()
            .contains(&msg_clear(2, store.dir())));
        assert_eq!(String::from_utf8(out).unwrap(), "Deleted 2 terms.\n");
        let view = store.view().unwrap();
        let left: Vec<&str> = view.entries().iter().map(|e| e.written.as_str()).collect();
        assert_eq!(left, ["SPOKE", "UCSF"]);
        assert!(!view.file().tombstoned("ucsf", EntryTier::Public));
        assert!(view.file().tombstoned("epas1", EntryTier::Public));
    }

    /// "Dismiss the rest" leaves a row another surface kept or accepted while the
    /// person was answering.
    #[test]
    fn review_never_dismisses_a_row_kept_meanwhile() {
        let (_root, store) = fresh();
        let ids = store
            .write(|file| {
                let mut ids = Vec::new();
                for written in ["EPAS1", "SPOKE", "PHD2"] {
                    let outcome = file.add(
                        NewTerm {
                            source: EntrySource::Agent,
                            status: EntryStatus::Suggested,
                            ..NewTerm::person(written, EntryTier::Public)
                        },
                        Reach::All,
                        store::schema::now(),
                    )?;
                    let AddOutcome::Created { id, .. } = outcome else {
                        panic!("{outcome:?}");
                    };
                    ids.push(id);
                }
                Ok(ids)
            })
            .unwrap();
        let other = other_process(&store);
        let spoke = ids[1].clone();
        let mut prompter = Meanwhile {
            script: Script {
                selections: VecDeque::from([vec![0]]),
                confirms: VecDeque::from([true]),
                ..Script::default()
            },
            meanwhile: Some(Box::new(move || {
                other
                    .write(|file| file.accept(&spoke, store::schema::now()))
                    .unwrap();
            })),
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut input: &[u8] = b"";
        let result = run(
            Action::Review,
            &terminal(),
            &store,
            &mut prompter,
            &mut Streams {
                out: &mut out,
                err: &mut err,
                input: &mut input,
            },
        );
        assert_eq!(result, Ok(()));
        assert_eq!(String::from_utf8(out).unwrap(), "Kept 1. Dismissed 1.\n");
        let view = store.view().unwrap();
        let mut left: Vec<&str> = view.entries().iter().map(|e| e.written.as_str()).collect();
        left.sort();
        assert_eq!(left, ["EPAS1", "SPOKE"]);
        assert!(!view.file().tombstoned("spoke", EntryTier::Public));
        assert!(view.file().tombstoned("phd2", EntryTier::Public));
    }

    /// The CLI asks the core's secret scan as well as the store's validator: a Hugging
    /// Face token passes `validate::term` (its run holds no digit and no capital) and is
    /// refused by `redact_text`, on `add`, `edit --term` and a piped `import -`.
    #[test]
    fn the_core_secret_scan_refuses_what_the_validator_lets_through() {
        let token = "hf_abcdefghijklmnopqrstuvwxyzabcd";
        assert!(
            store::validate::term(token).is_ok(),
            "only the core catches it"
        );
        assert!(is_secret(token));
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        let before = std::fs::read(store.store_path()).unwrap();

        let added = go(&store, &terminal(), add_action(token, false));
        let failure = added.result.unwrap_err();
        assert_eq!(failure.exit, 1);
        assert_eq!(
            failure.message,
            format!("{token}: {}", store::validate::MSG_TERM_IDENTIFIER)
        );
        let spoken = go(
            &store,
            &terminal(),
            Action::Add {
                terms: vec!["UCSF".into()],
                spoken: vec![token.into()],
                private: false,
            },
        );
        assert!(spoken.result.is_err(), "a spoken form is scanned too");

        let edited = go(
            &store,
            &terminal(),
            Action::Edit {
                term: Some("EPAS1".into()),
                id: None,
                new_term: Some(token.into()),
                spoken: None,
            },
        );
        assert_eq!(
            edited.result,
            Err(Failure::failed(store::validate::MSG_TERM_IDENTIFIER))
        );

        let imported = run_with(
            &store,
            &piped(),
            &mut Script::default(),
            &format!("{token}\n"),
            import_action(),
        );
        assert_eq!(imported.result, Ok(()));
        assert_eq!(
            imported.out,
            format!(
                "Imported: 0 new, 0 merged, 0 duplicate, 1 invalid.\nline 1: secret_shaped: {}\n",
                store::validate::MSG_TERM_IDENTIFIER
            )
        );
        assert_eq!(
            std::fs::read(store.store_path()).unwrap(),
            before,
            "nothing was stored"
        );
    }

    /// `export --include-private FILE` over an existing file other accounts can read
    /// leaves it 0600 before the private terms are in it.
    #[cfg(unix)]
    #[test]
    fn a_private_export_narrows_a_file_that_was_already_there() {
        use std::os::unix::fs::PermissionsExt;
        let (root, store) = fresh();
        go(&store, &terminal(), add_action("PHD2", true));
        let path = root.path().join("terms.json");
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut script = Script {
            choices: VecDeque::from([Some(0)]),
            ..Script::default()
        };
        let run = run_with(
            &store,
            &Facts {
                private_capability: true,
                ..terminal()
            },
            &mut script,
            "",
            Action::Export {
                output: Some(path.to_string_lossy().into_owned()),
                include_private: true,
            },
        );
        assert_eq!(run.result, Ok(()));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(std::fs::read_to_string(&path).unwrap().contains("PHD2"));
    }

    /// The sentences the dictionary commands print, word for word as the message
    /// catalog in `docs/dictation/surfaces.md` has them.
    #[test]
    fn the_dictionary_sentences_are_the_catalogs() {
        let folder = Path::new("/home/a/.config/biorouter/dictionary");
        let io_error = io::Error::from(io::ErrorKind::PermissionDenied);
        let cases: Vec<(String, &str)> = vec![
            (MSG_DICTIONARY_TTY.into(), "Changing the dictionary needs you at an interactive terminal. To add terms from a file or a pipe, use `biorouter dictionary import`."),
            (MSG_EXPORT_PRIVATE_REFUSED.into(), "Exporting private terms needs you at your own terminal and a private model configured on this computer. Run the command without --include-private to export public terms."),
            (MSG_DICTIONARY_AGENT.into(), "Changing the dictionary needs the person. The agent's terms go through dictionary__add_terms."),
            (MSG_PUBLICIZE_WARN.into(), "This term came from a private chat. Making it public lets it appear when you dictate into chats on public models."),
            (MSG_PUBLICIZE_WARN_NO_CHAT.into(), "This term is private. Making it public lets it appear when you dictate into chats on public models."),
            (MSG_EXPORT_PRIVATE.into(), "This file leaves Biorouter. It will include your private terms."),
            (MSG_EMPTY.into(), "No terms yet. Add the words dictation should spell your way, such as UCSF or EPAS1."),
            (MSG_SPOKEN_ONE_TERM.into(), "Use --spoken with one term at a time."),
            (MSG_IMPORT_FORMAT.into(), "Name the format with --format txt, csv, tsv or json."),
            (msg_no_term("EPAS1", None), "There is no term EPAS1 in the dictionary."),
            (msg_no_term("EPAS1", Some(EntryTier::Private)), "There is no private term EPAS1 in the dictionary."),
            (msg_typed_mismatch("PHD2"), "That did not match PHD2, so nothing was changed."),
            (msg_import_not_found("terms.txt"), "There is no file at terms.txt."),
            (msg_import_unreadable("terms.txt"), "terms.txt could not be read."),
            (msg_import_unreadable("-"), "Standard input could not be read."),
            (msg_import_too_large("terms.txt"), "terms.txt is larger than 16 MB; split it and import each part."),
            (msg_import_too_large("-"), "Standard input is larger than 16 MB; split it and import each part."),
            (msg_import_not_text("terms.txt"), "terms.txt is not UTF-8 text."),
            (msg_export_unwritable("terms.json", &io_error), "terms.json could not be written: permission denied."),
            (msg_clear(2, folder), "This deletes 2 terms from /home/a/.config/biorouter/dictionary."),
            (msg_clear(1, folder), "This deletes 1 term from /home/a/.config/biorouter/dictionary."),
            (msg_deleted(2), "Deleted 2 terms."),
            (msg_deleted(1), "Deleted 1 term."),
            (msg_same_letters("EPAS1"), "A term with the same letters exists: EPAS1"),
            (store::MSG_DICTIONARY_DUPLICATE.into(), "That spelling is already a term here, so the edit was not made."),
            (store::MSG_NOT_FOUND.into(), "No dictionary entry with that id."),
            (store::MSG_DICTIONARY_CAP.into(), "Your dictionary holds 5,000 active terms here, the most dictation can use. Delete terms you no longer dictate, then add this one."),
            (store::MSG_DICTIONARY_STALE.into(), "This term changed since you opened it. Reload it and try again."),
            (store::msg_dictionary_file(folder), "The dictionary file at /home/a/.config/biorouter/dictionary could not be read, so nothing was changed. Fix or remove it, then try again."),
            (StoreError::Io(io_error).to_string(), "The dictionary could not be written: permission denied."),
            (transfer::MSG_IMPORT_NOT_EXPORT.into(), "That file is not a Biorouter dictionary export."),
            (transfer::MSG_IMPORT_ROW_UNREADABLE.into(), "This row could not be read."),
            (transfer::ImportError::Unparsable("a bad quote".into()).to_string(), "That file could not be read: a bad quote."),
            (store::validate::MSG_TERM_FORMS.into(), "Use at most eight spoken forms and sixteen misrecognitions for a term."),
        ];
        for (sentence, catalog) in cases {
            assert_eq!(sentence, catalog);
        }
    }

    /// A piped `import -` merges and never deletes; export carries no ids or usage, and
    /// private rows only after MSG-EXPORT-PRIVATE at a terminal with a private model.
    #[test]
    fn import_merges_and_export_needs_the_private_confirmation() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        go(&store, &terminal(), add_action("PHD2", true));
        let run = run_with(
            &store,
            &piped(),
            &mut Script::default(),
            "EPAS1 = epas one\nUCSF\nhttps://x.org\n",
            import_action(),
        );
        assert_eq!(
            run.out,
            "Imported: 1 new, 1 merged, 0 duplicate, 1 invalid.\nline 3: url: A dictionary term cannot be a web address, an email address or a file path.\n"
        );
        assert_eq!(store.view().unwrap().entries().len(), 3, "nothing deleted");

        let refused = go(
            &store,
            &terminal(),
            Action::Export {
                output: None,
                include_private: true,
            },
        );
        assert_eq!(
            refused.result,
            Err(Failure::refused(MSG_EXPORT_PRIVATE_REFUSED))
        );
        let private_model = Facts {
            private_capability: true,
            ..terminal()
        };
        let mut script = Script {
            choices: VecDeque::from([Some(0)]),
            ..Script::default()
        };
        let run = run_with(
            &store,
            &private_model,
            &mut script,
            "",
            Action::Export {
                output: None,
                include_private: true,
            },
        );
        assert_eq!(script.asked, vec![MSG_EXPORT_PRIVATE]);
        let exported: Value = serde_json::from_str(&run.out).unwrap();
        let rows = exported["entries"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        let phd2 = rows.iter().find(|r| r["written"] == "PHD2").unwrap();
        assert_eq!(
            (phd2["tier"].clone(), phd2["affiliation"].clone()),
            (json!("private"), Value::Null)
        );
        assert!(rows
            .iter()
            .all(|r| r.get("id").is_none() && r.get("uses").is_none()));

        // A piped import of that export marks the private row private, and a piped
        // caller's report counts it apart, never as new or already present.
        let (_other_root, other) = fresh();
        let run = run_with(
            &other,
            &piped(),
            &mut Script::default(),
            &run.out,
            Action::Import {
                input: "-".into(),
                format: None,
                dry_run: false,
            },
        );
        assert_eq!(
            run.out,
            "Imported: 2 new, 0 merged, 0 duplicate, 0 invalid.\nPrivate terms received: 1.\n"
        );
        let imported = other.view().unwrap();
        let phd2 = imported.file().rows_written("PHD2", Reach::All)[0].clone();
        assert_eq!((phd2.tier, phd2.affiliation), (EntryTier::Private, None));
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        let (_root, store) = fresh();
        let run = run_with(
            &store,
            &piped(),
            &mut Script::default(),
            "UCSF\nEPAS1\n",
            Action::Import {
                input: "-".into(),
                format: Some("txt".into()),
                dry_run: true,
            },
        );
        assert_eq!(
            run.out,
            "Dry run: 2 new, 0 merged, 0 duplicate, 0 invalid.\n"
        );
        assert!(!store.store_path().exists());
    }

    /// A store this version cannot read refuses writes with MSG-DICTIONARY-FILE (exit
    /// 1), lists nothing, and stays byte identical.
    #[test]
    fn a_corrupt_store_is_reported_and_left_alone() {
        let (_root, store) = fresh();
        go(&store, &terminal(), add_action("EPAS1", false));
        std::fs::write(store.store_path(), b"{ not json").unwrap();
        let message = store::msg_dictionary_file(&store.store_path());
        let added = go(&store, &terminal(), add_action("UCSF", false));
        assert_eq!(added.result, Err(Failure::failed(message.clone())));
        let listed = go(
            &store,
            &piped(),
            Action::List {
                format: ListFormat::Json,
                status: StatusFilter::All,
            },
        );
        assert_eq!(listed.result, Err(Failure::failed(message)));
        assert!(listed.out.is_empty());
        assert_eq!(std::fs::read(store.store_path()).unwrap(), b"{ not json");
    }

    /// The tier proof is built in this file once, inside `change_tier`.
    #[test]
    fn the_tier_proof_is_built_only_after_the_typed_term() {
        let source = include_str!("dictionary.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let needle = concat!("UserDictionaryTierChange::", "from_user_action(");
        assert_eq!(production.matches(needle).count(), 1);
        let body = production
            .split("fn change_tier(")
            .nth(1)
            .and_then(|rest| rest.split("\nfn ").next())
            .unwrap();
        let typed = body.find("typed.trim() != row.written").unwrap();
        let built = body.find(needle).unwrap();
        assert!(
            typed < built,
            "the proof comes after the typed confirmation"
        );
    }

    // ── chat word suggestions the person measures (owner, 2026-11-13) ─────────────────

    /// Each word as the chat extractor writes it after finding it in two chats, so it is
    /// listed for review. The ids, in the order of `words`.
    fn found_in_chats(store: &DictionaryStore, tier: EntryTier, words: &[&str]) -> Vec<EntryId> {
        store
            .write(|file| {
                let now = store::schema::now();
                for word in words {
                    for chat in ["chat-a", "chat-b"] {
                        file.observe_auto_suggestion(word, tier, chat, None, now)?;
                    }
                }
                Ok(())
            })
            .unwrap();
        let view = store.view().unwrap();
        words
            .iter()
            .map(|word| {
                let row = view
                    .entries()
                    .iter()
                    .find(|e| e.written == *word && e.tier == tier)
                    .unwrap();
                assert_eq!(row.seen, 2, "{word} is listed");
                row.id.clone()
            })
            .collect()
    }

    /// An agent's term waiting for review, with its reason.
    fn agent_suggestion(store: &DictionaryStore, written: &str) {
        store
            .write(|file| {
                file.add(
                    NewTerm {
                        source: EntrySource::Agent,
                        status: EntryStatus::Suggested,
                        reason: Some("named in the chat".into()),
                        ..NewTerm::person(written, EntryTier::Public)
                    },
                    Reach::All,
                    store::schema::now(),
                )
            })
            .unwrap();
    }

    /// Suggestions on, and `dismissed` decisions that were all dismissals.
    fn after_dismissals(store: &DictionaryStore, dismissed: usize) {
        store
            .write(|file| {
                file.settings.learn_from_chats = true;
                for _ in 0..dismissed {
                    file.chat_learning.record(false);
                }
                Ok(())
            })
            .unwrap();
    }

    fn learning(store: &DictionaryStore) -> store::ChatLearning {
        store.view().unwrap().file().chat_learning.clone()
    }

    fn suggestions_as(store: &DictionaryStore, facts: &Facts, format: ListFormat) -> Run {
        let run = go(store, facts, Action::Suggestions { format });
        assert_eq!(run.result, Ok(()), "{}", run.err);
        run
    }

    /// `suggestions` lists what `list --status suggested` lists, one line each with where
    /// it came from: a private suggestion only at a terminal, and never an `auto` row
    /// found in one chat. It needs no terminal.
    #[test]
    fn suggestions_list_what_the_caller_can_list_with_where_each_came_from() {
        let (_root, store) = fresh();
        let public = found_in_chats(&store, EntryTier::Public, &["HIF2A", "EPAS1"]);
        let private = found_in_chats(&store, EntryTier::Private, &["PHD2"]);
        agent_suggestion(&store, "SPOKE");
        store
            .write(|file| {
                file.observe_auto_suggestion(
                    "ONCE1",
                    EntryTier::Public,
                    "chat-a",
                    None,
                    store::schema::now(),
                )
            })
            .unwrap();
        let spoke = store
            .view()
            .unwrap()
            .file()
            .rows_written("SPOKE", Reach::All)[0]
            .id
            .clone();

        let at_terminal = suggestions_as(&store, &terminal(), ListFormat::Text);
        assert_eq!(
            at_terminal.out,
            format!(
                "EPAS1 · found in your chats · {}\nHIF2A · found in your chats · {}\nPHD2 · found in your chats · private · {}\nSPOKE · added by the agent · named in the chat · {spoke}\n",
                public[1], public[0], private[0]
            )
        );
        for facts in [piped(), agent()] {
            let run = suggestions_as(&store, &facts, ListFormat::Text);
            assert_eq!(run.out.lines().count(), 3, "{}", run.out);
            assert!(!run.out.contains("PHD2") && !run.err.contains("PHD2"));
            assert!(!run.out.contains("ONCE1"));
        }

        let json: Value =
            serde_json::from_str(&suggestions_as(&store, &piped(), ListFormat::Json).out).unwrap();
        assert_eq!(json["suggestions"].as_array().unwrap().len(), 3);
        assert_eq!(json["suggestions"][0]["written"], "EPAS1");
        assert_eq!(json["suggestions"][0]["source"], "auto");
        assert_eq!(
            (
                &json["required_chats"],
                &json["recent_kept"],
                &json["recent_decisions"],
                &json["offer_turning_off"]
            ),
            (&json!(2), &json!(0), &json!(0), &json!(false))
        );

        let (_empty_root, empty) = fresh();
        let nothing = suggestions_as(&empty, &terminal(), ListFormat::Text);
        assert!(nothing.out.is_empty());
        assert!(nothing.err.ends_with(&format!("{MSG_NO_SUGGESTIONS}\n")));
        let piped_nothing = suggestions_as(&empty, &piped(), ListFormat::Text);
        assert_eq!(piped_nothing.err.lines().count(), 1, "the folder line only");
    }

    /// Once most of 20 recent decisions were dismissals and suggestions are on, the
    /// listing offers to turn them off, naming the chats a word now needs; with
    /// suggestions off it does not.
    #[test]
    fn suggestions_offer_to_turn_off_once_most_were_dismissed() {
        let (_root, store) = fresh();
        let ids = found_in_chats(&store, EntryTier::Public, &["EPAS1"]);
        after_dismissals(&store, 19);
        let quiet = suggestions_as(&store, &terminal(), ListFormat::Text);
        assert_eq!(quiet.out.lines().count(), 1, "19 decisions change nothing");

        after_dismissals(&store, 1);
        let offer = msg_suggest_fewer(4);
        let text = suggestions_as(&store, &terminal(), ListFormat::Text);
        assert_eq!(
            text.out,
            format!("EPAS1 · found in your chats · {}\n\n{offer}\n", ids[0])
        );
        let json: Value =
            serde_json::from_str(&suggestions_as(&store, &piped(), ListFormat::Json).out).unwrap();
        assert_eq!(
            (
                &json["required_chats"],
                &json["recent_kept"],
                &json["recent_decisions"],
                &json["offer_turning_off"]
            ),
            (&json!(4), &json!(0), &json!(20), &json!(true))
        );

        store
            .write(|file| {
                file.settings.learn_from_chats = false;
                Ok(())
            })
            .unwrap();
        let off = suggestions_as(&store, &terminal(), ListFormat::Text);
        assert!(!off.out.contains(&offer), "{}", off.out);
    }

    /// `suggestions keep` and `dismiss` take a written form, case folded, or an id; each
    /// decision on a chat suggestion is counted, a dismissed word leaves its tombstone,
    /// and a name that is no waiting suggestion is reported (exit 1) while the rest go.
    #[test]
    fn suggestions_keep_and_dismiss_by_word_or_id_and_count_each_decision() {
        let (_root, store) = fresh();
        let ids = found_in_chats(&store, EntryTier::Public, &["EPAS1", "HIF2A", "VHL"]);
        agent_suggestion(&store, "SPOKE");

        let kept = go(
            &store,
            &terminal(),
            Action::KeepSuggestions {
                terms: vec!["epas1".into()],
            },
        );
        assert_eq!(kept.result, Ok(()));
        assert_eq!(kept.out, "Kept EPAS1.\n");
        assert_eq!(learning(&store).recent, vec![true]);

        let dismissed = go(
            &store,
            &terminal(),
            Action::DismissSuggestions {
                terms: vec![ids[1].to_string(), "NOPE".into(), "EPAS1".into()],
            },
        );
        assert_eq!(dismissed.out, "Dismissed HIF2A.\n");
        assert_eq!(
            dismissed.result,
            Err(Failure::failed(
                [msg_no_suggestion("NOPE"), msg_no_suggestion("EPAS1")].join("\n")
            )),
            "a kept word is no longer a suggestion"
        );
        let after = learning(&store);
        assert_eq!((after.kept, after.dismissed), (1, 1));
        assert_eq!(after.recent, vec![true, false]);

        let agent_row = go(
            &store,
            &terminal(),
            Action::DismissSuggestions {
                terms: vec!["spoke".into()],
            },
        );
        assert_eq!(agent_row.out, "Dismissed SPOKE.\n");
        assert_eq!(
            learning(&store).decisions(),
            2,
            "the agent's suggestion does not measure the chat extractor"
        );

        let view = store.view().unwrap();
        let left: Vec<(&str, EntryStatus)> = view
            .entries()
            .iter()
            .map(|e| (e.written.as_str(), e.status))
            .collect();
        assert_eq!(
            left,
            [
                ("EPAS1", EntryStatus::Active),
                ("VHL", EntryStatus::Suggested)
            ]
        );
        assert!(view.file().tombstoned("hif2a", EntryTier::Public));
        assert!(view.file().tombstoned("spoke", EntryTier::Public));
    }

    /// A word suggested both public and private (a private row from before the extractor
    /// stopped writing them) is kept in both tiers, and the private one says so. Only the
    /// public one is counted: the measure is answered to every caller, so it never counts
    /// a private row. (The private twin is found first: the store does not suggest a
    /// private twin of a word already listed in public.)
    #[test]
    fn keeping_a_word_suggested_in_both_tiers_keeps_both() {
        let (_root, store) = fresh();
        found_in_chats(&store, EntryTier::Private, &["EPAS1"]);
        found_in_chats(&store, EntryTier::Public, &["EPAS1"]);
        let kept = go(
            &store,
            &terminal(),
            Action::KeepSuggestions {
                terms: vec!["EPAS1".into()],
            },
        );
        assert_eq!(kept.result, Ok(()));
        assert_eq!(kept.out, "Kept EPAS1 (private).\nKept EPAS1.\n");
        assert_eq!(learning(&store).recent, vec![true], "the public row alone");
    }

    /// The decision that tips the measure prints the offer after what it did.
    #[test]
    fn the_dismissal_that_tips_the_measure_offers_to_turn_suggestions_off() {
        let (_root, store) = fresh();
        let ids = found_in_chats(&store, EntryTier::Public, &["EPAS1"]);
        after_dismissals(&store, 19);
        let run = go(
            &store,
            &terminal(),
            Action::DismissSuggestions {
                terms: vec![ids[0].to_string()],
            },
        );
        assert_eq!(run.result, Ok(()));
        assert_eq!(
            run.out,
            format!("Dismissed EPAS1.\n{}\n", msg_suggest_fewer(4))
        );
    }

    /// `dismiss ID` counts a dismissed chat suggestion as a **Dismiss**, as `accept`
    /// counts a **Keep**.
    #[test]
    fn dismiss_by_id_counts_a_chat_suggestion_as_dismissed() {
        let (_root, store) = fresh();
        let ids = found_in_chats(&store, EntryTier::Public, &["EPAS1", "HIF2A"]);
        let dismissed = go(
            &store,
            &terminal(),
            Action::Dismiss {
                ids: vec![ids[0].to_string()],
            },
        );
        assert_eq!(dismissed.result, Ok(()));
        assert_eq!(dismissed.out, "Dismissed EPAS1.\n");
        let accepted = go(
            &store,
            &terminal(),
            Action::Accept {
                ids: vec![ids[1].to_string()],
            },
        );
        assert_eq!(accepted.result, Ok(()));
        let after = learning(&store);
        assert_eq!(after.recent, vec![false, true]);
        assert!(store
            .view()
            .unwrap()
            .file()
            .tombstoned("epas1", EntryTier::Public));
    }

    /// `review` counts each chat suggestion it keeps or dismisses, and "Dismiss the rest"
    /// still deletes an agent row the person has not kept, uncounted.
    #[test]
    fn review_counts_each_decision_on_a_chat_suggestion() {
        let (_root, store) = fresh();
        found_in_chats(&store, EntryTier::Public, &["EPAS1", "HIF2A", "VHL"]);
        store
            .write(|file| {
                file.add(
                    NewTerm {
                        source: EntrySource::Agent,
                        ..NewTerm::person("SPOKE", EntryTier::Public)
                    },
                    Reach::All,
                    store::schema::now(),
                )
            })
            .unwrap();
        let mut script = Script {
            selections: VecDeque::from([vec![0]]),
            confirms: VecDeque::from([true]),
            ..Script::default()
        };
        let run = run_with(&store, &terminal(), &mut script, "", Action::Review);
        assert_eq!(run.result, Ok(()));
        assert_eq!(run.out, "Kept 1. Dismissed 3.\n");
        assert_eq!(learning(&store).recent, vec![true, false, false]);
        let view = store.view().unwrap();
        assert_eq!(view.entries().len(), 1);
        assert_eq!(view.entries()[0].written, "EPAS1");
        assert!(view.file().tombstoned("spoke", EntryTier::Public));
        assert!(view.file().tombstoned("vhl", EntryTier::Public));
    }

    /// A review that keeps or dismisses anything while most are dismissed offers to turn
    /// suggestions off; one that decided nothing does not, though the offer applies.
    #[test]
    fn a_review_that_leaves_most_dismissed_offers_to_turn_suggestions_off() {
        let (_root, store) = fresh();
        found_in_chats(&store, EntryTier::Public, &["EPAS1", "HIF2A"]);
        after_dismissals(&store, 20);
        assert!(offer_turning_off(store.view().unwrap().file()).is_some());
        let mut nothing = Script {
            selections: VecDeque::from([vec![]]),
            confirms: VecDeque::from([false]),
            ..Script::default()
        };
        let idle = run_with(&store, &terminal(), &mut nothing, "", Action::Review);
        assert_eq!(idle.out, "Kept 0. Dismissed 0.\n", "no decision, no offer");

        let mut script = Script {
            selections: VecDeque::from([vec![0]]),
            confirms: VecDeque::from([true]),
            ..Script::default()
        };
        let run = run_with(&store, &terminal(), &mut script, "", Action::Review);
        assert_eq!(
            run.out,
            format!("Kept 1. Dismissed 1.\n{}\n", msg_suggest_fewer(4))
        );
    }

    /// `settings` adds the person's measure only once there is a decision, so a store
    /// whose owner never reviewed a chat suggestion prints its three settings alone.
    #[test]
    fn settings_show_the_measure_only_after_a_decision() {
        let (_root, store) = fresh();
        let show = Action::Settings {
            agent_writes: None,
            learn_from_chats: None,
            suggest_from_chats: None,
        };
        let before = go(&store, &piped(), show.clone());
        assert!(
            before.out.ends_with("learn_from_chats: off\n"),
            "{}",
            before.out
        );
        assert!(!before.out.contains("suggestions:"));
        let ids = found_in_chats(&store, EntryTier::Public, &["EPAS1", "HIF2A", "VHL"]);
        go(
            &store,
            &terminal(),
            Action::KeepSuggestions {
                terms: vec![ids[0].to_string()],
            },
        );
        go(
            &store,
            &terminal(),
            Action::DismissSuggestions {
                terms: vec![ids[1].to_string()],
            },
        );
        let after = go(&store, &piped(), show);
        assert!(
            after.out.ends_with(
                "learn_from_chats: off\nsuggestions: kept 1 of 2 recent; a word needs 2 chats\n"
            ),
            "{}",
            after.out
        );
    }

    /// Turning suggestions on is the person choosing them: the offer to turn them off
    /// waits for 20 more decisions, as the daemon's route has it. Turning on what is
    /// already on changes nothing.
    #[test]
    fn turning_suggestions_on_quiets_the_offer() {
        let (_root, store) = fresh();
        after_dismissals(&store, 20);
        store
            .write(|file| {
                file.settings.learn_from_chats = false;
                Ok(())
            })
            .unwrap();
        assert!(learning(&store).offers_turning_off());
        let on = || Action::Settings {
            agent_writes: None,
            learn_from_chats: Some(true),
            suggest_from_chats: None,
        };
        let mut yes = Script {
            confirms: VecDeque::from([true]),
            ..Script::default()
        };
        let run = run_with(&store, &terminal(), &mut yes, "", on());
        assert_eq!(run.result, Ok(()));
        assert!(run.out.ends_with(
            "learn_from_chats: on\nsuggestions: kept 0 of 20 recent; a word needs 4 chats\n"
        ));
        let after = learning(&store);
        assert_eq!(after.chose_on_at, Some(20));
        assert!(!after.offers_turning_off());
        assert!(offer_turning_off(store.view().unwrap().file()).is_none());

        store
            .write(|file| {
                file.chat_learning.chose_on_at = None;
                Ok(())
            })
            .unwrap();
        let mut silent = Script::default();
        let again = run_with(&store, &terminal(), &mut silent, "", on());
        assert_eq!(again.result, Ok(()));
        assert!(silent.asked.is_empty(), "{:?}", silent.asked);
        assert_eq!(learning(&store).chose_on_at, None, "already on");
    }

    /// A scan a test drives: the reports it makes, then how it ends.
    struct Canned {
        reports: Vec<ScanProgress>,
        ended: std::result::Result<ScanProgress, ScanRefusal>,
        called: bool,
        stopped: bool,
    }

    impl Canned {
        fn ending(state: ScanState, read: u32, total: u32, words: u32) -> Self {
            Self {
                reports: Vec::new(),
                ended: Ok(progress(state, read, total, words)),
                called: false,
                stopped: false,
            }
        }
    }

    impl ChatScanner for Canned {
        async fn scan(
            &mut self,
            stop: &AtomicBool,
            report: &mut (dyn FnMut(&ScanProgress) + Send),
        ) -> std::result::Result<ScanProgress, ScanRefusal> {
            self.called = true;
            self.stopped = stop.load(Ordering::Acquire);
            for progress in &self.reports {
                report(progress);
            }
            self.ended.clone()
        }
    }

    fn progress(state: ScanState, read: u32, total: u32, words: u32) -> ScanProgress {
        ScanProgress {
            state,
            chats_read: read,
            chats_total: total,
            words_found: words,
        }
    }

    async fn scan_with(
        store: &DictionaryStore,
        facts: &Facts,
        scanner: &mut Canned,
        in_place: bool,
    ) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let stop = AtomicBool::new(false);
        let result = run_scan(
            facts,
            store,
            scanner,
            &stop,
            &mut ScanStreams {
                out: &mut out,
                err: &mut err,
                in_place,
            },
        )
        .await;
        Run {
            result,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    fn suggestions_on(store: &DictionaryStore) {
        after_dismissals(store, 0);
    }

    /// A scan is the person's: refused in an agent's shell and without a terminal before
    /// it reads anything, and refused with the page's sentence (exit 1) while suggestions
    /// are off, under managed policy, or with a store it cannot read.
    #[tokio::test]
    async fn a_scan_needs_the_person_and_suggestions_on() {
        let (_root, store) = fresh();
        for (facts, sentence) in [
            (piped(), MSG_DICTIONARY_TTY),
            (agent(), MSG_DICTIONARY_AGENT),
        ] {
            let mut scanner = Canned::ending(ScanState::Finished, 0, 0, 0);
            let run = scan_with(&store, &facts, &mut scanner, false).await;
            assert_eq!(run.result, Err(Failure::refused(sentence)));
            assert!(run.err.is_empty() && !scanner.called);
        }
        let off = format!("{}\n{MSG_SCAN_TURN_ON}", suggestion_scan::MSG_SCAN_OFF);
        let mut scanner = Canned::ending(ScanState::Finished, 0, 0, 0);
        let run = scan_with(&store, &terminal(), &mut scanner, false).await;
        assert_eq!(run.result, Err(Failure::failed(off)));
        assert!(run.out.is_empty() && !scanner.called);

        suggestions_on(&store);
        let reason = "Your organization's policy keeps the agent out of the dictionary.";
        let managed = Facts {
            agent_writes: Rule::Forbidden(reason.into()),
            ..terminal()
        };
        let run = scan_with(&store, &managed, &mut scanner, false).await;
        assert_eq!(
            run.result,
            Err(Failure::failed(suggestion_scan::MSG_SCAN_POLICY))
        );
        std::fs::write(store.store_path(), b"{ not json").unwrap();
        let run = scan_with(&store, &terminal(), &mut scanner, false).await;
        assert_eq!(
            run.result,
            Err(Failure::failed(suggestion_scan::MSG_SCAN_UNREADABLE))
        );
        assert!(!scanner.called);
    }

    /// At a terminal the progress line is drawn in place and cleared at the end; on any
    /// other stream it is one line at the end. The last sentences go to stdout.
    #[tokio::test]
    async fn a_scan_draws_its_progress_and_ends_with_the_pages_sentences() {
        let (_root, store) = fresh();
        suggestions_on(&store);
        let reports = vec![
            progress(ScanState::Running, 0, 0, 0),
            progress(ScanState::Running, 0, 3, 0),
            progress(ScanState::Running, 1, 3, 1),
            progress(ScanState::Running, 3, 3, 2),
            progress(ScanState::Finished, 3, 3, 2),
        ];
        let ended = "Read 3 chats. 2 new words to review.\nReview them with `biorouter dictionary suggestions`.\n";

        let mut scanner = Canned {
            reports: reports.clone(),
            ..Canned::ending(ScanState::Finished, 3, 3, 2)
        };
        let drawn = scan_with(&store, &terminal(), &mut scanner, true).await;
        assert_eq!(drawn.result, Ok(()));
        assert!(scanner.called && !scanner.stopped);
        assert_eq!(drawn.out, ended);
        let progress_part = drawn.err.split_once('\n').unwrap().1;
        assert_eq!(
            progress_part,
            "\r\x1b[2KReading chat 1 of 3, 0 words found so far\
             \r\x1b[2KReading chat 2 of 3, 1 word found so far\
             \r\x1b[2KReading chat 3 of 3, 2 words found so far\
             \r\x1b[2K"
        );

        let mut scanner = Canned {
            reports,
            ..Canned::ending(ScanState::Finished, 3, 3, 2)
        };
        let lined = scan_with(&store, &terminal(), &mut scanner, false).await;
        assert_eq!(lined.out, ended);
        assert_eq!(
            lined.err.split_once('\n').unwrap().1,
            "Reading chat 3 of 3, 2 words found so far\n"
        );
    }

    #[tokio::test]
    async fn a_scan_says_how_it_ended() {
        let (_root, store) = fresh();
        suggestions_on(&store);
        let review = MSG_SCAN_REVIEW;
        for (state, read, total, words, said) in [
            (
                ScanState::Finished,
                0,
                0,
                0,
                "Read 0 chats. No new words found.\n".to_string(),
            ),
            (
                ScanState::Finished,
                1,
                1,
                0,
                "Read 1 chat. No new words found.\n".to_string(),
            ),
            (
                ScanState::Finished,
                40,
                40,
                1,
                format!("Read 40 chats. 1 new word to review.\n{review}\n"),
            ),
            (
                ScanState::Stopped,
                12,
                40,
                3,
                format!("Stopped after 12 of 40 chats. 3 new words to review.\n{review}\n"),
            ),
            (
                ScanState::Stopped,
                0,
                1,
                0,
                "Stopped after 0 of 1 chat. No new words found.\n".to_string(),
            ),
        ] {
            let mut scanner = Canned::ending(state, read, total, words);
            let run = scan_with(&store, &terminal(), &mut scanner, false).await;
            assert_eq!(run.result, Ok(()), "{state:?}");
            assert_eq!(run.out, said);
        }
        let mut failed = Canned::ending(ScanState::Failed, 2, 40, 0);
        let run = scan_with(&store, &terminal(), &mut failed, false).await;
        assert_eq!(run.result, Err(Failure::failed(MSG_SCAN_FAILED)));
        assert!(run.out.is_empty());
        let mut refused = Canned {
            ended: Err(ScanRefusal::Off),
            ..Canned::ending(ScanState::Finished, 0, 0, 0)
        };
        let run = scan_with(&store, &terminal(), &mut refused, false).await;
        assert_eq!(
            run.result.unwrap_err().message.lines().next(),
            Some(suggestion_scan::MSG_SCAN_OFF),
            "suggestions turned off after the scan was asked for"
        );
    }

    /// A session's synchronous `run` cannot wait for a scan, so it names the command.
    #[test]
    fn run_names_the_scan_command_instead_of_scanning() {
        let (_root, store) = fresh();
        let run = go(&store, &terminal(), Action::ScanSuggestions);
        assert_eq!(run.result, Err(Failure::failed(MSG_SCAN_FROM_THE_COMMAND)));
    }

    /// The sentences the suggestion commands print, word for word.
    #[test]
    fn the_suggestion_sentences_read_as_written() {
        let cases: Vec<(String, &str)> = vec![
            (msg_suggest_fewer(3), "You dismissed most suggestions, so Biorouter now suggests fewer: a word has to show up in 3 chats. Turn them off with `biorouter dictionary settings --learn-from-chats off`."),
            (MSG_NO_SUGGESTIONS.into(), "No suggestions to review."),
            (msg_no_suggestion("EPAS1"), "There is no suggestion EPAS1 to review."),
            (MSG_SCAN_FAILED.into(), "Biorouter could not finish reading your chats."),
            (MSG_SCAN_REVIEW.into(), "Review them with `biorouter dictionary suggestions`."),
            (MSG_SCAN_TURN_ON.into(), "To turn it on here, run `biorouter dictionary settings --learn-from-chats on`."),
            (MSG_SCAN_FROM_THE_COMMAND.into(), "Run `biorouter dictionary suggestions scan` to find words in your recent chats."),
            (line_scan_progress(&progress(ScanState::Running, 11, 40, 3)), "Reading chat 12 of 40, 3 words found so far"),
        ];
        for (sentence, written) in cases {
            assert_eq!(sentence, written);
        }
    }
}
