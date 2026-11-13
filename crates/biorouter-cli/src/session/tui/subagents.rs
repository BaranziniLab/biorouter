//! The subagent pane: this chat's subagents beside (or above) the history, and
//! a read-only view of one subagent's chat.
//!
//! The pane draws [`PaneModel`], a plain view model built from the shared
//! roster (`biorouter::agents::subagent_roster::session_subagents`). That is
//! the same answer the desktop side panel and `biorouter session subagents`
//! read, so the terminal cannot disagree with them about a child's status,
//! model or setup. The model is plain data on purpose: the TestBackend tests
//! build rows by hand and never need a store, an agent or the roster.
//!
//! Layout (decided in `tui::draw`):
//! - inner width >= [`WIDE_MIN_WIDTH`]: a right-hand pane beside the history,
//!   a third of the width clamped to [`PANE_MIN_WIDTH`]..=[`PANE_MAX_WIDTH`];
//! - narrower: a strip above the status lines, 1 to [`STRIP_MAX_ROWS`] rows,
//!   with a "+N more" line when the rows do not fit;
//! - hidden while the chat has no subagents, unless the person toggled it on.
//!
//! Keys: `Ctrl-B` shows or hides the pane; `Ctrl-O` moves focus into it (and
//! back). With focus in the pane, ↑/↓ select, Enter opens the selected
//! subagent's chat, Esc returns to the input, and typing a character returns
//! to the input and types it, so the pane never swallows a message. In a
//! subagent's chat, Esc returns to the parent; input is read-only.

use std::time::Duration;

use biorouter::agents::orchestration_plan::{PhaseCounts, PhaseKind, PhaseStatus};
use biorouter::agents::subagent_roster::{SessionSubagents, SubagentRow, SubagentRunStatus};
use biorouter::agents::AgentEvent;
use biorouter::conversation::message::{Message, MessageContent, ProvenanceKind};
use biorouter::privacy::SessionClassification;
use biorouter::session_events::{SessionBusEvent, Subscription};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::Frame;
use rmcp::model::Role;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, StatusInfo, ACCENT};
use crate::commands::session_subagents::{setup_counts, short_model_label};

/// How often the idle loop re-reads the roster while a child is active, and
/// the streaming loop's throttle for the same read.
pub const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
/// While the pane is shown but every child has finished, nothing in this
/// process can change a row until the parent's next turn (which re-reads on
/// its own tick), so the idle loop re-reads only every this many ticks.
pub const IDLE_POLL_EVERY: u32 = 4;
/// Inner width at which the pane moves from a strip to a side pane.
pub const WIDE_MIN_WIDTH: u16 = 120;
pub const PANE_MIN_WIDTH: u16 = 34;
pub const PANE_MAX_WIDTH: u16 = 50;
/// Tallest the narrow strip may grow.
pub const STRIP_MAX_ROWS: u16 = 6;
/// Up to this many subagents, each row takes two lines: the label (the role,
/// else the name), then the setup, which is left out when it says nothing.
pub const TWO_LINE_MAX_ROWS: usize = 3;
/// An active child with no recorded activity for this long reads as "quiet".
/// A heuristic, so it is labelled "quiet", never "stalled". The plan's own
/// threshold, so a row and its phase's "N quiet" count can never disagree.
pub const QUIET_AFTER_SECS: u64 = biorouter::agents::orchestration_plan::QUIET_AFTER_SECONDS;

const DIM: Style = Style::new().add_modifier(Modifier::DIM);
const LABEL_W: usize = 11;
const LOCK: &str = "🔒";
/// A row that leads with its role shows the session name after it only when at
/// least this many cells of the name fit; a stub of two letters says nothing.
const SECONDARY_NAME_MIN: usize = 8;
/// Widest the session name is printed under its role in `/subagents list`.
const LIST_NAME_MAX: usize = 72;
/// A spawn-context record's task instructions show this many lines, folded
/// after that like the rest of the record.
const SPAWN_CONTEXT_TASK_MAX_LINES: usize = 12;
/// A record with no task section to anchor on shows its first lines.
const SPAWN_CONTEXT_HEAD_LINES: usize = 6;

// ── view model ──────────────────────────────────────────────────────────────

/// One subagent as the pane draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneRow {
    pub session_id: String,
    pub name: String,
    pub role: Option<String>,
    pub phase_id: Option<String>,
    pub status: SubagentRunStatus,
    pub loop_phase: Option<String>,
    pub quiet_seconds: Option<u64>,
    pub elapsed_seconds: Option<u64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub extensions: Vec<String>,
    pub skills: Vec<String>,
    pub knowledge_bases: Vec<String>,
    pub private: bool,
    pub total_tokens: Option<i64>,
    pub message_count: i64,
    pub tool_call_count: i64,
    pub result_line: Option<String>,
}

impl PaneRow {
    /// A row with only the identity and the status set (tests fill the rest).
    #[cfg(test)]
    pub fn new(
        session_id: impl Into<String>,
        name: impl Into<String>,
        status: SubagentRunStatus,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            name: name.into(),
            role: None,
            phase_id: None,
            status,
            loop_phase: None,
            quiet_seconds: None,
            elapsed_seconds: None,
            provider: None,
            model: None,
            extensions: Vec::new(),
            skills: Vec::new(),
            knowledge_bases: Vec::new(),
            private: false,
            total_tokens: None,
            message_count: 0,
            tool_call_count: 0,
            result_line: None,
        }
    }

    pub fn from_row(row: &SubagentRow) -> Self {
        Self {
            session_id: row.session_id.clone(),
            name: row.name.clone(),
            role: row.role.clone().filter(|r| !r.trim().is_empty()),
            phase_id: row.phase_id.clone(),
            status: row.status,
            loop_phase: row.loop_phase.clone(),
            quiet_seconds: row.quiet_seconds,
            elapsed_seconds: row.elapsed_seconds,
            provider: row.provider_name.clone(),
            model: row.model_name.clone(),
            extensions: row.extensions.clone(),
            skills: row.skills.clone(),
            knowledge_bases: row.knowledge_bases.clone(),
            private: row.privacy_tier == SessionClassification::Private,
            total_tokens: row.total_tokens,
            message_count: row.message_count,
            tool_call_count: row.tool_call_count,
            result_line: row.result_line.clone(),
        }
    }

    pub fn is_quiet(&self) -> bool {
        self.status.is_active() && self.quiet_seconds.is_some_and(|s| s >= QUIET_AFTER_SECS)
    }

    fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.session_id
        } else {
            &self.name
        }
    }

    /// What the row leads with: the role the parent gave the child
    /// (`celsius`), else its session name. The name is derived from the task's
    /// instructions, so it reads like a sentence ("Convert 37 degrees Celsius
    /// to…") and siblings of one fan-out start alike; the role is what tells
    /// them apart. The desktop's `subagentTitle`.
    pub fn label(&self) -> &str {
        self.role.as_deref().unwrap_or_else(|| self.display_name())
    }

    /// The session name, shown second and dimmed, when the row leads with a
    /// role that says something else.
    pub fn secondary_name(&self) -> Option<&str> {
        let role = self.role.as_deref()?;
        let name = self.name.trim();
        (!name.is_empty() && !name.eq_ignore_ascii_case(role)).then_some(name)
    }
}

/// One phase of the chat's orchestration plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PanePhase {
    pub id: String,
    pub title: String,
    pub kind: PhaseKind,
    pub status: PhaseStatus,
    pub counts: PhaseCounts,
    pub agent_ids: Vec<String>,
    pub handoff: Option<String>,
    /// The title (else the id) of the earlier gated phase this open phase
    /// waits for, when one holds it.
    pub held_by: Option<String>,
    /// What the phase's gate has come to, when it has one.
    pub gate: Option<PaneGate>,
}

/// A phase gate as the pane marks it on its phase's heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneGate {
    /// Not asked yet, or asked and the phase is still open to try again.
    Waiting,
    /// Its last evaluation passed.
    Passed,
    /// Its last evaluation did not pass; the agent may try again.
    NotPassed,
    /// Every attempt is spent, or it could not be asked: the person decides.
    Blocked,
    /// The person passed or skipped it.
    ByYou,
}

impl PaneGate {
    pub fn of(gate: &biorouter::agents::orchestration_plan::PhaseGateSnapshot) -> Self {
        if gate.person_decision.is_some() {
            return Self::ByYou;
        }
        if gate.blocked || gate.needs_person {
            return Self::Blocked;
        }
        match gate.last_passed {
            Some(true) => Self::Passed,
            Some(false) => Self::NotPassed,
            None => Self::Waiting,
        }
    }

    /// The words on the phase's heading: a gate passes or does not, never
    /// "fails".
    fn words(self) -> &'static str {
        match self {
            Self::Waiting => "gated",
            Self::Passed => "gate passed",
            Self::NotPassed => "gate not passed yet",
            Self::Blocked => "gate blocked · needs you",
            Self::ByYou => "gate decided by you",
        }
    }
}

impl PanePhase {
    #[cfg(test)]
    pub fn new(id: impl Into<String>, kind: PhaseKind, status: PhaseStatus) -> Self {
        let id = id.into();
        Self {
            title: id.clone(),
            id,
            kind,
            status,
            counts: PhaseCounts::default(),
            agent_ids: Vec::new(),
            handoff: None,
            held_by: None,
            gate: None,
        }
    }

