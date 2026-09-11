//! The planning gate: native control of a multi-step turn's checklist.
//!
//! The Todo capability could always keep a checklist; nothing made the model
//! keep one. The only trigger was an advisory paragraph in `system.md`,
//! `TodoClient::get_moim` renders nothing while the list is empty, and the
//! hard-coded "don't stop with unchecked todos" gate was removed for
//! re-injecting a fake user message forever (the NOTE near the top of
//! `agent.rs`). Measured on 2026-09-10: three multi-step QA sessions, zero
//! checklists. This module is the harness half of the fix — three behaviours,
//! each bounded, each of which a model that disagrees can get past:
//!
//! 1. **Reminder.** A turn whose prompt [`classify_request`] reads as several
//!    steps, in a session whose checklist is empty, carries a "write the
//!    checklist first" line in its MOIM block until the list exists.
//! 2. **Redirect.** The first batch of non-Todo tool calls in such a turn is
//!    refused with a pointer back to `todo__todo_write` — ONCE per turn. A model
//!    that states a reason and repeats the call gets it run.
//! 3. **Stop check.** A turn that created or changed the checklist cannot end
//!    while items are unfinished, unless its final message names each one. It
//!    blocks at most [`STOP_HOOK_BLOCK_CAP`] times per turn and, unlike the
//!    Stop-hook counter, the count does NOT reset when tools run — so a model
//!    that keeps stopping cannot cycle until `max_turns`.
//!
//! **Scope** is one predicate, [`enforcement_applies`], read by both the turn
//! and the system prompt so the prompt can never promise what the turn does not
//! do: tool-running modes only (not Chat), not a subagent, not a coding-agent
//! provider (its tool calls run through the bridge, which never passes the
//! reply loop's gate), and only while `todo__todo_write` is on the model's
//! roster. Disable the Todo capability and none of it fires.
//!
//! Everything that runs inside the reply loop lives here, as `Agent` methods,
//! so the `reply_internal` generator only calls them — its `poll` frame sits a
//! few percent under the thread stack in debug builds, and every line kept out
//! of it is paid for two or three times over during delegation.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::PoisonError;

use once_cell::sync::Lazy;
use regex::Regex;
use rmcp::model::{Role, Tool};

use crate::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME;
use crate::agents::todo_extension::{is_todo_tool_name, TODO_WRITE_TOOL_NAME};
use crate::config::BioRouterMode;
use crate::conversation::message::ToolRequest;
use crate::conversation::Conversation;
use crate::hooks::STOP_HOOK_BLOCK_CAP;
use crate::session::extension_data::{TodoItem, TodoState, TodoStatus};
use crate::session::session_manager::SessionType;
use crate::session::Session;
use crate::tool_inspection::{InspectionAction, InspectionResult};

use super::Agent;

/// `InspectionResult::inspector_name` on the gate's refusals, so the denial
/// path hands the model the real reason instead of claiming the user declined.
pub(crate) const PLANNING_GATE_NAME: &str = "planning_gate";

/// The two Todo calls that put items on an empty checklist. A batch carrying
/// one is writing the plan in the same step as the work, so the gate lets the
/// whole batch through.
const CHECKLIST_SEEDING_TOOLS: [&str; 2] = ["todo__todo_write", "todo__todo_add"];

/// Sessions whose turn state one [`Agent`] keeps at once. An entry outlives its
/// turn only until the session's next turn replaces it; the bound is for a
/// daemon hosting many chats on one agent (`biorouter web`).
const MAX_TRACKED_SESSIONS: usize = 256;

// ---------------------------------------------------------------------------
// The classifier
// ---------------------------------------------------------------------------

/// Why a request reads as several steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiStep {
    /// A numbered (`1.`, `2)`, `(3)`, `Step 4:`) or bulleted list of actions.
    List { items: usize },
    /// Three or more instructions in a row, each opening with an action verb.
    Instructions { count: usize },
    /// Two or more instructions joined by an explicit sequencing word — "then",
    /// "after that", "finally", "steps".
    Sequenced { count: usize },
}

impl MultiStep {
    fn describe(self) -> String {
        match self {
            Self::List { items } => format!("a list of {items} steps"),
            Self::Instructions { count } => format!("{count} separate instructions"),
            Self::Sequenced { count } => format!("{count} instructions in sequence"),
        }
    }
}

/// Code is pasted data, never the request's own steps: a stack trace with
/// numbered frames is not a plan.
static FENCED_CODE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)```.*?(?:```|\z)").expect("valid regex"));

/// Inline code, replaced by a neutral word so `write `x.txt`` keeps its verb.
static INLINE_CODE: Lazy<Regex> = Lazy::new(|| Regex::new(r"`[^`\n]+`").expect("valid regex"));

/// A numbered-list marker: `1.` `2)` `(3)` or `Step 4:`, after the start of the
/// text, whitespace or a bracket, and before whitespace. The whitespace on both
/// sides is what keeps `3.12` and `v1.2` out.
static NUMBERED_MARKER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:^|[\s(\[])(?:step\s+)?\(?(\d{1,2})[.):]\s+").expect("valid regex")
});

/// A bulleted line.
static BULLET: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^[ \t]*[-*+•][ \t]+(\S[^\n]*)$").expect("valid regex"));

/// Where one instruction ends and the next may begin. `.` counts only before
/// whitespace, so `hello.txt` and `3.12` stay whole.
static CLAUSE_BREAK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)[.!?;:](?:\s+|$)|[,\n]|\b(?:and|then|after\s+that|afterwards?|finally|lastly|also|plus)\b",
    )
    .expect("valid regex")
});

/// An explicit statement that the work comes in order.
static SEQUENCE_CUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:then|after\s+that|afterwards?|finally|lastly|followed\s+by|once\s+(?:that|this|it)(?:'s|\s+is)?\s+(?:done|finished|complete)|steps?)\b",
    )
    .expect("valid regex")
});

/// A request for an explanation, not for work. "How do I create a venv and
/// then install the deps?" names two actions in sequence and wants neither
/// performed; the system prompt already says to answer those first.
static INFORMATIONAL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:how\s+(?:to|do|does|did|can|could|would|should|might|is|are)|what(?:'s|\s+is)\s+the\s+(?:best\s+)?way\s+to|explain\s+how|walk\s+me\s+through)\b",
    )
    .expect("valid regex")
});

