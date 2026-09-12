use crate::session::message_to_markdown;
use anyhow::{Context, Result};

use crate::commands::needs_terminal;
use crate::commands::session_grouping::{
    group_by_parent, listed_session_types, liveness_label, render_child, Liveness, SessionRow,
};
use biorouter::privacy::declassify::DeclassifyOutcome;
use biorouter::session::{generate_diagnostics, Session, SessionManager};
use biorouter::utils::safe_truncate;
use cliclack::{confirm, multiselect, select};
use regex::Regex;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const TRUNCATED_DESC_LENGTH: usize = 60;

/// What `session remove` says when it would have to ask before deleting and
/// there is no terminal to ask on (QA-D F5a / F9).
pub(crate) const REMOVE_CONFIRMATION_NEEDS_A_TERMINAL: &str =
    "`biorouter session remove` asks before it deletes anything and needs a terminal to ask on; \
     to remove without asking, re-run it with --yes.";

/// What `session remove` says when it was given nothing to select by, so it
/// would open its picker, and there is no terminal to draw one on.
pub(crate) const REMOVE_PICKER_NEEDS_A_TERMINAL: &str =
    "`biorouter session remove` with no --session-id, --name or --regex opens a picker, which \
     needs a terminal; name what to remove with one of those flags and add --yes.";

/// Which rows a listing — and a removal that selects from one — looks through.
///
/// The default is what `session list` has always shown: `user` and `scheduled`
/// sessions that have recorded at least one message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionScope {
    /// Also subagent runs (`sub_agent` rows). Implies [`Self::include_empty`],
    /// as `--subagents` always has — see [`fetch_sessions`].
    pub subagents: bool,
    /// Also sessions that have not recorded a message yet: what a bare
    /// `biorouter` that exited before its first prompt, `doctor --fix` and
    /// `term init` leave behind (QA-D F5b measured 5,313 of 10,855 rows).
    pub include_empty: bool,
}

impl SessionScope {
    /// Every row any listing can show: what a `--name` addresses.
    const EVERYTHING: Self = Self {
        subagents: true,
        include_empty: true,
    };
}

/// How `session remove` picks its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveSelector {
    /// `--session-id`: exactly that row, whatever its type and whether or not
    /// it has recorded a message.
    Id(String),
    /// `--name`: the one row carrying that name.
    Name(String),
    /// `--regex`: every row in the scope whose id matches.
    Regex(String),
    /// Nothing given: an interactive picker over the scope.
    Pick,
}

impl RemoveSelector {
    /// The selector the flags describe, in the precedence the command has
    /// always applied: an id beats a name beats a regex.
    pub fn from_flags(
        session_id: Option<String>,
        name: Option<String>,
        regex: Option<String>,
    ) -> Self {
        match (session_id, name, regex) {
            (Some(id), _, _) => Self::Id(id),
            (None, Some(name), _) => Self::Name(name),
            (None, None, Some(regex)) => Self::Regex(regex),
            (None, None, None) => Self::Pick,
        }
    }
}

/// The row `id` names, of any type, with or without messages.
///
/// ⚠ **A direct read, not a search of a listing.** This used to look the id up
/// in `list_sessions()`, which is `user`/`scheduled` only and INNER JOINs
/// `messages`, so `session remove --session-id` answered "not found" for a
/// message-less row (QA-D F5b) and for every subagent run (QA-F F9) — rows that
/// `session export` and `session rename` reached by the same id without
/// complaint. An id is unambiguous; there is nothing to filter.
async fn session_by_id(session_manager: &SessionManager, id: &str) -> Result<Session> {
    match session_manager.get_session(id, false).await {
        Ok(session) => Ok(session),
        // The storage layer's own wording for an absent row, matched the way the
        // daemon's `DELETE /sessions/{id}` matches it. Anything else is a real
        // store failure and must not be reported as a typo.
        Err(e) if e.to_string().contains("not found") => {
            Err(anyhow::anyhow!("Session ID '{}' not found.", id))
        }
        Err(e) => Err(e.context(format!("could not read session '{id}'"))),
    }
}

/// The ONE row named `name`, among every row a name addresses elsewhere in
/// the CLI (`resolve_session_by_name`'s set).
///
/// ⚠ **A name shared by several rows is refused, never guessed.** This used to
/// delete the first match. Every session a bare `biorouter` creates reads back
/// as `New chat` (its stored `CLI Session` is canonicalised on read), and
/// subagent names are written by the model, so "the first match" is a deletion
/// of an arbitrary one of them — and with `--yes` nobody would even see which.
async fn session_by_name(session_manager: &SessionManager, name: &str) -> Result<Session> {
    let mut matches: Vec<Session> = fetch_sessions(session_manager, SessionScope::EVERYTHING)
        .await?
        .into_iter()
        .filter(|s| s.name == name)
        .collect();
    match matches.len() {
        0 => Err(anyhow::anyhow!("Session with name '{}' not found.", name)),
        1 => Ok(matches.remove(0)),
        count => {
            let ids: Vec<&str> = matches.iter().map(|s| s.id.as_str()).collect();
            Err(anyhow::anyhow!(
                "{count} sessions are named '{name}' ({}), so the name does not say which to \
                 remove; remove one with --session-id <id>.",
                ids.join(", ")
            ))
        }
    }
}

/// Delete `sessions`, asking first unless `yes`.
///
/// ⚠ **No terminal and no `--yes` is a refusal, checked before anything is
/// printed or deleted.** This called `cliclack::confirm` unconditionally, which
/// under a pipe died with cliclack's bare `Error: not connected` even with `y`
/// piped in (QA-D F5a) — and when a controlling terminal does exist, cliclack
/// reads the keyboard through `/dev/tty` and ignores the pipe, so `echo y |`
/// would sit waiting for a key the user thinks they already sent.
async fn remove_sessions(
    session_manager: &SessionManager,
    sessions: Vec<Session>,
    yes: bool,
    terminal: bool,
) -> Result<()> {
    if !yes {
        needs_terminal::require(terminal, REMOVE_CONFIRMATION_NEEDS_A_TERMINAL)?;

        println!("The following sessions will be removed:");
        for session in &sessions {
            println!("- {} {}", session.id, session.name);
        }

        let should_delete = confirm("Are you sure you want to delete these sessions?")
            .initial_value(false)
            .interact()?;
        if !should_delete {
            println!("Skipping deletion of the sessions.");
            return Ok(());
        }
    }

    for session in sessions {
        session_manager.delete_session(&session.id).await?;
        println!("Session `{}` removed.", session.id);
    }

    Ok(())
}

fn prompt_interactive_session_removal(sessions: &[Session]) -> Result<Vec<Session>> {
    if sessions.is_empty() {
        println!("No sessions to delete.");
        return Ok(vec![]);
    }

    let mut selector = multiselect(
        "Select sessions to delete (use spacebar, Enter to confirm, Ctrl+C to cancel):",
    );

    let display_map: std::collections::HashMap<String, Session> = sessions
        .iter()
        .map(|s| {
            let desc = if s.name.is_empty() {
                "(no name)"
            } else {
                &s.name
            };
            let truncated_desc = safe_truncate(desc, TRUNCATED_DESC_LENGTH);
            let display_text = format!("{} - {} ({})", s.updated_at, truncated_desc, s.id);
            (display_text, s.clone())
        })
        .collect();

    for display_text in display_map.keys() {
        selector = selector.item(display_text.clone(), display_text.clone(), "");
    }

    let selected_display_texts: Vec<String> = selector.interact()?;

    let selected_sessions: Vec<Session> = selected_display_texts
        .into_iter()
        .filter_map(|text| display_map.get(&text).cloned())
        .collect();

    Ok(selected_sessions)
}

pub async fn handle_session_remove(
    selector: RemoveSelector,
    scope: SessionScope,
    yes: bool,
) -> Result<()> {
    remove_in(
        &SessionManager::instance(),
        selector,
        scope,
        yes,
        needs_terminal::prompt_can_run(),
    )
    .await
}

/// [`handle_session_remove`] over an explicit store and terminal answer, so
/// every rule below is testable against a throwaway database.
///
/// Rows are resolved BEFORE the terminal is consulted (the picker aside, which
/// needs the terminal to resolve anything at all): a script with a mistyped id
/// is told "not found", not told to add `--yes` and then told "not found".
async fn remove_in(
    session_manager: &SessionManager,
    selector: RemoveSelector,
    scope: SessionScope,
    yes: bool,
    terminal: bool,
) -> Result<()> {
    let matched_sessions: Vec<Session> = match selector {
        RemoveSelector::Id(id) => vec![session_by_id(session_manager, &id).await?],
        RemoveSelector::Name(name) => vec![session_by_name(session_manager, &name).await?],
        RemoveSelector::Regex(regex_val) => {
            let session_regex = Regex::new(&regex_val)
                .with_context(|| format!("Invalid regex pattern '{}'", regex_val))?;

            let matched: Vec<Session> = fetch_sessions(session_manager, scope)
                .await?
                .into_iter()
                .filter(|session| session_regex.is_match(&session.id))
                .collect();

            if matched.is_empty() {
                println!(
                    "Regex string '{}' does not match any sessions{}",
                    regex_val,
                    widen_hint(scope)
                );
                return Ok(());
            }
            matched
        }
        RemoveSelector::Pick => {
            needs_terminal::require(terminal, REMOVE_PICKER_NEEDS_A_TERMINAL)?;
            let all_sessions = fetch_sessions(session_manager, scope).await?;
            if all_sessions.is_empty() {
                return Err(anyhow::anyhow!("No sessions found."));
            }
            prompt_interactive_session_removal(&all_sessions)?
        }
    };

    if matched_sessions.is_empty() {
        return Ok(());
    }

    remove_sessions(session_manager, matched_sessions, yes, terminal).await
}

