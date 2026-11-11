//! MOIM ("message of the moment"): the `<info-msg>` block of automatic context
//! (the clock, the working directory, the workspace map, the checklist and
//! every other platform extension's part) the model is sent with its requests.
//!
//! # Anchored blocks, and why
//!
//! The block used to be stripped and re-inserted at the tail of EVERY request.
//! So request n's input was never a prefix of request n+1's: the block sat
//! after request n's last tool result, and in n+1 that place held the next
//! assistant message instead. Measured on Versa (Azure OpenAI Responses), whose
//! gateway reuses a cached request only when an earlier request's WHOLE input
//! is a prefix of the new one: `cache_read` stayed at the 35,572 tokens of the
//! system prompt and tools on every request of every chat, and the conversation
//! was never cached. A provider with true prefix caching (Anthropic's
//! breakpoints, OpenAI's automatic cache, llama.cpp's KV reuse) lost less, the
//! last tool round of every request, but lost it on every request too.
//!
//! Now each block stays where it was first injected ([`MoimAnchors`], stored
//! with the chat under `moim_anchors.v1`, so a restart sends the same bytes),
//! and a new block is appended at the tail only when the context it carries
//! changed, holding only the parts that changed. When only new messages were
//! appended, request n's whole input is a byte-identical prefix of request
//! n+1's. A changed part still reaches the model at the tail of the request it
//! changed in.
//!
//! What changes how often:
//!   * The clock is stated to the minute, but refreshed only when a block is
//!     written anyway or [`CLOCK_REFRESH_SECS`] after it was last stated.
//!   * The workspace map changes whenever the agent writes a file, so it is
//!     refreshed on the clock's schedule only, never with every change.
//!   * Everything else (the checklist, an orchestration plan, the preview
//!     panel, the module list, the working directory) as soon as it changes.
//!   * The planning gate's reminder is never anchored: it is for the requests
//!     it is raised on, so it rides in a block of its own at their very end.
//!
//! Growth is bounded by consolidation: blocks whose every part a later block
//! restated are dropped, at the next stale-result elision batch (which breaks
//! the prefix anyway) or once they hold [`CONSOLIDATE_MIN_TOKENS`]. A history
//! rewrite (compaction, an edit, a rewind) that removes a block's anchor resets
//! the anchors, and the next request starts over with one full block.
//!
//! `BIOROUTER_MOIM_ANCHORED=false` restores the old block-at-the-tail.

use crate::agents::extension_manager::{ExtensionManager, MOIM_CLOSE_TAG, MOIM_OPEN_TAG};
use crate::context_budget::{estimate_tokens, max_moim_tokens, truncate_to_tokens};
use crate::conversation::message::Message;
use crate::conversation::{Conversation, SharedNormalizer};
use crate::session::extension_data::ExtensionState;
use crate::session::SessionManager;
use rmcp::model::Role;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use tokio_util::sync::CancellationToken;

// Test-only utility. Do not use in production code. No `test` directive due to call outside crate.
thread_local! {
    pub static SKIP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Kill switch for anchored blocks. `false` re-injects one full block at the
/// tail of every request, as before.
pub const ANCHORED_KEY: &str = "BIOROUTER_MOIM_ANCHORED";

/// How long a stated clock stands before a block is written for the clock
/// alone. Any block written for another reason states the current minute.
pub const CLOCK_REFRESH_SECS: i64 = 15 * 60;

/// Superseded blocks are dropped once they hold this many estimated tokens,
/// even with no elision batch to ride on. Dropping one breaks the prefix from
/// that block on, so it is done rarely and only for a real saving.
pub const CONSOLIDATE_MIN_TOKENS: usize = 20_000;

/// Section keys. A platform extension's part is [`extension_key`].
pub const CLOCK_KEY: &str = "clock";
pub const CWD_KEY: &str = "cwd";
pub const SCOPE_KEY: &str = "scope";
pub const WORKSPACE_MAP_KEY: &str = "workspace_map";

/// The key of platform extension `name`'s part.
pub fn extension_key(name: &str) -> String {
    format!("ext:{name}")
}

/// The first line of a block that carries only what changed.
pub const UPDATE_HEADER: &str =
    "Update to the automatic context above. Anything not listed here is unchanged.";

/// One keyed part of the automatic context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoimSection {
    pub key: String,
    pub text: String,
}

impl MoimSection {
    pub fn new(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            text: text.into(),
        }
    }
}

/// A one-line part, shown in the block's head (no blank line around it).
fn is_head_key(key: &str) -> bool {
    matches!(key, CLOCK_KEY | CWD_KEY | SCOPE_KEY)
}

/// A part refreshed only on the clock's schedule ([`CLOCK_REFRESH_SECS`]).
fn is_slow_key(key: &str) -> bool {
    matches!(key, CLOCK_KEY | WORKSPACE_MAP_KEY)
}

/// A part whose disappearance is not announced. The map is missing only when
/// its walk was skipped or ran out of time, and the earlier map is still the
/// better guess.
fn removal_is_silent(key: &str) -> bool {
    matches!(key, CLOCK_KEY | WORKSPACE_MAP_KEY)
}

/// What an announced removal calls the part.
fn removal_label(key: &str) -> String {
    match key {
        CWD_KEY => "working directory".to_string(),
        SCOPE_KEY => "Crew scope".to_string(),
        other => match other.strip_prefix("ext:") {
            Some(name) => format!("{name} context"),
            None => other.to_string(),
        },
    }
}

/// The body of a block: head lines first, each other part between blank lines,
/// then announced removals.
fn render_body(entries: &[(&str, Option<&str>)]) -> String {
    let mut body = String::new();
    for (key, text) in entries {
        if let (true, Some(text)) = (is_head_key(key), text) {
            body.push_str(text);
            body.push('\n');
        }
    }
    for (key, text) in entries {
        if let (false, Some(text)) = (is_head_key(key), text) {
            body.push('\n');
            body.push_str(text);
            body.push('\n');
        }
    }
    let removed: Vec<String> = entries
        .iter()
        .filter(|(_, text)| text.is_none())
        .map(|(key, _)| removal_label(key))
        .collect();
    if !removed.is_empty() {
        body.push('\n');
        body.push_str(&format!(
            "No longer current, ignore the earlier copy: {}.\n",
            removed.join(", ")
        ));
    }
    body
}

/// Every section in one block, as `ExtensionManager::collect_moim` returns it.
pub fn render_full_block(sections: &[MoimSection]) -> String {
    let entries: Vec<(&str, Option<&str>)> = sections
        .iter()
        .map(|s| (s.key.as_str(), Some(s.text.as_str())))
        .collect();
    format!("{MOIM_OPEN_TAG}\n{}{MOIM_CLOSE_TAG}", render_body(&entries))
}

/// True if `text` is (only) a MOIM `<info-msg>` block, ignoring surrounding
/// whitespace. `collect_moim` emits the whole block as one text content item,
/// so an exactly-matching item is an agent-injected MOIM, never user prose.
fn is_moim_block(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with(MOIM_OPEN_TAG) && trimmed.ends_with(MOIM_CLOSE_TAG)
}