    fn label(&self) -> String {
        if self.title.trim().is_empty() {
            self.id.clone()
        } else {
            self.title.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanePlan {
    pub title: String,
    pub phases: Vec<PanePhase>,
}

/// Everything the pane draws. Rows are oldest first (spawn order), which is
/// also the numbering `/subagents list` and `/attach <n>` use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaneModel {
    pub rows: Vec<PaneRow>,
    pub plan: Option<PanePlan>,
}

impl PaneModel {
    pub fn from_snapshot(snapshot: &SessionSubagents) -> Self {
        Self {
            rows: snapshot.subagents.iter().map(PaneRow::from_row).collect(),
            plan: snapshot.plan.as_ref().map(|plan| {
                // A held phase names its holder as the pane names phases: by
                // title, else by id.
                let label = |id: &str| {
                    plan.phases
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.title.trim())
                        .filter(|title| !title.is_empty())
                        .unwrap_or(id)
                        .to_string()
                };
                PanePlan {
                    title: plan.title.clone(),
                    phases: plan
                        .phases
                        .iter()
                        .map(|p| PanePhase {
                            id: p.id.clone(),
                            title: p.title.clone(),
                            kind: p.kind,
                            status: p.status,
                            counts: p.counts.clone(),
                            agent_ids: p.agent_ids.clone(),
                            handoff: p.handoff.clone(),
                            held_by: p.held_by.as_deref().map(label),
                            gate: p.gate.as_ref().map(PaneGate::of),
                        })
                        .collect(),
                }
            }),
        }
    }

    pub fn any_active(&self) -> bool {
        self.rows.iter().any(|r| r.status.is_active())
    }

    pub fn row(&self, session_id: &str) -> Option<&PaneRow> {
        self.rows.iter().find(|r| r.session_id == session_id)
    }

    pub fn tally(&self) -> Tally {
        Tally::of(self.rows.iter())
    }

    /// The rows, grouped the way the pane draws them: under the plan's phases
    /// when there is a plan; by state when there is no plan and more than
    /// [`TWO_LINE_MAX_ROWS`] rows; otherwise one group without a header.
    pub fn groups(&self) -> Vec<Group> {
        if let Some(plan) = &self.plan {
            let mut placed = vec![false; self.rows.len()];
            let mut groups = Vec::new();
            for (pi, phase) in plan.phases.iter().enumerate() {
                let mut rows = Vec::new();
                for (ri, row) in self.rows.iter().enumerate() {
                    if placed[ri] {
                        continue;
                    }
                    let in_phase = match &row.phase_id {
                        Some(id) => id == &phase.id,
                        None => phase.agent_ids.iter().any(|a| a == &row.session_id),
                    };
                    if in_phase {
                        placed[ri] = true;
                        rows.push(ri);
                    }
                }
                groups.push(Group {
                    head: Some(GroupHead::Phase(pi)),
                    rows,
                });
            }
            let rest: Vec<usize> = (0..self.rows.len()).filter(|&i| !placed[i]).collect();
            if !rest.is_empty() {
                groups.push(Group {
                    head: Some(GroupHead::Unphased),
                    rows: rest,
                });
            }
            return groups;
        }
        if self.rows.len() > TWO_LINE_MAX_ROWS {
            return [
                Bucket::Active,
                Bucket::Blocked,
                Bucket::Failed,
                Bucket::Stopped,
                Bucket::Done,
                Bucket::Unknown,
            ]
            .into_iter()
            .filter_map(|bucket| {
                let rows: Vec<usize> = self
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| Bucket::of(r.status) == bucket)
                    .map(|(i, _)| i)
                    .collect();
                (!rows.is_empty()).then_some(Group {
                    head: Some(GroupHead::State(bucket)),
                    rows,
                })
            })
            .collect();
        }
        vec![Group {
            head: None,
            rows: (0..self.rows.len()).collect(),
        }]
    }

    /// Row indices in the order the pane draws them.
    pub fn display_order(&self) -> Vec<usize> {
        self.groups().into_iter().flat_map(|g| g.rows).collect()
    }

    /// Resolve an `/attach` argument: a 1-based number in spawn order, a
    /// session id or a unique prefix of one, or a role or name (exact, then a
    /// unique prefix, ignoring case). `Err` carries the sentence the person
    /// reads. Roles count because they are what the rows lead with.
    pub fn resolve(&self, arg: &str) -> Result<String, String> {
        let arg = arg.trim();
        if self.rows.is_empty() {
            return Err("This chat has no subagents yet.".to_string());
        }
        if let Ok(n) = arg.parse::<usize>() {
            return match n.checked_sub(1).and_then(|i| self.rows.get(i)) {
                Some(row) => Ok(row.session_id.clone()),
                None => Err(format!(
                    "There is no subagent number {n}. This chat has {}; /subagents list shows them.",
                    self.rows.len()
                )),
            };
        }
        if let Some(row) = self.rows.iter().find(|r| r.session_id == arg) {
            return Ok(row.session_id.clone());
        }
        let lower = arg.to_lowercase();
        let names = |r: &PaneRow| {
            [Some(r.name.as_str()), r.role.as_deref()]
                .into_iter()
                .flatten()
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        };
        let exact: Vec<&PaneRow> = self
            .rows
            .iter()
            .filter(|r| names(r).contains(&lower))
            .collect();
        if let [one] = exact.as_slice() {
            return Ok(one.session_id.clone());
        }
        let matches: Vec<&PaneRow> = if exact.is_empty() {
            self.rows
                .iter()
                .filter(|r| {
                    r.session_id.starts_with(arg) || names(r).iter().any(|n| n.starts_with(&lower))
                })
                .collect()
        } else {
            exact
        };
        match matches.as_slice() {
            [one] => Ok(one.session_id.clone()),
            [] => Err(format!("No subagent of this chat matches \"{arg}\".")),
            many => Err(format!(
                "\"{arg}\" matches {} subagents. Use its number from /subagents list.",
                many.len()
            )),
        }
    }
}

/// Counts over a set of rows. `running` is queued, running or stopping;
/// `failed` is failed or interrupted; `stopped` is incomplete, which is what
/// a subagent the person stopped (Ctrl-C, Stop) ends as, so it is not counted
/// as a failure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub total: usize,
    pub running: usize,
    pub done: usize,
    pub failed: usize,
    pub stopped: usize,
    pub blocked: usize,
    pub quiet: usize,
}

impl Tally {
    fn of<'a>(rows: impl Iterator<Item = &'a PaneRow>) -> Self {
        let mut t = Tally::default();
        for row in rows {
            t.total += 1;
            match Bucket::of(row.status) {
                Bucket::Active => t.running += 1,
                Bucket::Done => t.done += 1,
                Bucket::Failed => t.failed += 1,
                Bucket::Stopped => t.stopped += 1,
                Bucket::Blocked => t.blocked += 1,
                Bucket::Unknown => {}
            }
            if row.is_quiet() {
                t.quiet += 1;
            }
        }
        t
    }

    fn text(&self) -> String {
        let mut s = format!(
            "running {} · done {}/{}",
            self.running, self.done, self.total
        );
        if self.failed > 0 {
            s.push_str(&format!(" · failed {}", self.failed));
        }
        if self.stopped > 0 {
            s.push_str(&format!(" · stopped {}", self.stopped));
        }
        if self.blocked > 0 {
            s.push_str(&format!(" · blocked {}", self.blocked));
        }
        if self.quiet > 0 {
            s.push_str(&format!(" · {} quiet", self.quiet));
        }
        s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Active,
    Blocked,
    Failed,
    Stopped,
    Done,
    Unknown,
}