/// The flags that would widen a selection that found nothing, or nothing when
/// the scope is already as wide as it goes.
fn widen_hint(scope: SessionScope) -> &'static str {
    match (scope.subagents, scope.include_empty) {
        (true, _) => "",
        (false, true) => " (--subagents also searches subagent runs)",
        (false, false) => {
            " (--include-empty also searches sessions with no messages, --subagents also \
             searches subagent runs)"
        }
    }
}

/// The rows a listing sees, for a given [`SessionScope`].
///
/// BR-71 Task 38b: `list_sessions()` filters `sub_agent` rows out in SQL, so the
/// flag has to widen the *query* — a display-only change would show nothing new.
/// Split out of `handle_session_list` (which is bound to the
/// `SessionManager::instance()` singleton and so untestable) purely so that
/// defect has a regression guard:
/// `the_subagents_flag_widens_the_query_not_just_the_rendering`.
///
/// The default scope is the historical behaviour byte for byte:
/// `list_sessions()` IS `list_sessions_by_types(&[User, Scheduled])`.
async fn fetch_sessions(
    session_manager: &SessionManager,
    scope: SessionScope,
) -> Result<Vec<Session>> {
    let subagents = scope.subagents;
    if subagents || scope.include_empty {
        // ⚠ **A subagent that produced nothing was invisible here.** The
        // historical query INNER JOINs `messages`, so a child spawned and ended
        // before its first message is not returned by SQL at all — and this
        // listing is the ONLY surface in the product that shows subagent runs
        // (History filters to `user`/`scheduled`). So the one place a user could
        // go looking for a child that died early reported that it never
        // existed, which reads as "no subagent was spawned" rather than "the
        // subagent produced nothing".
        //
        // `--include-empty` reaches the same query for the rows a bare
        // `biorouter` leaves behind (QA-D F5b). Widened only on these flags:
        // without either, the behaviour is byte for byte what it was, so the
        // sidebar's deliberate hiding of message-less rows is untouched.
        return session_manager
            .list_sessions_by_types_including_empty(listed_session_types(subagents))
            .await;
    }
    session_manager
        .list_sessions_by_types(listed_session_types(subagents))
        .await
}

/// Resolve a session id from a name (or a literal id), including subagent runs.
///
/// BR-71 Task 38b fact 2: `lookup_session_id`'s name branch used
/// `list_sessions()`, so `--name` could never reach a subagent while
/// `--session-id` always could. Lives here, next to the listing that shows those
/// names, and takes the manager as an argument so it is testable.
pub async fn resolve_session_by_name(
    session_manager: &SessionManager,
    name: &str,
) -> Result<Option<String>> {
    let sessions = fetch_sessions(session_manager, SessionScope::EVERYTHING).await?;
    Ok(sessions
        .into_iter()
        .find(|s| s.name == name || s.id == name)
        .map(|s| s.id))
}

pub async fn handle_session_list(
    format: String,
    ascending: bool,
    working_dir: Option<PathBuf>,
    limit: Option<usize>,
    scope: SessionScope,
) -> Result<()> {
    let subagents = scope.subagents;
    let session_manager = SessionManager::instance();
    let mut sessions = fetch_sessions(&session_manager, scope).await?;

    if let Some(ref pat) = working_dir {
        let pat_lower = pat.to_string_lossy().to_lowercase();
        sessions.retain(|s| {
            s.working_dir
                .to_string_lossy()
                .to_lowercase()
                .contains(&pat_lower)
        });
    }

    if ascending {
        sessions.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
    } else {
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    }

    if !subagents {
        if let Some(n) = limit {
            sessions.truncate(n);
        }

        match format.as_str() {
            "json" => {
                println!("{}", serde_json::to_string(&sessions)?);
            }
            _ => {
                if sessions.is_empty() {
                    println!("No sessions found");
                    return Ok(());
                }

                println!("Available sessions:");
                for session in sessions {
                    let output =
                        format!("{} - {} - {}", session.id, session.name, session.updated_at);
                    println!("{}", output);
                }
            }
        }
        return Ok(());
    }

    print_grouped_sessions(&format, limit, &sessions).await
}

/// The `--subagents` half of [`handle_session_list`]: resolve liveness, nest
/// children under their parent, and print.
///
/// Split out so `handle_session_list` stays under the
/// `clippy::too_many_lines` baseline. The cut is the function's own
/// `if !subagents` fork, so each half is one whole output mode rather than an
/// arbitrary slice — and `sessions` arrives already filtered and sorted,
/// because both modes share that work.
async fn print_grouped_sessions(
    format: &str,
    limit: Option<usize>,
    sessions: &[Session],
) -> Result<()> {
    // The daemon owns liveness. With none reachable, say "unknown" rather than
    // printing "done" over a run that is still going.
    //
    // ⚠ The reason is reported ONCE on stderr rather than guessed at per row.
    // "no daemon" is frequently the wrong diagnosis: an agent-spawned shell has
    // `BIOROUTER_SERVER__SECRET_KEY` stripped by `strip_daemon_private_env`, so
    // this fails with a daemon running perfectly well, and a row that blamed a
    // missing daemon would send someone hunting a problem that does not exist.
    // stderr, so a `--format json` consumer reading stdout is unaffected.
    let live: Option<std::collections::HashSet<String>> =
        match crate::commands::session_watch::running_session_ids().await {
            Ok(ids) => Some(ids),
            Err(err) => {
                eprintln!("note: subagent liveness is unknown: {err}");
                None
            }
        };

    let rows: Vec<SessionRow> = sessions
        .iter()
        .map(|s| SessionRow {
            id: s.id.clone(),
            name: s.name.clone(),
            session_type: s.session_type,
            parent_session_id: s.parent_session_id.clone(),
            updated_at: s.updated_at,
            message_count: s.message_count,
        })
        .collect();
    let mut groups = group_by_parent(rows);
    // ⚠ `limit` caps the TOP-LEVEL rows, after grouping. Truncating the flat row
    // list first would let one parent's six children consume a `--limit 5`.
    if let Some(n) = limit {
        groups.truncate(n);
    }

    let liveness_of = |id: &str| match &live {
        None => Liveness::Unknown,
        Some(ids) if ids.contains(id) => Liveness::Running,
        Some(_) => Liveness::Finished,
    };

    // `SessionRow` is a projection for the pure helpers and deliberately carries
    // no `working_dir`. The JSON arm still has to emit one — the flat
    // (`--subagents`-less) arm serialises whole `Session`s and includes it, and
    // it is the field the sibling `--working-dir` filter matches on, so a script
    // that adds `--subagents` must not silently lose it.
    let working_dirs: std::collections::HashMap<&str, &std::path::Path> = sessions
        .iter()
        .map(|s| (s.id.as_str(), s.working_dir.as_path()))
        .collect();
    let as_json = |row: &SessionRow| {
        serde_json::json!({
            "id": row.id,
            "name": row.name,
            "session_type": row.session_type.to_string(),
            "parent_session_id": row.parent_session_id,
            "working_dir": working_dirs.get(row.id.as_str()),
            "updated_at": row.updated_at,
            "message_count": row.message_count,
            "live": liveness_label(liveness_of(&row.id)),
        })
    };

    match format {
        "json" => {
            let payload: Vec<serde_json::Value> = groups
                .iter()
                .map(|group| {
                    serde_json::json!({
                        "session": as_json(&group.session),
                        "children": group.children.iter().map(&as_json).collect::<Vec<_>>(),
                        // The group-level `live` is the group session's, repeated
                        // here so a consumer can read a group's state without
                        // descending into it. Children carry their own inline.
                        "live": liveness_label(liveness_of(&group.session.id)),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string(&payload)?);
        }
        _ => {
            if groups.is_empty() {
                println!("No sessions found");
                return Ok(());
            }

            println!("Available sessions:");
            for group in groups {
                println!(
                    "{} - {} - {}",
                    group.session.id, group.session.name, group.session.updated_at
                );
                for child in &group.children {
                    println!("{}", render_child(child, liveness_of(&child.id)));
                }
            }
        }
    }
    Ok(())
}

/// Issue #56 — the terminal's half of the export gate.
///
/// ⚠ **The gate runs BEFORE the transcript is read**, and that ordering is the
/// same rule `session_reach` states for its five routes: a refusal that had
/// already loaded the conversation into this process has not protected it, it
/// has merely declined to print it. `export_privacy_gate_precedes_the_read` is
/// what holds the ordering.
///
/// The gate itself lives in the shared library
/// ([`SessionManager::authorize_export`]) so the desktop's export route can call
/// exactly the same code rather than a second implementation of the same policy
/// — this is only the terminal's presentation of it: the capability comes from
/// the model this terminal runs, the system-authentication prompt is the
/// platform's own, and the copy is the shared constant.
///
/// `Ok(false)` means "refused, and the reason has been printed" — the caller
/// returns without writing anything.
async fn authorize_export_at_terminal(
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<bool> {
    use biorouter::privacy::system_auth::AuthOutcome;
    use biorouter::session::session_manager::{
        authenticate_export, ExportDecision, EXPORT_NOT_PROTECTED,
    };

    let capability = crate::session::privacy::caller_capability().await;
    // Owned, and outside the loop: `authenticate_export` takes a slice, and a
    // `&[session_id.to_string()]` built inside the `match` scrutinee is a
    // temporary held across an `.await`.
    let named = [session_id.to_string()];
    let mut authorization = None;
    // At most two passes: the first decides, and if it asks for the operating
    // system the second spends what the prompt granted. A loop rather than
    // nested `if`s so the second pass goes through the SAME decision — the row
    // is re-read, so a chat that changed under the prompt is judged on what it
    // is now.
    for _ in 0..2 {
        match session_manager
            .authorize_export(session_id, capability, authorization.as_ref())
            .await?
        {
            ExportDecision::Unrestricted | ExportDecision::Authorized => return Ok(true),
            ExportDecision::SessionNotFound => {
                return Err(anyhow::anyhow!("Session '{session_id}' not found."))
            }
            ExportDecision::CapabilityRequired => {
                let models = crate::session::privacy::available_private_models().await;
                println!(
                    "{}",
                    crate::session::privacy::export_capability_refusal(session_id, &models)
                );
                return Ok(false);
            }
            ExportDecision::SystemAuthenticationRequired if authorization.is_none() => {
                // The copy comes BEFORE the password dialog: a user must know
                // what they are authorising before they authorise it, and "the
                // file will not be protected" is the entire point of asking.
                println!("{EXPORT_NOT_PROTECTED}");
                match authenticate_export(&named).await {
                    Ok(granted) => authorization = Some(granted),
                    // "This machine cannot raise the prompt" is not the user's
                    // answer, and reporting it as one would tell a Linux user
                    // with no polkit that they declined something they were
                    // never shown. It carries the platform's own advice, so it
                    // surfaces as an error rather than as a refusal.
                    Err(refusal) if refusal.outcome == AuthOutcome::Unavailable => {
                        return Err(anyhow::anyhow!("{refusal}"))
                    }
                    Err(refusal) => {
                        println!("{refusal} Nothing was exported.");
                        return Ok(false);
                    }
                }
            }
            ExportDecision::SystemAuthenticationRequired => {
                println!(
                    "The system authentication did not cover this chat, so nothing was exported."
                );
                return Ok(false);
            }
        }
    }
    Ok(false)
}

pub async fn handle_session_export(
    session_id: String,
    output_path: Option<PathBuf>,
    format: String,
) -> Result<()> {
    let session_manager = SessionManager::instance();
    // ⚠ FIRST. Everything below this line reads the transcript.
    if !authorize_export_at_terminal(&session_manager, &session_id).await? {
        return Ok(());
    }
    let session = match session_manager.get_session(&session_id, true).await {
        Ok(session) => session,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Session '{}' not found or failed to read: {}",
                session_id,
                e
            ));
        }
    };

    let output = match format.as_str() {
        "json" => serde_json::to_string_pretty(&session)?,
        "yaml" => serde_yaml::to_string(&session)?,
        "markdown" => {
            let conversation = session
                .conversation
                .ok_or_else(|| anyhow::anyhow!("Session has no messages"))?;
            export_session_to_markdown(conversation.messages().to_vec(), &session.name)
        }
        _ => return Err(anyhow::anyhow!("Unsupported format: {}", format)),
    };

    if let Some(output_path) = output_path {
        fs::write(&output_path, output).with_context(|| {
            format!("Failed to write to output file: {}", output_path.display())
        })?;
        println!("Session exported to {}", output_path.display());
    } else {
        println!("{}", output);
    }

    Ok(())
}

/// Maximum session name length, mirroring the server route
/// (`routes/session.rs` MAX_NAME_LENGTH) so the CLI and daemon agree.
const MAX_SESSION_NAME_LENGTH: usize = 200;

pub async fn handle_session_rename(session_id: &str, new_name: String) -> Result<()> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("A session name cannot be empty."));
    }
    if trimmed.chars().count() > MAX_SESSION_NAME_LENGTH {
        return Err(anyhow::anyhow!(
            "Session name is too long ({} chars); the maximum is {}.",
            trimmed.chars().count(),
            MAX_SESSION_NAME_LENGTH
        ));
    }

    let session_manager = SessionManager::instance();
    // Confirm the session exists so we report a clear error rather than silently
    // creating/updating a non-existent record.
    session_manager
        .get_session(session_id, false)
        .await
        .map_err(|e| anyhow::anyhow!("Session '{}' not found: {}", session_id, e))?;

    session_manager
        .update(session_id)
        .user_provided_name(trimmed)
        .apply()
        .await
        .with_context(|| format!("Failed to rename session '{}'", session_id))?;

    println!("Renamed session {} → {}", session_id, trimmed);
    Ok(())
}