/// BR-2: bound the size of the injected MOIM `<info-msg>` block. A Platform
/// extension's `get_moim` (e.g. a huge todo list) could otherwise balloon the
/// block that is re-injected every action. Head-truncate the body while keeping
/// the opening and closing tags intact so the result still parses as a MOIM
/// block (`is_moim_block`) and merges correctly. `max_tokens == 0` disables it.
fn cap_moim_block(moim: String, max_tokens: usize) -> String {
    if max_tokens == 0 || estimate_tokens(&moim) <= max_tokens {
        return moim;
    }
    // Preserve the closing tag; truncate_to_tokens keeps the leading `<info-msg>`.
    let body = moim
        .strip_suffix(MOIM_CLOSE_TAG)
        .unwrap_or(&moim)
        .trim_end();
    let reserve = estimate_tokens(MOIM_CLOSE_TAG) + 1;
    let head_budget = max_tokens.saturating_sub(reserve).max(1);
    let truncated = truncate_to_tokens(body, head_budget, "moim");
    format!("{truncated}\n{MOIM_CLOSE_TAG}")
}

/// Remove any previously injected MOIM before inserting a fresh one, so exactly
/// one `<info-msg>` block is ever present. Only drops a *user* message whose
/// content is nothing but MOIM block(s) — i.e. an agent-only injection. A MOIM
/// that `fix_conversation` merged alongside real user text lives in its own
/// content item next to the user's prose, so that turn has a non-MOIM item and
/// is left untouched (never strip a user's actual turn).
fn strip_existing_moim(messages: &mut Vec<Message>) {
    messages.retain(|message| {
        if message.role != Role::User {
            return true;
        }
        let all_moim = !message.content.is_empty()
            && message
                .content
                .iter()
                .all(|c| c.as_text().is_some_and(is_moim_block));
        !all_moim
    });
}

/// The first line of every injected block: what the block is, said to the model.
///
/// After a tool result the block is the newest user turn the model sees, and a
/// bare `<info-msg>` there reads as something to answer. Measured 2026-10-05 on
/// Versa `gpt-5.6-luna-2026-07-09`, replaying one captured agent turn (a shell
/// call whose output the user asked for): without this line 1 of 12 replies gave
/// the output, the rest were empty or "Understood."; with it, 35 of 36 did.
/// gpt-5.6-sol, gpt-5.6-terra, gpt-5.5 and gpt-5.4-mini answered 12 of 12 either
/// way. It is plain text inside the user-role block, so it grants the block no
/// authority it did not already have.
const MOIM_PREAMBLE: &str = "Automatic context from Biorouter, not a message from the user. \
                             Do not reply to it; continue the user's task.";

/// What normalizing the conversation with the block in it may report without
/// the block being dropped: the merges the block's own user row causes, and
/// two cleanups of rows the conversation already holds (trailing whitespace
/// trimmed from an assistant reply, and a row that is empty, or left empty by
/// that trim, removed). A request sent without the block is normalized the
/// same way, so neither says anything about where the block went.
///
/// The cleanups used to drop the block. A reply cut off by a retried server
/// error is kept, and it often ends in whitespace (`"The quick brown fox "`),
/// so the retry was sent without it (live, against a scripted llama-server).
/// The same went for every later request in a chat whose stored history has
/// an assistant reply ending in a newline. Anything else, such as a tool
/// request or response the block's position orphaned, still drops it.
const EXPECTED_ISSUES: [&str; 4] = [
    "Merged consecutive user messages",
    "Merged consecutive assistant messages",
    "Trimmed trailing whitespace from assistant message",
    "Removed empty message",
];

/// Put [`MOIM_PREAMBLE`] first in the block, directly after the opening tag and
/// ahead of any reminder, so the size cap (which keeps the head) never removes it.
fn with_preamble(moim: String) -> String {
    moim.replacen(
        MOIM_OPEN_TAG,
        &format!("{MOIM_OPEN_TAG}\n{MOIM_PREAMBLE}"),
        1,
    )
}

/// Put `reminder` at the head of the block, directly after the opening tag.
///
/// The head, not the tail, because [`cap_moim_block`] keeps the head: a
/// reminder appended after a large workspace map would be the first thing the
/// cap cut.
fn with_reminder(moim: String, reminder: Option<&str>) -> String {
    match reminder.map(str::trim).filter(|text| !text.is_empty()) {
        Some(reminder) => moim.replacen(MOIM_OPEN_TAG, &format!("{MOIM_OPEN_TAG}\n{reminder}"), 1),
        None => moim,
    }
}

/// Normalize `messages` (with the block(s) in them) and keep the result only
/// when the block caused nothing beyond [`EXPECTED_ISSUES`].
fn normalize_with_blocks(
    normalizer: &SharedNormalizer,
    messages: Vec<Message>,
) -> Option<Conversation> {
    let (fixed, issues) = normalizer.normalize(Conversation::new_unvalidated(messages));
    let has_unexpected_issues = issues.iter().any(|issue| {
        !EXPECTED_ISSUES
            .iter()
            .any(|expected| issue.contains(expected))
    });
    if has_unexpected_issues {
        tracing::warn!("MOIM injection caused unexpected issues: {:?}", issues);
        return None;
    }
    Some(fixed)
}

/// Where a fresh block goes: after the last assistant message AND any
/// trailing tool responses that follow it. Inserting between an assistant
/// tool_call and its tool_result would put a user message there, which the
/// OpenAI API rejects with a 400 error.
fn tail_index(messages: &[Message]) -> usize {
    let after_last_assistant = messages
        .iter()
        .rposition(|m| m.role == Role::Assistant)
        .map(|i| i + 1)
        .unwrap_or(0);
    messages[after_last_assistant..]
        .iter()
        .position(|m| !m.is_tool_response())
        .map(|offset| after_last_assistant + offset)
        .unwrap_or(messages.len())
}

/// Inject the MOIM `<info-msg>` block into the conversation handed to the model,
/// returning the (re-normalized) conversation and whether a block was injected.
///
/// This is the stateless form: one full block, at the tail, replacing any
/// block already there. The agent loop uses [`inject_anchored_moim`], which
/// falls back to this when a chat's anchors cannot be read or used.
///
/// `reminder` is a first-party line the agent loop wants in front of the model
/// — the planning gate's "write the checklist first". It rides inside the
/// block, so it is never stored in the transcript.
///
/// BR-56: normalization goes through the agent's [`SharedNormalizer`], which
/// re-fixes only the messages appended since the last call instead of the whole
/// history — this runs on every provider call, so in a long multi-tool turn it was
/// the single most repeated O(history) pass in the loop.
pub async fn inject_moim(
    session_id: &str,
    conversation: Conversation,
    extension_manager: &ExtensionManager,
    working_dir: &Path,
    normalizer: &SharedNormalizer,
    cancel: Option<&CancellationToken>,
    reminder: Option<&str>,
) -> (Conversation, bool) {
    if SKIP.with(|f| f.get()) {
        return (conversation, false);
    }
    match extension_manager
        .collect_moim(session_id, working_dir, cancel)
        .await
    {
        Some(moim) => inject_at_tail(conversation, moim, normalizer, reminder),
        None => (conversation, false),
    }
}

fn inject_at_tail(
    conversation: Conversation,
    moim: String,
    normalizer: &SharedNormalizer,
    reminder: Option<&str>,
) -> (Conversation, bool) {
    let moim = cap_moim_block(
        with_preamble(with_reminder(moim, reminder)),
        max_moim_tokens(),
    );
    let mut messages = conversation.messages().clone();
    // Drop any stale MOIM from a prior loop iteration first, so a long
    // multi-tool turn never accumulates several near-identical (and
    // differently-timestamped) `<info-msg>` blocks.
    strip_existing_moim(&mut messages);
    let idx = tail_index(&messages);
    messages.insert(idx, Message::user().with_text(moim));
    match normalize_with_blocks(normalizer, messages) {
        Some(fixed) => (fixed, true),
        None => (conversation, false),
    }
}