impl Bucket {
    fn of(status: SubagentRunStatus) -> Self {
        match status {
            SubagentRunStatus::Queued
            | SubagentRunStatus::Running
            | SubagentRunStatus::Cancelling => Bucket::Active,
            SubagentRunStatus::Blocked => Bucket::Blocked,
            SubagentRunStatus::Failed | SubagentRunStatus::Interrupted => Bucket::Failed,
            SubagentRunStatus::Incomplete => Bucket::Stopped,
            SubagentRunStatus::Completed => Bucket::Done,
            SubagentRunStatus::Unknown => Bucket::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Bucket::Active => "running",
            Bucket::Blocked => "blocked",
            Bucket::Failed => "failed",
            Bucket::Stopped => "stopped",
            Bucket::Done => "done",
            Bucket::Unknown => "unknown",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Bucket::Active => "◐",
            Bucket::Blocked => "⏸",
            Bucket::Failed | Bucket::Stopped => "✗",
            Bucket::Done => "●",
            Bucket::Unknown => "·",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupHead {
    /// Index into the plan's phases.
    Phase(usize),
    /// Rows the plan does not place in any phase.
    Unphased,
    State(Bucket),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub head: Option<GroupHead>,
    pub rows: Vec<usize>,
}

// ── pane state ──────────────────────────────────────────────────────────────

/// The pane's state: the model, whether it is shown, focus and selection.
#[derive(Debug, Default)]
pub struct SubagentPane {
    pub model: PaneModel,
    /// `None` follows the chat (shown while it has subagents); `Some` is the
    /// person's own choice from `Ctrl-B` or `/subagents`.
    pub toggle: Option<bool>,
    /// Keys go to the pane rather than the input.
    pub focused: bool,
    /// Selected row, by session id so it survives a re-read that reorders.
    pub selected: Option<String>,
    /// First list line drawn (side pane) or first row drawn (strip).
    pub scroll: usize,
    /// A row Enter asked to open; the event loop loads it (that needs the
    /// session store, which a key handler does not have).
    pub pending_open: Option<String>,
    /// Ticks since the last idle re-read, for [`IDLE_POLL_EVERY`].
    pub idle_ticks: u32,
}

impl SubagentPane {
    pub fn visible(&self) -> bool {
        self.toggle.unwrap_or(!self.model.rows.is_empty())
    }

    pub fn toggle_visible(&mut self) {
        let shown = !self.visible();
        self.toggle = Some(shown);
        if !shown {
            self.focused = false;
        }
    }

    /// Show the pane and move focus into it, selecting a row if none is.
    pub fn focus(&mut self) {
        if !self.visible() {
            self.toggle = Some(true);
        }
        self.focused = true;
        if self.selected_row().is_none() {
            self.selected = self.default_selection();
        }
    }

    /// Replace the model. Returns true when anything the pane draws changed.
    pub fn set_model(&mut self, model: PaneModel) -> bool {
        if self.model == model {
            return false;
        }
        self.model = model;
        // A pane that is no longer drawn cannot hold focus: one that followed
        // the chat hides itself when the roster empties (its children deleted
        // elsewhere), and focus left on it would swallow arrows and Enter while
        // the input showed no cursor.
        if !self.visible() {
            self.focused = false;
            self.selected = None;
            return true;
        }
        if self.selected.is_some() && self.selected_row().is_none() {
            self.selected = if self.focused {
                self.default_selection()
            } else {
                None
            };
        }
        true
    }

    /// Whether the idle loop should re-read the roster on this tick.
    pub fn wants_poll(&mut self, child_open: bool) -> bool {
        if self.model.any_active() || child_open {
            self.idle_ticks = 0;
            return true;
        }
        if !self.visible() {
            return false;
        }
        self.idle_ticks += 1;
        if self.idle_ticks >= IDLE_POLL_EVERY {
            self.idle_ticks = 0;
            return true;
        }
        false
    }

    pub fn selected_row(&self) -> Option<&PaneRow> {
        self.selected.as_deref().and_then(|id| self.model.row(id))
    }

    fn default_selection(&self) -> Option<String> {
        let order = self.model.display_order();
        order
            .iter()
            .find(|&&i| self.model.rows[i].status.is_active())
            .or(order.first())
            .map(|&i| self.model.rows[i].session_id.clone())
    }

    pub fn move_selection(&mut self, delta: i64) {
        let order = self.model.display_order();
        if order.is_empty() {
            self.selected = None;
            return;
        }
        let current = self.selected.as_deref().and_then(|id| {
            order
                .iter()
                .position(|&i| self.model.rows[i].session_id == id)
        });
        let next = match current {
            None => 0,
            Some(pos) => (pos as i64 + delta).clamp(0, order.len() as i64 - 1) as usize,
        };
        self.selected = Some(self.model.rows[order[next]].session_id.clone());
    }

    pub fn select_edge(&mut self, last: bool) {
        let order = self.model.display_order();
        let pick = if last { order.last() } else { order.first() };
        self.selected = pick.map(|&i| self.model.rows[i].session_id.clone());
    }

    /// The row the list keeps in view: the selection while focused, else the
    /// first active row, so a running child is on screen without a key press.
    fn anchor(&self) -> Option<usize> {
        if self.focused {
            if let Some(id) = self.selected.as_deref() {
                return self.model.rows.iter().position(|r| r.session_id == id);
            }
        }
        self.model
            .display_order()
            .into_iter()
            .find(|&i| self.model.rows[i].status.is_active())
    }

    /// One-line summary for the hints line while the pane is hidden.
    pub fn summary(&self) -> Option<String> {
        let t = self.model.tally();
        if t.total == 0 {
            return None;
        }
        let noun = if t.total == 1 {
            "subagent"
        } else {
            "subagents"
        };
        Some(if t.running > 0 {
            format!("◐ {} of {} {noun} running", t.running, t.total)
        } else {
            format!("{} {noun} · done {}/{}", t.total, t.done, t.total)
        })
    }
}

// ── formatting ──────────────────────────────────────────────────────────────

pub fn status_glyph(status: SubagentRunStatus) -> &'static str {
    match status {
        SubagentRunStatus::Queued => "○",
        SubagentRunStatus::Running | SubagentRunStatus::Cancelling => "◐",
        SubagentRunStatus::Completed => "●",
        SubagentRunStatus::Blocked => "⏸",
        SubagentRunStatus::Failed | SubagentRunStatus::Incomplete => "✗",
        SubagentRunStatus::Interrupted => "⊘",
        SubagentRunStatus::Unknown => "·",
    }
}

fn status_style(status: SubagentRunStatus) -> Style {
    match status {
        SubagentRunStatus::Queued | SubagentRunStatus::Unknown => DIM,
        SubagentRunStatus::Running | SubagentRunStatus::Cancelling => Style::new().fg(ACCENT),
        SubagentRunStatus::Completed => Style::default(),
        SubagentRunStatus::Blocked | SubagentRunStatus::Interrupted => {
            Style::new().fg(Color::Yellow)
        }
        SubagentRunStatus::Failed | SubagentRunStatus::Incomplete => Style::new().fg(Color::Red),
    }
}

pub fn status_word(status: SubagentRunStatus) -> &'static str {
    match status {
        SubagentRunStatus::Queued => "queued",
        SubagentRunStatus::Running => "running",
        SubagentRunStatus::Cancelling => "stopping",
        SubagentRunStatus::Completed => "done",
        SubagentRunStatus::Blocked => "blocked",
        SubagentRunStatus::Incomplete => "incomplete",
        SubagentRunStatus::Failed => "failed",
        SubagentRunStatus::Interrupted => "interrupted",
        SubagentRunStatus::Unknown => "unknown",
    }
}

fn phase_glyph(status: PhaseStatus) -> &'static str {
    match status {
        PhaseStatus::Pending => "○",
        PhaseStatus::InProgress => "◐",
        PhaseStatus::Completed => "●",
        PhaseStatus::Blocked => "⏸",
        PhaseStatus::Skipped => "⊘",
    }
}

fn phase_style(status: PhaseStatus) -> Style {
    match status {
        PhaseStatus::Pending | PhaseStatus::Skipped => DIM,
        PhaseStatus::InProgress => Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        PhaseStatus::Completed => Style::new().add_modifier(Modifier::BOLD),
        PhaseStatus::Blocked => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    }
}

/// `1m42s`, `42s`, `1h05m`.
pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// A coarse age for "quiet 4m": seconds, minutes or hours, whole units.
fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

