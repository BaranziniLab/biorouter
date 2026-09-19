//! Refuse tool access to the **session database** — the transcript store every
//! conversation on this machine is written to.
//!
//! # The hole this closes
//!
//! Every privacy control in this campaign is a gate on a *BioRouter API*:
//! `chatrecall` filters rows the caller's tier may not see, the KB listings ask
//! the tier question before they answer, the spawn gate compares affiliations.
//! All of them read `~/.config/biorouter/sessions/sessions.db` on the other
//! side, and all of them are reached through code that knows whose session is
//! asking.
//!
//! `cat ~/.config/biorouter/sessions/sessions.db` reaches the same bytes with
//! none of that. So does `sqlite3 … "select content from messages"`, and so
//! does `text_editor(command="view", path=…)`. The file is one `read(2)` away
//! from every tool that takes a path, and nothing in
//! `crates/biorouter/src/security/` guarded it: the directory held exactly one
//! filesystem inspector, [`crate::security::global_memory`], and it guards
//! `~/.config/biorouter/memory`.
//!
//! The store is not one conversation's data. It is every conversation's — every
//! session any project on this computer ever ran, including the ones classified
//! `private`. Reading it out of band is the widest disclosure in the product.
//!
//! # It is about the channel, not the tier
//!
//! This gate does **not** read `session.privacy_tier`, and that is deliberate.
//! A tier decides which rows a caller may be *shown through an API that
//! filters*; it says nothing about whether a caller may read the file the API
//! reads from. A private-capability caller that opens the file gets the public
//! sessions too — and the private ones belonging to every other project. So the
//! refusal fires identically for both tiers, and
//! [`a_private_capability_caller_is_refused_too`] is the regression that keeps
//! it that way.
//!
//! # The siblings are not optional
//!
//! SQLite in WAL mode — which this store uses (`SessionStorage::create_pool`
//! sets `journal_mode = WAL`) — keeps recently committed pages in
//! `sessions.db-wal` until a checkpoint folds them back. A barrier on the bare
//! `sessions.db` therefore leaves the **newest** turns readable in the file
//! beside it, which is the opposite of the intended ordering; `sessions.db-shm`
//! is the shared-memory index into it. All three are refused, and so is
//! anything else under the `sessions/` directory: the store is a directory, a
//! future journal suffix (`-journal`, a `.bak` a user drops there) is the same
//! disclosure, and nothing legitimate reads that directory through a tool.
//!
//! # Refusing to read it, not refusing to talk about it
//!
//! [`crate::security::global_memory`] learned this the expensive way and its
//! module docs carry the worked examples: a check that walked every string in
//! the payload also refused `text_editor(path="docs/foo.md", file_text="…the
//! store lives in `~/.config/biorouter/memory`…")`, because a path-match ends on
//! a backtick or a space and that is exactly how prose spells a path. The agent
//! could no longer document the feature, edit the repository's own docs, or
//! commit either.
//!
//! The same trap is *worse* here, because `sessions.db` is named all over this
//! repository — in `CLAUDE.md`, in `docs/`, in commit messages about this very
//! gate, and in the test fixtures of the session layer itself. A gate that
//! fires on those fires constantly, and a gate that fires constantly gets
//! switched off.
//!
//! So the check asks **which argument** carries the mention ([`ArgumentRole`]):
//! a path the tool will open, a shell command line read as the shell reads it,
//! or a script body. A `file_text` written *to* some other path is a quotation,
//! and a `-m` commit message is one shell word rather than a path operand.
//! Neither goes near the store, and neither is refused.
//!
//! # What this is not
//!
//! At the **inspector** door it is a *literal-reference* barrier, with the same
//! stated limit as the global-memory one: a command that computes its path
//! (`p="$HOME/.config"; cat "$p/biorouter/sessions/sessions.db"`) walks past,
//! because nothing there resolves paths. [`SessionStoreInspector`] is the tool
//! call a model *wrote*; that is all a `ToolInspector` can ever see.
//!
//! # Three doors, and which ones this closes
//!
//! [`crate::security::global_memory`] learned that a gate at the inspector chain
//! alone is a gate at one door of three, and the other two are the ones a
//! runtime-assembled reference walks through. The same three are closed here,
//! and by the same instruments:
//!
//! | Door | What guards the store there |
//! |---|---|
//! | The agent loop | [`SessionStoreInspector`], in the inspector chain (`agents/agent.rs`) |
//! | Tool calls a script makes from inside `execute_code` | [`uninspected_boundary_refusal`], in `agents/code_execution_extension.rs`'s JS-host loop |
//! | `POST /agent/call_tool` | [`uninspected_boundary_refusal`], in `biorouter-server/src/routes/agent.rs` |
//!
//! The two boundary doors are **stronger** than the inspector, not weaker: they
//! read the dispatched tool name and the already-evaluated arguments, so
//! `cat "$p/…/sessions.db"` arrives there as the command it will actually run.
//! The inspector still covers the outer `execute_code` call itself, whose `code`
//! argument is read as a script body — a literal store path inside the script is
//! refused before the script ever runs.
//!
//! ⚠ **The fourth seam is still open, and it is the unforgeable one.** A refusal
//! inside the filesystem tools themselves — `biorouter-mcp`'s developer and
//! computer-controller servers, which is where `global_memory_file_barrier.rs`
//! put the equivalent for the memory root — would refuse the store whatever tool,
//! whatever mode, and whatever route reached them, including an MCP client
//! talking to `biorouter mcp developer` over stdio with no agent anywhere. That
//! is a `biorouter-mcp` change and is reported as the open seam, not silently
//! skipped.

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::config::paths::Paths;
use crate::config::BioRouterMode;
use crate::conversation::message::{Message, ToolRequest};
use crate::security::policy::command::{redirect_targets, ParsedCommand};
use crate::session::session_manager::{DB_NAME, SESSIONS_FOLDER};
use crate::tool_inspection::{InspectionAction, InspectionResult, ToolInspector};
use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