// ---------------------------------------------------------------------------
// Anchored blocks
// ---------------------------------------------------------------------------

/// Where a block sits: right after one message, or right before one. Both by
/// stored message id, so the place survives a restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoimAnchor {
    After(String),
    Before(String),
}

impl MoimAnchor {
    fn id(&self) -> &str {
        match self {
            Self::After(id) | Self::Before(id) => id,
        }
    }
}

/// One block as it was injected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchoredMoim {
    pub anchor: MoimAnchor,
    /// The exact text inserted, so every later request sends the same bytes.
    pub text: String,
    /// What the block states: a part's text, or `None` for a part it says is
    /// no longer current.
    pub states: BTreeMap<String, Option<String>>,
    /// When it was written (Unix seconds), which is the clock it states when
    /// it states one.
    pub written_at: i64,
}

/// A chat's injected blocks, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoimAnchors {
    #[serde(default)]
    pub blocks: Vec<AnchoredMoim>,
}

impl ExtensionState for MoimAnchors {
    const EXTENSION_NAME: &'static str = "moim_anchors";
    const VERSION: &'static str = "v1";
}

impl MoimAnchors {
    /// What the model has been told, part by part: the newest statement of
    /// each.
    fn effective(&self) -> BTreeMap<&str, &str> {
        let mut effective = BTreeMap::new();
        for block in &self.blocks {
            for (key, text) in &block.states {
                match text {
                    Some(text) => effective.insert(key.as_str(), text.as_str()),
                    None => effective.remove(key.as_str()),
                };
            }
        }
        effective
    }

    /// When the clock was last stated.
    fn clock_written_at(&self) -> Option<i64> {
        self.blocks
            .iter()
            .rev()
            .find(|block| block.states.contains_key(CLOCK_KEY))
            .map(|block| block.written_at)
    }

    /// Indexes of blocks a later block restates entirely.
    fn superseded(&self) -> Vec<usize> {
        let mut later: BTreeSet<&str> = BTreeSet::new();
        let mut out = Vec::new();
        for (index, block) in self.blocks.iter().enumerate().rev() {
            if block.states.keys().all(|key| later.contains(key.as_str())) {
                out.push(index);
            }
            later.extend(block.states.keys().map(String::as_str));
        }
        out.reverse();
        out
    }

    fn superseded_tokens(&self) -> usize {
        self.superseded()
            .into_iter()
            .map(|index| estimate_tokens(&self.blocks[index].text))
            .sum()
    }

    /// Drop every block a later one restates entirely.
    fn consolidate(&mut self) -> bool {
        let drop: BTreeSet<usize> = self.superseded().into_iter().collect();
        if drop.is_empty() {
            return false;
        }
        let mut index = 0;
        self.blocks.retain(|_| {
            let keep = !drop.contains(&index);
            index += 1;
            keep
        });
        true
    }
}

/// The anchor for a block injected now: after the message before the tail
/// position, or, at the very start, before the first message. Only an
/// agent-visible message with an id can anchor; `None` when neither can.
fn choose_anchor(messages: &[Message]) -> Option<MoimAnchor> {
    let idx = tail_index(messages);
    let anchorable = |m: &Message| m.metadata.agent_visible && m.id.is_some();
    if idx > 0 && anchorable(&messages[idx - 1]) {
        return messages[idx - 1].id.clone().map(MoimAnchor::After);
    }
    match messages.get(idx) {
        Some(next) if anchorable(next) => next.id.clone().map(MoimAnchor::Before),
        _ => None,
    }
}

/// What [`plan_blocks`] decided: insert `state`'s blocks, write `state` back
/// when `changed`, and put `unanchored` at the tail.
#[derive(Debug, PartialEq, Eq)]
struct BlockPlan {
    state: MoimAnchors,
    changed: bool,
    /// A block that is due but that nothing can anchor (the message it would
    /// follow has no stored id yet). It goes at the tail of this request only
    /// and is not stored, so the next request, which still sees the change,
    /// writes it again where it can be anchored.
    unanchored: Option<String>,
}

/// The parts an update block carries: every section whose text differs from
/// what the anchored blocks already show (slow parts such as the clock only
/// when due, or when a block is written anyway), then announced removals.
fn update_entries(
    state: &MoimAnchors,
    fresh: &[MoimSection],
    now: i64,
) -> Vec<(String, Option<String>)> {
    let effective = state.effective();
    let clock_due = state
        .clock_written_at()
        .is_none_or(|at| now.saturating_sub(at) >= CLOCK_REFRESH_SECS);
    let mut entries = Vec::new();
    let mut slow = Vec::new();
    for section in fresh {
        let shown = effective.get(section.key.as_str()).copied();
        if shown == Some(section.text.as_str()) {
            continue;
        }
        let entry = (section.key.clone(), Some(section.text.clone()));
        if !is_slow_key(&section.key) || shown.is_none() || clock_due {
            entries.push(entry);
        } else if section.key == CLOCK_KEY {
            // Refreshed for free when a block is written anyway.
            slow.push(entry);
        }
    }
    let fresh_keys: BTreeSet<&str> = fresh.iter().map(|s| s.key.as_str()).collect();
    for key in effective.keys() {
        if fresh_keys.contains(key) {
            continue;
        }
        if !removal_is_silent(key) {
            entries.push((key.to_string(), None));
        }
    }
    if entries.iter().any(|(key, _)| key != CLOCK_KEY) {
        entries.extend(slow);
    }
    entries
}

/// Decide the blocks for this request. Pure, so the whole policy is tested on
/// its own: `fresh` is the context as it is now, `now` the time in Unix
/// seconds, and `consolidate` says the prefix is being broken anyway (an
/// elision batch ran for this request).
fn plan_blocks(
    mut state: MoimAnchors,
    messages: &[Message],
    fresh: &[MoimSection],
    now: i64,
    consolidate: bool,
    max_tokens: usize,
) -> BlockPlan {
    let mut changed = false;

    // A history rewrite took a block's anchor away (a compaction, an edit, a
    // rewind): its prefix is gone anyway, so start over with one full block.
    let ids: BTreeSet<&str> = messages
        .iter()
        .filter(|m| m.metadata.agent_visible)
        .filter_map(|m| m.id.as_deref())
        .collect();
    if state
        .blocks
        .iter()
        .any(|block| !ids.contains(block.anchor.id()))
    {
        state = MoimAnchors::default();
        changed = true;
    }

    if (consolidate || state.superseded_tokens() >= CONSOLIDATE_MIN_TOKENS) && state.consolidate() {
        changed = true;
    }

    // (key, Some(text) | None for an announced removal), in `fresh`'s order.
    let full = state.blocks.is_empty();
    let mut entries: Vec<(String, Option<String>)> = if full {
        fresh
            .iter()
            .map(|s| (s.key.clone(), Some(s.text.clone())))
            .collect()
    } else {
        update_entries(&state, fresh, now)
    };
    if entries.is_empty() {
        return BlockPlan {
            state,
            changed,
            unanchored: None,
        };
    }

    // Show the parts in the order `fresh` lists them, removals last.
    let order: HashMap<&str, usize> = fresh
        .iter()
        .enumerate()
        .map(|(i, s)| (s.key.as_str(), i))
        .collect();
    entries.sort_by_key(|(key, text)| {
        (
            text.is_none(),
            order.get(key.as_str()).copied().unwrap_or(usize::MAX),
        )
    });
    let body_entries: Vec<(&str, Option<&str>)> = entries
        .iter()
        .map(|(key, text)| (key.as_str(), text.as_deref()))
        .collect();
    let header = if full {
        String::new()
    } else {
        format!("{UPDATE_HEADER}\n")
    };
    let core = format!(
        "{MOIM_OPEN_TAG}\n{header}{}{MOIM_CLOSE_TAG}",
        render_body(&body_entries)
    );
    let text = cap_moim_block(with_preamble(core), max_tokens);
    let Some(anchor) = choose_anchor(messages) else {
        return BlockPlan {
            state,
            changed,
            unanchored: Some(text),
        };
    };
    state.blocks.push(AnchoredMoim {
        anchor,
        text,
        states: entries.into_iter().collect(),
        written_at: now,
    });
    BlockPlan {
        state,
        changed: true,
        unanchored: None,
    }
}