pub fn format_tokens(n: i64) -> String {
    let n = n.max(0) as f64;
    if n < 1_000.0 {
        format!("{n}")
    } else if n < 1_000_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{:.1}M", n / 1_000_000.0)
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("{n} {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The row's model as every surface names it (`gpt-5.5`, not
/// `gpt-5.5-2026-04-24`), else its provider.
fn model_label(row: &PaneRow) -> Option<String> {
    short_model_label(row.model.as_deref(), row.provider.as_deref())
}

fn setup_parts(row: &PaneRow, compact: bool) -> Vec<String> {
    let mut parts: Vec<String> = model_label(row).into_iter().collect();
    parts.extend(setup_counts(
        row.extensions.len(),
        row.skills.len(),
        row.knowledge_bases.len(),
        compact,
    ));
    if let Some(secs) = row.elapsed_seconds {
        parts.push(format_elapsed(secs));
    }
    if row.is_quiet() {
        parts.push(format!(
            "quiet {}",
            format_age(row.quiet_seconds.unwrap_or(0))
        ));
    }
    parts
}

/// `gpt-5.5 · 3 ext · 2 skills · 1 knowledge base · 1m42s` (plus
/// `· quiet 4m`): the desktop's second line, and like it, a count that is zero
/// is left out.
pub fn setup_summary(row: &PaneRow) -> String {
    setup_parts(row, false).join(" · ")
}

/// [`setup_summary`] for a line of `width` cells. Knowledge bases are written
/// `1 kb base` only when the words do not fit; when even that does not fit,
/// the line keeps the parts that do and ends in `…`, so a count or a time is
/// never cut in half (`1m…`).
pub fn setup_summary_fit(row: &PaneRow, width: usize) -> String {
    let full = setup_summary(row);
    if UnicodeWidthStr::width(full.as_str()) <= width {
        return full;
    }
    let parts = setup_parts(row, true);
    let compact = parts.join(" · ");
    if UnicodeWidthStr::width(compact.as_str()) <= width {
        return compact;
    }
    let mut out = String::new();
    for part in &parts {
        let next = if out.is_empty() {
            part.clone()
        } else {
            format!("{out} · {part}")
        };
        if UnicodeWidthStr::width(next.as_str()) + 2 > width {
            break;
        }
        out = next;
    }
    if out.is_empty() {
        return super::clip_cells(&compact, width);
    }
    out.push_str(" …");
    out
}

/// The dim tail of a one-line row: what it is doing, or how it ended.
fn dense_tail(row: &PaneRow) -> String {
    let mut parts = Vec::new();
    match row.status {
        SubagentRunStatus::Running | SubagentRunStatus::Completed => {}
        other => parts.push(status_word(other).to_string()),
    }
    if let Some(secs) = row.elapsed_seconds {
        parts.push(format_elapsed(secs));
    }
    if row.is_quiet() {
        parts.push(format!(
            "quiet {}",
            format_age(row.quiet_seconds.unwrap_or(0))
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

fn phase_head_text(phase: &PanePhase, counts: &PhaseCounts) -> String {
    let mut s = phase_head_counts(phase, counts);
    // E-7: the phase's gate, passed or blocked, on its own heading.
    if let Some(gate) = phase.gate {
        s.push_str(" · ");
        s.push_str(gate.words());
    }
    s
}

fn phase_head_counts(phase: &PanePhase, counts: &PhaseCounts) -> String {
    let mut s = format!("{} {}", phase_glyph(phase.status), phase.label());
    // A phase an earlier gate holds says what it waits for, in place of "not
    // started" or "starting", which it cannot do until that gate is through.
    let held = phase
        .held_by
        .as_deref()
        .map(crate::commands::session_gates::held_words);
    if counts.total == 0 {
        match (&held, phase.status) {
            (Some(held), PhaseStatus::Pending | PhaseStatus::InProgress) => {
                s.push_str(" · ");
                s.push_str(held);
            }
            _ => s.push_str(match phase.status {
                PhaseStatus::Pending => " · not started",
                PhaseStatus::InProgress => " · starting",
                PhaseStatus::Completed => " · done",
                PhaseStatus::Blocked => " · blocked",
                PhaseStatus::Skipped => " · skipped",
            }),
        }
        if let (Some(held), PhaseStatus::Blocked) = (&held, phase.status) {
            s.push_str(" · ");
            s.push_str(held);
        }
        return s;
    }
    s.push_str(&format!(" {}/{}", counts.done, counts.total));
    if counts.running > 0 {
        s.push_str(&format!(" · {} running", counts.running));
    }
    if counts.quiet > 0 {
        s.push_str(&format!(" · {} quiet", counts.quiet));
    }
    if counts.failed > 0 {
        s.push_str(&format!(" · {} failed", counts.failed));
    }
    if counts.blocked > 0 {
        s.push_str(&format!(" · {} blocked", counts.blocked));
    }
    if let Some(held) = held {
        s.push_str(" · ");
        s.push_str(&held);
    }
    s
}

/// The plan's counts for a phase, or counts made from the rows it holds when
/// the plan reports none for a phase that has rows (a plan read that raced a
/// spawn). The plan's own counts win whenever it has any, so this pane shows
/// what the desktop and the CLI show.
fn phase_counts(model: &PaneModel, phase: &PanePhase, rows: &[usize]) -> PhaseCounts {
    if phase.counts.total > 0 || rows.is_empty() {
        return phase.counts.clone();
    }
    let t = Tally::of(rows.iter().map(|&i| &model.rows[i]));
    PhaseCounts {
        total: t.total as u32,
        done: t.done as u32,
        running: t.running as u32,
        blocked: t.blocked as u32,
        failed: t.failed as u32,
        quiet: t.quiet as u32,
    }
}

fn group_head_line(model: &PaneModel, group: &Group, width: usize) -> Option<Line<'static>> {
    let (text, style) = match group.head? {
        GroupHead::Phase(pi) => {
            let phase = &model.plan.as_ref()?.phases[pi];
            let counts = phase_counts(model, phase, &group.rows);
            (phase_head_text(phase, &counts), phase_style(phase.status))
        }
        // The rows under it say how many there are; a number after the label
        // read as part of it ("no phase 2").
        GroupHead::Unphased => (
            "· no phase".to_string(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        GroupHead::State(bucket) => (
            format!("{} {} {}", bucket.glyph(), bucket.label(), group.rows.len()),
            Style::new().add_modifier(Modifier::BOLD),
        ),
    };
    Some(Line::from(Span::styled(
        super::clip_cells(&text, width),
        style,
    )))
}

/// Truncate a line's spans to `width` display cells, ending in `…` when cut.
fn clip_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let total: usize = spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if total <= width {
        return spans;
    }
    let budget = width.saturating_sub(1);
    let mut used = 0usize;
    let mut out = Vec::new();
    for span in spans {
        if used >= budget {
            break;
        }
        let mut text = String::new();
        for ch in span.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + cw > budget {
                used = budget;
                break;
            }
            text.push(ch);
            used += cw;
        }
        out.push(Span::styled(text, span.style));
    }
    out.push(Span::styled("…", DIM));
    out
}

fn row_marker(pane: &SubagentPane, row: &PaneRow, open_child: Option<&str>) -> Span<'static> {
    let selected = pane.focused && pane.selected.as_deref() == Some(row.session_id.as_str());
    if selected {
        Span::styled("▸ ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
    } else if open_child == Some(row.session_id.as_str()) {
        Span::styled("› ", Style::new().fg(ACCENT))
    } else {
        Span::raw("  ")
    }
}

fn name_spans(pane: &SubagentPane, row: &PaneRow, open_child: Option<&str>) -> Vec<Span<'static>> {
    let selected = pane.focused && pane.selected.as_deref() == Some(row.session_id.as_str());
    let mut name_style = Style::new().add_modifier(Modifier::BOLD);
    if selected {
        name_style = name_style.fg(ACCENT).add_modifier(Modifier::REVERSED);
    }
    let mut spans = vec![
        row_marker(pane, row, open_child),
        Span::styled(status_glyph(row.status), status_style(row.status)),
        Span::raw(" "),
    ];
    if row.private {
        spans.push(Span::raw(format!("{LOCK} ")));
    }
    spans.push(Span::styled(row.label().to_string(), name_style));
    spans
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

/// Append the session name, dimmed, to a row that leads with its role, when at
/// least [`SECONDARY_NAME_MIN`] cells of it fit in `width`; the line's clip
/// then ends it in `…`.
fn push_secondary_name(spans: &mut Vec<Span<'static>>, row: &PaneRow, width: usize) {
    let Some(name) = row.secondary_name() else {
        return;
    };
    if spans_width(spans) + 2 + SECONDARY_NAME_MIN > width {
        return;
    }
    spans.push(Span::styled(format!("  {name}"), DIM));
}

/// What a list line shows, so the scroller knows where the selection sits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Head,
    Row(usize),
}

fn list_lines(
    pane: &SubagentPane,
    width: usize,
    open_child: Option<&str>,
) -> Vec<(LineKind, Line<'static>)> {
    let model = &pane.model;
    let two_line = model.rows.len() <= TWO_LINE_MAX_ROWS;
    let mut out = Vec::new();
    for group in model.groups() {
        if let Some(head) = group_head_line(model, &group, width) {
            out.push((LineKind::Head, head));
        }
        for &ri in &group.rows {
            let row = &model.rows[ri];
            let mut first = name_spans(pane, row, open_child);
            if two_line {
                push_secondary_name(&mut first, row, width);
                out.push((LineKind::Row(ri), Line::from(clip_spans(first, width))));
                let setup = setup_summary_fit(row, width.saturating_sub(4));
                if !setup.is_empty() {
                    let second = vec![Span::raw("    "), Span::styled(setup, DIM)];
                    out.push((LineKind::Row(ri), Line::from(clip_spans(second, width))));
                }
            } else {
                first.push(Span::styled(dense_tail(row), DIM));
                push_secondary_name(&mut first, row, width);
                out.push((LineKind::Row(ri), Line::from(clip_spans(first, width))));
            }
        }
    }
    out
}

fn labelled(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<LABEL_W$}"), DIM),
        Span::raw(value),
    ])
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

fn phase_title(model: &PaneModel, phase_id: &str) -> String {
    model
        .plan
        .as_ref()
        .and_then(|p| p.phases.iter().find(|ph| ph.id == phase_id))
        .map(PanePhase::label)
        .unwrap_or_else(|| phase_id.to_string())
}

/// The selected row's setup, standing in for the desktop's hover card.
pub fn details_lines(model: &PaneModel, row: &PaneRow) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(status_glyph(row.status), status_style(row.status)),
        Span::raw(" "),
        Span::styled(
            row.label().to_string(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}", status_word(row.status)), DIM),
        Span::raw(if row.private {
            format!("  {LOCK} private")
        } else {
            String::new()
        }),
    ])];
    if let Some(role) = &row.role {
        lines.push(labelled("role", role.clone()));
    }
    if let Some(name) = row.secondary_name() {
        lines.push(labelled("name", name.to_string()));
    }
    if let Some(phase) = &row.phase_id {
        lines.push(labelled("phase", phase_title(model, phase)));
    }
    let model_text = match (row.model.as_deref(), row.provider.as_deref()) {
        (Some(m), Some(p)) => format!("{m} · {p}"),
        (Some(m), None) => m.to_string(),
        (None, Some(p)) => p.to_string(),
        (None, None) => "unknown".to_string(),
    };
    lines.push(labelled("model", model_text));
    lines.push(labelled("extensions", list_or_none(&row.extensions)));
    lines.push(labelled("skills", list_or_none(&row.skills)));
    lines.push(labelled("knowledge", list_or_none(&row.knowledge_bases)));
    if let Some(tokens) = row.total_tokens {
        lines.push(labelled("tokens", format_tokens(tokens)));
    }
    lines.push(labelled(
        "tool calls",
        format!(
            "{} · {}",
            row.tool_call_count.max(0),
            plural(row.message_count.max(0) as usize, "message", "messages")
        ),
    ));
    let mut state = Vec::new();
    if let Some(phase) = row.loop_phase.as_deref().filter(|_| row.status.is_active()) {
        state.push(phase.replace('_', " "));
    }
    if let Some(secs) = row.elapsed_seconds {
        state.push(format_elapsed(secs));
    }
    if row.is_quiet() {
        state.push(format!(
            "quiet {}",
            format_age(row.quiet_seconds.unwrap_or(0))
        ));
    }
    if !state.is_empty() {
        lines.push(labelled("activity", state.join(" · ")));
    }
    if let Some(result) = row.result_line.as_deref().filter(|r| !r.trim().is_empty()) {
        lines.push(labelled("result", result.to_string()));
    }
    lines
}