/// This inspector's name, as it appears on an [`InspectionResult`].
pub const SESSION_STORE_INSPECTOR_NAME: &str = "session_store";

/// SQLite's write-ahead log and its shared-memory index. Named as constants
/// because the whole point of this module is that they are not an afterthought:
/// the WAL holds the newest turns, so a barrier that omits it protects the old
/// conversations and leaks the current one.
const WAL_SUFFIX: &str = "-wal";
const SHM_SUFFIX: &str = "-shm";

/// The places one session store occupies on disk, resolved the same way the
/// production store resolves them — [`Paths::data_dir`] +
/// [`SESSIONS_FOLDER`] + [`DB_NAME`], which is exactly what
/// `SessionStorage::new` joins. Deriving it rather than spelling it keeps the
/// gate correct under `BIOROUTER_PATH_ROOT` (the test seam) and on whatever
/// `data_dir` the platform hands back.
///
/// The directory comes first and subsumes the three files for prefix matching.
/// The files are listed anyway: they are the three names the gate is *about*,
/// and a test that asserts each one by name is the only kind that notices if
/// the directory form is ever narrowed.
fn session_store_paths() -> Vec<PathBuf> {
    let dir = Paths::data_dir().join(SESSIONS_FOLDER);
    let db = dir.join(DB_NAME);

    let sidecar = |suffix: &str| {
        let mut path = db.clone().into_os_string();
        path.push(suffix);
        PathBuf::from(path)
    };

    vec![dir, db.clone(), sidecar(WAL_SUFFIX), sidecar(SHM_SUFFIX)]
}

/// Every spelling of those places a tool argument might carry: the absolute
/// path, and — when the store is under the user's home — the `~`, `$HOME` and
/// `${HOME}` abbreviations a shell command normally uses.
fn session_store_path_forms() -> Vec<String> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let mut forms = Vec::new();
    for path in session_store_paths() {
        forms.push(path.to_string_lossy().into_owned());
        if let Some(home) = home.as_ref() {
            if let Ok(tail) = path.strip_prefix(std::path::Path::new(home)) {
                let tail = tail.to_string_lossy();
                forms.push(format!("~/{tail}"));
                forms.push(format!("$HOME/{tail}"));
                forms.push(format!("${{HOME}}/{tail}"));
            }
        }
    }
    forms
}

