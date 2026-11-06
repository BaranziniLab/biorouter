//! BR-71 CLI parity (operator requirement, 2026-07-30): subagent runs are
//! discoverable from a terminal, grouped under the session that spawned them.
//!
//! The rule, stated once so the GUI's grouped History (Task 38) can be held to
//! the same one:
//!
//!   1. only a `sub_agent` row ever nests — `parent_session_id` alone is not
//!      enough, because a diverged user chat may carry one too;
//!   2. the tree is exactly one level deep;
//!   3. **no row is ever dropped.** Anything that cannot nest under a top-level
//!      parent — an orphan, a grandchild, a cycle — stays top level.
//!
//! The renderer counterpart is `groupSessionsByParent` in
//! `ui/desktop/src/components/sessions/sessionGrouping.ts`, which follows the
//! same three rules. One difference is deliberate: this listing orders a
//! parent's children oldest first, History newest first.

use biorouter::agents::subagent_roster::{
    SubagentRunStatus, SUBAGENT_OUTCOME_STATE, SUBAGENT_OUTCOME_VERSION,
};
use biorouter::session::session_manager::SessionType;
use biorouter::session::Session;
use chrono::{DateTime, Utc};

/// The fields a listing needs. A projection rather than `biorouter::session::
/// Session` so the pure helpers below can be tested without a store.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub name: String,
    pub session_type: SessionType,
    pub parent_session_id: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub message_count: usize,
    /// The model the session is bound to, when one is recorded.
    pub model: Option<String>,
    /// A subagent's durable outcome record, written when its run ended.
    /// `None` for a run still going, one that predates the record, and every
    /// session that is not a subagent.
    pub outcome: Option<SubagentRunStatus>,
}

impl SessionRow {
    /// The projection of a stored session, ONE spelling for every listing.
    pub fn from_session(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            name: session.name.clone(),
            session_type: session.session_type,
            parent_session_id: session.parent_session_id.clone(),
            updated_at: session.updated_at,
            message_count: session.message_count,
            model: session
                .model_config
                .as_ref()
                .map(|config| config.model_name.clone())
                .filter(|name| !name.is_empty())
                .or_else(|| session.provider_name.clone()),
            outcome: recorded_outcome(session),
        }
    }
}

/// The `status` of a subagent's outcome record (`subagent_outcome.v1`), or
/// `None` when there is no record or it cannot be read. An unreadable record is
/// treated as absent, never as a status it does not state.
pub fn recorded_outcome(session: &Session) -> Option<SubagentRunStatus> {
    if session.session_type != SessionType::SubAgent {
        return None;
    }
    let record = session
        .extension_data
        .get_extension_state(SUBAGENT_OUTCOME_STATE, SUBAGENT_OUTCOME_VERSION)?;
    serde_json::from_value(record.get("status")?.clone()).ok()
}

/// Whether the daemon says this session has a turn in flight.
///
/// Three-valued, for the same reason `SessionLiveness` in
/// `workspace_extension.rs` is: with no daemon reachable the honest answer is
/// "not knowable from here", and collapsing that into `Finished` prints
/// "finished" over a run that is still going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Running,
    Finished,
    Unknown,
}

pub struct Group {
    pub session: SessionRow,
    pub children: Vec<SessionRow>,
}

/// The session types a listing queries, in ONE place.
///
/// `SessionManager::list_sessions()` is exactly
/// `list_sessions_by_types(&[User, Scheduled])`, so a `sub_agent` row is
/// filtered out in SQL and no display change can reveal one — `--subagents` has
/// to widen the query itself. Both CLI callers (the listing and the `--name`
/// lookup) read this function rather than open-coding the array, so the two
/// cannot drift into disagreeing about what is addressable.
pub fn listed_session_types(subagents: bool) -> &'static [SessionType] {
    if subagents {
        &[
            SessionType::User,
            SessionType::Scheduled,
            SessionType::SubAgent,
        ]
    } else {
        &[SessionType::User, SessionType::Scheduled]
    }
}

/// The parent a row would nest under, ignoring for now whether that parent is
/// itself top level. `p != row.id` rejects a row that names itself as its own
/// parent.
///
/// A free function rather than a closure inside `group_by_parent`: the returned
/// `&str` borrows from `row`, and a closure cannot express that higher-ranked
/// relationship (`error: lifetime may not live long enough`).
fn wanted_parent<'a>(
    row: &'a SessionRow,
    ids: &std::collections::HashSet<&str>,
) -> Option<&'a str> {
    match (&row.session_type, &row.parent_session_id) {
        (SessionType::SubAgent, Some(p)) if p != &row.id && ids.contains(p.as_str()) => {
            Some(p.as_str())
        }
        _ => None,
    }
}