/// The one-line form of [`details_lines`] for the narrow strip.
fn details_one_line(row: &PaneRow) -> String {
    let mut parts = vec![row.model.clone().unwrap_or_else(|| "model unknown".into())];
    if !row.extensions.is_empty() {
        parts.push(format!("ext: {}", row.extensions.join(", ")));
    }
    if !row.skills.is_empty() {
        parts.push(format!("skills: {}", row.skills.join(", ")));
    }
    if !row.knowledge_bases.is_empty() {
        parts.push(format!("knowledge: {}", row.knowledge_bases.join(", ")));
    }
    if let Some(t) = row.total_tokens {
        parts.push(format!("{} tokens", format_tokens(t)));
    }
    if row.tool_call_count > 0 {
        parts.push(plural(
            row.tool_call_count as usize,
            "tool call",
            "tool calls",
        ));
    }
    if let Some(name) = row.secondary_name() {
        parts.push(name.to_string());
    }
    format!("    {} · ↵ open", parts.join(" · "))
}

fn header_line(pane: &SubagentPane, width: usize) -> Line<'static> {
    let mut spans = vec![Span::styled(
        "Subagents",
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    )];
    if !pane.model.rows.is_empty() {
        spans.push(Span::styled(
            format!("  {}", pane.model.tally().text()),
            DIM,
        ));
    }
    Line::from(clip_spans(spans, width))
}

/// The side pane's header: the counts beside the title when they fit, else
/// on a line of their own (never cut, since they are the pane's summary).
fn header_lines(pane: &SubagentPane, width: usize) -> Vec<Line<'static>> {
    let title = Span::styled(
        "Subagents",
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    );
    if pane.model.rows.is_empty() {
        return vec![Line::from(title)];
    }
    let counts = pane.model.tally().text();
    if UnicodeWidthStr::width("Subagents  ") + UnicodeWidthStr::width(counts.as_str()) <= width {
        vec![Line::from(vec![
            title,
            Span::styled(format!("  {counts}"), DIM),
        ])]
    } else {
        vec![
            Line::from(title),
            Line::from(Span::styled(super::clip_cells(&counts, width), DIM)),
        ]
    }
}

fn empty_line() -> Line<'static> {
    Line::from(Span::styled(
        "No subagents in this chat yet. ^B hides this pane.",
        DIM,
    ))
}

// ── side pane (wide) ────────────────────────────────────────────────────────

/// Width of the side pane for an inner width of `inner_width`.
pub fn side_width(inner_width: u16) -> u16 {
    (inner_width / 3).clamp(PANE_MIN_WIDTH, PANE_MAX_WIDTH)
}