/// Does `text` name `store` *as a path*, rather than merely start with its
/// characters?
///
/// `<data>/sessions-archive/…` and `<data>/sessions.db-old` are ordinary paths
/// a plain `contains` would deny; `<store>/x`, a bare `<store>` and a quoted
/// `"<store>"` are the store. So the match has to end on a path boundary.
///
/// The line falls at the *store directory*, not at the three filenames: a file
/// under `sessions/` is store data whatever it is called (a `-journal`, a copy
/// dropped there, the legacy per-session JSONL files), while a lookalike beside
/// that directory is not.
///
/// `-` is deliberately **not** a boundary character even though the two
/// siblings end in one: `sessions.db-wal` is matched by its own form (and by
/// the directory's), so nothing needs a loose ending on `sessions.db` — and
/// treating `-` as a boundary would refuse `<data>/sessions.db-notes.md`.
fn names_path(text: &str, store: &str) -> bool {
    text.match_indices(store).any(|(at, matched)| {
        text.get(at + matched.len()..)
            .and_then(|after| after.chars().next())
            // Nothing after the match is the bare store path, which counts.
            .is_none_or(|c| {
                c == '/' || c == '\\' || c.is_whitespace() || "\"'`)];,;:&|".contains(c)
            })
    })
}

/// What an argument's value *is*, which decides how a mention of the store's
/// path inside it should be read. See the module docs: this distinction is the
/// difference between a gate and an outage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgumentRole {
    /// A filesystem path the tool will resolve and open.
    Path,
    /// A shell command line, read as the shell reads it.
    ShellCommand,
    /// Executable source text, whose literal references are scanned whole.
    Script,
}

/// Argument keys whose value is a **directory**, on top of the shared
/// [`crate::security::is_path_argument_key`] list. The store is a directory, so
/// `shell(working_directory=<dir>)` reaches it as surely as a file path does —
/// and `cd <dir> && sqlite3 sessions.db` is the shape that motivates it.
const DIRECTORY_ARG_KEYS: &[&str] = &["dir", "directory", "working_directory", "local_dir"];

/// Argument keys whose value is executable source text (`execute_code.code`,
/// `compute_python.code`, and generic external `script` fields).
const SCRIPT_ARG_KEYS: &[&str] = &["code", "script"];

fn argument_role(key: &str) -> Option<ArgumentRole> {
    let lowered = key.to_ascii_lowercase();
    if crate::security::is_path_argument_key(&lowered)
        || DIRECTORY_ARG_KEYS.contains(&lowered.as_str())
    {
        return Some(ArgumentRole::Path);
    }
    if crate::security::is_command_argument_key(&lowered) {
        return Some(ArgumentRole::ShellCommand);
    }
    if SCRIPT_ARG_KEYS.contains(&lowered.as_str()) {
        return Some(ArgumentRole::Script);
    }
    None
}

/// Does this shell command line name the store *as a path operand*?
///
/// A raw substring search cannot answer this, because the same characters mean
/// two different things:
///
/// ```text
/// sqlite3 ~/.config/biorouter/sessions/sessions.db "select * from messages"
/// git commit -m "docs: sessions live in ~/.config/biorouter/sessions"
/// ```
///
/// So the command goes through [`ParsedCommand`], the policy engine's
/// tokenizer, and each argv token is tested. Quoting is what separates them:
/// `shlex` returns the commit message as **one token** containing spaces, which
/// is not a path, while the `sqlite3` operand is a token that *is* the store.
/// Reusing the parser also buys the pipeline/`;`/`&&` split, redirect targets,
/// and the recursion through `sh -c`.
///
/// Unlike [`names_path`], whitespace inside a token does not end a match here:
/// a token with whitespace in it is a quoted sentence, which is the case this
/// function exists to let through.
fn shell_command_names_store(command: &str, forms: &[String]) -> bool {
    let token_is_store = |token: &str| {
        forms.iter().any(|form| {
            token.strip_prefix(form.as_str()).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')
            })
        })
    };
    let parsed = ParsedCommand::parse(command, std::path::Path::new("."));
    parsed
        .segments
        .iter()
        .flat_map(|segment| segment.argv.iter())
        .any(|token| token_is_store(token))
        // `> <store>` is a write target, not an argv token — and overwriting
        // the transcript store is no better than reading it.
        || redirect_targets(command).iter().any(|t| token_is_store(t))
}