/// Nest `sub_agent` rows under the parent they name, when that parent is in the
/// same page. Everything else — including a subagent whose parent was deleted or
/// falls outside the page — stays top level.
///
/// The listing is exactly ONE level deep, so a row may only nest under a parent
/// that is itself top level. That restriction is not cosmetic: a bucket is only
/// ever read back for a row that ended up top level, so filing a row under a key
/// that itself nested writes it somewhere nobody reads and the row disappears
/// from the listing. Depth-2 delegation, a self-parenting row and a parent/child
/// cycle all reach that state. Such a row therefore stays top level — the same
/// treatment an orphan gets, and for the same reason: in a monitoring surface a
/// row rendered in a slightly less useful place beats a row that is not rendered
/// at all. Guarded by `no_row_is_ever_dropped_by_grouping`.
pub fn group_by_parent(rows: Vec<SessionRow>) -> Vec<Group> {
    use std::collections::{HashMap, HashSet};
    let ids: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();

    // Every row that wants to nest. A row in here cannot also be nested UNDER,
    // which is what keeps the tree one level deep and every row reachable.
    let would_nest: HashSet<&str> = rows
        .iter()
        .filter(|row| wanted_parent(row, &ids).is_some())
        .map(|row| row.id.as_str())
        .collect();

    let mut children: HashMap<String, Vec<SessionRow>> = HashMap::new();
    let mut top: Vec<SessionRow> = Vec::new();

    for row in &rows {
        match wanted_parent(row, &ids).filter(|parent| !would_nest.contains(parent)) {
            Some(p) => children.entry(p.to_string()).or_default().push(row.clone()),
            None => top.push(row.clone()),
        }
    }

    top.into_iter()
        .map(|session| {
            let mut kids = children.remove(&session.id).unwrap_or_default();
            kids.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
            Group {
                session,
                children: kids,
            }
        })
        .collect()
}

/// The machine-readable spelling of `Liveness`, for the `--format json` arm —
/// so a script gets the same three-valued answer the text arm prints, including
/// the "unknown" case a boolean could not express.
pub fn liveness_label(liveness: Liveness) -> &'static str {
    match liveness {
        Liveness::Running => "running",
        Liveness::Finished => "finished",
        Liveness::Unknown => "unknown",
    }
}

/// What a child's state reads as in a listing, from the daemon's liveness and
/// the child's own outcome record. Glyphs are the ones `biorouter session
/// subagents` draws ([`super::session_subagents::status_glyph`]).
///
/// A live turn wins over a recorded outcome, because a finished child can be
/// steered into a new turn and its record then describes the previous one. With
/// no turn in flight, or liveness unknowable, a recorded outcome is the answer:
/// it was written when the run ended, so it does not need a daemon to be true.
pub fn child_state(liveness: Liveness, outcome: Option<SubagentRunStatus>) -> String {
    use super::session_subagents::{status_glyph, status_label};
    match (liveness, outcome) {
        (Liveness::Running, _) => format!(
            "{} {}",
            status_glyph(SubagentRunStatus::Running),
            status_label(SubagentRunStatus::Running)
        ),
        (_, Some(status)) => format!("{} {}", status_glyph(status), status_label(status)),
        // Not running, and no record of how it ended: a run from before outcome
        // records, or one whose process went away. "ended" says exactly that
        // much and no more.
        (Liveness::Finished, None) => "· ended".to_string(),
        // ⚠ Not "(no daemon)". A missing daemon is only one of several reasons
        // liveness can be unknown — a stripped secret, a non-200 and an
        // unreadable body reach here too — and naming the wrong one sends the
        // reader after a problem that does not exist. `handle_session_list`
        // prints the actual reason once, on stderr.
        (Liveness::Unknown, None) => "· state unknown".to_string(),
    }
}