pub fn draw_side(f: &mut Frame, pane: &mut SubagentPane, area: Rect, open_child: Option<&str>) {
    let border = if pane.focused {
        Style::new().fg(ACCENT)
    } else {
        Style::new().fg(Color::Indexed(240))
    };
    let hint = if pane.focused {
        " ↑↓ select · ↵ open · esc back "
    } else {
        " ^O select · ^B hide "
    };
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(border)
        .padding(Padding::new(1, 0, 0, 0))
        .title_bottom(Line::from(Span::styled(hint, DIM)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;

    let mut head = header_lines(pane, width);
    if let Some(plan) = pane
        .model
        .plan
        .as_ref()
        .filter(|p| !p.title.trim().is_empty())
    {
        head.push(Line::from(Span::styled(
            super::clip_cells(&plan.title, width),
            DIM,
        )));
    }
    let details = if pane.focused {
        pane.selected_row()
            .map(|row| details_lines(&pane.model, row))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // Wrapped here, one row per line, so the height below is the height drawn.
    let details: Vec<Line<'static>> = details
        .iter()
        .flat_map(|line| super::wrap_line(line, inner.width))
        .collect();
    let details_h = if details.is_empty() {
        0
    } else {
        // One rule line above the block, and never more than half the pane.
        (details.len().min(u16::MAX as usize - 1) as u16 + 1).min(inner.height / 2)
    };
    let regions = Layout::vertical([
        Constraint::Length(head.len() as u16),
        Constraint::Min(1),
        Constraint::Length(details_h),
    ])
    .split(inner);
    f.render_widget(Paragraph::new(head), regions[0]);

    if pane.model.rows.is_empty() {
        f.render_widget(
            Paragraph::new(empty_line()).wrap(Wrap { trim: false }),
            regions[1],
        );
    } else {
        draw_side_list(f, pane, regions[1], open_child);
    }

    if details_h > 0 {
        let mut lines = vec![Line::from(Span::styled("─".repeat(width), DIM))];
        lines.extend(details);
        f.render_widget(Paragraph::new(lines), regions[2]);
    }
}

fn draw_side_list(f: &mut Frame, pane: &mut SubagentPane, area: Rect, open_child: Option<&str>) {
    let width = area.width as usize;
    let height = area.height as usize;
    if height == 0 {
        return;
    }
    let lines = list_lines(pane, width, open_child);
    let total = lines.len();
    let window = side_window(&lines, height, pane.scroll, pane.anchor());
    pane.scroll = window.start;

    let rows_in = |range: std::ops::Range<usize>| {
        let mut seen: Vec<usize> = Vec::new();
        for (kind, _) in &lines[range] {
            if let LineKind::Row(ri) = kind {
                if !seen.contains(ri) {
                    seen.push(*ri);
                }
            }
        }
        seen.len()
    };
    let mut out: Vec<Line> = Vec::new();
    if window.top_more {
        out.push(Line::from(Span::styled(
            format!("  ↑ {} more", rows_in(0..window.start)),
            DIM,
        )));
    }
    out.extend(
        lines[window.start..window.end]
            .iter()
            .map(|(_, line)| line.clone()),
    );
    if window.bottom_more {
        out.push(Line::from(Span::styled(
            format!("  +{} more", rows_in(window.end..total)),
            DIM,
        )));
    }
    f.render_widget(Paragraph::new(out), area);
}

/// Which list lines the side pane draws: `start..end`, plus whether a line
/// above or below says how many rows are out of view.
#[derive(Debug, PartialEq, Eq)]
struct SideWindow {
    start: usize,
    end: usize,
    top_more: bool,
    bottom_more: bool,
}

fn side_window(
    lines: &[(LineKind, Line<'static>)],
    height: usize,
    scroll: usize,
    anchor: Option<usize>,
) -> SideWindow {
    let total = lines.len();
    if total <= height || height < 3 {
        let start = if total <= height {
            0
        } else {
            scroll.min(total - height)
        };
        return SideWindow {
            start,
            end: (start + height).min(total),
            top_more: false,
            bottom_more: false,
        };
    }
    // With a top indicator the body is one line shorter, and one more when a
    // bottom indicator is needed too.
    let at = |start: usize| {
        let top_more = start > 0;
        let mut body = height - usize::from(top_more);
        let bottom_more = start + body < total;
        if bottom_more {
            body -= 1;
        }
        SideWindow {
            start,
            end: start + body,
            top_more,
            bottom_more,
        }
    };
    let has_row = |range: std::ops::Range<usize>| {
        lines[range]
            .iter()
            .any(|(kind, _)| matches!(kind, LineKind::Row(_)))
    };
    let max_start = total + 1 - height;
    let span = anchor.and_then(|ri| {
        let first = lines.iter().position(|(k, _)| *k == LineKind::Row(ri))?;
        let last = lines.iter().rposition(|(k, _)| *k == LineKind::Row(ri))?;
        // Keep the group header above the anchor in view when it fits.
        let first = if first > 0 && lines[first - 1].0 == LineKind::Head {
            first - 1
        } else {
            first
        };
        Some((first, last))
    });
    let shows = |w: &SideWindow| span.is_none_or(|(first, last)| first >= w.start && last < w.end);

    let mut start = scroll.min(max_start);
    if let Some((first, _)) = span {
        if first < start {
            start = first;
        }
        while !shows(&at(start)) && start < max_start {
            start += 1;
        }
    }
    // An indicator that would count no rows (only a phase header is out of
    // view) gives its line back to the list.
    if start > 0 && !has_row(0..start) && shows(&at(0)) {
        start = 0;
    }
    let window = at(start);
    if window.bottom_more && !has_row(window.end..total) && shows(&at(max_start)) {
        return at(max_start);
    }
    window
}

// ── strip (narrow) ──────────────────────────────────────────────────────────

/// Rows the narrow strip takes, 0 when the pane is hidden.
pub fn strip_height(pane: &SubagentPane) -> u16 {
    if !pane.visible() {
        return 0;
    }
    let rows = pane.model.rows.len();
    if rows == 0 {
        return 1;
    }
    let plan = u16::from(pane.model.plan.is_some());
    let details = u16::from(pane.focused && pane.selected_row().is_some());
    (1 + plan + details + rows.min(STRIP_MAX_ROWS as usize) as u16).clamp(1, STRIP_MAX_ROWS)
}

/// The plan's phases on one line: `● plan 1/1  ◐ research 3/5 · 1 quiet`.
fn phases_line(model: &PaneModel, width: usize) -> Option<Line<'static>> {
    let plan = model.plan.as_ref()?;
    let groups = model.groups();
    let mut spans = vec![Span::raw("  ")];
    for (pi, phase) in plan.phases.iter().enumerate() {
        let rows = groups
            .iter()
            .find(|g| g.head == Some(GroupHead::Phase(pi)))
            .map(|g| g.rows.clone())
            .unwrap_or_default();
        let counts = phase_counts(model, phase, &rows);
        if pi > 0 {
            spans.push(Span::styled("   ", DIM));
        }
        spans.push(Span::styled(
            phase_head_text(phase, &counts),
            phase_style(phase.status),
        ));
    }
    Some(Line::from(clip_spans(spans, width)))
}

pub fn draw_strip(f: &mut Frame, pane: &mut SubagentPane, area: Rect, open_child: Option<&str>) {
    let width = area.width as usize;
    let height = area.height as usize;
    if height == 0 || width == 0 {
        return;
    }
    let mut out = vec![header_line(pane, width)];
    if pane.model.rows.is_empty() {
        if height > 1 {
            out.push(empty_line());
        } else {
            out[0] = Line::from(clip_spans(
                vec![
                    Span::styled(
                        "Subagents",
                        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  none in this chat yet · ^B hides this", DIM),
                ],
                width,
            ));
        }
        f.render_widget(Paragraph::new(out), area);
        return;
    }
    if let Some(line) = phases_line(&pane.model, width) {
        if out.len() < height {
            out.push(line);
        }
    }
    let details = if pane.focused {
        pane.selected_row().map(details_one_line)
    } else {
        None
    };
    let room = height
        .saturating_sub(out.len())
        .saturating_sub(usize::from(details.is_some()));

    let order = pane.model.display_order();
    let n = order.len();
    let (shown, more) = if n <= room {
        (room.min(n), 0)
    } else {
        let shown = room.saturating_sub(1);
        (shown, n - shown)
    };
    // Keep the anchor in the window. Unfocused, the anchor is the first active
    // row and it leads the window, so the few rows a strip has go to children
    // that are still working; focused, the window follows the selection.
    let anchor_pos = pane
        .anchor()
        .and_then(|ri| order.iter().position(|&i| i == ri));
    let mut start = pane.scroll.min(n.saturating_sub(shown.max(1)));
    if let Some(pos) = anchor_pos {
        if !pane.focused {
            start = pos.min(n.saturating_sub(shown.max(1)));
        } else if pos < start {
            start = pos;
        } else if shown > 0 && pos >= start + shown {
            start = pos + 1 - shown;
        }
    }
    pane.scroll = start;
    for &ri in order.iter().skip(start).take(shown) {
        let row = &pane.model.rows[ri];
        let mut spans = name_spans(pane, row, open_child);
        spans.push(Span::styled(dense_tail(row), DIM));
        if let Some(model) = model_label(row) {
            spans.push(Span::styled(format!(" · {model}"), DIM));
        }
        push_secondary_name(&mut spans, row, width);
        out.push(Line::from(clip_spans(spans, width)));
    }
    if more > 0 {
        out.push(Line::from(Span::styled(format!("  +{more} more"), DIM)));
    }
    if let Some(details) = details {
        out.push(Line::from(Span::styled(
            super::clip_cells(&details, width),
            DIM,
        )));
    }
    f.render_widget(Paragraph::new(out), area);
}

// ── `/subagents list` ───────────────────────────────────────────────────────

/// The numbered list `/subagents list` prints into the chat history.
pub fn list_into(app: &mut App) {
    let model = app.subagents.model.clone();
    app.push_blank();
    if model.rows.is_empty() {
        app.push_note("This chat has no subagents yet.");
        return;
    }
    app.push_line(Line::from(vec![
        Span::styled(
            "Subagents",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {}", model.tally().text()), DIM),
    ]));
    for (i, row) in model.rows.iter().enumerate() {
        let mut spans = vec![
            Span::styled(format!("  {:>2}. ", i + 1), DIM),
            Span::styled(status_glyph(row.status), status_style(row.status)),
            Span::raw(" "),
        ];
        if row.private {
            spans.push(Span::raw(format!("{LOCK} ")));
        }
        spans.push(Span::styled(
            row.label().to_string(),
            Style::new().add_modifier(Modifier::BOLD),
        ));
        let mut tail = vec![status_word(row.status).to_string()];
        if let Some(phase) = &row.phase_id {
            tail.push(format!("phase {}", phase_title(&model, phase)));
        }
        let setup = setup_summary(row);
        if !setup.is_empty() {
            tail.push(setup);
        }
        spans.push(Span::styled(format!("  {}", tail.join(" · ")), DIM));
        app.push_line(Line::from(spans));
        if let Some(name) = row.secondary_name() {
            app.push_line(Line::from(Span::styled(
                format!("        {}", super::clip_cells(name, LIST_NAME_MAX)),
                DIM,
            )));
        }
    }
    app.push_raw(
        "Read one with /attach <number>, or press ^O, pick a row and press ↵.",
        DIM,
    );
}

// ── child view ──────────────────────────────────────────────────────────────

/// A subagent's chat shown read-only in the history area.
pub struct ChildView {
    pub session_id: String,
    /// The session's own name, used when the roster has no row for it.
    pub name: String,
    pub model: Option<String>,
    /// Renders the transcript with the same code as the parent's history.
    pub transcript: App,
    /// Live events from the child's bus; `None` once it closed.
    pub subscription: Option<Subscription>,
    /// The transcript must be re-read from the store (a turn ended, the
    /// history was replaced, or the bus lagged).
    pub needs_reload: bool,
    /// Esc puts focus back in the pane (the view was opened from it).
    pub return_to_pane: bool,
    pub debug: bool,
    /// Ids of the messages the last store read drew. The bus subscription is
    /// opened BEFORE that read so nothing published in between is lost, which
    /// means an event can arrive for a message the read already drew; this is
    /// how it is skipped instead of drawn twice.
    seen_ids: std::collections::HashSet<String>,
}

impl ChildView {
    pub fn new(session_id: impl Into<String>, return_to_pane: bool, debug: bool) -> Self {
        Self {
            session_id: session_id.into(),
            name: String::new(),
            model: None,
            transcript: App::new(StatusInfo::default()),
            subscription: None,
            needs_reload: false,
            return_to_pane,
            debug,
            seen_ids: std::collections::HashSet::new(),
        }
    }

    /// Replace the transcript with `messages`, keeping the reader's scroll.
    pub fn load(&mut self, messages: &[Message]) {
        let scroll = self.transcript.scroll;
        let t = &mut self.transcript;
        t.scrollback.clear();
        t.stream_text.clear();
        t.stream_id = None;
        t.stream_start = None;
        t.stream_dirty = false;
        t.last_tool_result = None;
        t.wrap_cache = None;
        self.seen_ids = messages.iter().filter_map(|m| m.id.clone()).collect();
        for message in messages {
            render_message(t, message, self.debug);
        }
        if messages.iter().all(|m| !m.is_user_visible()) {
            t.push_note("Nothing in this subagent's chat yet.");
        }
        t.scroll = scroll;
        self.needs_reload = false;
    }

    /// Apply one live event from the child's bus.
    pub fn apply(&mut self, event: SessionBusEvent) {
        match event {
            SessionBusEvent::Agent(AgentEvent::Message(message)) => {
                if !message.is_user_visible()
                    || message
                        .id
                        .as_ref()
                        .is_some_and(|id| self.seen_ids.contains(id))
                {
                    return;
                }
                if super::is_stream_text(&message) {
                    let delta = message.as_concat_text();
                    self.transcript.stream_delta(message.id.clone(), &delta);
                } else {
                    self.transcript.stream_commit();
                    render_message(&mut self.transcript, &message, self.debug);
                }
            }
            SessionBusEvent::Agent(AgentEvent::HistoryReplaced(_)) => self.needs_reload = true,
            SessionBusEvent::Agent(AgentEvent::TurnAborted { code, message }) => {
                self.transcript.stream_commit();
                self.transcript
                    .push_error(&format!("{}: {message}", code.wire_code()));
            }
            SessionBusEvent::TurnFinished { .. } | SessionBusEvent::TurnError { .. } => {
                // The store is the record: re-read it so what streamed live is
                // replaced by what was kept, in order and without duplicates.
                self.transcript.stream_commit();
                self.needs_reload = true;
            }
            SessionBusEvent::TurnStarted { .. } | SessionBusEvent::Agent(_) => {}
        }
    }

    /// Apply the result of an awaited `recv`.
    pub fn apply_recv(&mut self, received: Result<SessionBusEvent, RecvError>) {
        match received {
            Ok(event) => self.apply(event),
            Err(RecvError::Lagged(_)) => self.needs_reload = true,
            Err(RecvError::Closed) => {
                self.subscription = None;
                self.needs_reload = true;
            }
        }
    }

    /// Apply every event already waiting on the bus, without awaiting.
    /// Returns true when anything was applied.
    pub fn drain(&mut self) -> bool {
        let mut events = Vec::new();
        let mut changed = false;
        if let Some(sub) = self.subscription.as_mut() {
            for _ in 0..1024 {
                match sub.try_recv() {
                    Ok(event) => events.push(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Lagged(_)) => {
                        self.needs_reload = true;
                        changed = true;
                    }
                    Err(TryRecvError::Closed) => {
                        self.subscription = None;
                        self.needs_reload = true;
                        changed = true;
                        break;
                    }
                }
            }
        }
        changed |= !events.is_empty();
        for event in events {
            self.apply(event);
        }
        changed
    }
}

/// Whether `message` is a subagent's spawn-context record. Identified the way
/// the desktop identifies it (`spawnContextOf` in `useSubagentSession.ts`, and
/// the "Spawn context" chip in `ProvenanceChip.tsx`): by its provenance stamp,
/// never by its text, which the parent's instructions can imitate.
pub fn is_spawn_context(message: &Message) -> bool {
    message
        .metadata
        .provenance
        .as_ref()
        .is_some_and(|p| p.kind == ProvenanceKind::SpawnContext)
}

/// The lines of a spawn-context record the child view shows, and how many lines
/// follow them unshown.
///
/// The record (`persist_spawn_context` in `subagent_handler.rs`) is the spawning
/// session, the task's instructions, the grants and then the whole rendered
/// system prompt: 143 lines for a one-line task. What a reader opens a child to
/// see is the task, so the preview runs to the end of the `### Task
/// instructions` section (at most [`SPAWN_CONTEXT_TASK_MAX_LINES`] of it) and
/// folds the rest, as the desktop's clamp does. The record's own `## ` heading
/// is dropped, because the "Spawn context" label says it. A record with no task
/// section shows its first [`SPAWN_CONTEXT_HEAD_LINES`] lines.
pub fn spawn_context_preview(text: &str) -> (Vec<&str>, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let mut start = 0;
    if lines
        .first()
        .is_some_and(|l| l.starts_with("## ") && l.to_lowercase().contains("spawn context"))
    {
        start = 1;
    }
    while start < lines.len() && lines[start].trim().is_empty() {
        start += 1;
    }
    let task = lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, l)| l.trim_end() == "### Task instructions")
        .map(|(i, _)| i);
    let mut end = match task {
        Some(heading) => {
            // The section runs to the next line-start `### ` heading, the
            // boundary the record's own writer and the desktop's reader use.
            let section_end = lines[heading + 1..]
                .iter()
                .position(|l| l.starts_with("### "))
                .map_or(lines.len(), |i| heading + 1 + i);
            section_end.min(heading + 1 + SPAWN_CONTEXT_TASK_MAX_LINES)
        }
        None => (start + SPAWN_CONTEXT_HEAD_LINES).min(lines.len()),
    };
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    (lines[start..end].to_vec(), lines.len() - end)
}

/// Draw a spawn-context record: a "Spawn context" label, the preview, and a
/// `⋯ N more lines` marker for the rest. `debug` draws all of it.
fn push_spawn_context(t: &mut App, text: &str, debug: bool) {
    let bar = Style::new().fg(ACCENT).add_modifier(Modifier::DIM);
    let row = |body: Span<'static>| Line::from(vec![Span::styled("▌ ", bar), body]);
    t.push_blank();
    t.push_line(row(Span::styled(
        "Spawn context",
        Style::new().add_modifier(Modifier::BOLD),
    )));
    let (shown, hidden) = if debug {
        (text.lines().collect(), 0)
    } else {
        spawn_context_preview(text)
    };
    for line in shown {
        t.push_line(row(Span::raw(line.to_string())));
    }
    if hidden > 0 {
        let noun = if hidden == 1 { "line" } else { "lines" };
        t.push_line(row(Span::styled(
            format!("⋯ {hidden} more {noun}"),
            DIM.add_modifier(Modifier::ITALIC),
        )));
    }
    t.push_blank();
}

/// Render one stored or live message of a child's chat: the spawn-context
/// record folded, the parent's instructions as a user turn, everything else
/// through `App::push_message`.
fn render_message(t: &mut App, message: &Message, debug: bool) {
    if !message.is_user_visible() {
        return;
    }
    if is_spawn_context(message) {
        let text = message
            .content
            .iter()
            .filter_map(MessageContent::as_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            push_spawn_context(t, &text, debug);
        }
        return;
    }
    if message.role == Role::User
        && message
            .content
            .iter()
            .all(|c| matches!(c, MessageContent::Text(_)))
    {
        let text = message
            .content
            .iter()
            .filter_map(MessageContent::as_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            t.push_user(&text);
        }
        return;
    }
    t.push_message(message, debug);
}

/// Wait for the open child's next bus event; never resolves without one, so
/// the idle loop's `select!` can always include it.
pub async fn next_child_event(child: Option<&mut ChildView>) -> Result<SessionBusEvent, RecvError> {
    match child.and_then(|c| c.subscription.as_mut()) {
        Some(sub) => sub.recv().await,
        None => std::future::pending().await,
    }
}

/// The child view's header: name, status and model, and how to leave.
pub fn draw_child(f: &mut Frame, child: &mut ChildView, row: Option<&PaneRow>, area: Rect) {
    let regions = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
    let width = regions[0].width as usize;
    let name = row.map(|r| r.label().to_string()).unwrap_or_else(|| {
        if child.name.is_empty() {
            child.session_id.clone()
        } else {
            child.name.clone()
        }
    });
    let mut spans = vec![
        Span::styled(
            "◂ Subagent ",
            Style::new()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    if let Some(row) = row {
        spans.push(Span::styled(
            status_glyph(row.status),
            status_style(row.status),
        ));
        spans.push(Span::raw(" "));
        if row.private {
            spans.push(Span::raw(format!("{LOCK} ")));
        }
    }
    spans.push(Span::styled(
        name,
        Style::new().add_modifier(Modifier::BOLD),
    ));
    let mut tail = Vec::new();
    if let Some(row) = row {
        tail.push(status_word(row.status).to_string());
        if let Some(secs) = row.elapsed_seconds {
            tail.push(format_elapsed(secs));
        }
    }
    let model = match row {
        Some(row) => model_label(row),
        None => short_model_label(child.model.as_deref(), None),
    };
    if let Some(model) = model {
        tail.push(model);
    }
    if !tail.is_empty() {
        spans.push(Span::styled(format!("  {}", tail.join(" · ")), DIM));
    }
    spans.push(Span::styled("  · esc returns to your chat", DIM));
    let header = vec![
        Line::from(clip_spans(spans, width)),
        Line::from(Span::styled("─".repeat(width), DIM)),
    ];
    f.render_widget(Paragraph::new(header), regions[0]);
    super::draw_history(f, &mut child.transcript, regions[1]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, status: SubagentRunStatus) -> PaneRow {
        PaneRow::new(id, name, status)
    }

    #[test]
    fn elapsed_and_tokens_read_like_the_desktop() {
        assert_eq!(format_elapsed(42), "42s");
        assert_eq!(format_elapsed(102), "1m42s");
        assert_eq!(format_elapsed(3900), "1h05m");
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(12_345), "12.3k");
        assert_eq!(format_tokens(1_234_567), "1.2M");
    }

    #[test]
    fn a_shared_roster_row_becomes_a_pane_row_with_its_lock() {
        let now = chrono::Utc::now();
        let shared = SubagentRow {
            session_id: "c1".into(),
            name: "gene survey".into(),
            role: Some("researcher".into()),
            phase_id: Some("research".into()),
            status: SubagentRunStatus::Running,
            loop_phase: Some("tool_batch".into()),
            quiet_seconds: Some(300),
            created_at: now,
            updated_at: now,
            ended_at: None,
            elapsed_seconds: Some(61),
            provider_name: Some("versa_azure".into()),
            model_name: Some("gpt-5.5-2026-04-24".into()),
            extensions: vec!["developer".into()],
            skills: vec![],
            knowledge_bases: vec!["soul".into()],
            privacy_tier: SessionClassification::Private,
            total_tokens: Some(1200),
            message_count: 4,
            tool_call_count: 2,
            result_line: None,
        };
        let model = PaneModel::from_snapshot(&SessionSubagents {
            parent_session_id: "p".into(),
            subagents: vec![shared],
            plan: None,
        });
        let r = &model.rows[0];
        assert!(r.private);
        assert!(r.is_quiet());
        assert_eq!(
            r.model.as_deref(),
            Some("gpt-5.5-2026-04-24"),
            "the row keeps the full id for its details"
        );
        assert_eq!(
            setup_summary(r),
            "gpt-5.5 · 1 ext · 1 knowledge base · 1m01s · quiet 5m",
            "the short model, and no count that is zero"
        );
        assert_eq!(r.label(), "researcher", "the role leads");
        assert_eq!(r.secondary_name(), Some("gene survey"));
        assert_eq!(model.tally().running, 1);
        assert_eq!(model.tally().quiet, 1);
    }

    #[test]
    fn groups_follow_the_plan_then_the_state_then_nothing() {
        let mut model = PaneModel {
            rows: vec![
                row("a", "a", SubagentRunStatus::Completed),
                row("b", "b", SubagentRunStatus::Running),
            ],
            plan: None,
        };
        assert_eq!(model.groups().len(), 1);
        assert!(model.groups()[0].head.is_none());

        model.rows.extend([
            row("c", "c", SubagentRunStatus::Failed),
            row("d", "d", SubagentRunStatus::Running),
        ]);
        let heads: Vec<_> = model.groups().into_iter().map(|g| g.head).collect();
        assert_eq!(
            heads,
            vec![
                Some(GroupHead::State(Bucket::Active)),
                Some(GroupHead::State(Bucket::Failed)),
                Some(GroupHead::State(Bucket::Done)),
            ]
        );
        assert_eq!(model.display_order(), vec![1, 3, 2, 0]);

        model.rows[0].phase_id = Some("plan".into());
        let mut research = PanePhase::new("research", PhaseKind::Research, PhaseStatus::InProgress);
        // A row with no phase id is placed by the plan's own agent list.
        research.agent_ids = vec!["b".into()];
        model.plan = Some(PanePlan {
            title: "Survey".into(),
            phases: vec![
                PanePhase::new("plan", PhaseKind::Plan, PhaseStatus::Completed),
                research,
            ],
        });
        let groups = model.groups();
        assert_eq!(groups[0].rows, vec![0]);
        assert_eq!(groups[1].rows, vec![1]);
        assert_eq!(groups[2].head, Some(GroupHead::Unphased));
        assert_eq!(groups[2].rows, vec![2, 3]);
    }

    /// A phase an earlier gate holds says what it waits for, by the holder's
    /// title, in place of "not started"; with rows it says so after its
    /// counts. A phase nothing holds reads as before.
    #[test]
    fn a_held_phase_says_which_gate_it_waits_for() {
        use biorouter::agents::orchestration_plan::{OrchestrationSnapshot, PhaseSnapshot};
        let phase =
            |id: &str, title: &str, status: PhaseStatus, held_by: Option<&str>| PhaseSnapshot {
                id: id.into(),
                title: title.into(),
                goal: None,
                kind: PhaseKind::Research,
                status,
                depends_on: Vec::new(),
                agent_ids: Vec::new(),
                counts: PhaseCounts::default(),
                handoff: None,
                started_at: None,
                finished_at: None,
                gate: None,
                held_by: held_by.map(str::to_string),
            };
        let model = PaneModel::from_snapshot(&SessionSubagents {
            parent_session_id: "p".into(),
            subagents: Vec::new(),
            plan: Some(OrchestrationSnapshot {
                title: "Moon".into(),
                goal: None,
                revision: 2,
                phases: vec![
                    phase("research", "Research", PhaseStatus::InProgress, None),
                    phase("write", "Write", PhaseStatus::Pending, Some("research")),
                    phase("review", "", PhaseStatus::Pending, Some("gone")),
                ],
            }),
        });
        let phases = &model.plan.as_ref().unwrap().phases;
        assert_eq!(phases[1].held_by.as_deref(), Some("Research"));
        assert_eq!(phases[2].held_by.as_deref(), Some("gone"));
        assert_eq!(
            phase_head_text(&phases[0], &PhaseCounts::default()),
            "◐ Research · starting"
        );
        assert_eq!(
            phase_head_text(&phases[1], &PhaseCounts::default()),
            "○ Write · waits for Research's gate"
        );
        let counts = PhaseCounts {
            total: 2,
            done: 1,
            running: 1,
            ..PhaseCounts::default()
        };
        assert_eq!(
            phase_head_text(&phases[1], &counts),
            "○ Write 1/2 · 1 running · waits for Research's gate"
        );
        let mut blocked = phases[1].clone();
        blocked.status = PhaseStatus::Blocked;
        assert_eq!(
            phase_head_text(&blocked, &PhaseCounts::default()),
            "⏸ Write · blocked · waits for Research's gate"
        );
    }

    /// E-9: a subagent the person stopped (it ends incomplete) is counted as
    /// stopped, never as a failure; E-8: the rows a plan does not place sit
    /// under "no phase", with no count run into the label.
    #[test]
    fn stopped_subagents_are_not_failures_and_no_phase_has_no_count() {
        let mut model = PaneModel {
            rows: vec![
                row("a", "a", SubagentRunStatus::Completed),
                row("b", "b", SubagentRunStatus::Incomplete),
                row("c", "c", SubagentRunStatus::Incomplete),
                row("d", "d", SubagentRunStatus::Failed),
            ],
            plan: None,
        };
        let tally = model.tally();
        assert_eq!((tally.stopped, tally.failed), (2, 1));
        assert_eq!(tally.text(), "running 0 · done 1/4 · failed 1 · stopped 2");
        model.plan = Some(PanePlan {
            title: "Plan".into(),
            phases: vec![PanePhase::new(
                "build",
                PhaseKind::Implementation,
                PhaseStatus::Completed,
            )],
        });
        let groups = model.groups();
        let unphased = groups
            .iter()
            .find(|g| g.head == Some(GroupHead::Unphased))
            .unwrap();
        let line = group_head_line(&model, unphased, 40).unwrap();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "· no phase");
    }

    /// E-7: a phase's heading marks its gate: passed, blocked (the person's
    /// to decide), not passed yet, or decided by the person.
    #[test]
    fn a_phase_heading_marks_its_gate() {
        use biorouter::agents::orchestration_plan::PhaseGateSnapshot;
        let mut gate: PhaseGateSnapshot = serde_json::from_value(serde_json::json!({
            "threshold": 0.6,
            "questionCount": 1,
            "attempts": 1,
            "maxAttempts": 3,
            "lastPassed": true,
            "lastMinP": 0.74,
            "blocked": false,
            "needsPerson": false,
        }))
        .expect("a gate snapshot");
        let mut build = PanePhase::new("build", PhaseKind::Implementation, PhaseStatus::Completed);
        build.title = "Build".into();
        build.gate = Some(PaneGate::of(&gate));
        let one = PhaseCounts {
            total: 1,
            done: 1,
            ..PhaseCounts::default()
        };
        assert_eq!(phase_head_text(&build, &one), "● Build 1/1 · gate passed");

        gate.last_passed = Some(false);
        gate.blocked = true;
        gate.needs_person = true;
        build.status = PhaseStatus::Blocked;
        build.gate = Some(PaneGate::of(&gate));
        assert_eq!(
            phase_head_text(&build, &one),
            "⏸ Build 1/1 · gate blocked · needs you"
        );
        gate.blocked = false;
        gate.needs_person = false;
        assert_eq!(PaneGate::of(&gate), PaneGate::NotPassed);
        gate.last_passed = None;
        assert_eq!(PaneGate::of(&gate), PaneGate::Waiting);
    }

    #[test]
    fn attach_resolves_numbers_ids_and_names() {
        let model = PaneModel {
            rows: vec![
                row("20261104_aa1", "research api", SubagentRunStatus::Running),
                row(
                    "20261104_bb2",
                    "research docs",
                    SubagentRunStatus::Completed,
                ),
                row("20261104_cc3", "writer", SubagentRunStatus::Completed),
            ],
            plan: None,
        };
        assert_eq!(model.resolve("2").unwrap(), "20261104_bb2");
        assert!(model
            .resolve("4")
            .unwrap_err()
            .contains("no subagent number 4"));
        assert!(model.resolve("0").is_err());
        assert_eq!(model.resolve("20261104_cc3").unwrap(), "20261104_cc3");
        assert_eq!(model.resolve("Writer").unwrap(), "20261104_cc3");
        assert_eq!(model.resolve("research a").unwrap(), "20261104_aa1");
        assert!(model.resolve("research").unwrap_err().contains("matches 2"));
        assert!(model
            .resolve("nothing")
            .unwrap_err()
            .contains("No subagent"));
        assert!(PaneModel::default().resolve("1").is_err());
    }

    #[test]
    fn set_model_reports_change_and_keeps_a_surviving_selection() {
        let mut pane = SubagentPane::default();
        let model = PaneModel {
            rows: vec![
                row("a", "a", SubagentRunStatus::Completed),
                row("b", "b", SubagentRunStatus::Running),
            ],
            plan: None,
        };
        assert!(pane.set_model(model.clone()));
        assert!(
            !pane.set_model(model.clone()),
            "an identical read is no change"
        );
        pane.focus();
        assert_eq!(pane.selected.as_deref(), Some("b"), "the active row first");
        pane.move_selection(-1);
        assert_eq!(pane.selected.as_deref(), Some("a"));
        let mut next = model;
        next.rows.remove(0);
        assert!(pane.set_model(next));
        assert_eq!(pane.selected.as_deref(), Some("b"));
    }

    /// A pane shown only because the chat had subagents hides itself when a
    /// read finds none (deleted elsewhere), and focus goes with it, so keys
    /// reach the input again. A pane the person pinned open stays, with focus.
    #[test]
    fn focus_leaves_a_pane_that_an_empty_roster_hides() {
        let one = PaneModel {
            rows: vec![row("a", "a", SubagentRunStatus::Running)],
            plan: None,
        };
        let mut pane = SubagentPane::default();
        pane.set_model(one.clone());
        pane.focus();
        assert!(pane.focused && pane.visible());
        assert_eq!(pane.toggle, None, "shown by default, not pinned");

        assert!(pane.set_model(PaneModel::default()));
        assert!(!pane.visible());
        assert!(!pane.focused, "focus cannot stay on a pane nothing draws");
        assert_eq!(pane.selected, None);

        let mut pinned = SubagentPane::default();
        pinned.set_model(one);
        pinned.toggle_visible();
        pinned.toggle_visible();
        assert_eq!(pinned.toggle, Some(true));
        pinned.focus();
        pinned.set_model(PaneModel::default());
        assert!(
            pinned.visible() && pinned.focused,
            "a pinned pane is still drawn"
        );
    }

    #[test]
    fn the_idle_loop_polls_fast_while_active_and_slowly_while_only_shown() {
        let mut pane = SubagentPane::default();
        assert!(!pane.wants_poll(false), "hidden and empty: nothing to read");
        assert!(pane.wants_poll(true), "an open child is followed");
        pane.set_model(PaneModel {
            rows: vec![row("a", "a", SubagentRunStatus::Completed)],
            plan: None,
        });
        let polls = (0..8).filter(|_| pane.wants_poll(false)).count();
        assert_eq!(polls, 8 / IDLE_POLL_EVERY as usize);
        pane.model.rows[0].status = SubagentRunStatus::Running;
        assert!(pane.wants_poll(false));
    }

    #[test]
    fn the_child_view_streams_live_and_rereads_when_a_turn_ends() {
        let mut child = ChildView::new("c1", true, false);
        child.load(&[Message::user().with_text("Survey the literature")]);
        child.apply(SessionBusEvent::Agent(AgentEvent::Message(
            Message::assistant()
                .with_id("r1")
                .with_text("Reading three"),
        )));
        child.apply(SessionBusEvent::Agent(AgentEvent::Message(
            Message::assistant().with_id("r1").with_text(" papers"),
        )));
        child.transcript.render_stream_if_dirty();
        let text: String = child
            .transcript
            .scrollback
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text.contains("Survey the literature"), "{text}");
        assert!(text.contains("Reading three papers"), "{text}");
        assert!(!child.needs_reload);
        child.apply(SessionBusEvent::TurnFinished {
            reason: "stop".into(),
            token_state: None,
        });
        assert!(child.needs_reload);
        assert!(
            child.transcript.stream_start.is_none(),
            "the stream committed"
        );
        child.load(&[]);
        assert!(!child.needs_reload);
    }
}