pub async fn handle_session_diverge(session_id: &str, name: Option<String>) -> Result<()> {
    let session_manager = SessionManager::instance();
    let branched = session_manager
        .diverge_session(session_id, name, None)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to diverge session '{}': {}", session_id, e))?;

    // Machine-readable id on stdout (scriptable), human hint on stderr.
    println!("{}", branched.id);
    eprintln!(
        "Diverged '{}' → new session '{}' (\"{}\"). Resume it with: biorouter session --resume --session-id {}",
        session_id, branched.id, branched.name, branched.id
    );
    Ok(())
}

/// Issue #56 Task 31 / §12.4 — declassify a chat from the terminal, by id.
///
/// **Why the CLI needs its own door.** `list_sessions` filters to (`user`,
/// `scheduled`), so a private `Hidden`, `SubAgent` or `Terminal` chat has no GUI
/// declassification surface at all: History cannot show it, and a control that
/// cannot be selected is not a control. The obvious fix — a "System sessions"
/// filter in History — surfaces 511 hidden sessions on this machine into a
/// user-facing list, which is a regression traded for an edge case. So this
/// works by **id** and consults no session type at all; a Step 5 gate greps this
/// function's body for `SessionType` and expects none.
///
/// ⚠ **This is the second place in the tree that mints
/// `privacy::declassify::UserConfirmation`**, and that is a deliberate widening
/// of a claim `declassify.rs`'s audit used to make with one member. What the
/// audit still guarantees is that the set is *closed and named*: adding a third
/// door turns the build red. What it no longer says is "the only door is an HTTP
/// route behind the user-action header". The honest statement of the residual is
/// that an agent holding `developer__shell` can drive this command — and that
/// same agent can already `sqlite3` the classification column directly, so the
/// store was never protected from the shell in the first place. What the shell
/// cannot do through this door is declassify *silently*: the ledger row
/// [`biorouter::privacy::declassify::declassify`] writes is identical whichever
/// door was used.
pub async fn declassify_command(session_id: &str) -> Result<()> {
    let session_manager = SessionManager::instance();
    let outcome = declassify_by_id(&session_manager, session_id, &mut TerminalPrompt).await?;
    println!("{}", render_declassify_outcome(session_id, outcome));
    Ok(())
}

/// How the terminal asks §12.4's graded confirmation. A trait so the whole of
/// [`declassify_by_id`] — the grading, the escalation, the writing — is testable
/// without a tty, which is the only way the "a refused prompt writes nothing"
/// assertion can exist at all.
pub(crate) trait DeclassifyPrompt {
    /// §12.4's weak control: one yes/no, for a chat that merely ran a turn
    /// against a private endpoint.
    fn confirm_single_click(&mut self, session_id: &str) -> Result<bool>;

    /// §12.4's strong control: retype `phrase`. `None` means the user backed
    /// out.
    ///
    /// `notice` is the already-rendered sentence saying WHY this chat is on the
    /// strong control, printed verbatim. It is passed in rather than composed
    /// here because [`TerminalPrompt`] is the one implementation a test cannot
    /// drive, and the wording is the thing under test: see
    /// [`render_declassify_prompt_notice`] and
    /// [`DECLASSIFY_ESCALATION_NOTICE`], which are pure and are asserted per
    /// provenance.
    fn ask_phrase(
        &mut self,
        session_id: &str,
        phrase: &str,
        notice: &str,
    ) -> Result<Option<String>>;
}

/// Why this chat is being asked for the typed phrase, as one sentence.
///
/// ⚠ **It does not say "reached a private data source" unless the chat did.**
/// That sentence shipped for every provenance, and it is false for the two that
/// dominate day one: the one-time migration marks a chat `backfill:<provider>`
/// from the model it was last bound to, having observed nothing it reached, and
/// an `imported` chat arrived already marked. The per-provenance clause lives in
/// `biorouter::privacy::declassify::strong_confirmation_reason`, beside the
/// grading it must agree with, and is shared with the daemon and the desktop
/// dialog.
pub(crate) fn render_declassify_prompt_notice(
    session_id: &str,
    privacy_reason: Option<&str>,
) -> Option<String> {
    biorouter::privacy::declassify::strong_confirmation_reason(privacy_reason)
        .map(|why| format!("Session {session_id} {why}, so declassifying it needs confirmation."))
}

/// What the terminal says when the grade moved between the read and the write —
/// the escalation arm of [`declassify_by_id`].
///
/// ⚠ **Deliberately not [`render_declassify_prompt_notice`]'s sentence.** The
/// provenance this process read is by definition the stale one, so any clause
/// derived from it is a claim about the conversation that the refusal did not
/// establish. The desktop dialog steps around the same trap in its `escalated`
/// branch; this is the terminal's copy of that reasoning.
pub(crate) const DECLASSIFY_ESCALATION_NOTICE: &str =
    "That request was refused: this chat's record has changed since it was read, so it now takes \
     the typed confirmation.";

/// The real one.
struct TerminalPrompt;