/// One indented line for a child, carrying the things that separate two
/// siblings of one fan-out: its own label, its id, how far it got, and the
/// model it ran on.
pub fn render_child(row: &SessionRow, liveness: Liveness) -> String {
    use super::session_subagents::{printable, SUBAGENT_NAME_PREFIX};
    let name = printable(
        row.name
            .strip_prefix(SUBAGENT_NAME_PREFIX)
            .unwrap_or(&row.name),
    );
    let model = row
        .model
        .as_deref()
        .map(|model| format!("  {}", printable(model)))
        .unwrap_or_default();
    format!(
        "  └─ {} [{}]  {}{}  {} msgs  {}",
        name,
        child_state(liveness, row.outcome),
        row.id,
        model,
        row.message_count,
        row.updated_at.format("%Y-%m-%d %H:%M")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, kind: SessionType, parent: Option<&str>, name: &str) -> SessionRow {
        SessionRow {
            id: id.to_string(),
            name: name.to_string(),
            session_type: kind,
            parent_session_id: parent.map(str::to_string),
            updated_at: chrono::Utc::now(),
            message_count: 3,
            model: None,
            outcome: None,
        }
    }

    #[test]
    fn subagents_nest_under_their_parent_and_orphans_stay_top_level() {
        let rows = vec![
            row("p1", SessionType::User, None, "Migration review"),
            row(
                "c1",
                SessionType::SubAgent,
                Some("p1"),
                "Subagent: audit the migration",
            ),
            row(
                "c2",
                SessionType::SubAgent,
                Some("gone"),
                "Subagent: benchmark",
            ),
        ];
        let grouped = group_by_parent(rows);

        let parent = grouped.iter().find(|g| g.session.id == "p1").unwrap();
        assert_eq!(
            parent
                .children
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c1"]
        );
        // A child whose parent is not in this page must remain reachable, not
        // vanish — same rule as Task 38's `groupSessionsByParent`.
        assert!(grouped.iter().any(|g| g.session.id == "c2"));
        // …and it must not ALSO appear as a child of anything.
        assert_eq!(grouped.iter().filter(|g| g.session.id == "c2").count(), 1);
        assert!(!grouped
            .iter()
            .any(|g| g.children.iter().any(|c| c.id == "c2")));
    }

    /// The discriminating half: a parent row that is itself a `sub_agent`
    /// (depth-2 delegation) must not swallow its sibling.
    #[test]
    fn a_row_is_nested_only_when_it_is_itself_a_subagent() {
        let rows = vec![
            row("p1", SessionType::User, None, "root"),
            // A USER row that happens to carry a parent id (a diverged session
            // reusing the column would look like this) is NOT a subagent.
            row("u2", SessionType::User, Some("p1"), "user chat"),
        ];
        let grouped = group_by_parent(rows);
        assert!(
            grouped.iter().any(|g| g.session.id == "u2"),
            "only session_type == sub_agent nests"
        );
    }

    /// Grouping is a MONITORING surface, so its one absolute rule is that no row
    /// may disappear. A single-pass "file under my parent if my parent is on the
    /// page" does lose rows: the bucket is only ever read back for rows that
    /// ended up top level, so anything filed under a key that itself nested is
    /// written and never retrieved.
    ///
    /// Three shapes reach that state, and every one of them is silent:
    ///   * depth-2 delegation `p1 ← c1 ← g1` (`g1` files under `c1`, which is
    ///     itself filed under `p1`),
    ///   * a self-parenting row (`id == parent_session_id`),
    ///   * a two-row cycle `a ↔ b`, where neither row is ever top level.
    ///
    /// Depth-2 is prevented by policy today (`subagent_handler.rs` disables
    /// `subagents_enabled` inside a `SubAgent` session), so this is a guard
    /// against the policy changing — which is exactly when a silent drop would
    /// be hardest to notice.
    #[test]
    fn no_row_is_ever_dropped_by_grouping() {
        let reachable = |grouped: &[Group]| -> std::collections::HashSet<String> {
            grouped
                .iter()
                .flat_map(|g| {
                    std::iter::once(g.session.id.clone())
                        .chain(g.children.iter().map(|c| c.id.clone()))
                })
                .collect()
        };

        // Depth-2 delegation.
        let grouped = group_by_parent(vec![
            row("p1", SessionType::User, None, "root"),
            row("c1", SessionType::SubAgent, Some("p1"), "child"),
            row("g1", SessionType::SubAgent, Some("c1"), "grandchild"),
        ]);
        assert_eq!(
            reachable(&grouped),
            ["p1", "c1", "g1"].map(str::to_string).into_iter().collect(),
            "a grandchild must stay reachable, not vanish into a bucket nobody reads"
        );

        // A row that names itself as its parent.
        let grouped = group_by_parent(vec![row("s1", SessionType::SubAgent, Some("s1"), "self")]);
        assert_eq!(
            reachable(&grouped),
            ["s1"].map(str::to_string).into_iter().collect(),
            "a self-parenting row must not file itself under itself"
        );

        // A two-row cycle.
        let grouped = group_by_parent(vec![
            row("a", SessionType::SubAgent, Some("b"), "a"),
            row("b", SessionType::SubAgent, Some("a"), "b"),
        ]);
        assert_eq!(
            reachable(&grouped),
            ["a", "b"].map(str::to_string).into_iter().collect(),
            "a cycle must degrade to two top-level rows, not to an empty listing"
        );
    }

    #[test]
    fn a_child_line_shows_its_recorded_outcome_and_model() {
        let mut r = row("c1", SessionType::SubAgent, Some("p1"), "Subagent: audit");
        r.model = Some("gpt-5.5".to_string());
        r.outcome = Some(SubagentRunStatus::Failed);
        let finished = render_child(&r, Liveness::Finished);
        assert!(
            finished.starts_with("  └─ audit [✗ failed]  c1  gpt-5.5  3 msgs"),
            "{finished}"
        );
        // A recorded outcome is true without a daemon: it was written when the
        // run ended.
        assert!(render_child(&r, Liveness::Unknown).contains("[✗ failed]"));
        // A live turn wins: the record describes the previous turn.
        assert!(render_child(&r, Liveness::Running).contains("[◐ running]"));
        // No record: "ended" when the daemon says it is not running, and still
        // "state unknown" when nothing could be asked.
        r.outcome = None;
        assert!(render_child(&r, Liveness::Finished).contains("[· ended]"));
        assert!(render_child(&r, Liveness::Unknown).contains("[· state unknown]"));
        r.model = None;
        assert!(render_child(&r, Liveness::Unknown).contains("]  c1  3 msgs"));
    }

    #[test]
    fn every_recorded_outcome_reads_with_its_own_glyph() {
        use SubagentRunStatus as S;
        for (status, expected) in [
            (S::Completed, "● completed"),
            (S::Blocked, "⏸ blocked"),
            (S::Incomplete, "✗ incomplete"),
            (S::Failed, "✗ failed"),
            (S::Interrupted, "⊘ interrupted"),
        ] {
            assert_eq!(child_state(Liveness::Finished, Some(status)), expected);
        }
    }

    #[test]
    fn the_outcome_is_read_from_the_subagent_outcome_record_only() {
        let mut session = Session {
            id: "c1".to_string(),
            name: "Subagent: audit".to_string(),
            session_type: SessionType::SubAgent,
            parent_session_id: Some("p1".to_string()),
            provider_name: Some("versa_azure".to_string()),
            model_config: Some(biorouter::model::ModelConfig::new("gpt-5.5").unwrap()),
            ..Session::default()
        };
        assert_eq!(recorded_outcome(&session), None, "no record, no outcome");
        session.extension_data.set_extension_state(
            SUBAGENT_OUTCOME_STATE,
            SUBAGENT_OUTCOME_VERSION,
            serde_json::json!({
                "status": "blocked",
                "ended_at": "2026-11-04T10:00:00Z",
                "result_line": "Which table?",
            }),
        );
        let row = SessionRow::from_session(&session);
        assert_eq!(row.outcome, Some(SubagentRunStatus::Blocked));
        assert_eq!(row.model.as_deref(), Some("gpt-5.5"));

        // A status this build does not know is no status, never a guess.
        session.extension_data.set_extension_state(
            SUBAGENT_OUTCOME_STATE,
            SUBAGENT_OUTCOME_VERSION,
            serde_json::json!({ "status": "teleported" }),
        );
        assert_eq!(recorded_outcome(&session), None);

        // Only a subagent carries one: a user chat with the same key is not read.
        session.session_type = SessionType::User;
        session.extension_data.set_extension_state(
            SUBAGENT_OUTCOME_STATE,
            SUBAGENT_OUTCOME_VERSION,
            serde_json::json!({ "status": "completed" }),
        );
        assert_eq!(recorded_outcome(&session), None);

        // With no model name, the provider stands in.
        session.model_config = None;
        assert_eq!(
            SessionRow::from_session(&session).model.as_deref(),
            Some("versa_azure")
        );
    }

    #[test]
    fn a_running_row_is_rendered_differently_from_a_finished_one_and_from_an_unknown_one() {
        let mut r = row("c1", SessionType::SubAgent, Some("p1"), "Subagent: audit");
        r.message_count = 12;
        let running = render_child(&r, Liveness::Running);
        let finished = render_child(&r, Liveness::Finished);
        let unknown = render_child(&r, Liveness::Unknown);
        assert_ne!(running, finished);
        assert_ne!(finished, unknown);
        assert_ne!(running, unknown);
        // Identity: the id prefix and the message count are what separate two
        // siblings whose labels were truncated to the same prefix.
        assert!(running.contains("c1"));
        // ⚠ `contains("12")` would be near-vacuous: `updated_at` is rendered as
        // `%Y-%m-%d %H:%M`, which contains "12" for a large slice of the day, so
        // that assertion passes even if the message count is dropped entirely.
        assert!(running.contains("12 msgs"));
    }
}