/// Verbs that open an instruction to DO something — to files, data, code, a
/// service. Base forms only: an imperative uses them, a description ("it
/// validates each row") does not. Answer-shaped verbs (explain, describe,
/// summarize, tell, compare, show, translate) are left out on purpose: three
/// questions in a row are not three steps of work.
const ACTION_VERB_LIST: &str = "\
    add adjust aggregate align analyze analyse annotate append apply archive assemble \
    attach audit automate backup benchmark bisect build bump calculate cancel capture cd \
    change check checkout chmod clean clear clone close cluster collect combine commit \
    compile compress compute concatenate configure connect convert copy correct count \
    crawl create crop curl debug decompress decrypt deduplicate delete deploy detect \
    diff disable download draw drop dump duplicate edit email embed enable encode \
    encrypt erase estimate evaluate execute expand export extend extract fetch filter \
    find fit fix flatten fork format gather generate get grep group gunzip gzip hash \
    identify implement import increase index ingest initialize initialise insert inspect \
    install integrate join kill label launch lint link list load locate lookup make map \
    mark measure merge migrate mkdir modify monitor mount move mv normalize normalise \
    notify open optimize optimise organize organise package parse paste patch ping plot \
    populate post prepare preprocess print process profile prune publish pull push put \
    query rank read rebase rebuild record recompute redact redo reduce refactor refresh \
    regenerate register reindex reinstall release reload remove rename render reorder \
    reorganize repair replace replicate reproduce request rerun resample reset reshape \
    resize resolve restart restore restructure retrain retrieve retry revert review \
    rewrite rm rotate run sample save scaffold scale scan schedule scrape search seed \
    select send separate serialize set setup share shuffle sign simulate slice smooth \
    sort split stage standardize start stash stop store strip submit subset subtract sum \
    swap switch symlink sync tabulate tag tar test tidy tokenize touch trace track train \
    transfer transform transpose trigger trim truncate tune uncompress undo uninstall \
    unpack unzip update upgrade upload validate verify visualize visualise watch wget \
    wire wrap write zip";

static ACTION_VERBS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| ACTION_VERB_LIST.split_whitespace().collect());

/// Words that can precede an instruction's verb without changing it. Longest
/// first, so "i need you to" is stripped before "i need to" is tried.
const FILLERS: &[&[&str]] = &[
    &["i", "would", "like", "you", "to"],
    &["i'd", "like", "you", "to"],
    &["i", "want", "you", "to"],
    &["i", "need", "you", "to"],
    &["go", "ahead", "and"],
    &["make", "sure", "to"],
    &["make", "sure", "you"],
    &["i", "need", "to"],
    &["i", "want", "to"],
    &["we", "need", "to"],
    &["you", "need", "to"],
    &["after", "that"],
    &["can", "you"],
    &["could", "you"],
    &["would", "you"],
    &["will", "you"],
    &["you", "should"],
    &["let", "us"],
    &["try", "to"],
    &["please"],
    &["kindly"],
    &["then"],
    &["and"],
    &["also"],
    &["now"],
    &["just"],
    &["next"],
    &["finally"],
    &["lastly"],
    &["first"],
    &["firstly"],
    &["second"],
    &["secondly"],
    &["third"],
    &["thirdly"],
    &["afterwards"],
    &["afterward"],
    &["let's"],
    &["lets"],
    &["step"],
    &["so"],
];

/// Read the user's prompt and say whether it asks for several steps of work.
///
/// Deliberately a small, conservative heuristic — English cues only, and a
/// false negative is cheap (the system prompt still asks the model to plan)
/// where a false positive costs one redirected tool call. It fires on:
///
/// * a numbered or bulleted list whose items open with action verbs — or of
///   three or more items introduced by an instruction ("Build a page with:
///   1. a header 2. a footer 3. a nav bar");
/// * three or more instructions, each opening with an action verb;
/// * two or more instructions joined by a sequencing word ("then", "after
///   that", "finally", "steps").
///
/// A request for an explanation ("how do I …") never fires, and fenced or
/// inline code is ignored.
pub fn classify_request(text: &str) -> Option<MultiStep> {
    let prose = prose_only(text);
    if INFORMATIONAL.is_match(&prose) {
        return None;
    }
    if let Some(items) = listed_steps(&prose) {
        return Some(MultiStep::List { items });
    }
    let count = action_clause_count(&prose);
    if count >= 3 {
        Some(MultiStep::Instructions { count })
    } else if count >= 2 && SEQUENCE_CUE.is_match(&prose) {
        Some(MultiStep::Sequenced { count })
    } else {
        None
    }
}

fn prose_only(text: &str) -> String {
    let without_fences = FENCED_CODE.replace_all(text, "\n");
    INLINE_CODE.replace_all(&without_fences, "x").into_owned()
}

/// The longest list in `prose` that reads as steps, as its item count.
fn listed_steps(prose: &str) -> Option<usize> {
    [numbered_list(prose), bulleted_list(prose)]
        .into_iter()
        .flatten()
        .filter(|(intro, items)| list_is_steps(intro, items))
        .map(|(_, items)| items.len())
        .max()
}

fn list_is_steps(intro: &str, items: &[&str]) -> bool {
    if items.len() < 2 {
        return false;
    }
    let actions = items.iter().filter(|item| starts_with_action(item)).count();
    actions >= 2 || (items.len() >= 3 && intro_is_instruction(intro))
}

/// Does the line that introduces a list tell the model to do something?
fn intro_is_instruction(intro: &str) -> bool {
    let line = intro.trim_end().rsplit('\n').next().unwrap_or_default();
    action_clause_count(line) >= 1
}

/// The longest `1, 2, 3, …` run of numbered markers, as the text before it and
/// the items it numbers.
fn numbered_list(prose: &str) -> Option<(&str, Vec<&str>)> {
    // (marker start, item start) for each marker in the run being built.
    let mut best: Vec<(usize, usize)> = Vec::new();
    let mut run: Vec<(usize, usize)> = Vec::new();
    for captures in NUMBERED_MARKER.captures_iter(prose) {
        let (Some(whole), Some(number)) = (captures.get(0), captures.get(1)) else {
            continue;
        };
        let Ok(number) = number.as_str().parse::<usize>() else {
            continue;
        };
        if number == run.len() + 1 {
            run.push((whole.start(), whole.end()));
        } else if number == 1 {
            if run.len() > best.len() {
                best = std::mem::take(&mut run);
            } else {
                run.clear();
            }
            run.push((whole.start(), whole.end()));
        }
    }
    if run.len() > best.len() {
        best = run;
    }
    let &(list_start, _) = best.first().filter(|_| best.len() >= 2)?;
    // Every offset here is a regex match boundary or a `find` result, so a
    // char boundary; `get` only states that without an index that could panic.
    let items = best
        .iter()
        .enumerate()
        .map(|(index, &(_, item_start))| {
            let end = match best.get(index + 1) {
                Some(&(next_marker, _)) => next_marker,
                // The last item ends with its line: an inline list has one line,
                // and prose after a line list is not part of its last step.
                None => prose
                    .get(item_start..)
                    .and_then(|rest| rest.find('\n'))
                    .map_or(prose.len(), |offset| item_start + offset),
            };
            prose.get(item_start..end).unwrap_or_default().trim()
        })
        .collect();
    Some((prose.get(..list_start).unwrap_or_default(), items))
}