/// Does `args` name the session store *in an argument that would open it*?
///
/// Walks nested objects and arrays, because a path arrives as
/// `{"opts": {"paths": ["…"]}}` as readily as a top-level `path`; the walk
/// descends through keys with no role of their own, looking for one that has.
fn references_session_store(args: &Map<String, Value>) -> bool {
    fn in_map(map: &Map<String, Value>, forms: &[String]) -> bool {
        map.iter().any(|(key, value)| match argument_role(key) {
            Some(role) => in_role(value, forms, role),
            None => descend(value, forms),
        })
    }

    fn descend(value: &Value, forms: &[String]) -> bool {
        match value {
            Value::Object(map) => in_map(map, forms),
            Value::Array(items) => items.iter().any(|item| descend(item, forms)),
            _ => false,
        }
    }

    fn in_role(value: &Value, forms: &[String], role: ArgumentRole) -> bool {
        match value {
            Value::String(text) => match role {
                ArgumentRole::Path | ArgumentRole::Script => {
                    forms.iter().any(|form| names_path(text, form))
                }
                ArgumentRole::ShellCommand => shell_command_names_store(text, forms),
            },
            Value::Array(items) => items.iter().any(|item| in_role(item, forms, role)),
            Value::Object(map) => map.values().any(|v| in_role(v, forms, role)),
            _ => false,
        }
    }

    let forms = session_store_path_forms();
    in_map(args, &forms)
}

/// What the model is told. Naming the route that *does* work is the
/// load-bearing half: a refusal that only says no gets retried, or gets
/// reported to the user as a broken product.
fn session_store_refusal() -> String {
    "Refused: this call names Biorouter's session database by path. That file (and its \
     -wal / -shm siblings) holds the transcript of every conversation on this computer, \
     across every project (including sessions classified private), and reading it as a \
     file bypasses every check that decides which of them you are allowed to see. The \
     refusal is about the channel, not about your capability: it applies to a \
     private-capability session exactly as it does to a public one. Use the chatrecall \
     tool to search past conversations or load one by id (it returns only what this \
     session may see), and ask the user directly for anything it will not give you. The \
     user can browse, export or delete their own sessions from the Biorouter session \
     list. This project's own files are not affected."
        .to_string()
}

/// Classify a tool call against the session store. `None` means "this call does
/// not name it" — which is every call but a handful.
pub fn session_store_gate(args: &Map<String, Value>) -> Option<String> {
    references_session_store(args).then(session_store_refusal)
}

/// The refusal a dispatch boundary that never reaches a [`ToolInspector`] must
/// return for a tool call naming the session store, or `None` if the call does
/// not name it.
///
/// ⚠ **This is not a copy of [`SessionStoreInspector`] at another door; it is a
/// strictly stronger check at a door the inspector cannot reach.** The inspector
/// reads the tool call a model *wrote*, so the module docs' stated limit applies
/// there: `p="$HOME/.config"; cat "$p/biorouter/sessions/sessions.db"` computes
/// its path and walks past. A boundary reads the **dispatched** name and the
/// **already-evaluated** arguments — the shell command line as it will actually
/// run, the path as it will actually be opened — so at these two doors the
/// runtime-assembled form is refused too. That walk-past is precisely what made
/// [`crate::security::global_memory`]'s boundary refusals necessary, and it was
/// this module's openly-declared gap until these doors were closed.
///
/// It returns the SAME sentence [`session_store_gate`] does, deliberately.
/// `global_memory`'s boundary refusal varies its remedy by door because its
/// remedy is *ask the user*, and neither door can; the session store has no such
/// branch — it is refused in a conversation, in a script and over HTTP alike,
/// and the one route that still works (`chatrecall`) is named in the one
/// sentence. Two spellings of one refusal is how a rule becomes two
/// slightly-different rules.
///
/// `boundary` is therefore not read by the decision at all — only logged, so an
/// operator can tell which door a refusal came from.
pub fn uninspected_boundary_refusal(
    tool_name: &str,
    args: Option<&Map<String, Value>>,
    boundary: crate::security::UninspectedBoundary,
) -> Option<String> {
    let reason = session_store_gate(args?)?;
    tracing::warn!(
        counter.biorouter.session_store_uninspected_refused = 1,
        tool_name = %tool_name,
        boundary = ?boundary,
        "Refused a tool that named the session database at a boundary no inspector sees"
    );
    Some(reason)
}

/// Inspector that refuses tool reads of the session database, in every mode
/// that runs tools. See the module docs for the policy.
pub struct SessionStoreInspector;