/// Put every block at its anchor. A block anchored after a message also goes
/// after any tool responses that directly follow it, so it can never separate
/// a call from its result; blocks sharing a place keep their order.
fn insert_blocks(messages: Vec<Message>, blocks: &[AnchoredMoim]) -> Vec<Message> {
    let position: HashMap<&str, usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| m.id.as_deref().map(|id| (id, i)))
        .collect();
    let mut placed: Vec<(usize, &AnchoredMoim)> = blocks
        .iter()
        .filter_map(|block| {
            let at = *position.get(block.anchor.id())?;
            let gap = match block.anchor {
                MoimAnchor::Before(_) => at,
                MoimAnchor::After(_) => {
                    let mut gap = at + 1;
                    while messages.get(gap).is_some_and(Message::is_tool_response) {
                        gap += 1;
                    }
                    gap
                }
            };
            Some((gap, block))
        })
        .collect();
    placed.sort_by_key(|(gap, _)| *gap);
    let mut out = Vec::with_capacity(messages.len() + placed.len());
    let mut pending = placed.into_iter().peekable();
    for (index, message) in messages.into_iter().enumerate() {
        while let Some((_, block)) = pending.next_if(|(gap, _)| *gap == index) {
            out.push(Message::user().with_text(block.text.clone()));
        }
        out.push(message);
    }
    for (_, block) in pending {
        out.push(Message::user().with_text(block.text.clone()));
    }
    out
}

fn anchoring_enabled() -> bool {
    crate::config::Config::global()
        .get_param::<bool>(ANCHORED_KEY)
        .unwrap_or(true)
}

/// [`inject_moim`] with anchored blocks (see the module docs): the agent
/// loop's form. `elision_batch_ran` says this request already breaks the
/// prefix, so superseded blocks are dropped with it.
#[allow(clippy::too_many_arguments)]
pub async fn inject_anchored_moim(
    session_id: &str,
    conversation: Conversation,
    extension_manager: &ExtensionManager,
    working_dir: &Path,
    normalizer: &SharedNormalizer,
    cancel: Option<&CancellationToken>,
    reminder: Option<&str>,
    session_manager: &SessionManager,
    elision_batch_ran: bool,
) -> (Conversation, bool) {
    inject_anchored_moim_at(
        session_id,
        conversation,
        extension_manager,
        working_dir,
        normalizer,
        cancel,
        reminder,
        session_manager,
        elision_batch_ran,
        chrono::Local::now(),
    )
    .await
}