impl DeclassifyPrompt for TerminalPrompt {
    fn confirm_single_click(&mut self, session_id: &str) -> Result<bool> {
        Ok(confirm(format!(
            "Declassify session {session_id}? It will no longer be restricted to private models."
        ))
        .initial_value(false)
        .interact()?)
    }

    fn ask_phrase(
        &mut self,
        _session_id: &str,
        phrase: &str,
        notice: &str,
    ) -> Result<Option<String>> {
        println!("{notice}");
        let typed: String = cliclack::input(format!(
            "Type the last six characters of the session id ({phrase}) to confirm, or leave \
             blank to cancel"
        ))
        .required(false)
        .interact()?;
        Ok(if typed.trim().is_empty() {
            None
        } else {
            Some(typed)
        })
    }
}

/// The testable core of [`declassify_command`].
///
/// The read here decides which control to **show**; the writer decides, inside
/// its own transaction, whether that was the right one — the check-then-act
/// `privacy::declassify` documents. So a weak prompt that comes back
/// `ConfirmationRequired` is not a bug and not a loop: the chat reached a
/// private data source between the two, and the answer is to escalate to the
/// strong control once, exactly as the desktop dialog does.
///
/// ⚠ The proof-of-user is minted in **one** place inside this function, before
/// the loop. Two call sites (one per grade) would read more naturally and would
/// break `the_proof_of_user_is_constructed_in_exactly_two_places`, whose per-file
/// count is what stops a second, unguarded mint from hiding in a file that is
/// already a permitted member of the set.
///
/// ⚠ **DR-20's system authentication is raised here too, and it is the LAST
/// thing before the write** (Task 55). The loop escalates at most twice — once
/// to the typed phrase, once to the operating system — and each escalation is
/// guarded by its own flag, so a user who says no is not asked again. A `turn:*`
/// chat reaches neither escalation and keeps its single click.
pub(crate) async fn declassify_by_id(
    session_manager: &SessionManager,
    session_id: &str,
    prompt: &mut dyn DeclassifyPrompt,
) -> Result<DeclassifyOutcome> {
    use biorouter::privacy::declassify::{
        authenticate_declassification, confirmation_phrase, declassify,
        requires_typed_confirmation, SystemAuthorization, UserConfirmation,
    };
    use biorouter::privacy::system_auth::AuthOutcome;
    use biorouter::privacy::SessionClassification;

    let session = session_manager
        .get_session(session_id, false)
        .await
        .map_err(|e| anyhow::anyhow!("Session '{}' not found: {}", session_id, e))?;

    // Answered before anything is asked: there is nothing to confirm about a
    // no-op, and after a successful declassification the provenance reads
    // `declassified_by_user`, which grades onto the STRONG control — so a second
    // run would otherwise demand a phrase the first run never showed.
    if session.privacy_tier == SessionClassification::Public {
        return Ok(DeclassifyOutcome::AlreadyPublic);
    }

    let phrase = confirmation_phrase(session_id);
    // `Some` exactly when the strong control applies, by construction — the same
    // predicate decides both — so this match cannot show the strong copy to a
    // chat that is getting the single click.
    let notice = render_declassify_prompt_notice(session_id, session.privacy_reason.as_deref());
    let mut typed: Option<String> = if let Some(notice) = notice.as_deref() {
        debug_assert!(requires_typed_confirmation(
            session.privacy_reason.as_deref()
        ));
        match prompt.ask_phrase(session_id, &phrase, notice)? {
            Some(typed) => Some(typed),
            None => return Ok(DeclassifyOutcome::ConfirmationRequired),
        }
    } else if prompt.confirm_single_click(session_id)? {
        None
    } else {
        return Ok(DeclassifyOutcome::ConfirmationRequired);
    };

    let ok = UserConfirmation::from_typed_confirmation();
    let named = [session_id.to_string()];
    let mut escalated = false;
    let mut authorization: Option<SystemAuthorization> = None;
    loop {
        let outcome = declassify(
            session_manager,
            session_id,
            typed.as_deref(),
            authorization.as_ref(),
            &ok,
        )
        .await?;
        match outcome {
            // The grade moved under us. Ask once for the control it moved to; a
            // second refusal is the user's answer, not a reason to ask again.
            DeclassifyOutcome::ConfirmationRequired if !escalated => {
                escalated = true;
                // NOT `notice`: the provenance this function read is the stale
                // one — that is what "the grade moved" means — so a clause
                // derived from it would be a claim about the conversation that
                // nothing here established.
                typed = prompt.ask_phrase(session_id, &phrase, DECLASSIFY_ESCALATION_NOTICE)?;
                if typed.is_none() {
                    return Ok(outcome);
                }
            }
            // DR-20. Everything else has passed — the row is private, the grade
            // demands the strong control, the phrase matched — so this is the
            // moment to ask the operating system, and no earlier.
            DeclassifyOutcome::SystemAuthenticationRequired if authorization.is_none() => {
                match authenticate_declassification(&named).await {
                    Ok(granted) => authorization = Some(granted),
                    // "This machine cannot raise the prompt" is not the user's
                    // answer, and reporting it as one would tell a Linux user
                    // with no polkit that they declined something they were
                    // never shown. It carries the platform's own advice, so it
                    // surfaces as an error rather than as an outcome.
                    Err(refusal) if refusal.outcome == AuthOutcome::Unavailable => {
                        return Err(anyhow::anyhow!("{refusal}"));
                    }
                    // A refusal IS the user's answer, and it reads like every
                    // other refusal at this terminal: nothing changed.
                    Err(_) => return Ok(outcome),
                }
            }
            _ => return Ok(outcome),
        }
    }
}

/// What to print. Separated from the work so the wording is testable and so the
/// three non-writing outcomes cannot be reported as a success.
pub(crate) fn render_declassify_outcome(session_id: &str, outcome: DeclassifyOutcome) -> String {
    match outcome {
        DeclassifyOutcome::Declassified => format!(
            "Session {session_id} is now public. It may run on any model, and the change is \
             recorded in the classification ledger."
        ),
        DeclassifyOutcome::AlreadyPublic => {
            format!("Session {session_id} is already public. Nothing changed.")
        }
        DeclassifyOutcome::ConfirmationRequired => format!(
            "Session {session_id} was NOT declassified: the confirmation was not given. The chat \
             is unchanged."
        ),
        DeclassifyOutcome::SystemAuthenticationRequired => format!(
            "Session {session_id} was NOT declassified: the system authentication was not \
             completed. The chat is unchanged."
        ),
        DeclassifyOutcome::SessionNotFound => {
            format!("Session {session_id} no longer exists. Nothing changed.")
        }
    }
}

pub async fn handle_diagnostics(session_id: &str, output_path: Option<PathBuf>) -> Result<()> {
    println!(
        "Generating diagnostics bundle for session '{}'...",
        session_id
    );

    let session_manager = SessionManager::instance();
    let diagnostics_data = generate_diagnostics(&session_manager, session_id)
        .await
        .with_context(|| {
            format!(
                "Failed to write to generate diagnostics bundle for session '{}'",
                session_id
            )
        })?;

    let output_file = if let Some(path) = output_path {
        path.clone()
    } else {
        PathBuf::from(format!("diagnostics_{}.zip", session_id))
    };

    let mut file = fs::File::create(&output_file).context(format!(
        "Failed to create output file: {}",
        output_file.display()
    ))?;

    file.write_all(&diagnostics_data)
        .context("Failed to write diagnostics data")?;

    println!("Diagnostics bundle saved to: {}", output_file.display());

    Ok(())
}

fn export_session_to_markdown(
    messages: Vec<biorouter::conversation::message::Message>,
    session_name: &String,
) -> String {
    let mut markdown_output = String::new();

    markdown_output.push_str(&format!("# Session Export: {}\n\n", session_name));

    if messages.is_empty() {
        markdown_output.push_str("*(This session has no messages)*\n");
        return markdown_output;
    }

    markdown_output.push_str(&format!("*Total messages: {}*\n\n---\n\n", messages.len()));

    // Track if the last message had tool requests to properly handle tool responses
    let mut skip_next_if_tool_response = false;

    for message in &messages {
        // Check if this is a User message containing only ToolResponses
        let is_only_tool_response = message.role == rmcp::model::Role::User
            && message.content.iter().all(|content| {
                matches!(
                    content,
                    biorouter::conversation::message::MessageContent::ToolResponse(_)
                )
            });

        // If the previous message had tool requests and this one is just tool responses,
        // don't create a new User section - we'll attach the responses to the tool calls
        if skip_next_if_tool_response && is_only_tool_response {
            // Export the tool responses without a User heading
            markdown_output.push_str(&message_to_markdown(message, false));
            markdown_output.push_str("\n\n---\n\n");
            skip_next_if_tool_response = false;
            continue;
        }

        // Reset the skip flag - we'll update it below if needed
        skip_next_if_tool_response = false;

        // Output the role prefix except for tool response-only messages
        if !is_only_tool_response {
            let role_prefix = match message.role {
                rmcp::model::Role::User => "### User:\n",
                rmcp::model::Role::Assistant => "### Assistant:\n",
            };
            markdown_output.push_str(role_prefix);
        }

        // Add the message content
        markdown_output.push_str(&message_to_markdown(message, false));
        markdown_output.push_str("\n\n---\n\n");

        // Check if this message has any tool requests, to handle the next message differently
        if message.content.iter().any(|content| {
            matches!(
                content,
                biorouter::conversation::message::MessageContent::ToolRequest(_)
            )
        }) {
            skip_next_if_tool_response = true;
        }
    }

    markdown_output
}