#[async_trait]
impl ToolInspector for SessionStoreInspector {
    fn name(&self) -> &'static str {
        SESSION_STORE_INSPECTOR_NAME
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn inspect(
        &self,
        tool_requests: &[ToolRequest],
        _messages: &[Message],
        biorouter_mode: BioRouterMode,
        // Deliberately unread. The store is refused by channel, not by tier —
        // see the module docs and `a_private_capability_caller_is_refused_too`.
        _session: &crate::session::Session,
    ) -> Result<Vec<InspectionResult>> {
        // Chat dispatches no tools; the agent splices a canned response.
        if biorouter_mode == BioRouterMode::Chat {
            return Ok(vec![]);
        }

        let mut results = Vec::new();
        for request in tool_requests {
            let Ok(tool_call) = &request.tool_call else {
                continue;
            };
            let Some(args) = tool_call.arguments.as_ref() else {
                continue;
            };
            let Some(reason) = session_store_gate(args) else {
                continue;
            };
            tracing::warn!(
                counter.biorouter.session_store_read_refused = 1,
                tool_name = %tool_call.name,
                tool_request_id = %request.id,
                "Refused a tool that named the session database by path"
            );
            results.push(InspectionResult {
                tool_request_id: request.id.clone(),
                action: InspectionAction::Deny,
                reason,
                confidence: 1.0,
                inspector_name: self.name().to_string(),
                finding_id: Some(format!("SDB-{}", Uuid::new_v4().simple())),
            });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::SessionClassification;
    use rmcp::model::CallToolRequestParams;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    /// The three filenames, built independently of the production helper so the
    /// test cannot agree with the code by construction.
    fn store_files() -> Vec<String> {
        let db = Paths::data_dir()
            .join(SESSIONS_FOLDER)
            .join(DB_NAME)
            .to_string_lossy()
            .into_owned();
        vec![db.clone(), format!("{db}-wal"), format!("{db}-shm")]
    }

    // --- the gate ---------------------------------------------------------

    /// The whole point. `cat`, `sqlite3` and `text_editor` are the three ways a
    /// model actually reaches a file, and each is tried against each of the
    /// three filenames — the bare database *and* the two SQLite siblings, since
    /// the write-ahead log holds the newest turns.
    #[test]
    fn every_read_of_every_store_file_is_refused() {
        for file in store_files() {
            for (label, arguments) in [
                ("cat", json!({ "command": format!("cat {file}") })),
                (
                    "sqlite3",
                    json!({ "command": format!("sqlite3 {file} \"select content from messages\"") }),
                ),
                (
                    "text_editor",
                    json!({ "command": "view", "path": file.clone() }),
                ),
                // The other shapes that read the same bytes.
                (
                    "strings",
                    json!({ "command": format!("strings {file} | head") }),
                ),
                (
                    "copy out",
                    json!({ "command": format!("cp {file} /tmp/x") }),
                ),
                ("quoted", json!({ "command": format!("cat \"{file}\"") })),
                // A path nested inside a structure, and inside a script body.
                ("nested", json!({ "opts": { "paths": [file.clone()] } })),
                (
                    "script",
                    json!({ "code": format!("shell({{command: 'cat {file}'}})") }),
                ),
                // Writing to it is the same breach in the other direction.
                ("redirect", json!({ "command": format!("echo x > {file}") })),
            ] {
                let refusal = session_store_gate(&args(arguments.clone()));
                assert!(
                    refusal.is_some(),
                    "{label} reached the session store: {arguments}"
                );
                let refusal = refusal.unwrap();
                assert!(
                    refusal.contains("chatrecall"),
                    "the refusal must name the route that still works, or the \
                     model reports the product as broken: {refusal}"
                );
            }
        }
    }

    /// `cd` into the directory and open the database by its bare name — the
    /// obvious way around a gate that only knows one absolute string. The
    /// directory itself is a store path, so the `cd` operand is caught.
    #[test]
    fn reaching_the_store_through_its_directory_is_refused() {
        let dir = Paths::data_dir()
            .join(SESSIONS_FOLDER)
            .to_string_lossy()
            .into_owned();
        for command in [
            format!("cd {dir} && sqlite3 sessions.db .dump"),
            format!("ls -la {dir}"),
            format!("tar czf /tmp/x.tgz {dir}"),
            // Everything *in* that directory is store data, not just the three
            // filenames: a `-journal` from a non-WAL moment, a copy a user
            // dropped there, the legacy per-session JSONL files the importer
            // still reads. The barrier is the directory.
            format!("cat {dir}/sessions.db-archive.txt"),
            format!("cat {dir}/2026-07-31.jsonl"),
        ] {
            assert!(
                session_store_gate(&args(json!({ "command": command.clone() }))).is_some(),
                "the store's directory is the store: {command}"
            );
        }
        assert!(
            session_store_gate(&args(json!({ "working_directory": dir }))).is_some(),
            "a tool run inside the store's directory reaches it"
        );
    }

    /// Refusing by path must not refuse by resemblance. Each of these is an
    /// ordinary file or an unrelated directory, and a gate that trips on them
    /// is a gate someone turns off.
    ///
    /// Note where the line falls: the barrier is the *store directory*, so
    /// these are all paths **outside** it that merely look like it. A file
    /// inside `sessions/` is store data whatever it is called, and
    /// [`reaching_the_store_through_its_directory_is_refused`] pins that.
    #[test]
    fn only_the_store_itself_is_refused() {
        let data_dir = Paths::data_dir().to_string_lossy().into_owned();

        for (label, arguments) in [
            // Directories whose names merely start with the store's.
            (
                "archive dir",
                json!({ "command": format!("cat {data_dir}/sessions-archive/sessions.db") }),
            ),
            (
                "backup",
                json!({ "command": format!("cat {data_dir}/sessions.db-archive.txt") }),
            ),
            // A stray file beside the store directory, sharing its basename.
            (
                "same basename, wrong place",
                json!({ "command": format!("sqlite3 {data_dir}/sessions.db .schema") }),
            ),
            // The config/data directory the store lives in: backing it up, or
            // reading the config file, is an ordinary request.
            ("parent dir", json!({ "command": format!("ls {data_dir}") })),
            (
                "config file",
                json!({ "command": format!("cat {data_dir}/config.yaml") }),
            ),
            // A project's *own* sqlite file that happens to share the name.
            (
                "same basename",
                json!({ "command": "sqlite3 ./data/sessions.db .schema" }),
            ),
            ("unrelated", json!({ "command": "ls -la /tmp" })),
            (
                "unrelated editor",
                json!({ "command": "view", "path": "crates/biorouter/src/session/mod.rs" }),
            ),
        ] {
            assert!(
                session_store_gate(&args(arguments.clone())).is_none(),
                "{label} is not the session store and must not be refused: {arguments}"
            );
        }
    }

    /// The distinction `global_memory.rs` already solves, restated here because
    /// getting it wrong is how this inspector would be disabled: *reading* the
    /// store is refused, *talking about* it is not. `sessions.db` is named all
    /// over this repository — CLAUDE.md, docs/, the session layer's own tests,
    /// and the commit message for this gate.
    ///
    /// Nothing below goes near the file: a `file_text` is written to the `path`
    /// beside it, and a commit message is one shell word.
    #[test]
    fn prose_that_merely_names_the_store_is_not_refused() {
        for file in store_files() {
            for (label, arguments) in [
                (
                    "docs write",
                    json!({
                        "command": "write",
                        "path": "docs/privacy/session-store.md",
                        "file_text": format!(
                            "Transcripts live in `{file}`, which tools may not read.\n"
                        ),
                    }),
                ),
                // The same path in a plain sentence, no backticks — a match ends
                // on whitespace too.
                (
                    "release notes",
                    json!({
                        "command": "write",
                        "path": "docs/releases/notes/v1.88.6.md",
                        "file_text": format!("The gate refuses tool reads of {file}.\n"),
                    }),
                ),
                (
                    "doc edit",
                    json!({
                        "command": "str_replace",
                        "path": "CLAUDE.md",
                        "old_str": format!("Session history: `{file}`"),
                        "new_str": format!("Session history: `{file}` (guarded)"),
                    }),
                ),
                // Committing that documentation.
                (
                    "commit",
                    json!({ "command": format!(
                        "git commit -m \"docs(privacy): {file} is off limits to tools\""
                    ) }),
                ),
                (
                    "commit, single quotes",
                    json!({ "command": format!(
                        "git commit -m 'fix(privacy): stop tool reads of {file}'"
                    ) }),
                ),
                // Telling the *user* where their transcripts are is something
                // the agent must be able to do.
                (
                    "answer to the user",
                    json!({
                        "command": "write",
                        "path": "README.md",
                        "file_text": format!("Your chats are stored in {file}.\n"),
                    }),
                ),
            ] {
                assert!(
                    session_store_gate(&args(arguments.clone())).is_none(),
                    "{label} only mentions the store and must not be refused: {arguments}"
                );
            }
        }
    }

    /// The narrowing above must not become the bypass: an argument that is a
    /// path, or a shell word in a path position, still names the store
    /// operatively. This is the discriminating half of the test above — a
    /// "fix" that let these through would be worse than the outage it avoids.
    #[test]
    fn a_path_argument_is_still_refused_after_the_prose_narrowing() {
        for file in store_files() {
            for arguments in [
                json!({ "command": "view", "path": file.clone() }),
                // Innocent content, but the target is the store.
                json!({ "command": "write", "path": file.clone(), "file_text": "hello\n" }),
                json!({ "command": format!("cat {file}") }),
                // ⚠ The inner path is QUOTED, and on Windows that is the whole
                // difference between a command that reads the store and one
                // that cannot. `sh -c "cat C:\Users\…\sessions.db"` hands the
                // inner line to a POSIX shell, which reads every `\` as an
                // escape and opens `C:UsersAppData…` — a file that is not the
                // store, and that `shell_command_names_store`'s POSIX
                // tokenizer therefore (correctly) declines to call the store.
                // Quoting is what makes the path survive the sub-shell, so it
                // is the spelling that actually names the store operatively on
                // both platforms — which is the claim this case exists to make.
                json!({ "command": format!("sh -c \"cat '{file}'\"") }),
            ] {
                assert!(
                    session_store_gate(&args(arguments.clone())).is_some(),
                    "{arguments} points at the session store and must be refused"
                );
            }
        }
    }

    // --- the two uninspected boundaries -----------------------------------

    /// The doors [`SessionStoreInspector`] never sees answer the same way it
    /// does, at both boundaries and for every store file.
    ///
    /// ⚠ These are the *decision*'s corners. That the decision is actually
    /// consulted at each door is a different claim and cannot be made here:
    /// `crates/biorouter/tests/session_store_dispatch_boundary.rs` drives a
    /// script's inner tool call through the real dispatcher, and
    /// `crates/biorouter-server/tests/agent_call_tool_session_store.rs` drives
    /// the real HTTP handler. A guard with no caller is this campaign's
    /// signature failure, and a unit test cannot tell you it has one.
    #[test]
    fn both_uninspected_boundaries_refuse_every_store_file() {
        for boundary in [
            crate::security::UninspectedBoundary::ExecuteCodeScript,
            crate::security::UninspectedBoundary::AgentCallToolRoute,
        ] {
            for file in store_files() {
                for (label, arguments) in [
                    ("cat", json!({ "command": format!("cat {file}") })),
                    ("view", json!({ "command": "view", "path": file.clone() })),
                    ("redirect", json!({ "command": format!("echo x > {file}") })),
                ] {
                    let refusal = uninspected_boundary_refusal(
                        "developer__shell",
                        Some(&args(arguments.clone())),
                        boundary,
                    );
                    assert_eq!(
                        refusal.as_deref(),
                        Some(session_store_refusal().as_str()),
                        "{label} reached the store at {boundary:?}, or answered it in words \
                         the inspector does not use: {arguments}"
                    );
                }
            }
        }
    }

    /// The boundary is silent on everything else — including a call with no
    /// arguments at all, which is the shape `POST /agent/call_tool` produces
    /// from a non-object `arguments` field.
    ///
    /// A boundary that refused broadly would take out both doors at once: every
    /// tool call a script makes and every call over the route.
    #[test]
    fn the_boundaries_are_silent_on_everything_but_the_store() {
        let boundary = crate::security::UninspectedBoundary::ExecuteCodeScript;
        assert_eq!(
            uninspected_boundary_refusal("developer__shell", None, boundary),
            None,
            "a call with no arguments was refused"
        );
        for arguments in [
            json!({ "command": "ls -la /tmp" }),
            json!({ "command": "view", "path": "src/main.rs" }),
            // The near miss: a directory whose name merely starts with the
            // store's.
            json!({ "command": format!(
                "cat {}/sessions-archive/sessions.db",
                Paths::data_dir().to_string_lossy()
            ) }),
        ] {
            assert_eq!(
                uninspected_boundary_refusal(
                    "developer__shell",
                    Some(&args(arguments.clone())),
                    boundary
                ),
                None,
                "{arguments} is not the session store and must not be refused"
            );
        }
    }

    /// The refusal is the SAME sentence at both doors and in the agent loop.
    ///
    /// `global_memory`'s boundary refusal varies its remedy by door because its
    /// remedy is *ask the user*, and neither door can. The session store has no
    /// such branch — it is refused everywhere — so two spellings would be two
    /// rules waiting to drift apart, which is the failure this campaign names
    /// most often.
    #[test]
    fn the_boundary_and_the_inspector_say_exactly_the_same_thing() {
        let arguments = args(json!({ "command": format!("cat {}", store_files()[0]) }));
        let script = uninspected_boundary_refusal(
            "developer__shell",
            Some(&arguments),
            crate::security::UninspectedBoundary::ExecuteCodeScript,
        );
        let route = uninspected_boundary_refusal(
            "developer__shell",
            Some(&arguments),
            crate::security::UninspectedBoundary::AgentCallToolRoute,
        );
        assert_eq!(script, route);
        assert_eq!(script.as_deref(), session_store_gate(&arguments).as_deref());
    }

    // --- the inspector, end to end ----------------------------------------

    fn request(id: &str, name: &str, arguments: Value) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                name: name.to_string().into(),
                arguments: Some(args(arguments)),
                meta: None,
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    fn session_at(tier: SessionClassification) -> crate::session::Session {
        crate::session::Session {
            privacy_tier: tier,
            ..Default::default()
        }
    }

    async fn inspect_cat(
        mode: BioRouterMode,
        session: &crate::session::Session,
    ) -> Vec<InspectionResult> {
        let db = Paths::data_dir()
            .join(SESSIONS_FOLDER)
            .join(DB_NAME)
            .to_string_lossy()
            .into_owned();
        SessionStoreInspector
            .inspect(
                &[request(
                    "r1",
                    "developer__shell",
                    json!({ "command": format!("cat {db}") }),
                )],
                &[],
                mode,
                session,
            )
            .await
            .unwrap()
    }

    /// Auto mode is the loudest case — the permission inspector approves every
    /// tool there deterministically — but a `SmartApprove` read-only grade on
    /// `shell` and a user `AlwaysAllow` are the same unfiltered read, so the
    /// gate is active in every mode that dispatches tools.
    #[tokio::test]
    async fn every_tool_running_mode_refuses_the_read() {
        for mode in [
            BioRouterMode::Auto,
            BioRouterMode::Approve,
            BioRouterMode::SmartApprove,
        ] {
            let results = inspect_cat(mode, &session_at(SessionClassification::Public)).await;
            let r = results
                .first()
                .unwrap_or_else(|| panic!("{mode:?} must refuse the read"));
            assert_eq!(r.action, InspectionAction::Deny);
            assert_eq!(r.inspector_name, SESSION_STORE_INSPECTOR_NAME);
            assert!(r.finding_id.as_deref().unwrap_or("").starts_with("SDB-"));
            assert!(
                !r.reason.trim().is_empty(),
                "a refusal with no reason reaches the model as \"the user declined\""
            );
        }
    }

    /// The gate is about the channel, not the tier. A private-capability
    /// session reading the file gets every *other* session's transcript with
    /// it, public and private alike — so its clearance buys it nothing here.
    #[tokio::test]
    async fn a_private_capability_caller_is_refused_too() {
        let results = inspect_cat(
            BioRouterMode::Auto,
            &session_at(SessionClassification::Private),
        )
        .await;
        let r = results
            .first()
            .expect("a private-tier session must be refused as well");
        assert_eq!(r.action, InspectionAction::Deny);
        assert!(
            r.reason.contains("private"),
            "the refusal must say the tier is not the question: {}",
            r.reason
        );
    }

    #[tokio::test]
    async fn chat_mode_is_inert() {
        let results = inspect_cat(
            BioRouterMode::Chat,
            &session_at(SessionClassification::Public),
        )
        .await;
        assert!(results.is_empty(), "Chat dispatches no tools at all");
    }

    #[tokio::test]
    async fn ordinary_tools_pass_untouched() {
        let results = SessionStoreInspector
            .inspect(
                &[
                    request(
                        "r1",
                        "developer__shell",
                        json!({ "command": "ls -la /tmp" }),
                    ),
                    request(
                        "r2",
                        "developer__text_editor",
                        json!({ "command": "view", "path": "src/main.rs" }),
                    ),
                ],
                &[],
                BioRouterMode::Auto,
                &session_at(SessionClassification::Public),
            )
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "the gate must be silent on everything but the store: {results:?}"
        );
    }
}