/// [`inject_anchored_moim`] at a given time, so a test can let time pass.
#[allow(clippy::too_many_arguments)]
pub async fn inject_anchored_moim_at(
    session_id: &str,
    conversation: Conversation,
    extension_manager: &ExtensionManager,
    working_dir: &Path,
    normalizer: &SharedNormalizer,
    cancel: Option<&CancellationToken>,
    reminder: Option<&str>,
    session_manager: &SessionManager,
    elision_batch_ran: bool,
    now: chrono::DateTime<chrono::Local>,
) -> (Conversation, bool) {
    if SKIP.with(|f| f.get()) {
        return (conversation, false);
    }
    let Some(sections) = extension_manager
        .collect_moim_sections_at(session_id, working_dir, cancel, now)
        .await
    else {
        return (conversation, false);
    };
    let tail = |conversation| {
        inject_at_tail(
            conversation,
            render_full_block(&sections),
            normalizer,
            reminder,
        )
    };
    if !anchoring_enabled() {
        return tail(conversation);
    }
    let state = match session_manager
        .get_extension_state(
            session_id,
            MoimAnchors::EXTENSION_NAME,
            MoimAnchors::VERSION,
        )
        .await
    {
        Ok(Some(value)) => MoimAnchors::from_value(&value).unwrap_or_else(|e| {
            tracing::debug!("MOIM anchors unreadable ({e}); starting over");
            MoimAnchors::default()
        }),
        Ok(None) => MoimAnchors::default(),
        Err(e) => {
            tracing::debug!("could not read MOIM anchors ({e}); injecting at the tail");
            return tail(conversation);
        }
    };

    let mut messages = conversation.messages().clone();
    strip_existing_moim(&mut messages);
    let BlockPlan {
        state,
        changed,
        unanchored,
    } = plan_blocks(
        state,
        &messages,
        &sections,
        now.timestamp(),
        elision_batch_ran,
        max_moim_tokens(),
    );
    let mut with_blocks = insert_blocks(messages, &state.blocks);
    if let Some(text) = unanchored {
        let idx = tail_index(&with_blocks);
        with_blocks.insert(idx, Message::user().with_text(text));
    }
    // The planning gate's reminder is for the requests it is raised on only,
    // so it is never anchored: a block of its own at the very end, gone from
    // the first request the gate no longer asks for it (`agents::planning_gate`).
    if let Some(reminder) = reminder.map(str::trim).filter(|r| !r.is_empty()) {
        let block = cap_moim_block(
            with_preamble(with_reminder(
                format!("{MOIM_OPEN_TAG}\n{MOIM_CLOSE_TAG}"),
                Some(reminder),
            )),
            max_moim_tokens(),
        );
        with_blocks.push(Message::user().with_text(block));
    }
    let Some(fixed) = normalize_with_blocks(normalizer, with_blocks) else {
        return (conversation, false);
    };
    if changed {
        let value = match state.to_value() {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!("MOIM anchors did not serialize ({e})");
                return (fixed, true);
            }
        };
        if let Err(e) = session_manager
            .update_extension_state(
                session_id,
                MoimAnchors::EXTENSION_NAME,
                MoimAnchors::VERSION,
                move |_| Ok(value),
            )
            .await
        {
            // This request still carries the block; the next one, not
            // knowing it, writes it again at its own tail.
            tracing::warn!(session_id, "could not store MOIM anchors ({e})");
        }
    }
    (fixed, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolRequestParams;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_moim_injection_before_assistant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Hello"),
            Message::assistant().with_text("Hi"),
            Message::user().with_text("Bye"),
        ]);
        let (result, _injected) = inject_moim(
            "test-session-id",
            conv,
            &em,
            &working_dir,
            &SharedNormalizer::new(),
            None,
            None,
        )
        .await;
        let msgs = result.messages();

        // MOIM now inserts after the last assistant, merging with the current
        // (last) user turn — not with the historical "Hello" message.
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content[0].as_text().unwrap(), "Hello");
        assert_eq!(msgs[1].content[0].as_text().unwrap(), "Hi");

        // msgs[2] is MOIM merged with "Bye" (the current turn)
        let current_turn = msgs[2]
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("");
        assert!(current_turn.contains("<info-msg>"));
        assert!(current_turn.contains("Working directory: /test/dir"));
        assert!(current_turn.contains("Bye"));

        // Historical message must NOT contain MOIM
        assert!(!msgs[0].content[0].as_text().unwrap().contains("<info-msg>"));
    }

    #[tokio::test]
    async fn test_moim_injection_no_assistant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![Message::user().with_text("Hello")]);
        let (result, _injected) = inject_moim(
            "test-session-id",
            conv,
            &em,
            &working_dir,
            &SharedNormalizer::new(),
            None,
            None,
        )
        .await;

        assert_eq!(result.messages().len(), 1);

        let merged_content = result.messages()[0]
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("");
        assert!(merged_content.contains("Hello"));
        assert!(merged_content.contains("<info-msg>"));
        assert!(merged_content.contains("Working directory: /test/dir"));
    }

    #[tokio::test]
    async fn test_moim_with_tool_calls() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = PathBuf::from("/test/dir");

        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Search for something"),
            Message::assistant()
                .with_text("I'll search for you")
                .with_tool_request(
                    "search_1",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "search".into(),
                        arguments: None,
                        meta: None,
                    }),
                ),
            Message::user().with_tool_response(
                "search_1",
                Ok(rmcp::model::CallToolResult {
                    content: vec![],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
            Message::assistant()
                .with_text("I need to search more")
                .with_tool_request(
                    "search_2",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "search".into(),
                        arguments: None,
                        meta: None,
                    }),
                ),
            Message::user().with_tool_response(
                "search_2",
                Ok(rmcp::model::CallToolResult {
                    content: vec![],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            ),
        ]);

        let (result, _injected) = inject_moim(
            "test-session-id",
            conv,
            &em,
            &working_dir,
            &SharedNormalizer::new(),
            None,
            None,
        )
        .await;
        let msgs = result.messages();

        // MOIM must insert AFTER all tool_results following the last assistant, so that
        // no user message lands between an assistant tool_call and its tool_result.
        // Conversation ends with [assistant+call_2, user+result_2], so MOIM appends at end.
        assert_eq!(msgs.len(), 6);

        // MOIM is at the last position (after both tool results)
        let moim_msg = &msgs[5];
        let has_moim = moim_msg
            .content
            .iter()
            .any(|c| c.as_text().is_some_and(|t| t.contains("<info-msg>")));

        assert!(
            has_moim,
            "MOIM should be appended after the last tool response, not between tool_call and tool_result"
        );

        // The second-to-last message must be the tool_result, not MOIM
        assert!(
            msgs[4].is_tool_response(),
            "tool_result must remain before MOIM"
        );
    }

    /// A standalone `<info-msg>` block (an agent-only injection, e.g. left over
    /// from a prior loop iteration) is removed so exactly one MOIM survives.
    fn sample_moim() -> String {
        format!("{MOIM_OPEN_TAG}\nIt is currently 2020-01-01 00:00:00\nWorking directory: /x\n{MOIM_CLOSE_TAG}")
    }

    fn count_info_msgs(conv: &Conversation) -> usize {
        conv.messages()
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| c.as_text())
            .filter(|t| t.contains(MOIM_OPEN_TAG))
            .count()
    }

    #[test]
    fn test_strip_removes_agent_only_moim() {
        let mut messages = vec![
            Message::user().with_text("real question"),
            Message::assistant().with_text("ok"),
            Message::user().with_text(sample_moim()),
        ];
        strip_existing_moim(&mut messages);
        assert_eq!(messages.len(), 2, "agent-only MOIM message must be dropped");
        assert!(messages.iter().all(|m| m
            .content
            .iter()
            .all(|c| !c.as_text().unwrap().contains(MOIM_OPEN_TAG))));
    }

    #[test]
    fn test_strip_preserves_moim_merged_into_user_turn() {
        // A MOIM that `fix_conversation` merged with real user prose sits in its
        // own content item next to the user's text; that turn must survive.
        let mut messages = vec![Message::user()
            .with_text(sample_moim())
            .with_text("the user's actual question")];
        strip_existing_moim(&mut messages);
        assert_eq!(messages.len(), 1, "a real user turn must never be stripped");
        let joined = messages[0]
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("");
        assert!(joined.contains("the user's actual question"));
    }

    /// BR-2: an oversized MOIM block is head-truncated but still parses as a
    /// MOIM block (opening + closing tags intact), so dedup and merging work.
    #[test]
    fn test_cap_moim_block_truncates_but_keeps_tags() {
        let huge_body = "x".repeat(40_000); // ~10k tokens
        let moim = format!("{MOIM_OPEN_TAG}\nIt is currently now\n{huge_body}\n{MOIM_CLOSE_TAG}");
        let capped = cap_moim_block(moim, 100);

        assert!(
            is_moim_block(&capped),
            "capped block must still parse as MOIM"
        );
        assert!(capped.starts_with(MOIM_OPEN_TAG));
        assert!(capped.trim_end().ends_with(MOIM_CLOSE_TAG));
        assert!(capped.contains("elided to fit the context budget"));
        assert!(estimate_tokens(&capped) <= 120, "stays near the budget");
    }

    /// BR-2: a normal-sized MOIM block passes through unchanged.
    #[test]
    fn test_cap_moim_block_noop_when_small() {
        let moim = sample_moim();
        assert_eq!(cap_moim_block(moim.clone(), 8_000), moim);
    }

    /// The planning gate's reminder rides inside the one block, at its head, so
    /// the size cap — which keeps the head — cannot be what removes it.
    #[tokio::test]
    async fn a_reminder_rides_at_the_head_of_the_block_and_survives_the_cap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let conv = Conversation::new_unvalidated(vec![Message::user().with_text("do it")]);
        let (result, injected) = inject_moim(
            "test-session-id",
            conv,
            &em,
            &PathBuf::from("/test/dir"),
            &SharedNormalizer::new(),
            None,
            Some("Planning required: write the checklist first."),
        )
        .await;
        assert!(injected);
        assert_eq!(count_info_msgs(&result), 1, "one block, not two");
        let block = result.messages()[0]
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .find(|t| t.contains(MOIM_OPEN_TAG))
            .expect("the block")
            .to_string();
        assert!(
            block.starts_with(&format!(
                "{MOIM_OPEN_TAG}\n{MOIM_PREAMBLE}\nPlanning required: write the checklist first."
            )),
            "{block}"
        );

        let huge = format!(
            "{MOIM_OPEN_TAG}\nIt is currently now\n{}\n{MOIM_CLOSE_TAG}",
            "x".repeat(40_000)
        );
        let capped = cap_moim_block(with_preamble(with_reminder(huge, Some("KEEP ME"))), 100);
        assert!(capped.contains("KEEP ME"), "{capped}");
        assert!(capped.contains(MOIM_PREAMBLE), "{capped}");
        assert!(is_moim_block(&capped));

        // No reminder, or a blank one, leaves the block exactly as it was.
        assert_eq!(with_reminder(sample_moim(), None), sample_moim());
        assert_eq!(with_reminder(sample_moim(), Some("  ")), sample_moim());
    }

    /// Without a reminder the preamble is the block's first line, and a block that
    /// carries it is still one `strip_existing_moim` recognises and replaces.
    #[tokio::test]
    async fn every_block_opens_with_the_preamble() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Hello"),
            Message::assistant().with_text("Hi"),
            Message::user().with_text(with_preamble(sample_moim())),
        ]);
        let (result, injected) = inject_moim(
            "test-session-id",
            conv,
            &em,
            &PathBuf::from("/test/dir"),
            &SharedNormalizer::new(),
            None,
            None,
        )
        .await;
        assert!(injected);
        assert_eq!(count_info_msgs(&result), 1, "the old block was replaced");
        let block = result
            .messages()
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| c.as_text())
            .find(|t| t.contains(MOIM_OPEN_TAG))
            .expect("the block")
            .to_string();
        assert!(
            block.starts_with(&format!(
                "{MOIM_OPEN_TAG}\n{MOIM_PREAMBLE}\nIt is currently"
            )),
            "{block}"
        );
        assert_eq!(block.matches(MOIM_PREAMBLE).count(), 1, "{block}");
    }

    /// A reply cut off by a retried server error is kept, and it often ends in
    /// whitespace. Trimming it used to drop the block from the retry (live,
    /// against a scripted llama-server: "MOIM injection caused unexpected
    /// issues: [Trimmed trailing whitespace from assistant message, ...]"), and
    /// from every later request of a chat whose history holds such a reply.
    #[tokio::test]
    async fn trailing_whitespace_in_a_reply_keeps_the_block() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let note = Message::user()
            .with_text("<biorouter-loop-guard>\ncontinue\n</biorouter-loop-guard>")
            .with_visibility(false, true);
        for conversation in [
            // The retry after a cut-off reply.
            vec![
                Message::user().with_text("Write one sentence about foxes."),
                Message::assistant().with_text("The quick brown fox "),
                note.clone(),
            ],
            // A reply that was only whitespace.
            vec![
                Message::user().with_text("test"),
                Message::assistant().with_text("\n\n"),
                note,
            ],
            // A later turn, after a stored reply that ends in a newline.
            vec![
                Message::user().with_text("Write two lines about foxes."),
                Message::assistant().with_text("Foxes are quick.\nFoxes are clever.\n"),
                Message::user().with_text("Thanks"),
            ],
        ] {
            let (result, injected) = inject_moim(
                "test-session-id",
                Conversation::new_unvalidated(conversation),
                &em,
                &PathBuf::from("/test/dir"),
                &SharedNormalizer::new(),
                None,
                None,
            )
            .await;
            assert!(injected, "{:?}", result.messages());
            assert_eq!(count_info_msgs(&result), 1);
        }
    }

    /// A cleanup the block's position can cause still drops it: here the
    /// normalizer removes a tool request nothing answers.
    #[tokio::test]
    async fn an_orphaned_tool_request_still_drops_the_block() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Search for something"),
            Message::assistant().with_tool_request(
                "search_1",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "search".into(),
                    arguments: None,
                    meta: None,
                }),
            ),
            Message::user().with_text("never mind"),
        ]);
        let (_, injected) = inject_moim(
            "test-session-id",
            conv,
            &em,
            &PathBuf::from("/test/dir"),
            &SharedNormalizer::new(),
            None,
            None,
        )
        .await;
        assert!(!injected);
    }

    /// BR-2: a cap of 0 disables MOIM truncation.
    #[test]
    fn test_cap_moim_block_disabled_with_zero() {
        let huge = format!("{MOIM_OPEN_TAG}\n{}\n{MOIM_CLOSE_TAG}", "x".repeat(40_000));
        assert_eq!(cap_moim_block(huge.clone(), 0), huge);
    }

    #[tokio::test]
    async fn test_inject_dedups_prior_moim() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = std::path::PathBuf::from("/test/dir");

        // Conversation already carries a stale standalone MOIM from a prior
        // iteration; injecting a fresh one must not leave two behind.
        let conv = Conversation::new_unvalidated(vec![
            Message::user().with_text("Hello"),
            Message::assistant().with_text("Hi"),
            Message::user().with_text(sample_moim()),
        ]);
        let (result, _injected) = inject_moim(
            "test-session-id",
            conv,
            &em,
            &working_dir,
            &SharedNormalizer::new(),
            None,
            None,
        )
        .await;

        assert_eq!(
            count_info_msgs(&result),
            1,
            "exactly one MOIM must be present after injection"
        );
        // The freshly injected MOIM carries the live working directory, not the
        // stale sample's `/x`.
        assert!(result
            .messages()
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| c.as_text())
            .any(|t| t.contains("Working directory: /test/dir")));
    }
}