/// Prompt the user to interactively select a session
///
/// Shows a list of available sessions and lets the user select one
pub async fn prompt_interactive_session_selection(
    session_manager: &SessionManager,
) -> Result<String> {
    let sessions = session_manager.list_sessions().await?;

    if sessions.is_empty() {
        return Err(anyhow::anyhow!("No sessions found"));
    }

    // Build the selection prompt
    let mut selector = select("Select a session to export:");

    // Map to display text
    let display_map: std::collections::HashMap<String, Session> = sessions
        .iter()
        .map(|s| {
            let desc = if s.name.is_empty() {
                "(no name)"
            } else {
                &s.name
            };
            let truncated_desc = safe_truncate(desc, TRUNCATED_DESC_LENGTH);

            let display_text = format!("{} - {} ({})", s.updated_at, truncated_desc, s.id);
            (display_text, s.clone())
        })
        .collect();

    // Add each session as an option
    for display_text in display_map.keys() {
        selector = selector.item(display_text.clone(), display_text.clone(), "");
    }

    // Add a cancel option
    let cancel_value = String::from("cancel");
    selector = selector.item(cancel_value, "Cancel", "Cancel export");

    // Get user selection
    let selected_display_text: String = selector.interact()?;

    if selected_display_text == "cancel" {
        return Err(anyhow::anyhow!("Export canceled"));
    }

    // Retrieve the selected session
    if let Some(session) = display_map.get(&selected_display_text) {
        Ok(session.id.clone())
    } else {
        Err(anyhow::anyhow!("Invalid selection"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::conversation::message::Message;
    use biorouter::session::session_manager::SessionType;
    use tempfile::TempDir;

    /// `session list --subagents`, which has always implied message-less rows.
    const SUBAGENTS: SessionScope = SessionScope {
        subagents: true,
        include_empty: false,
    };

    /// `session list --include-empty`.
    const INCLUDE_EMPTY: SessionScope = SessionScope {
        subagents: false,
        include_empty: true,
    };

    /// One row of each kind `session remove` must reach, in a throwaway store:
    /// a chat with a message, a chat with NONE (what a bare `biorouter` that
    /// exited before its first prompt leaves — QA-D F5b), and a subagent run
    /// (QA-F F9). Returned as `(store, chat, empty, subagent)`.
    async fn store_with_three_row_kinds(dir: &TempDir) -> (SessionManager, String, String, String) {
        let sm = SessionManager::new(dir.path().to_path_buf());
        let chat = sm
            .create_session(
                dir.path().to_path_buf(),
                "Cohort review".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(&chat.id, &Message::user().with_text("hello"))
            .await
            .unwrap();
        let empty = sm
            .create_session(
                dir.path().to_path_buf(),
                "CLI Session".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let subagent = sm
            .create_session(
                dir.path().to_path_buf(),
                "Subagent: audit the cohort".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        sm.add_message(&subagent.id, &Message::user().with_text("audit it"))
            .await
            .unwrap();
        (sm, chat.id, empty.id, subagent.id)
    }

    async fn exists(sm: &SessionManager, id: &str) -> bool {
        sm.get_session(id, false).await.is_ok()
    }

    /// QA-D F5b + QA-F F9: `session remove --session-id` reaches EVERY kind of
    /// row, and `--yes` removes it with no terminal at all.
    ///
    /// The first block is the fixture's own control: the listing the old lookup
    /// searched really does hide two of the three rows, so a pass below is a
    /// statement about the new lookup rather than about a fixture that never
    /// reproduced the defect.
    #[tokio::test]
    async fn remove_by_id_reaches_a_message_less_row_and_a_subagent_run() {
        let dir = TempDir::new().unwrap();
        let (sm, chat, empty, subagent) = store_with_three_row_kinds(&dir).await;

        let old_view: Vec<String> = fetch_sessions(&sm, SessionScope::default())
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            old_view,
            vec![chat.clone()],
            "the listing the old lookup searched must hide the empty and subagent rows, or \
             this test proves nothing"
        );

        for id in [&empty, &subagent, &chat] {
            remove_in(
                &sm,
                RemoveSelector::Id(id.clone()),
                SessionScope::default(),
                true,
                false,
            )
            .await
            .unwrap_or_else(|e| panic!("remove --session-id {id} --yes failed: {e:#}"));
            assert!(!exists(&sm, id).await, "{id} is still in the store");
        }
    }

    /// Without `--yes` and without a terminal, the refusal is the typed one
    /// (exit 2, a sentence naming `--yes`) — and NOTHING is deleted.
    #[tokio::test]
    async fn without_yes_and_without_a_terminal_remove_refuses_and_deletes_nothing() {
        let dir = TempDir::new().unwrap();
        let (sm, chat, empty, subagent) = store_with_three_row_kinds(&dir).await;

        for id in [&chat, &empty, &subagent] {
            let err = remove_in(
                &sm,
                RemoveSelector::Id(id.clone()),
                SessionScope::default(),
                false,
                false,
            )
            .await
            .unwrap_err();
            let refusal = err
                .downcast_ref::<needs_terminal::NeedsTerminal>()
                .unwrap_or_else(|| panic!("not the typed refusal: {err:#}"));
            assert_eq!(refusal.to_string(), REMOVE_CONFIRMATION_NEEDS_A_TERMINAL);
            assert!(REMOVE_CONFIRMATION_NEEDS_A_TERMINAL.contains("--yes"));
            assert!(
                exists(&sm, id).await,
                "{id} was deleted by a refused remove"
            );
        }

        // The picker cannot run either, and says so before listing anything.
        let err = remove_in(
            &sm,
            RemoveSelector::Pick,
            SessionScope::default(),
            true,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.downcast_ref::<needs_terminal::NeedsTerminal>()
                .map(ToString::to_string)
                .as_deref(),
            Some(REMOVE_PICKER_NEEDS_A_TERMINAL)
        );
    }

    /// An id that names no row is "not found" — exit 1, not the terminal
    /// refusal — even with no terminal: rows are resolved before the terminal
    /// is consulted, so a script is told about its typo first.
    #[tokio::test]
    async fn an_unknown_id_is_not_found_before_the_terminal_is_consulted() {
        let dir = TempDir::new().unwrap();
        let (sm, ..) = store_with_three_row_kinds(&dir).await;

        let err = remove_in(
            &sm,
            RemoveSelector::Id("20990101_1".to_string()),
            SessionScope::default(),
            false,
            false,
        )
        .await
        .unwrap_err();
        assert!(err
            .downcast_ref::<needs_terminal::NeedsTerminal>()
            .is_none());
        assert_eq!(err.to_string(), "Session ID '20990101_1' not found.");
    }

    /// `--regex` and the picker select from the scope, and the scope flags
    /// widen it to the rows `session list` hides.
    #[tokio::test]
    async fn regex_removal_selects_from_the_scope_the_flags_describe() {
        for (scope, expect_removed) in [
            (SessionScope::default(), [true, false, false]),
            (INCLUDE_EMPTY, [true, true, false]),
            (SUBAGENTS, [true, true, true]),
        ] {
            let dir = TempDir::new().unwrap();
            let (sm, chat, empty, subagent) = store_with_three_row_kinds(&dir).await;

            remove_in(
                &sm,
                RemoveSelector::Regex(".".to_string()),
                scope,
                true,
                false,
            )
            .await
            .unwrap();

            for (id, removed) in [&chat, &empty, &subagent].into_iter().zip(expect_removed) {
                assert_eq!(
                    !exists(&sm, id).await,
                    removed,
                    "scope {scope:?}: {id} removed={}",
                    !exists(&sm, id).await
                );
            }
        }
    }

    /// `--name` must never guess. Every session a bare `biorouter` creates is
    /// stored as `CLI Session` and reads back as the default name, `New chat`,
    /// so a shared name is the ordinary case — and deleting "the first match",
    /// silently under `--yes`, would delete an arbitrary chat.
    #[tokio::test]
    async fn a_name_shared_by_several_sessions_is_refused_and_a_unique_one_is_removed() {
        use biorouter::session::session_manager::DEFAULT_SESSION_NAME;

        let dir = TempDir::new().unwrap();
        let (sm, chat, empty, subagent) = store_with_three_row_kinds(&dir).await;
        let twin = sm
            .create_session(
                dir.path().to_path_buf(),
                "CLI Session".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        assert_eq!(
            sm.get_session(&twin.id, false).await.unwrap().name,
            DEFAULT_SESSION_NAME,
            "the fixture assumes a CLI-created row reads back under the default name"
        );

        let err = remove_in(
            &sm,
            RemoveSelector::Name(DEFAULT_SESSION_NAME.to_string()),
            SessionScope::default(),
            true,
            false,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            err.contains(&empty) && err.contains(&twin.id),
            "both candidates must be named: {err}"
        );
        assert!(err.contains("--session-id"), "{err}");
        assert!(exists(&sm, &empty).await && exists(&sm, &twin.id).await);

        // A name held by one row is removed — including a subagent's, which the
        // old `list_sessions()` lookup could not see at all.
        remove_in(
            &sm,
            RemoveSelector::Name("Subagent: audit the cohort".to_string()),
            SessionScope::default(),
            true,
            false,
        )
        .await
        .unwrap();
        assert!(!exists(&sm, &subagent).await);
        assert!(exists(&sm, &chat).await, "only the named row goes");
    }

    /// `session list --include-empty` shows the rows a bare `biorouter` leaves
    /// behind, and still hides subagent runs unless `--subagents` is given.
    #[tokio::test]
    async fn the_include_empty_flag_widens_the_listing_to_message_less_rows() {
        let dir = TempDir::new().unwrap();
        let (sm, chat, empty, subagent) = store_with_three_row_kinds(&dir).await;

        let mut listed: Vec<String> = fetch_sessions(&sm, INCLUDE_EMPTY)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        listed.sort();
        let mut expected = vec![chat, empty];
        expected.sort();
        assert_eq!(listed, expected);
        assert!(!listed.contains(&subagent));
    }

    #[test]
    fn remove_flags_map_to_one_selector_in_the_historical_precedence() {
        let s = |v: &str| Some(v.to_string());
        assert_eq!(
            RemoveSelector::from_flags(s("20260910_2"), s("x"), s(".")),
            RemoveSelector::Id("20260910_2".to_string())
        );
        assert_eq!(
            RemoveSelector::from_flags(None, s("x"), s(".")),
            RemoveSelector::Name("x".to_string())
        );
        assert_eq!(
            RemoveSelector::from_flags(None, None, s(".")),
            RemoveSelector::Regex(".".to_string())
        );
        assert_eq!(
            RemoveSelector::from_flags(None, None, None),
            RemoveSelector::Pick
        );
    }

    /// Issue #56 — **the export gate is consulted, and it is consulted first.**
    ///
    /// Two assertions, and the second is the one that matters. A gate that ran
    /// *after* `get_session(&session_id, true)` would have loaded the private
    /// transcript into this process before deciding not to print it, which is
    /// not a refusal — it is a decision not to print something already read.
    /// The same ordering rule `routes/session_reach.rs` states for its five
    /// routes, with the same reasoning.
    ///
    /// A source scan because `handle_session_export` is bound to the
    /// `SessionManager::instance()` singleton — one process-wide store that a
    /// unit test cannot point at a fixture of its own — so it cannot be driven
    /// from a unit test at all. (Until `src/test_sandbox.rs` that singleton
    /// opened the developer's REAL session database, which is why this comment
    /// stood while the sweep corrected 24 stale copies of the claim elsewhere.
    /// The `#[ctor]` now pins it inside a throwaway root: that changes *where*
    /// it writes, not that it is a singleton, so the reason this is a source
    /// scan is unchanged.) What *is* driven for real is the decision itself,
    /// over in `session_manager.rs`'s `export_gate` module; this holds the
    /// wiring and the order, which that module cannot see.
    #[test]
    fn the_export_gate_is_called_before_the_transcript_is_read() {
        let src = include_str!("session.rs");
        let body = src
            .split_once("pub async fn handle_session_export(")
            .expect("handle_session_export is gone")
            .1;
        let body = body.split_once("\n}\n").map_or(body, |(b, _)| b);

        let gate = body
            .find("authorize_export_at_terminal(")
            .expect("`session export` no longer consults the export privacy gate");
        let read = body
            .find("get_session(&session_id, true)")
            .expect("the transcript read is no longer spelled the way this audit looks for");
        assert!(
            gate < read,
            "the export gate runs AFTER the transcript is read; a refusal at that point has \
             already loaded the private conversation into this process"
        );

        // …and the terminal presenter really reaches the SHARED decision, not a
        // second copy of the policy written here. `authorize_export` lives on
        // `SessionManager` so the desktop's export route can call the same code.
        let presenter = src
            .split_once("async fn authorize_export_at_terminal(")
            .expect("the terminal presenter is gone")
            .1;
        let presenter = presenter.split_once("\n}\n").map_or(presenter, |(b, _)| b);
        assert!(
            presenter.contains(".authorize_export("),
            "the terminal decides for itself instead of calling the shared gate"
        );
        assert!(
            presenter.contains("authenticate_export("),
            "the terminal never raises the system-authentication prompt"
        );
        assert!(
            presenter.contains("EXPORT_NOT_PROTECTED"),
            "the terminal never tells the user the file will not be protected"
        );
        assert!(
            presenter.contains("export_capability_refusal("),
            "the terminal does not offer the repair for a public-model export"
        );
    }

    /// The subagent listing shows a child that produced NOTHING.
    ///
    /// The regression is in SQL (`INNER JOIN messages`) and this listing is the
    /// only surface in the product that shows subagent runs at all, so a child
    /// that died before its first message simply did not exist as far as the
    /// user could tell. Driven through `fetch_sessions`, the function the
    /// command really calls, so a fix applied only to the storage layer and
    /// never wired here fails.
    #[tokio::test]
    async fn a_subagent_that_produced_nothing_is_still_listed() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let parent = sm
            .create_session(
                dir.path().to_path_buf(),
                "Migration review".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.add_message(&parent.id, &Message::user().with_text("hello"))
            .await
            .unwrap();
        let silent = sm
            .create_session(
                dir.path().to_path_buf(),
                "Subagent: died before saying anything".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();

        let listed: Vec<String> = fetch_sessions(&sm, SUBAGENTS)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert!(
            listed.contains(&silent.id),
            "an empty subagent is invisible to the one listing that can show it: {listed:?}"
        );
        assert!(listed.contains(&parent.id));

        // …and the default listing is unchanged: it still hides message-less
        // rows, because that is what keeps "Untitled chat" placeholders out of
        // every listing the desktop builds from the same query.
        let default: Vec<String> = fetch_sessions(&sm, SessionScope::default())
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(default, vec![parent.id]);
    }

    /// A `SessionManager` over a throwaway directory, plus one `User` row and
    /// one `SubAgent` row that are identical in every way except their type.
    ///
    /// ⚠ Each row gets a message. `list_sessions_by_types` INNER JOINs
    /// `messages`, so a session with none is invisible whatever its type — a
    /// fixture without this passes the "subagent is hidden" half for entirely
    /// the wrong reason and then fails the other half.
    async fn store_with_a_user_and_a_subagent_session(
        dir: &TempDir,
    ) -> (SessionManager, String, String) {
        let sm = SessionManager::new(dir.path().to_path_buf());
        let parent = sm
            .create_session(
                dir.path().to_path_buf(),
                "Migration review".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let child = sm
            .create_session(
                dir.path().to_path_buf(),
                "Subagent: audit the migration".to_string(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        for id in [&parent.id, &child.id] {
            sm.add_message(id, &Message::user().with_text("hello"))
                .await
                .unwrap();
        }
        (sm, parent.id, child.id)
    }

    /// BR-71 Task 38b, fact 1 — the defect this task exists to fix. A subagent
    /// row is filtered out **in SQL**: `list_sessions()` is exactly
    /// `list_sessions_by_types(&[User, Scheduled])`, so no amount of formatting
    /// makes one appear. `--subagents` therefore has to widen the QUERY.
    ///
    /// This is the assertion that fails if `fetch_sessions` is ever reverted to
    /// `list_sessions()`; the pure grouping tests cannot see that regression at
    /// all, because grouping is only ever handed rows the query already
    /// returned.
    #[tokio::test]
    async fn the_subagents_flag_widens_the_query_not_just_the_rendering() {
        let dir = TempDir::new().unwrap();
        let (sm, parent_id, child_id) = store_with_a_user_and_a_subagent_session(&dir).await;

        let narrow = fetch_sessions(&sm, SessionScope::default()).await.unwrap();
        assert!(
            narrow.iter().any(|s| s.id == parent_id),
            "the default listing still shows user sessions"
        );
        assert!(
            !narrow.iter().any(|s| s.id == child_id),
            "without the flag a subagent row is invisible. This is the defect, \
             and it lives in the SQL type filter"
        );

        let wide = fetch_sessions(&sm, SUBAGENTS).await.unwrap();
        assert!(
            wide.iter().any(|s| s.id == child_id),
            "--subagents must widen the query; a rendering-only change shows nothing new"
        );
        assert!(
            wide.iter().any(|s| s.id == parent_id),
            "widening must ADD a type, not swap one out"
        );
    }

    /// BR-71 Task 38b, fact 2 — the same hole in `--name`. `--session-id`
    /// always worked, which is why this one is easy to miss: by-id resolves,
    /// by-name silently does not. A user cannot attach to a run they cannot
    /// name.
    #[tokio::test]
    async fn a_subagent_run_is_addressable_by_name() {
        let dir = TempDir::new().unwrap();
        let (sm, parent_id, child_id) = store_with_a_user_and_a_subagent_session(&dir).await;

        assert_eq!(
            resolve_session_by_name(&sm, "Subagent: audit the migration")
                .await
                .unwrap(),
            Some(child_id),
            "a subagent must be resolvable by name, not only by id"
        );
        assert_eq!(
            resolve_session_by_name(&sm, "Migration review")
                .await
                .unwrap(),
            Some(parent_id),
            "widening the lookup must not break the existing by-name path"
        );
        assert_eq!(
            resolve_session_by_name(&sm, "no such session")
                .await
                .unwrap(),
            None,
            "an unknown name is still not found"
        );
    }

    /// Arm DR-20's system-authentication seam to approve the next prompt.
    ///
    /// ⚠ **This compiles only because `biorouter` is a `[dev-dependency]` of
    /// this crate with `privacy-test-auth` on.** That is deliberate: if the
    /// feature is ever moved to `[dependencies]` — which would ship the bypass —
    /// nothing here changes, but
    /// `privacy::system_auth::tests::the_test_seam_cannot_be_compiled_into_a_shipped_profile`
    /// turns red. And if the dev-dependency is dropped, this line stops
    /// compiling, which is a loud failure rather than a test suite that starts
    /// asking the developer for their password.
    fn approve_the_next_system_prompt() {
        biorouter::privacy::system_auth_seam::reset();
        biorouter::privacy::system_auth_seam::answer_next_prompt(
            biorouter::privacy::system_auth::AuthOutcome::Approved,
        );
    }

    /// Issue #56 Task 31. A prompt that always gives the strongest answer the
    /// terminal could give: yes to the single click, and the phrase when one is
    /// asked for. It records what it was asked, so a test can tell the two
    /// controls apart.
    #[derive(Default)]
    struct AlwaysConfirms {
        single_clicks: usize,
        phrases_asked: usize,
        /// Every sentence the user was shown before being asked to retype, in
        /// order. Recorded so the WORDING is testable and not just the count —
        /// the shipped string claimed a private data source for every chat.
        notices: Vec<String>,
    }

    impl DeclassifyPrompt for AlwaysConfirms {
        fn confirm_single_click(&mut self, _session_id: &str) -> Result<bool> {
            self.single_clicks += 1;
            Ok(true)
        }

        fn ask_phrase(
            &mut self,
            _session_id: &str,
            phrase: &str,
            notice: &str,
        ) -> Result<Option<String>> {
            self.phrases_asked += 1;
            self.notices.push(notice.to_string());
            Ok(Some(phrase.to_string()))
        }
    }

    /// A prompt nobody answers: the user hit Ctrl-C, or said no.
    struct Refuses;

    impl DeclassifyPrompt for Refuses {
        fn confirm_single_click(&mut self, _session_id: &str) -> Result<bool> {
            Ok(false)
        }

        fn ask_phrase(
            &mut self,
            _session_id: &str,
            _phrase: &str,
            _notice: &str,
        ) -> Result<Option<String>> {
            Ok(None)
        }
    }

    /// A private session of `kind`, carrying `reason` as its provenance.
    async fn private_session_of_type(
        sm: &SessionManager,
        dir: &TempDir,
        kind: SessionType,
        reason: &str,
    ) -> String {
        let s = sm
            .create_session(dir.path().to_path_buf(), "a cohort chat".to_string(), kind)
            .await
            .unwrap();
        sm.add_message(&s.id, &Message::user().with_text("patient MRN 12345"))
            .await
            .unwrap();
        sm.update(&s.id)
            .raise_privacy(biorouter::privacy::SessionClassification::Private, reason)
            .apply()
            .await
            .unwrap();
        s.id
    }

    /// Issue #56 Task 31. `list_sessions` filters to (`user`, `scheduled`), so a
    /// private `Hidden`, `SubAgent` or `Terminal` chat has NO GUI
    /// declassification surface at all — the History list it would have to be
    /// selected from cannot show it.
    ///
    /// The obvious fix is a "System sessions" filter in History, and it is the
    /// wrong one: on this machine that surfaces 511 hidden sessions into a
    /// user-facing list, a regression traded for an edge case. The CLI escape
    /// hatch works by **id**, which is exactly why it does not need one.
    ///
    /// ⚠ `#[serial]` on the seam's own key, because it arms DR-20's
    /// **process-global** one-shot (`system_auth_seam::{NEXT, LAST}`) and so
    /// does its sibling below. Measured, not predicted: run just these two on
    /// two threads and 34 of 40 runs fail — this one seeing its arming consumed
    /// by the sibling's `reset()`, and the sibling seeing THIS one's `Approved`
    /// satisfy a prompt it had armed to `Denied`. The second direction is the
    /// one that matters: it makes a discriminating assertion pass for a reason
    /// that has nothing to do with the code under test.
    #[tokio::test]
    #[serial_test::serial(privacy_test_auth_seam)]
    async fn declassify_works_by_id_regardless_of_session_type() {
        use biorouter::privacy::declassify::DeclassifyOutcome;
        use biorouter::privacy::SessionClassification;

        for kind in [
            SessionType::Hidden,
            SessionType::SubAgent,
            SessionType::Terminal,
            SessionType::User,
        ] {
            let dir = TempDir::new().unwrap();
            let sm = SessionManager::new(dir.path().to_path_buf());
            let id = private_session_of_type(&sm, &dir, kind, "mcp:ucsfomopagent").await;

            // `mcp:*` grades onto the strong control, which since Task 55 also
            // means DR-20's system authentication. One arming per chat, because
            // the seam is one-shot for the same reason DR-20 admits no cached
            // grant.
            approve_the_next_system_prompt();
            let mut prompt = AlwaysConfirms::default();
            assert_eq!(
                declassify_by_id(&sm, &id, &mut prompt).await.unwrap(),
                DeclassifyOutcome::Declassified,
                "a private {kind:?} chat must be declassifiable by id"
            );
            assert_eq!(
                sm.get_session(&id, false).await.unwrap().privacy_tier,
                SessionClassification::Public
            );
            // `mcp:*` provenance grades onto §12.4's STRONG control, whatever
            // the session's type is.
            assert_eq!(prompt.phrases_asked, 1, "{kind:?}");
            assert_eq!(prompt.single_clicks, 0, "{kind:?}");
        }
    }

    /// …and the reason the escape hatch has to exist: three of those four types
    /// are invisible to every listing the GUI builds its History from.
    #[tokio::test]
    async fn three_of_those_four_types_have_no_listing_to_be_selected_from() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let mut hidden = vec![];
        for kind in [
            SessionType::Hidden,
            SessionType::SubAgent,
            SessionType::Terminal,
        ] {
            hidden.push(private_session_of_type(&sm, &dir, kind, "mcp:x").await);
        }
        let visible = private_session_of_type(&sm, &dir, SessionType::User, "mcp:x").await;

        let listed = sm.list_sessions().await.unwrap();
        assert!(listed.iter().any(|s| s.id == visible));
        for id in &hidden {
            assert!(
                !listed.iter().any(|s| &s.id == id),
                "{id} is listed after all. This test's premise is gone"
            );
        }
    }

    /// §12.4's grading, at the terminal. A chat that merely ran a turn gets the
    /// single click; everything else gets the typed phrase.
    #[tokio::test]
    async fn the_terminal_shows_the_control_the_provenance_grades_onto() {
        use biorouter::privacy::declassify::DeclassifyOutcome;

        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let weak = private_session_of_type(&sm, &dir, SessionType::User, "turn:versa_azure").await;
        let mut prompt = AlwaysConfirms::default();
        assert_eq!(
            declassify_by_id(&sm, &weak, &mut prompt).await.unwrap(),
            DeclassifyOutcome::Declassified
        );
        assert_eq!(prompt.single_clicks, 1);
        assert_eq!(prompt.phrases_asked, 0);

        // A second call on the now-public row is a no-op and asks nothing.
        let mut again = AlwaysConfirms::default();
        assert_eq!(
            declassify_by_id(&sm, &weak, &mut again).await.unwrap(),
            DeclassifyOutcome::AlreadyPublic
        );
        assert_eq!(again.single_clicks, 0);
        assert_eq!(again.phrases_asked, 0);
    }

    /// The sentence the terminal prints above the phrase field, **asserted per
    /// provenance**, because it shipped as one sentence — "reached a private
    /// data source" — for all of them.
    ///
    /// That is false for `backfill:*` and `imported`, and those are not an edge
    /// case: the one-time migration marks a chat by the model it was last bound
    /// to, so on a machine with history `backfill:*` is most of the private rows
    /// a user meets this control on. A single assertion on one provenance is
    /// exactly what let it ship, so this walks the vocabulary.
    #[test]
    fn each_provenance_is_given_the_reason_that_is_true_of_it() {
        let id = "20260101_120000";
        let cases: [(Option<&str>, &str); 8] = [
            (
                Some("mcp:ucsfomopagent"),
                "Session 20260101_120000 reached a private data source, so declassifying it needs \
                 confirmation.",
            ),
            (
                Some("inherited:20251231_090000"),
                "Session 20260101_120000 was created inside a private chat, so declassifying it \
                 needs confirmation.",
            ),
            (
                Some("diverged:20251231_090000"),
                "Session 20260101_120000 was branched out of a private chat, so declassifying it \
                 needs confirmation.",
            ),
            (
                Some("backfill:versa_azure"),
                "Session 20260101_120000 was marked private by the one-time migration, from the \
                 model it was last using rather than from anything it reached, so declassifying \
                 it needs confirmation.",
            ),
            (
                Some("imported"),
                "Session 20260101_120000 was imported already marked private, so declassifying it \
                 needs confirmation.",
            ),
            (
                Some("something_new"),
                "Session 20260101_120000 does not record an observed turn on a private model as \
                 the reason it is private, so declassifying it needs confirmation.",
            ),
            (
                Some(""),
                "Session 20260101_120000 does not record an observed turn on a private model as \
                 the reason it is private, so declassifying it needs confirmation.",
            ),
            (
                None,
                "Session 20260101_120000 does not record an observed turn on a private model as \
                 the reason it is private, so declassifying it needs confirmation.",
            ),
        ];
        for (reason, expected) in cases {
            assert_eq!(
                render_declassify_prompt_notice(id, reason).as_deref(),
                Some(expected),
                "the sentence a {reason:?} chat is shown"
            );
        }

        // A `turn:*` chat never sees this control at all, so it has no sentence
        // to be given a wrong one.
        assert_eq!(
            render_declassify_prompt_notice(id, Some("turn:versa_azure")),
            None
        );

        // And the escalation arm does not borrow any of them: the provenance it
        // would derive from is the stale one by definition.
        assert!(!DECLASSIFY_ESCALATION_NOTICE.contains("reached a private data source"));
        assert!(DECLASSIFY_ESCALATION_NOTICE.contains("has changed"));
    }

    /// …and the sentence above is the one `declassify_by_id` actually hands the
    /// prompt, for each provenance, through the real read of the stored row.
    ///
    /// The pure test cannot catch a call site that passes the wrong string (the
    /// escalation notice, a hardcoded sentence, the id twice); this walks the
    /// same vocabulary through the writer.
    ///
    /// ⚠ `#[serial]` on the seam's own key: the strong control now owes DR-20's
    /// system authentication, whose arming is process-global.
    #[tokio::test]
    #[serial_test::serial(privacy_test_auth_seam)]
    async fn the_reason_reaches_the_prompt_through_the_real_read() {
        use biorouter::privacy::declassify::DeclassifyOutcome;

        for (reason, must_say) in [
            ("mcp:ucsfomopagent", "reached a private data source"),
            (
                "inherited:20251231_090000",
                "was created inside a private chat",
            ),
            (
                "diverged:20251231_090000",
                "was branched out of a private chat",
            ),
            (
                "backfill:versa_azure",
                "was marked private by the one-time migration",
            ),
            ("imported", "was imported already marked private"),
            (
                "something_new",
                "does not record an observed turn on a private model",
            ),
        ] {
            let dir = TempDir::new().unwrap();
            let sm = SessionManager::new(dir.path().to_path_buf());
            let id = private_session_of_type(&sm, &dir, SessionType::User, reason).await;

            approve_the_next_system_prompt();
            let mut prompt = AlwaysConfirms::default();
            assert_eq!(
                declassify_by_id(&sm, &id, &mut prompt).await.unwrap(),
                DeclassifyOutcome::Declassified,
                "{reason}"
            );
            assert_eq!(prompt.notices.len(), 1, "{reason}");
            let said = &prompt.notices[0];
            assert!(
                said.contains(must_say),
                "a {reason} chat was told {said:?}, which does not say {must_say:?}"
            );
            assert!(
                said.contains(&id),
                "{reason}: {said:?} does not name the chat"
            );
            if reason != "mcp:ucsfomopagent" {
                assert!(
                    !said.contains("reached a private data source"),
                    "a {reason} chat was told it reached a private data source: {said:?}"
                );
            }
        }
    }

    /// A refusal at the prompt writes nothing. Both controls, because a "no"
    /// that declassified anyway is the one failure this whole surface exists to
    /// prevent.
    #[tokio::test]
    async fn a_prompt_nobody_answers_leaves_the_chat_private() {
        use biorouter::privacy::declassify::DeclassifyOutcome;
        use biorouter::privacy::SessionClassification;

        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        for reason in ["turn:versa_azure", "mcp:ucsfomopagent"] {
            let id = private_session_of_type(&sm, &dir, SessionType::User, reason).await;
            assert_eq!(
                declassify_by_id(&sm, &id, &mut Refuses).await.unwrap(),
                DeclassifyOutcome::ConfirmationRequired
            );
            assert_eq!(
                sm.get_session(&id, false).await.unwrap().privacy_tier,
                SessionClassification::Private,
                "a refused confirmation must leave the chat exactly as it was"
            );
        }
    }

    /// Issue #56 DR-20 / Task 55. The terminal door asks the operating system
    /// too, and a refusal there leaves the chat exactly as it was.
    ///
    /// ⚠ **The `turn:*` half is the discriminating one.** The seam is armed only
    /// for the `mcp:*` chat; an unarmed seam refuses by default, so if the weak
    /// control had gained a password prompt this test would fail on the
    /// `turn:*` chat rather than pass quietly.
    ///
    /// ⚠ That last sentence is only true with `BIOROUTER_PRIVACY_TEST_AUTH`
    /// **unset**, which is why the guard below is not decoration. `env_answer`
    /// is consulted whenever the one-shot arming is absent, so a developer or a
    /// CI runner with `BIOROUTER_PRIVACY_TEST_AUTH=approve` exported turns the
    /// unarmed seam into an approving one. Measured: with
    /// `requires_system_authentication` mutated to `true` — exactly the
    /// regression this test names — the assertion below fails with the variable
    /// unset and **passes** with it set to `approve`. The lock closes that.
    ///
    /// ⚠ `#[serial]` on the seam's own key — see
    /// `declassify_works_by_id_regardless_of_session_type` for the measurement.
    /// Without it, that test's `Approved` arming answers the `Denied` prompt
    /// this one raises, and the first assertion below passes reading
    /// `Declassified` — the discriminating test losing its power to a race.
    #[tokio::test]
    #[serial_test::serial(privacy_test_auth_seam)]
    async fn the_terminal_asks_the_operating_system_for_the_strong_control_only() {
        use biorouter::privacy::declassify::DeclassifyOutcome;
        use biorouter::privacy::system_auth::AuthOutcome;
        use biorouter::privacy::SessionClassification;

        let _env = env_lock::lock_env([("BIOROUTER_PRIVACY_TEST_AUTH", None::<&str>)]);

        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());

        // A denied prompt: the phrase was typed and matched, and the chat is
        // still private with nothing written.
        let strong = private_session_of_type(&sm, &dir, SessionType::User, "mcp:x").await;
        biorouter::privacy::system_auth_seam::reset();
        biorouter::privacy::system_auth_seam::answer_next_prompt(AuthOutcome::Denied);
        let mut prompt = AlwaysConfirms::default();
        assert_eq!(
            declassify_by_id(&sm, &strong, &mut prompt).await.unwrap(),
            DeclassifyOutcome::SystemAuthenticationRequired
        );
        assert_eq!(
            prompt.phrases_asked, 1,
            "the typed phrase must still be asked"
        );
        assert_eq!(
            sm.get_session(&strong, false).await.unwrap().privacy_tier,
            SessionClassification::Private,
            "a refused system authentication declassified the chat anyway"
        );

        // A platform with no prompter is an ERROR carrying the platform's own
        // advice, not the user's answer — telling a Linux user with no polkit
        // that they declined something they were never shown would be a lie.
        biorouter::privacy::system_auth_seam::reset();
        biorouter::privacy::system_auth_seam::answer_next_prompt(AuthOutcome::Unavailable);
        let err = declassify_by_id(&sm, &strong, &mut AlwaysConfirms::default())
            .await
            .expect_err("an unavailable prompter must not read as a refusal by the user");
        assert!(!err.to_string().is_empty(), "{err}");
        assert_eq!(
            sm.get_session(&strong, false).await.unwrap().privacy_tier,
            SessionClassification::Private
        );

        // Approved: both proofs given, and only then.
        approve_the_next_system_prompt();
        assert_eq!(
            declassify_by_id(&sm, &strong, &mut AlwaysConfirms::default())
                .await
                .unwrap(),
            DeclassifyOutcome::Declassified
        );

        // …and the weak control raises no prompt at all. The seam is left
        // UNARMED here on purpose: it defaults to refusing, so a `turn:*` chat
        // that asked for a password would come back
        // `SystemAuthenticationRequired` instead of `Declassified`.
        biorouter::privacy::system_auth_seam::reset();
        let weak = private_session_of_type(&sm, &dir, SessionType::User, "turn:versa_azure").await;
        assert_eq!(
            declassify_by_id(&sm, &weak, &mut AlwaysConfirms::default())
                .await
                .unwrap(),
            DeclassifyOutcome::Declassified,
            "the single-click control gained a password prompt it never shows the user"
        );
    }

    /// The four non-writing outcomes must not read as success. A user who is
    /// told "declassified" and finds the chat still refusing has been lied to by
    /// the one surface whose whole job is to be believed.
    #[test]
    fn only_the_writing_outcome_reports_a_declassification() {
        assert!(
            render_declassify_outcome("20260801_7", DeclassifyOutcome::Declassified)
                .contains("now public")
        );
        for outcome in [
            DeclassifyOutcome::AlreadyPublic,
            DeclassifyOutcome::ConfirmationRequired,
            DeclassifyOutcome::SystemAuthenticationRequired,
            DeclassifyOutcome::SessionNotFound,
        ] {
            let text = render_declassify_outcome("20260801_7", outcome);
            assert!(
                text.contains("Nothing changed") || text.contains("unchanged"),
                "{outcome:?} reported as a change: {text}"
            );
            assert!(!text.contains("is now public"), "{outcome:?}: {text}");
        }
    }

    /// An id that names no row is reported, not silently reported as success.
    #[tokio::test]
    async fn an_unknown_id_is_an_error_and_not_a_declassification() {
        let dir = TempDir::new().unwrap();
        let sm = SessionManager::new(dir.path().to_path_buf());
        let err = declassify_by_id(&sm, "29990101_000000", &mut AlwaysConfirms::default())
            .await
            .expect_err("an unknown id must not read as a successful declassification");
        assert!(err.to_string().contains("29990101_000000"), "{err}");
    }

    /// The rule lives in exactly one place, so the listing and the `--name`
    /// lookup cannot drift apart.
    #[test]
    fn the_widened_type_list_adds_subagents_and_removes_nothing() {
        let narrow = listed_session_types(false);
        let wide = listed_session_types(true);
        assert!(!narrow.contains(&SessionType::SubAgent));
        assert!(wide.contains(&SessionType::SubAgent));
        for kind in narrow {
            assert!(wide.contains(kind), "{kind:?} must survive the widening");
        }
    }
}