fn bulleted_list(prose: &str) -> Option<(&str, Vec<&str>)> {
    let mut first_start = None;
    let mut items = Vec::new();
    for captures in BULLET.captures_iter(prose) {
        let (Some(whole), Some(item)) = (captures.get(0), captures.get(1)) else {
            continue;
        };
        first_start.get_or_insert(whole.start());
        items.push(item.as_str().trim());
    }
    let start = first_start?;
    (items.len() >= 2).then(|| (prose.get(..start).unwrap_or_default(), items))
}

fn action_clause_count(prose: &str) -> usize {
    CLAUSE_BREAK
        .split(prose)
        .filter(|clause| starts_with_action(clause))
        .count()
}

fn starts_with_action(clause: &str) -> bool {
    let words = words(clause);
    strip_fillers(&words)
        .first()
        .is_some_and(|word| ACTION_VERBS.contains(word.as_str()))
}

/// Lower-cased words, keeping an inner apostrophe ("let's", "i'd").
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’'))
        .map(|word| {
            word.trim_matches(|c| c == '\'' || c == '’')
                .replace('’', "'")
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn strip_fillers(mut words: &[String]) -> &[String] {
    loop {
        // A list number or a stray count ("2 files") is not the verb.
        if words
            .first()
            .is_some_and(|word| word.chars().all(|c| c.is_ascii_digit()))
        {
            words = &words[1..];
            continue;
        }
        let Some(filler) = FILLERS.iter().find(|filler| {
            words.len() >= filler.len() && filler.iter().zip(words).all(|(a, b)| *a == b)
        }) else {
            return words;
        };
        words = &words[filler.len()..];
    }
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

/// Does the gate run for this conversation? The ONE answer, read by the turn
/// (`Agent::begin_planning_turn`) and by the system prompt
/// (`prepare_tools_and_prompt_for_provider`), so the prompt describes exactly
/// what the turn enforces.
///
/// * Chat mode runs no tools, so there is no tool call to redirect.
/// * A subagent's turn ends with an observe-only `SubagentStop`, never a
///   blockable Stop, so the stop check could not run there anyway; its task
///   was written by a parent model that keeps its own checklist.
/// * A coding-agent provider's tool calls run through the tool bridge, which
///   inspects them on its own path and never reaches the reply loop's gate.
/// * No `todo__todo_write` on the roster means nothing the gate could point at.
pub(crate) fn enforcement_applies<'a>(
    mode: BioRouterMode,
    is_subagent: bool,
    bridge_surface: bool,
    tool_names: impl IntoIterator<Item = &'a str>,
) -> bool {
    mode != BioRouterMode::Chat
        && !is_subagent
        && !bridge_surface
        && tool_names
            .into_iter()
            .any(|name| name == TODO_WRITE_TOOL_NAME)
}

// ---------------------------------------------------------------------------
// Turn state
// ---------------------------------------------------------------------------

/// One turn's gate state.
#[derive(Debug, Clone)]
struct TurnPlan {
    /// Why this turn's prompt reads as several steps, when it does.
    signal: Option<MultiStep>,
    /// The checklist had no items when the turn opened.
    list_was_empty: bool,
    /// [`fingerprint`] of the checklist when the turn opened.
    start_fingerprint: u64,
    /// The once-per-turn redirect has fired.
    redirect_spent: bool,
    /// The checklist has been seen with items since the turn opened.
    list_seen: bool,
    /// Checklist stop blocks this turn. Never reset by a tool call.
    stop_blocks: u32,
    /// Insertion order, for the bound.
    serial: u64,
}

impl TurnPlan {
    fn armed(&self) -> Option<MultiStep> {
        (self.list_was_empty && !self.list_seen && !self.redirect_spent)
            .then_some(self.signal)
            .flatten()
    }
}

#[derive(Debug, Default)]
struct Turns {
    plans: HashMap<String, TurnPlan>,
    serial: u64,
}

/// Per-session turn state for one [`Agent`].
///
/// ⚠ Owned by the agent, never process-global. Session ids are `YYYYMMDD_N`
/// per DATABASE, so two tests with their own stores routinely share one, and a
/// global keyed by it would let one test's turn arm another test's gate.
#[derive(Debug, Default)]
pub(crate) struct PlanningRegistry {
    turns: std::sync::Mutex<Turns>,
}

impl PlanningRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, Turns> {
        self.turns.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replace the session's state with `plan`, or clear it when the gate does
    /// not run this turn.
    fn begin(&self, session_id: &str, plan: Option<TurnPlan>) {
        let mut turns = self.lock();
        let Some(mut plan) = plan else {
            turns.plans.remove(session_id);
            return;
        };
        turns.serial += 1;
        plan.serial = turns.serial;
        turns.plans.insert(session_id.to_string(), plan);
        while turns.plans.len() > MAX_TRACKED_SESSIONS {
            let Some(oldest) = turns
                .plans
                .iter()
                .min_by_key(|(_, plan)| plan.serial)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            turns.plans.remove(&oldest);
        }
    }

    /// The turn's signal while the reminder and the redirect are live: a
    /// multi-step prompt, a list that was empty and has not been seen since,
    /// and the redirect not yet spent.
    fn armed(&self, session_id: &str) -> Option<MultiStep> {
        self.lock().plans.get(session_id).and_then(TurnPlan::armed)
    }

    fn note_list_exists(&self, session_id: &str) {
        if let Some(plan) = self.lock().plans.get_mut(session_id) {
            plan.list_seen = true;
        }
    }

    /// Spend the turn's one redirect. `None` when it is not armed — including
    /// when it already fired, which is the once-per-turn bound.
    fn spend_redirect(&self, session_id: &str) -> Option<MultiStep> {
        let mut turns = self.lock();
        let plan = turns.plans.get_mut(session_id)?;
        let signal = plan.armed()?;
        plan.redirect_spent = true;
        Some(signal)
    }

    /// `(start fingerprint, stop blocks so far)`, when the gate runs.
    fn stop_state(&self, session_id: &str) -> Option<(u64, u32)> {
        self.lock()
            .plans
            .get(session_id)
            .map(|plan| (plan.start_fingerprint, plan.stop_blocks))
    }

    fn record_stop_block(&self, session_id: &str) {
        if let Some(plan) = self.lock().plans.get_mut(session_id) {
            plan.stop_blocks += 1;
        }
    }
}

/// A stable fingerprint of a checklist — plan, ids, statuses and texts — so the
/// stop check can tell "this turn worked the list" from "the list is left over
/// from an earlier turn". An absent list and an empty one read the same.
fn fingerprint(state: Option<&TodoState>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state
        .map(TodoState::render)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

// ---------------------------------------------------------------------------
// Texts
// ---------------------------------------------------------------------------

/// The MOIM line a multi-step turn carries while its checklist is empty.
fn reminder_text(signal: MultiStep) -> String {
    format!(
        "Planning required: this request has {}, and your checklist is empty. Before you do \
         anything else, call `{TODO_WRITE_TOOL_NAME}` with one `- [ ]` item per step. Until \
         the checklist exists, the first other tool call you make this turn will be refused.",
        signal.describe()
    )
}

/// What a redirected call returns to the model.
fn redirect_reason(signal: MultiStep) -> String {
    format!(
        "Not run: this request has {}, and the checklist is still empty, so Biorouter needs \
         the plan first. Call `{TODO_WRITE_TOOL_NAME}` with one `- [ ]` item per step, then \
         make this call again. This refusal happens once per turn: if you have a reason not \
         to keep a checklist for this request, say so in your reply and repeat the call — it \
         will run.",
        signal.describe()
    )
}

fn status_label(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Pending => "not started",
        TodoStatus::InProgress => "in progress",
        TodoStatus::Blocked => "blocked",
        TodoStatus::Completed => "completed",
    }
}

fn id_list<'a>(items: impl IntoIterator<Item = &'a TodoItem>) -> String {
    const SHOWN: usize = 6;
    let ids: Vec<String> = items
        .into_iter()
        .map(|item| format!("#{}", item.id))
        .collect();
    if ids.len() > SHOWN {
        format!("{}, …", ids[..SHOWN].join(", "))
    } else {
        ids.join(", ")
    }
}

/// The notice for a turn that ends with the checklist open because the stop
/// check has spent its budget.
fn give_up_notice(open: usize) -> String {
    format!(
        "📋 The checklist still has {open} unfinished item(s) after {STOP_HOOK_BLOCK_CAP} \
         reminders; finishing anyway."
    )
}

// ---------------------------------------------------------------------------
// The redirect
// ---------------------------------------------------------------------------

/// Which calls in one batch the redirect refuses: every call except the Todo
/// tools and the workflow's structured-output tool — and none at all when the
/// batch itself seeds the checklist, because the plan then lands in the same
/// step as the work. A malformed call is left alone; it fails on its own.
fn redirect_targets(requests: &[ToolRequest]) -> Vec<String> {
    let calls: Vec<(&str, &str)> = requests
        .iter()
        .filter_map(|request| {
            let call = request.tool_call.as_ref().ok()?;
            Some((request.id.as_str(), call.name.as_ref()))
        })
        .collect();
    if calls
        .iter()
        .any(|(_, name)| CHECKLIST_SEEDING_TOOLS.contains(name))
    {
        return Vec::new();
    }
    calls
        .into_iter()
        .filter(|(_, name)| !is_todo_tool_name(name) && *name != FINAL_OUTPUT_TOOL_NAME)
        .map(|(id, _)| id.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// The stop check
// ---------------------------------------------------------------------------

/// What the stop check found wrong with ending the turn now.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChecklistObjection {
    /// For the model: every unfinished item, and the two ways out.
    feedback: String,
    /// For the user.
    notice: String,
    /// How many items are unfinished.
    open: usize,
}

/// The turn may not end while `state` has unfinished items, unless
/// `final_text` names every one of them — by `#N` id or by its text.
///
/// The feedback asks for a reason too, but only the naming is checked: a
/// deterministic check cannot tell a reason from a status recap, and a named
/// item is one the user can see was left open, which is the point.
fn checklist_objection(state: &TodoState, final_text: &str) -> Option<ChecklistObjection> {
    let open: Vec<&TodoItem> = state
        .items
        .iter()
        .filter(|item| item.status != TodoStatus::Completed)
        .collect();
    if open.is_empty() {
        return None;
    }
    let final_words = words(final_text);
    if open
        .iter()
        .all(|item| names_item(final_text, &final_words, item))
    {
        return None;
    }
    let listed = open
        .iter()
        .map(|item| {
            format!(
                "- #{} ({}) {}",
                item.id,
                status_label(item.status),
                item.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(ChecklistObjection {
        feedback: format!(
            "Before you finish: your checklist still has {} unfinished item(s):\n{listed}\n\
             Finish them, marking each one completed with `todo__todo_update` as you go. If an \
             item cannot or should not be done now, end your turn with a message that names \
             each unfinished item by its #N id and says why it is not done.",
            open.len()
        ),
        notice: format!(
            "📋 The checklist still has {} unfinished item(s) ({}); asking the agent to finish \
             them or say why not.",
            open.len(),
            id_list(open.iter().copied())
        ),
        open: open.len(),
    })
}

fn names_item(final_text: &str, final_words: &[String], item: &TodoItem) -> bool {
    mentions_id(final_text, &item.id) || contains_words(final_words, &words(&item.text))
}

/// `#3` names item 3; `#30` does not.
fn mentions_id(text: &str, id: &str) -> bool {
    let needle = format!("#{id}");
    text.match_indices(&needle).any(|(at, _)| {
        !text
            .get(at + needle.len()..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_ascii_digit())
    })
}

fn contains_words(haystack: &[String], needle: &[String]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// The text of the turn's final answer: every assistant message after the
/// last user-role message. A tool result and a steer are user-role here, so
/// this is exactly what the model said since it last heard anything.
fn final_reply_text(conversation: &Conversation) -> String {
    let messages = conversation.messages();
    let start = messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .map_or(0, |index| index + 1);
    messages[start..]
        .iter()
        .filter(|message| message.role == Role::Assistant)
        .map(|message| message.as_concat_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The checklist half of a turn's stop decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChecklistStop {
    /// Nothing to say: the gate does not run, the turn did not work the list,
    /// or every item is completed or named.
    Clear,
    /// Keep working: `feedback` to the model, `notice` to the user.
    Block { feedback: String, notice: String },
    /// Still open, but the per-turn budget is spent: finish, and say so.
    GiveUp { notice: String },
}

// ---------------------------------------------------------------------------
// The agent's side
// ---------------------------------------------------------------------------

impl Agent {
    /// Open this turn's gate state, once per reply, before the loop starts.
    ///
    /// `session` is the row `reply` read at the top of the turn, so its
    /// `extension_data` is the checklist as the turn found it.
    pub(super) fn begin_planning_turn(
        &self,
        session: &Session,
        signal: Option<MultiStep>,
        tools: &[Tool],
        toolshim_tools: &[Tool],
        bridge_surface: bool,
    ) {
        let enforced = enforcement_applies(
            self.config.biorouter_mode,
            session.session_type == SessionType::SubAgent,
            bridge_surface,
            // A toolshim turn hands the provider no tools and keeps them here.
            tools
                .iter()
                .chain(toolshim_tools)
                .map(|tool| tool.name.as_ref()),
        );
        if !enforced {
            self.planning.begin(&session.id, None);
            return;
        }
        // An unreadable blob (a newer build wrote it) counts as a list that
        // exists: the gate never nags about a checklist it cannot see.
        let (list_was_empty, start_fingerprint) = match TodoState::try_load(&session.extension_data)
        {
            Ok(state) => (
                state.as_ref().is_none_or(|state| state.items.is_empty()),
                fingerprint(state.as_ref()),
            ),
            Err(_) => (false, fingerprint(None)),
        };
        self.planning.begin(
            &session.id,
            Some(TurnPlan {
                signal,
                list_was_empty,
                start_fingerprint,
                redirect_spent: false,
                list_seen: false,
                stop_blocks: 0,
                serial: 0,
            }),
        );
    }

    /// The reminder for this provider call, while the turn is armed and the
    /// checklist is still empty.
    pub(super) async fn planning_reminder(&self, session_id: &str) -> Option<String> {
        let signal = self.planning.armed(session_id)?;
        if self.checklist_exists(session_id).await {
            self.planning.note_list_exists(session_id);
            return None;
        }
        Some(reminder_text(signal))
    }

    /// The once-per-turn redirect, as inspection results the permission merge
    /// turns into ordinary refusals. Empty unless the turn is armed, the batch
    /// has a call to refuse, and the checklist is still empty.
    pub(super) async fn planning_gate_denials(
        &self,
        session_id: &str,
        requests: &[ToolRequest],
    ) -> Vec<InspectionResult> {
        if self.planning.armed(session_id).is_none() {
            return Vec::new();
        }
        let targets = redirect_targets(requests);
        if targets.is_empty() {
            return Vec::new();
        }
        if self.checklist_exists(session_id).await {
            self.planning.note_list_exists(session_id);
            return Vec::new();
        }
        let Some(signal) = self.planning.spend_redirect(session_id) else {
            return Vec::new();
        };
        tracing::info!(
            session_id,
            refused = targets.len(),
            "planning gate: redirected the turn's first tool batch to todo_write"
        );
        let reason = redirect_reason(signal);
        targets
            .into_iter()
            .map(|tool_request_id| InspectionResult {
                tool_request_id,
                action: InspectionAction::Deny,
                reason: reason.clone(),
                confidence: 1.0,
                inspector_name: PLANNING_GATE_NAME.to_string(),
                finding_id: None,
            })
            .collect()
    }

    /// Whether the turn may end with the checklist as it stands.
    pub(super) async fn checklist_stop(
        &self,
        session_id: &str,
        conversation: &Conversation,
    ) -> ChecklistStop {
        let Some((start_fingerprint, blocks)) = self.planning.stop_state(session_id) else {
            return ChecklistStop::Clear;
        };
        // Disabled mid-turn: no tool left to finish the list with.
        if !self
            .extension_manager
            .is_extension_enabled(crate::agents::todo_extension::EXTENSION_NAME)
            .await
        {
            return ChecklistStop::Clear;
        }
        // Fail open on a read error or a blob this build cannot parse: a check
        // that cannot see the list does not keep a turn alive on its behalf.
        let Ok(session) = self
            .config
            .session_manager
            .get_session(session_id, false)
            .await
        else {
            return ChecklistStop::Clear;
        };
        let Ok(Some(state)) = TodoState::try_load(&session.extension_data) else {
            return ChecklistStop::Clear;
        };
        if fingerprint(Some(&state)) == start_fingerprint {
            // Not this turn's list: left over from an earlier turn and untouched.
            return ChecklistStop::Clear;
        }
        let Some(objection) = checklist_objection(&state, &final_reply_text(conversation)) else {
            return ChecklistStop::Clear;
        };
        if blocks >= STOP_HOOK_BLOCK_CAP {
            tracing::info!(
                session_id,
                open = objection.open,
                "planning gate: checklist still open at the stop-check cap; letting the turn end"
            );
            return ChecklistStop::GiveUp {
                notice: give_up_notice(objection.open),
            };
        }
        self.planning.record_stop_block(session_id);
        tracing::info!(
            session_id,
            open = objection.open,
            block = blocks + 1,
            "planning gate: sent the turn back to finish or name its open checklist items"
        );
        ChecklistStop::Block {
            feedback: objection.feedback,
            notice: objection.notice,
        }
    }

    /// Does the session's checklist have items right now? `true` on any error
    /// or an unreadable blob, so neither the reminder nor the redirect ever
    /// fires on a list it could not read.
    async fn checklist_exists(&self, session_id: &str) -> bool {
        match self
            .config
            .session_manager
            .get_session(session_id, false)
            .await
        {
            Ok(session) => match TodoState::try_load(&session.extension_data) {
                Ok(state) => state.is_some_and(|state| !state.items.is_empty()),
                Err(_) => true,
            },
            Err(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use rmcp::model::CallToolRequestParams;

    // -- classifier -------------------------------------------------------

    #[test]
    fn the_classifier_recognises_requests_that_have_several_steps() {
        let cases: &[(&str, MultiStep)] = &[
            (
                "1. create a temp dir 2. write hello.txt into it 3. count its bytes 4. delete it",
                MultiStep::List { items: 4 },
            ),
            (
                "1. Create a temp dir\n2. Write hello.txt into it\n3. Count its bytes\n4. Delete it",
                MultiStep::List { items: 4 },
            ),
            (
                "Please do the following:\n- clone the repo\n- build it\n- run the tests",
                MultiStep::List { items: 3 },
            ),
            (
                "Step 1: fetch the data. Step 2: plot it.",
                MultiStep::List { items: 2 },
            ),
            (
                "Build a landing page with: 1. a header 2. a pricing table 3. a footer",
                MultiStep::List { items: 3 },
            ),
            (
                "Create a temp dir, write hello.txt into it, count its bytes and delete it.",
                MultiStep::Instructions { count: 4 },
            ),
            (
                "Download the CSV, clean the missing values, and plot the distribution.",
                MultiStep::Instructions { count: 3 },
            ),
            (
                "Create the directory, then write hello.txt into it.",
                MultiStep::Sequenced { count: 2 },
            ),
            (
                "First download the dataset. After that, normalize the columns.",
                MultiStep::Sequenced { count: 2 },
            ),
            (
                "Can you run the tests and then fix any failures?",
                MultiStep::Sequenced { count: 2 },
            ),
            (
                "Write a script that downloads the data, then plot the results",
                MultiStep::Sequenced { count: 2 },
            ),
        ];
        for (prompt, expected) in cases {
            assert_eq!(
                classify_request(prompt),
                Some(*expected),
                "should read as several steps: {prompt:?}"
            );
        }
    }

    #[test]
    fn the_classifier_leaves_single_asks_questions_and_pasted_lists_alone() {
        let cases = [
            "",
            "   ",
            "What is the capital of France?",
            "Fix the typo in README.md",
            "Read README.md and tell me what this project does",
            "Explain the steps of glycolysis",
            "If the build fails then fix it",
            "Python 3.12 is out. Should I upgrade?",
            "Thanks, that worked!",
            "Which is better? 1. Postgres 2. SQLite",
            "Summarize this paper: 1. Introduction 2. Methods 3. Results",
            // Two instructions and no word that sequences them.
            "Create a new branch and commit these changes",
            // One task with requirements, described in the third person.
            "Please write a function that parses the file, validates each row, and saves it",
            // A how-to question names steps it does not want performed.
            "Can you tell me how to create a directory, write a file and then delete it in bash?",
            "How do I: 1. create a venv 2. install the deps 3. run the tests?",
            // Steps inside a code fence are pasted data.
            "Why does this fail?\n```\n1. open the file\n2. write the header\n3. close it\n```",
        ];
        for prompt in cases {
            assert_eq!(
                classify_request(prompt),
                None,
                "should not read as several steps: {prompt:?}"
            );
        }
    }

    // -- scope ------------------------------------------------------------

    #[test]
    fn the_gate_runs_only_where_it_can_do_what_it_says() {
        let roster = ["developer__shell", TODO_WRITE_TOOL_NAME];
        assert!(enforcement_applies(
            BioRouterMode::Auto,
            false,
            false,
            roster
        ));
        assert!(enforcement_applies(
            BioRouterMode::Approve,
            false,
            false,
            roster
        ));
        // No tool runs in Chat mode, so there is nothing to redirect.
        assert!(!enforcement_applies(
            BioRouterMode::Chat,
            false,
            false,
            roster
        ));
        assert!(!enforcement_applies(
            BioRouterMode::Auto,
            true,
            false,
            roster
        ));
        assert!(!enforcement_applies(
            BioRouterMode::Auto,
            false,
            true,
            roster
        ));
        // The capability is off, or its seeding tool is not granted.
        assert!(!enforcement_applies(
            BioRouterMode::Auto,
            false,
            false,
            ["developer__shell", "todo__todo_update"]
        ));
    }

    // -- the redirect -----------------------------------------------------

    fn request(id: &str, name: &str) -> ToolRequest {
        ToolRequest {
            id: id.to_string(),
            tool_call: Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: Some(serde_json::Map::new()),
            }),
            metadata: None,
            tool_meta: None,
        }
    }

    #[test]
    fn the_redirect_refuses_every_non_todo_call_unless_the_batch_seeds_the_list() {
        assert_eq!(
            redirect_targets(&[
                request("a", "developer__shell"),
                request("b", "fixture__step")
            ]),
            vec!["a".to_string(), "b".to_string()]
        );
        // The plan lands in the same step as the work: let the batch run.
        for seeding in CHECKLIST_SEEDING_TOOLS {
            assert!(
                redirect_targets(&[request("a", seeding), request("b", "developer__shell")])
                    .is_empty(),
                "{seeding} seeds the checklist"
            );
        }
        // A plan with no checklist does not satisfy the gate, and is not refused.
        assert_eq!(
            redirect_targets(&[
                request("a", "todo__plan_write"),
                request("b", "developer__shell")
            ]),
            vec!["b".to_string()]
        );
        assert!(redirect_targets(&[request("a", FINAL_OUTPUT_TOOL_NAME)]).is_empty());
        assert!(redirect_targets(&[request("a", "todo__todo_update")]).is_empty());
    }

    // -- turn state -------------------------------------------------------

    fn plan(signal: Option<MultiStep>, list_was_empty: bool) -> TurnPlan {
        TurnPlan {
            signal,
            list_was_empty,
            start_fingerprint: fingerprint(None),
            redirect_spent: false,
            list_seen: false,
            stop_blocks: 0,
            serial: 0,
        }
    }

    #[test]
    fn the_redirect_is_spent_once_and_the_list_disarms_the_gate() {
        let registry = PlanningRegistry::default();
        let steps = Some(MultiStep::List { items: 4 });

        registry.begin("s", Some(plan(steps, true)));
        assert_eq!(registry.armed("s"), steps);
        assert_eq!(registry.spend_redirect("s"), steps);
        assert_eq!(registry.spend_redirect("s"), None, "once per turn");
        assert_eq!(registry.armed("s"), None, "no reminder after the refusal");

        // A new turn re-arms it.
        registry.begin("s", Some(plan(steps, true)));
        assert_eq!(registry.armed("s"), steps);
        registry.note_list_exists("s");
        assert_eq!(registry.armed("s"), None);
        assert_eq!(registry.spend_redirect("s"), None);

        // A one-line turn, or a list that already existed, never arms.
        registry.begin("s", Some(plan(None, true)));
        assert_eq!(registry.armed("s"), None);
        registry.begin("s", Some(plan(steps, false)));
        assert_eq!(registry.armed("s"), None);
        // …but the stop check still has its baseline.
        assert!(registry.stop_state("s").is_some());

        // A turn the gate does not run for leaves nothing behind.
        registry.begin("s", None);
        assert_eq!(registry.stop_state("s"), None);
    }

    #[test]
    fn the_registry_is_bounded_and_evicts_the_oldest_session() {
        let registry = PlanningRegistry::default();
        for n in 0..(MAX_TRACKED_SESSIONS + 10) {
            registry.begin(&format!("s{n}"), Some(plan(None, true)));
        }
        let turns = registry.lock();
        assert_eq!(turns.plans.len(), MAX_TRACKED_SESSIONS);
        assert!(!turns.plans.contains_key("s0"));
        assert!(turns
            .plans
            .contains_key(&format!("s{}", MAX_TRACKED_SESSIONS + 9)));
    }

    // -- the stop check ---------------------------------------------------

    fn checklist(markdown: &str) -> TodoState {
        let mut state = TodoState::default();
        state.set_from_markdown(markdown);
        state
    }

    #[test]
    fn unfinished_items_block_the_stop_until_they_are_named() {
        let state = checklist(
            "- [x] create a temp dir\n- [x] write hello.txt into it\n\
             - [~] count its bytes\n- [ ] delete it",
        );

        let objection = checklist_objection(&state, "All done!").expect("two items are open");
        assert_eq!(objection.open, 2);
        assert!(objection
            .feedback
            .contains("#3 (in progress) count its bytes"));
        assert!(objection.feedback.contains("#4 (not started) delete it"));
        assert!(objection.feedback.contains("todo__todo_update"));
        assert!(objection.notice.contains("#3, #4"), "{}", objection.notice);

        // Naming only one of them is not enough.
        assert!(checklist_objection(&state, "#3 is still running.").is_some());
        // Every one, by id…
        assert!(checklist_objection(
            &state,
            "I stopped early: #3 needs the dir to exist and #4 would delete your data."
        )
        .is_none());
        // …or by its text.
        assert!(checklist_objection(
            &state,
            "I did not count its bytes, and I did not delete it: the disk is read-only."
        )
        .is_none());
        // `#30` is not `#3`.
        assert!(checklist_objection(&state, "#30 and #40 are open").is_some());
    }

    #[test]
    fn a_finished_or_empty_checklist_never_blocks_and_blocked_items_count_as_open() {
        assert!(checklist_objection(&checklist("- [x] one\n- [x] two"), "done").is_none());
        assert!(checklist_objection(&TodoState::default(), "done").is_none());
        let objection = checklist_objection(&checklist("- [x] one\n- [!] ask the user"), "done")
            .expect("a blocked item is unfinished");
        assert!(objection.feedback.contains("#2 (blocked) ask the user"));
    }

    #[test]
    fn the_fingerprint_moves_with_any_change_and_not_otherwise() {
        let before = checklist("- [ ] one\n- [ ] two");
        let mut after = before.clone();
        assert_eq!(fingerprint(Some(&before)), fingerprint(Some(&after)));
        after.update_item("2", Some(TodoStatus::Completed), None);
        assert_ne!(fingerprint(Some(&before)), fingerprint(Some(&after)));
        assert_eq!(fingerprint(None), fingerprint(Some(&TodoState::default())));
    }

    #[test]
    fn the_final_reply_is_what_the_model_said_after_it_last_heard_anything() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("do the thing"),
            Message::assistant().with_text("I'll start with #1"),
            Message::user().with_text("tool result"),
            Message::assistant().with_text("Finished #1."),
        ]);
        assert_eq!(final_reply_text(&conversation), "Finished #1.");
    }
}

/// The gate driven through the real reply loop: a scripted provider, the real
/// Todo capability, and an in-process fixture tool standing in for work.
#[cfg(test)]
mod agent_loop_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use futures::StreamExt;
    use rmcp::handler::server::router::tool::ToolRouter;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content, ServerCapabilities};
    use rmcp::{object, tool, tool_handler, tool_router};

    use crate::agents::extension::ExtensionConfig;
    use crate::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
    use crate::config::permission::PermissionManager;
    use crate::config::BioRouterMode;
    use crate::conversation::message::{Message, MessageContent};
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use crate::session::extension_data::{TodoState, TodoStatus};
    use crate::session::session_manager::SessionType;
    use crate::session::SessionManager;
    use rmcp::model::Tool;

    /// One scripted reply per provider call; records what each call was shown.
    struct ScriptedProvider {
        script: Vec<Message>,
        calls: AtomicUsize,
        seen: Mutex<Vec<(String, Vec<Message>)>>,
    }

    impl ScriptedProvider {
        fn new(script: Vec<Message>) -> Arc<Self> {
            Arc::new(Self {
                script,
                calls: AtomicUsize::new(0),
                seen: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        /// Every text the provider was shown on call `n` — the conversation,
        /// MOIM included — plus the tool results, flattened.
        fn shown(&self, n: usize) -> String {
            let seen = self.seen.lock().unwrap();
            seen[n]
                .1
                .iter()
                .flat_map(|message| message.content.iter())
                .map(|content| match content {
                    MessageContent::ToolResponse(response) => response
                        .tool_result
                        .as_ref()
                        .map(|result| {
                            result
                                .content
                                .iter()
                                .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default(),
                    other => other.as_text().map(str::to_string).unwrap_or_default(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        fn system_prompt(&self, n: usize) -> String {
            self.seen.lock().unwrap()[n].0.clone()
        }
    }

    #[async_trait::async_trait]
    impl Provider for ScriptedProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "scripted"
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("scripted-model")
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            system: &str,
            messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen
                .lock()
                .unwrap()
                .push((system.to_string(), messages.to_vec()));
            let reply = self
                .script
                .get(n)
                .cloned()
                .unwrap_or_else(|| Message::assistant().with_text("(script exhausted)"));
            Ok((
                reply,
                ProviderUsage::new(
                    "scripted-model".to_string(),
                    Usage::new(Some(10), Some(5), Some(15)),
                ),
            ))
        }
    }

    /// The work: one tool that counts how often it really ran.
    #[derive(Clone)]
    struct StepServer {
        tool_router: ToolRouter<Self>,
        runs: Arc<AtomicUsize>,
    }

    /// A step number, so a script that calls the tool many times does not trip
    /// the repetition guard — a different gate, with its own tests.
    #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
    struct StepArgs {
        #[serde(default)]
        n: u32,
    }

    #[tool_router(router = tool_router)]
    impl StepServer {
        fn new(runs: Arc<AtomicUsize>) -> Self {
            Self {
                tool_router: Self::tool_router(),
                runs,
            }
        }

        #[tool(description = "Do one step of the fixture task")]
        fn step(&self, args: Parameters<StepArgs>) -> Result<CallToolResult, rmcp::ErrorData> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(CallToolResult::success(vec![Content::text(format!(
                "step {} done",
                args.0.n
            ))]))
        }
    }

    #[tool_handler(router = self.tool_router)]
    impl rmcp::ServerHandler for StepServer {
        fn get_info(&self) -> rmcp::model::ServerInfo {
            rmcp::model::ServerInfo {
                capabilities: ServerCapabilities::builder().enable_tools().build(),
                ..Default::default()
            }
        }
    }

    fn call(id: &str, name: &str, arguments: serde_json::Value) -> Message {
        Message::assistant().with_tool_request(
            id,
            Ok(CallToolRequestParams {
                task: None,
                meta: None,
                name: name.to_string().into(),
                arguments: arguments.as_object().cloned(),
            }),
        )
    }

    struct Fixture {
        agent: Agent,
        session_id: String,
        runs: Arc<AtomicUsize>,
        _dirs: Vec<tempfile::TempDir>,
    }

    async fn fixture(todo: bool, provider: Arc<ScriptedProvider>) -> Fixture {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let permissions = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
        let agent = Agent::with_config(
            AgentConfig::new(
                Arc::clone(&session_manager),
                Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
                None,
                BioRouterMode::Auto,
            )
            .with_project_hooks(false),
        );
        if todo {
            agent
                .add_extension(ExtensionConfig::Platform {
                    name: "todo".into(),
                    description: "todo".into(),
                    bundled: Some(true),
                    available_tools: vec![],
                })
                .await
                .expect("enable the Todo capability");
        }
        let runs = Arc::new(AtomicUsize::new(0));
        agent
            .extension_manager
            .add_inprocess_server("fixture", StepServer::new(Arc::clone(&runs)))
            .await
            .expect("inject the fixture tool");
        let session = session_manager
            .create_session(
                work.path().to_path_buf(),
                "planning gate".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        agent
            .update_provider(provider, &session.id)
            .await
            .expect("bind the scripted provider");
        Fixture {
            agent,
            session_id: session.id,
            runs,
            _dirs: vec![data, work, permissions],
        }
    }

    /// Drive one user turn; return every message the stream yielded.
    async fn turn(fixture: &Fixture, prompt: &str) -> Vec<Message> {
        let stream = fixture
            .agent
            .reply(
                Message::user().with_text(prompt),
                SessionConfig {
                    id: fixture.session_id.clone(),
                    schedule_id: None,
                    max_turns: Some(20),
                    max_tool_calls: None,
                    budget: None,
                    retry_config: None,
                    reasoning_effort: None,
                },
                None,
            )
            .await
            .expect("the turn starts");
        tokio::pin!(stream);
        let mut yielded = Vec::new();
        while let Some(event) = stream.next().await {
            if let AgentEvent::Message(message) = event.expect("the turn runs") {
                yielded.push(message);
            }
        }
        yielded
    }

    fn notices(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                MessageContent::SystemNotification(notice) => Some(notice.msg.clone()),
                _ => None,
            })
            .collect()
    }

    async fn checklist(fixture: &Fixture) -> TodoState {
        let session = fixture
            .agent
            .config
            .session_manager
            .get_session(&fixture.session_id, false)
            .await
            .unwrap();
        TodoState::load(&session.extension_data).unwrap_or_default()
    }

    const FOUR_STEPS: &str =
        "1. create a temp dir 2. write hello.txt into it 3. count its bytes 4. delete it";

    #[tokio::test]
    async fn a_multi_step_turn_is_planned_redirected_worked_and_finished() {
        let provider = ScriptedProvider::new(vec![
            // 0: straight to work — refused once, pointed at the checklist.
            call("work-1", "fixture__step", serde_json::json!({"n": 1})),
            // 1: the plan.
            call(
                "plan",
                "todo__todo_write",
                serde_json::json!({"content": "- [ ] make the dir\n- [ ] write the file"}),
            ),
            // 2: the same work, which now runs.
            call("work-2", "fixture__step", serde_json::json!({"n": 1})),
            // 3: stops with the list open and nothing named — sent back.
            Message::assistant().with_text("All finished."),
            // 4: ticks the items, in one batch.
            call(
                "tick-1",
                "todo__todo_update",
                serde_json::json!({"id": "1", "status": "completed"}),
            )
            .with_tool_request(
                "tick-2",
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "todo__todo_update".into(),
                    arguments: Some(object!({"id": "2", "status": "completed"})),
                }),
            ),
            // 5: stops with every item completed.
            Message::assistant().with_text("Done: made the dir and wrote the file."),
        ]);
        let fixture = fixture(true, Arc::clone(&provider)).await;

        let yielded = turn(&fixture, FOUR_STEPS).await;

        assert_eq!(
            provider.calls(),
            6,
            "exactly the scripted turn, no more and no less"
        );
        // The prompt states what the gate enforces, because the gate runs.
        assert!(provider
            .system_prompt(0)
            .contains("Biorouter enforces the checklist"));
        // 1. The reminder rode the first call's context…
        assert!(
            provider.shown(0).contains("Planning required"),
            "{}",
            provider.shown(0)
        );
        // 2. …the first work call was refused with a pointer to todo_write…
        let after_redirect = provider.shown(1);
        assert!(
            after_redirect.contains("Not run: this request has a list of 4 steps"),
            "{after_redirect}"
        );
        assert!(after_redirect.contains("todo__todo_write"));
        // …and never ran; the repeat after the plan did, exactly once.
        assert_eq!(fixture.runs.load(Ordering::SeqCst), 1);
        // The reminder is gone once the list exists.
        assert!(!provider.shown(2).contains("Planning required"));
        // 3. The early stop was sent back with the open items named.
        let after_block = provider.shown(4);
        assert!(
            after_block.contains("your checklist still has 2 unfinished item(s)"),
            "{after_block}"
        );
        assert!(after_block.contains("#1 (not started) make the dir"));
        let shown_to_user = notices(&yielded);
        assert!(
            shown_to_user
                .iter()
                .any(|notice| notice.contains("📋") && notice.contains("#1, #2")),
            "{shown_to_user:?}"
        );
        // The list the chat summary reads: both items, both ticked.
        let state = checklist(&fixture).await;
        assert_eq!(state.items.len(), 2);
        assert!(state
            .items
            .iter()
            .all(|item| item.status == TodoStatus::Completed));
    }

    #[tokio::test]
    async fn a_one_line_turn_gets_no_reminder_no_redirect_and_no_list() {
        let provider = ScriptedProvider::new(vec![
            call("work", "fixture__step", serde_json::json!({"n": 1})),
            Message::assistant().with_text("Done."),
        ]);
        let fixture = fixture(true, Arc::clone(&provider)).await;

        turn(&fixture, "Run the fixture step.").await;

        assert_eq!(provider.calls(), 2);
        assert!(!provider.shown(0).contains("Planning required"));
        assert!(!provider.shown(1).contains("Not run:"));
        assert_eq!(fixture.runs.load(Ordering::SeqCst), 1, "the call ran");
        assert!(checklist(&fixture).await.items.is_empty());
    }

    #[tokio::test]
    async fn with_the_todo_capability_off_the_plain_behaviour_returns() {
        let provider = ScriptedProvider::new(vec![
            call("work", "fixture__step", serde_json::json!({"n": 1})),
            Message::assistant().with_text("All finished."),
        ]);
        let fixture = fixture(false, Arc::clone(&provider)).await;

        turn(&fixture, FOUR_STEPS).await;

        assert_eq!(provider.calls(), 2);
        assert!(!provider.shown(0).contains("Planning required"));
        assert!(!provider
            .system_prompt(0)
            .contains("Biorouter enforces the checklist"));
        assert_eq!(fixture.runs.load(Ordering::SeqCst), 1, "not redirected");
    }

    /// The stop check's two ways out: naming the open items ends the turn at
    /// once, and a model that never does is let go after the cap — which does
    /// not reset between blocks, because every block here is followed by tool
    /// calls that would reset the Stop-hook counter.
    #[tokio::test]
    async fn the_stop_check_accepts_named_items_and_gives_up_at_the_cap() {
        let seed = call(
            "plan",
            "todo__todo_write",
            serde_json::json!({"content": "- [ ] one\n- [ ] two"}),
        );

        // Named: the open items are explained, so the first stop ends the turn.
        let provider = ScriptedProvider::new(vec![
            seed.clone(),
            Message::assistant().with_text("I stopped: #1 and #2 need your credentials."),
        ]);
        let named = fixture(true, Arc::clone(&provider)).await;
        let yielded = turn(&named, FOUR_STEPS).await;
        assert_eq!(provider.calls(), 2);
        assert!(notices(&yielded)
            .iter()
            .all(|notice| !notice.contains("📋")));

        // Stubborn: works, stops unnamed, works, stops unnamed, …
        let mut script = vec![seed];
        for n in 0..(crate::hooks::STOP_HOOK_BLOCK_CAP + 1) {
            script.push(call(
                &format!("work-{n}"),
                "fixture__step",
                serde_json::json!({"n": n}),
            ));
            script.push(Message::assistant().with_text("All finished."));
        }
        let provider = ScriptedProvider::new(script);
        let stubborn = fixture(true, Arc::clone(&provider)).await;
        let yielded = turn(&stubborn, FOUR_STEPS).await;
        let cap = crate::hooks::STOP_HOOK_BLOCK_CAP as usize;
        // The seed, then (work, stop) for each block, then the last (work,
        // stop) that the cap lets through.
        assert_eq!(provider.calls(), 1 + 2 * (cap + 1));
        let shown = notices(&yielded);
        assert_eq!(
            shown
                .iter()
                .filter(|notice| notice.contains("asking the agent to finish"))
                .count(),
            cap,
            "{shown:?}"
        );
        assert!(
            shown
                .iter()
                .any(|notice| notice.contains("finishing anyway")),
            "{shown:?}"
        );
    }
}