#[cfg(test)]
mod anchored_tests {
    use super::*;
    use crate::agents::extension::ExtensionConfig;
    use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
    use crate::session::SessionType;
    use chrono::TimeZone;
    use rmcp::model::{
        CallToolRequestParams, CallToolResult, Content, InitializeResult, JsonObject,
        ListToolsResult,
    };
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    /// A platform extension whose per-turn part the test sets, standing in
    /// for the checklist.
    struct SettablePart(Arc<Mutex<Option<String>>>);

    #[async_trait::async_trait]
    impl McpClientTrait for SettablePart {
        async fn list_tools(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListToolsResult, Error> {
            Err(Error::TransportClosed)
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: Option<JsonObject>,
            _meta: McpMeta,
            _cancellation_token: CancellationToken,
        ) -> Result<CallToolResult, Error> {
            Err(Error::TransportClosed)
        }

        fn get_info(&self) -> Option<&InitializeResult> {
            None
        }

        async fn get_moim(&self, _session_id: &str) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct Chat {
        _dir: tempfile::TempDir,
        sm: SessionManager,
        em: ExtensionManager,
        session_id: String,
        work: std::path::PathBuf,
        todo: Arc<Mutex<Option<String>>>,
        normalizer: SharedNormalizer,
        messages: Vec<Message>,
        next_id: usize,
    }

    impl Chat {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let sm = SessionManager::new(dir.path().join("sessions"));
            let work = dir.path().join("work");
            std::fs::create_dir_all(&work).unwrap();
            let session = sm
                .create_session(work.clone(), "anchors".into(), SessionType::User)
                .await
                .unwrap();
            let em = ExtensionManager::new_without_provider(dir.path().to_path_buf());
            let todo = Arc::new(Mutex::new(None));
            em.add_client(
                "todo".to_string(),
                ExtensionConfig::Platform {
                    name: "todo".to_string(),
                    description: "test".to_string(),
                    bundled: Some(true),
                    available_tools: vec![],
                },
                Arc::new(SettablePart(todo.clone())),
                None,
                None,
            )
            .await;
            Self {
                _dir: dir,
                sm,
                em,
                session_id: session.id,
                work,
                todo,
                normalizer: SharedNormalizer::new(),
                messages: Vec::new(),
                next_id: 0,
            }
        }

        fn id(&mut self) -> String {
            self.next_id += 1;
            format!("m{:03}", self.next_id)
        }

        fn user(&mut self, text: &str) {
            let id = self.id();
            self.messages
                .push(Message::user().with_text(text).with_id(id));
        }

        fn tool_round(&mut self, call: &str, output: &str) {
            let id = self.id();
            self.messages.push(
                Message::assistant()
                    .with_text("working")
                    .with_tool_request(
                        call,
                        Ok(CallToolRequestParams {
                            task: None,
                            name: "developer__shell".into(),
                            arguments: None,
                            meta: None,
                        }),
                    )
                    .with_id(id),
            );
            let id = self.id();
            self.messages.push(
                Message::user()
                    .with_tool_response(
                        call,
                        Ok(CallToolResult {
                            content: vec![Content::text(output)],
                            structured_content: None,
                            is_error: Some(false),
                            meta: None,
                        }),
                    )
                    .with_id(id),
            );
        }

        fn answer(&mut self, text: &str) {
            let id = self.id();
            self.messages
                .push(Message::assistant().with_text(text).with_id(id));
        }

        /// One request as the agent loop builds it.
        async fn request(&self, minute: u32, batch: bool) -> Vec<Message> {
            self.request_with(minute, batch, None).await
        }

        async fn request_with(
            &self,
            minute: u32,
            batch: bool,
            reminder: Option<&str>,
        ) -> Vec<Message> {
            let now = chrono::Local
                .with_ymd_and_hms(2026, 11, 10, 9, 0, 0)
                .unwrap()
                + chrono::Duration::minutes(minute as i64);
            let (conversation, injected) = inject_anchored_moim_at(
                &self.session_id,
                Conversation::new_unvalidated(self.messages.clone()),
                &self.em,
                &self.work,
                &self.normalizer,
                None,
                reminder,
                &self.sm,
                batch,
                now,
            )
            .await;
            assert!(injected);
            conversation.messages().clone()
        }

        async fn anchors(&self) -> MoimAnchors {
            self.sm
                .get_extension_state(
                    &self.session_id,
                    MoimAnchors::EXTENSION_NAME,
                    MoimAnchors::VERSION,
                )
                .await
                .unwrap()
                .map(|value| MoimAnchors::from_value(&value).unwrap())
                .unwrap_or_default()
        }
    }

    /// The Responses API input (Versa's wire format) of a request.
    fn responses_input(messages: &[Message]) -> Vec<Value> {
        let model = crate::model::ModelConfig::new_or_fail("gpt-5.6-terra-2026-07-09");
        let payload = crate::providers::formats::openai_responses::create_responses_request(
            &model,
            "system prompt",
            messages,
            &[],
        )
        .unwrap();
        payload["input"].as_array().unwrap().clone()
    }

    /// Anthropic's messages, without the moving `cache_control` markers
    /// (they are not part of the cached prefix).
    fn anthropic_input(messages: &[Message]) -> Vec<Value> {
        let mut out = crate::providers::formats::anthropic::format_messages(messages);
        fn strip(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    map.remove("cache_control");
                    map.values_mut().for_each(strip);
                }
                Value::Array(items) => items.iter_mut().for_each(strip),
                _ => {}
            }
        }
        out.iter_mut().for_each(strip);
        out
    }

    fn openai_chat_input(messages: &[Message]) -> Vec<Value> {
        crate::providers::formats::openai::format_messages(
            messages,
            &crate::providers::utils::ImageFormat::OpenAi,
        )
    }

    fn assert_prefix(label: &str, earlier: &[Value], later: &[Value]) {
        assert!(
            later.len() >= earlier.len() && later[..earlier.len()] == *earlier,
            "{label}: the earlier request is not a prefix of the later one\nearlier: {}\nlater: {}",
            serde_json::to_string_pretty(earlier).unwrap(),
            serde_json::to_string_pretty(later).unwrap()
        );
    }

    fn assert_continues(earlier: &[Message], later: &[Message]) {
        assert_prefix(
            "responses",
            &responses_input(earlier),
            &responses_input(later),
        );
        assert_prefix(
            "anthropic",
            &anthropic_input(earlier),
            &anthropic_input(later),
        );
        assert_prefix(
            "openai chat",
            &openai_chat_input(earlier),
            &openai_chat_input(later),
        );
    }

    fn block_texts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| c.as_text())
            .filter(|t| is_moim_block(t))
            .map(str::to_string)
            .collect()
    }

    /// Several consecutive turns with tool calls, checklist changes and time
    /// passing: each request is the previous one plus its new tail, in every
    /// wire format, except where an elision batch consolidates the blocks or a
    /// compaction rewrites the history.
    #[tokio::test]
    async fn each_request_is_the_previous_one_plus_its_new_tail() {
        let mut chat = Chat::new().await;
        chat.user("Count the rows in the three files, then write a report.");
        let r1 = chat.request(0, false).await;
        assert_eq!(block_texts(&r1).len(), 1);
        assert!(block_texts(&r1)[0].contains("It is currently 2026-11-10 09:00:00"));

        // A tool round a minute later: nothing but the clock changed, and the
        // clock is not due, so no block is added.
        chat.tool_round("c1", "file a: 10 rows");
        let r2 = chat.request(1, false).await;
        assert_continues(&r1, &r2);
        assert_eq!(block_texts(&r2).len(), 1);

        // The checklist appears: one new block, at the tail, carrying it and
        // the current minute only.
        *chat.todo.lock().unwrap() = Some("Current tasks:\n- [ ] #1 count".into());
        chat.tool_round("c2", "file b: 20 rows");
        let r3 = chat.request(2, false).await;
        assert_continues(&r2, &r3);
        let blocks = block_texts(&r3);
        assert_eq!(blocks.len(), 2);
        let update = &blocks[1];
        assert!(update.contains(UPDATE_HEADER), "{update}");
        assert!(update.contains("#1 count"), "{update}");
        assert!(
            update.contains("It is currently 2026-11-10 09:02:00"),
            "{update}"
        );
        assert!(!update.contains("Working directory"), "{update}");
        let tail = r3.last().unwrap();
        assert!(
            tail.content
                .iter()
                .any(|c| c.as_text().is_some_and(|t| t.contains("#1 count"))),
            "a changed part reaches the model at the tail of the request it changed in"
        );

        // The same context again: nothing new.
        chat.tool_round("c3", "file c: 30 rows");
        let r4 = chat.request(5, false).await;
        assert_continues(&r3, &r4);
        assert_eq!(block_texts(&r4).len(), 2);

        // Twenty minutes on, the clock alone is due.
        chat.tool_round("c4", "report written");
        let r5 = chat.request(25, false).await;
        assert_continues(&r4, &r5);
        let blocks = block_texts(&r5);
        assert_eq!(blocks.len(), 3);
        assert!(blocks[2].contains("09:25:00"), "{}", blocks[2]);
        assert!(!blocks[2].contains("#1 count"), "{}", blocks[2]);

        // The turn ends; the next turn's first request continues it too.
        chat.answer("Done: 60 rows.");
        chat.user("Now add a summary line.");
        *chat.todo.lock().unwrap() = Some("Current tasks:\n- [x] #1 count".into());
        let r6 = chat.request(26, false).await;
        assert_continues(&r5, &r6);
        assert_eq!(block_texts(&r6).len(), 4);

        // A restart: a fresh normalizer and the anchors read back from the
        // store send the same bytes.
        let restarted = SharedNormalizer::new();
        let (again, _) = inject_anchored_moim_at(
            &chat.session_id,
            Conversation::new_unvalidated(chat.messages.clone()),
            &chat.em,
            &chat.work,
            &restarted,
            None,
            None,
            &chat.sm,
            false,
            chrono::Local
                .with_ymd_and_hms(2026, 11, 10, 9, 27, 0)
                .unwrap(),
        )
        .await;
        assert_eq!(
            responses_input(again.messages()),
            responses_input(&r6),
            "a restart sends the same request"
        );

        // An elision batch breaks the prefix anyway: superseded blocks go.
        // Block 2 (checklist + clock) and block 3 (clock) are restated by
        // block 4 (checklist + clock); block 1 still holds the directory.
        chat.tool_round("c5", "summary added");
        let r7 = chat.request(27, true).await;
        let blocks = block_texts(&r7);
        assert_eq!(blocks.len(), 2, "{blocks:#?}");
        assert!(blocks[0].contains("Working directory"));
        assert!(blocks[1].contains("[x] #1 count"));
        // ...and the request after it continues it again.
        chat.tool_round("c6", "checked");
        let r8 = chat.request(28, false).await;
        assert_continues(&r7, &r8);

        // A compaction replaces the history: the anchors are gone, so the
        // next request starts over with one full block.
        let summary_id = chat.id();
        chat.messages = vec![
            Message::user()
                .with_text("Summary of the work so far.")
                .with_id(summary_id),
            Message::assistant()
                .with_text("Continuing.")
                .with_id("m900"),
            Message::user().with_text("Carry on.").with_id("m901"),
        ];
        let r9 = chat.request(29, false).await;
        let blocks = block_texts(&r9);
        assert_eq!(blocks.len(), 1, "{blocks:#?}");
        assert!(!blocks[0].contains(UPDATE_HEADER));
        assert!(blocks[0].contains("Working directory"));
        assert!(blocks[0].contains("[x] #1 count"));
        assert_eq!(chat.anchors().await.blocks.len(), 1);
    }

    /// A part that goes away is said to be gone. The planning gate's reminder
    /// is never anchored: it rides at the very end of the requests it is
    /// raised on, and the request after it carries no trace of it.
    #[tokio::test]
    async fn removals_are_announced_and_the_reminder_is_never_anchored() {
        let mut chat = Chat::new().await;
        chat.user("Do three things.");
        let r1 = chat
            .request_with(0, false, Some("Write the checklist first."))
            .await;
        let blocks = block_texts(&r1);
        assert_eq!(blocks.len(), 2, "{blocks:#?}");
        assert!(!blocks[0].contains("Write the checklist first."));
        assert!(blocks[1].contains("Write the checklist first."));
        assert!(r1
            .last()
            .unwrap()
            .as_concat_text()
            .contains("Write the checklist first."));
        assert_eq!(chat.anchors().await.blocks.len(), 1);

        *chat.todo.lock().unwrap() = Some("Current tasks:\n- [ ] #1 a".into());
        chat.tool_round("c1", "ok");
        let r2 = chat.request(1, false).await;
        let blocks = block_texts(&r2);
        assert_eq!(blocks.len(), 2, "{blocks:#?}");
        assert!(blocks
            .iter()
            .all(|b| !b.contains("Write the checklist first.")));
        assert!(blocks[1].contains("#1 a"));

        // The checklist is cleared: said so.
        *chat.todo.lock().unwrap() = None;
        chat.tool_round("c2", "ok");
        let r3 = chat.request(2, false).await;
        assert_continues(&r2, &r3);
        let blocks = block_texts(&r3);
        assert_eq!(blocks.len(), 3);
        assert!(
            blocks[2].contains("No longer current, ignore the earlier copy: todo context."),
            "{}",
            blocks[2]
        );
    }

    /// A message with no stored id cannot anchor: the block goes at the tail
    /// of that request only, and nothing is stored.
    #[tokio::test]
    async fn an_unanchorable_tail_falls_back_to_the_stateless_block() {
        let chat = Chat::new().await;
        let messages = vec![Message::user().with_text("no id")];
        let (conversation, injected) = inject_anchored_moim_at(
            &chat.session_id,
            Conversation::new_unvalidated(messages),
            &chat.em,
            &chat.work,
            &chat.normalizer,
            None,
            None,
            &chat.sm,
            false,
            chrono::Local::now(),
        )
        .await;
        assert!(injected);
        assert_eq!(block_texts(conversation.messages()).len(), 1);
        assert!(chat.anchors().await.blocks.is_empty());
    }

    /// A change that arrives while the tail has no stored id keeps every
    /// anchored block in place and sends the new one at the tail of that
    /// request alone; the next request stores it once it can be anchored.
    #[tokio::test]
    async fn an_unanchorable_change_keeps_the_anchored_blocks() {
        let mut chat = Chat::new().await;
        chat.user("Go.");
        let r1 = chat.request(0, false).await;
        *chat.todo.lock().unwrap() = Some("Current tasks:\n- [ ] #1 a".into());
        // A tool round whose result is not stored yet.
        chat.tool_round("c1", "ok");
        chat.messages.last_mut().unwrap().id = None;
        let r2 = chat.request(1, false).await;
        let blocks = block_texts(&r2);
        assert_eq!(blocks.len(), 2, "{blocks:#?}");
        assert_eq!(
            blocks[0],
            block_texts(&r1)[0],
            "the first block is unchanged"
        );
        assert!(blocks[1].contains("#1 a"));
        assert_eq!(chat.anchors().await.blocks.len(), 1, "not stored");

        chat.messages.last_mut().unwrap().id = Some("m500".into());
        let r3 = chat.request(2, false).await;
        assert_eq!(block_texts(&r3).len(), 2);
        assert_eq!(chat.anchors().await.blocks.len(), 2, "stored now");
    }

    /// A block anchored after an assistant message is never put between its
    /// tool call and the result that came later.
    #[test]
    fn a_block_never_separates_a_call_from_its_result() {
        let call = Message::assistant()
            .with_tool_request(
                "c1",
                Ok(CallToolRequestParams {
                    task: None,
                    name: "developer__shell".into(),
                    arguments: None,
                    meta: None,
                }),
            )
            .with_id("a1");
        let result = Message::user()
            .with_tool_response(
                "c1",
                Ok(CallToolResult {
                    content: vec![Content::text("ok")],
                    structured_content: None,
                    is_error: Some(false),
                    meta: None,
                }),
            )
            .with_id("r1");
        let block = AnchoredMoim {
            anchor: MoimAnchor::After("a1".into()),
            text: format!("{MOIM_OPEN_TAG}\nx\n{MOIM_CLOSE_TAG}"),
            states: BTreeMap::new(),
            written_at: 0,
        };
        let out = insert_blocks(vec![call, result], &[block]);
        assert_eq!(out.len(), 3);
        assert!(out[1].is_tool_response());
        assert!(is_moim_block(out[2].content[0].as_text().unwrap()));
    }

    /// Superseded blocks are dropped without a batch once they are large
    /// enough to be worth a cache break, and not before.
    #[test]
    fn superseded_blocks_are_dropped_once_they_are_worth_it() {
        let messages = vec![Message::user().with_text("go").with_id("u1")];
        let block = |text: String, keys: &[&str]| AnchoredMoim {
            anchor: MoimAnchor::Before("u1".into()),
            text,
            states: keys
                .iter()
                .map(|k| (k.to_string(), Some("v".to_string())))
                .collect(),
            written_at: 0,
        };
        // The context as the blocks state it, so nothing new is due.
        let fresh = vec![
            MoimSection::new(CLOCK_KEY, "v"),
            MoimSection::new("ext:todo", "v"),
        ];
        let small = MoimAnchors {
            blocks: vec![
                block("a".repeat(400), &[CLOCK_KEY, "ext:todo"]),
                block("b".repeat(400), &[CLOCK_KEY, "ext:todo"]),
            ],
        };
        assert_eq!(small.superseded(), vec![0]);
        let BlockPlan { state, .. } = plan_blocks(small, &messages, &fresh, 0, false, 8_000);
        assert_eq!(state.blocks.len(), 2, "not worth a cache break yet");

        let large = MoimAnchors {
            blocks: vec![
                block("a".repeat(CONSOLIDATE_MIN_TOKENS * 5), &[CLOCK_KEY]),
                block("b".repeat(40), &[CLOCK_KEY]),
            ],
        };
        let BlockPlan { state, changed, .. } =
            plan_blocks(large, &messages, &fresh[..1], 0, false, 8_000);
        assert!(changed);
        assert_eq!(state.blocks.len(), 1);
        assert!(state.blocks[0].text.starts_with('b'));
    }
}
