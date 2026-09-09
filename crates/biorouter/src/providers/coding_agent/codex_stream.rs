//! A pure, push-based decoder for `codex app-server` notifications.
//!
//! This replaces the `absorb` fold in [`super::super::codex`], which reduced a
//! whole turn to "the final text, the usage, and whether it ended". That shape
//! cannot stream: the only text it ever saw was the one `item/completed` frame
//! that arrives *after* the model has finished writing, so the GUI showed a
//! spinner for the entire turn and then the answer all at once.
//!
//! # Why this is a synchronous state machine and not a stream
//!
//! There is no IO here, no process, no `async`. The provider owns the
//! `AppServer` pump (it must, because approvals arrive as server-originated
//! *requests* that have to be answered on the same channel) and hands each
//! notification to [`CodexDecoder::push`], which returns the events that
//! notification produced. Keeping the protocol semantics free of the transport
//! is what lets every rule below be tested against real recorded vendor frames
//! rather than against a fake server's idea of them — and the rules are subtle
//! enough that testing them any other way would not have caught the usage bug
//! documented on [`UsageSource`].
//!
//! # Frame vocabulary this handles
//!
//! Codex emits ~66 notification methods. Only a handful carry the turn:
//!
//! | method | meaning |
//! |---|---|
//! | `item/started` | an item (agent message, reasoning, tool call) opened |
//! | `item/agentMessage/delta` | one token of assistant text |
//! | `item/completed` | an item closed, carrying its **full** final payload |
//! | `item/reasoning/summaryTextDelta`, `item/reasoning/textDelta` | thinking |
//! | `thread/tokenUsage/updated` | cumulative + last-request token counts |
//! | `turn/completed` | the turn ended (with a status that may be `failed`) |
//! | `turn/failed`, `error` | failure dialects — see [`CodexTerminal`] |
//!
//! Everything else (`rawResponseItem/completed`, `thread/status/changed`,
//! `account/rateLimits/updated`, …) is deliberately dropped, but *counted*:
//! [`CodexDecoder::unhandled`] exists so a vendor adding a frame Biorouter needs
//! shows up as a number in a test rather than as silence in production.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use super::super::base::Usage;

/// One decoded thing that happened inside a Codex turn.
///
/// Deliberately *not* a `Message`: building conversation messages here would
/// bake the provider's rendering decisions into the protocol decoder, and the
/// tool variants are parked for phases 3/4 precisely so that mapping can be made
/// once, in one place, when the bridged-call plumbing lands.
#[derive(Debug, Clone, PartialEq)]
pub enum CodexEvent {
    /// One token (or run of tokens) of assistant text, keyed by the Codex item
    /// id it belongs to. Append it to whatever is showing for that item.
    TextDelta {
        /// The `itemId` from the delta frame; stable for the whole message.
        item_id: String,
        /// The new text. Never empty — an empty delta is dropped as noise.
        text: String,
    },
    /// The complete text of an agent message that was **never streamed**.
    ///
    /// A message that did stream does not produce this event; see the
    /// reconciliation rule on [`CodexDecoder::push`]. So this variant is always
    /// safe to append: it is only emitted for text the consumer has not seen.
    TextComplete {
        /// The `item.id` from the completed frame.
        item_id: String,
        /// The full text of the message.
        text: String,
    },
    /// One token of the model's reasoning.
    ///
    /// Both `summaryTextDelta` (the human-readable summary) and `textDelta` (raw
    /// chain of thought, only ever sent when the vendor decides to) land here,
    /// because Biorouter renders a single Thinking stream and a consumer that
    /// had to merge two channels would just concatenate them anyway.
    ReasoningDelta {
        /// The `itemId` from the delta frame.
        item_id: String,
        /// The new reasoning text.
        text: String,
    },
    /// A tool-ish item opened or closed. Parked for phases 3/4 — see
    /// [`CodexToolEvent`].
    ///
    /// Boxed because it is by far the largest payload here (a command's
    /// aggregated output plus two `Value`s) and text deltas are the hot path:
    /// unboxed, every one-token `TextDelta` in the vector would carry the tool
    /// variant's footprint. Also what `clippy::large_enum_variant` asks for, and
    /// this crate lints with `-D warnings`.
    Tool(Box<CodexToolEvent>),
    /// A token-usage snapshot. **Replaces** any previous snapshot; it is
    /// cumulative, not incremental. See [`UsageSource`].
    Usage(CodexUsage),
    /// A non-fatal report from the app server. Never ends the turn.
    ///
    /// Codex sends these during retries — the recorded auth-failure run emits
    /// five `Reconnecting… n/5` notices with `willRetry: true` before the real
    /// failure — so treating any of them as terminal would end a turn that was
    /// about to succeed.
    Notice {
        /// The message as the app server worded it.
        message: String,
        /// Whether Codex intends to retry. `false` means this notice is the
        /// explanation for the failure that is about to arrive.
        will_retry: bool,
    },
    /// The turn is over. No further events will be produced for it.
    Terminal(CodexTerminal),
    /// The frame was understood and carried nothing the consumer needs, or was
    /// not understood at all (in which case [`CodexDecoder::unhandled`] counts
    /// it). Returned rather than an empty vector so a caller can trace or count
    /// every frame it fed in.
    Ignored,
}

/// Where a [`CodexUsage`] came from. Exposed because the two sources disagree
/// about what they measure, and a test needs to be able to say which one won.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    /// `thread/tokenUsage/updated.tokenUsage.total` — the preferred source.
    ///
    /// ⚠ **`total`, never `last`.** `last` is per *model request*, and a turn
    /// that calls tools makes several requests: in the recorded `turn-tools`
    /// run the first snapshot reports `total.totalTokens` 19767 (identical to
    /// `last`, because only one request had happened), while the snapshot after
    /// the second request reports `total` 39660 against a `last` of 19893.
    /// Reading `last` at turn end therefore undercounts by every earlier
    /// request — silently, and by more the more tools the turn used.
    ///
    /// `total` is thread-cumulative rather than turn-cumulative, which is only
    /// correct here because the provider starts a **fresh thread per provider
    /// call** (`thread/start` in `codex.rs`'s `turn_on`). If that ever changes
    /// to a resumed thread, this figure stops being the per-call cost and the
    /// decoder must subtract the thread's usage at turn start.
    ThreadTokenUsage,
    /// `turn/completed.usage`, in snake_case.
    ///
    /// Kept as a fallback rather than deleted. The 0.149.0 recordings show
    /// `turn/completed` carrying no usage at all, but the sequence captured
    /// in-repo from a live 0.147.0 app server carries snake_case usage right
    /// there — so with the installed and the recorded versions disagreeing,
    /// dropping this read would silently zero every usage row on one of them.
    /// Used **only** when no `thread/tokenUsage/updated` ever arrived.
    TurnCompletedFallback,
}

/// A token-usage snapshot plus the provenance that makes it interpretable.
#[derive(Debug, Clone)]
pub struct CodexUsage {
    /// Mapped onto Biorouter's four **disjoint** buckets. Codex follows
    /// OpenAI's convention where the input count is cache-*inclusive*, so the
    /// cached subset is subtracted out here; leaving it in would double-count
    /// the cached prefix in `billed_total` and stop it reconciling with a bill.
    pub usage: Usage,
    /// Which frame this came from.
    pub source: UsageSource,
    /// The model's context window as Codex reports it, when it does. Useful for
    /// the occupancy gauge; absent on the fallback path.
    pub context_window: Option<i32>,
}

/// Hand-written because [`Usage`] derives no `PartialEq` of its own, and giving
/// it one would be an edit to `providers/base.rs` — a type every provider in the
/// tree shares. Comparing the five buckets field by field keeps the equality
/// local to this module, where the only thing that needs it is a test asserting
/// which usage source won.
impl PartialEq for CodexUsage {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.context_window == other.context_window
            && self.usage.input_tokens == other.usage.input_tokens
            && self.usage.output_tokens == other.usage.output_tokens
            && self.usage.total_tokens == other.usage.total_tokens
            && self.usage.cache_read_input_tokens == other.usage.cache_read_input_tokens
            && self.usage.cache_creation_input_tokens == other.usage.cache_creation_input_tokens
    }
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexTurnStatus {
    /// `turn/completed` with status `completed` (or with no status at all,
    /// which is how the older captured shape spells success).
    Completed,
    /// The user (or `turn/interrupt`) stopped it. Partial output stands.
    Interrupted,
    /// The turn failed; [`CodexTerminal::error`] carries why.
    Failed,
}

/// The terminal event for a turn.
///
/// # Three dialects, all accepted
///
/// 1. `turn/completed` with `turn.status: "failed"` and `turn.error.message` —
///    what a real 0.149.0 app server sends (recorded in the auth-failure run,
///    where the turn ends with a 401 message and an empty `items` array). This
///    is the shape the previous fold did **not** handle: it read only
///    `turn/completed` as success, so an authentication failure surfaced as
///    "Codex returned an empty response" with the real reason discarded.
/// 2. `turn/failed` with `error.message` — kept because the fold deliberately
///    tolerated both the app-server and the `codex exec` dialects, so this arm
///    may be the `exec` surface or an older app-server version.
/// 3. The status may also appear flat (`params.status` / `params.error`), which
///    the 0.147.0 schema suggests; both nestings are read.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexTerminal {
    /// How it ended.
    pub status: CodexTurnStatus,
    /// The failure message, present when `status` is [`CodexTurnStatus::Failed`]
    /// and the frame said why. Never `Some` for a clean completion.
    pub error: Option<String>,
    /// The turn id, when the frame carries one. Phase 5 needs a turn id for
    /// `turn/interrupt`, and this is a second place to learn it if the
    /// `turn/start` response was not kept.
    pub turn_id: Option<String>,
}

/// Which half of an item's lifecycle a [`CodexToolEvent`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexItemLifecycle {
    /// `item/started` — the call is in flight.
    Started,
    /// `item/completed` — the call resolved, one way or the other.
    Completed,
}

/// The kind of tool item, with the fields that identify it.
#[derive(Debug, Clone, PartialEq)]
pub enum CodexToolKind {
    /// A call into an MCP server. Once the tool bridge is in play these are
    /// Biorouter's own tools coming back around, which is what phase 3 pairs
    /// into `ToolRequest`/`ToolResponse` cards.
    McpToolCall {
        /// The MCP server name as Codex knows it.
        server: String,
        /// The tool name, still carrying whatever prefix the bridge gave it.
        tool: String,
    },
    /// The child's own shell. Under the read-only sandbox these should not
    /// happen at all, so one arriving is itself worth surfacing.
    CommandExecution {
        /// The command line, including Codex's `/bin/bash -lc "…"` wrapper.
        command: String,
        /// The working directory the command ran in.
        cwd: Option<String>,
    },
    /// The child editing files. Same note as `CommandExecution`.
    FileChange {
        /// The raw `changes` array (path, kind, unified diff per entry), kept
        /// verbatim rather than modelled: nothing consumes it yet, and inventing
        /// a shape now would be a guess that phase 4 has to undo.
        changes: Value,
    },
}

/// A tool item opening or closing.
///
/// **Parked.** Phases 3 and 4 turn these into the live tool cards the GUI
/// already knows how to render; this phase only decodes them so the shapes are
/// pinned by tests against the real schema before anything depends on them.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexToolEvent {
    /// The Codex item id. Pairs the `Started` with its `Completed`.
    pub id: String,
    /// What kind of tool item this is.
    pub kind: CodexToolKind,
    /// Which half of the lifecycle.
    pub lifecycle: CodexItemLifecycle,
    /// `inProgress` / `completed` / `failed` / `declined`, verbatim. Left as a
    /// string because `declined` (an approval refusal) is absent from the
    /// generated enum yet appears in the recorded approval-deny run, and a
    /// typed parse would have to invent a bucket for it.
    pub status: Option<String>,
    /// The MCP call's arguments, when the frame carries them.
    pub arguments: Option<Value>,
    /// The MCP call's result, when it succeeded.
    pub result: Option<Value>,
    /// The failure message, from `error.message`.
    pub error: Option<String>,
    /// A command's combined stdout+stderr.
    pub aggregated_output: Option<String>,
    /// A command's exit status.
    pub exit_code: Option<i64>,
    /// How long the call took, as Codex measured it.
    pub duration_ms: Option<i64>,
}

/// What a completed agent message turns out to owe the consumer.
///
/// Exists so the decision (which reads the per-item state) is separated from
/// acting on it (which writes the decoder's counters); see the comment in
/// [`CodexDecoder::agent_message_item`].
enum Reconcile {
    /// Nothing streamed — the whole text is new.
    Whole,
    /// The deltas already reconstruct the final text exactly.
    AlreadySeen,
    /// The deltas are a strict prefix; this is the missing tail.
    Suffix(String),
    /// The deltas and the final text disagree outright.
    Drift,
}

/// Per-agent-message bookkeeping, which exists only to answer one question:
/// *has the consumer already seen this text?*
#[derive(Debug, Default)]
struct AgentMessage {
    /// `commentary` or `final_answer`, learned from `item/started`. Codex writes
    /// commentary *before* it works and the real answer after, so a consumer
    /// that wants only the reply needs this to tell them apart.
    phase: Option<String>,
    /// Everything yielded as [`CodexEvent::TextDelta`] so far.
    streamed: String,
    /// Whether any delta arrived. Not `!streamed.is_empty()`: a message whose
    /// only delta was empty streamed nothing but must still not be re-emitted
    /// wholesale, and this is the flag that says so.
    saw_delta: bool,
    /// The authoritative text from `item/completed`, once it arrives.
    final_text: Option<String>,
}

/// Folds `codex app-server` notifications into [`CodexEvent`]s.
///
/// One decoder per turn. `push` is the whole surface; the accessors exist for
/// the end of the turn (final text, final usage) and for diagnostics.
#[derive(Debug, Default)]
pub struct CodexDecoder {
    /// Agent messages in arrival order, so `text()` reads out in the order the
    /// model wrote them.
    messages: Vec<(String, AgentMessage)>,
    /// item id → index into `messages`.
    index: HashMap<String, usize>,
    /// The most recent usage snapshot, and where it came from.
    usage: Option<CodexUsage>,
    /// Whether any `thread/tokenUsage/updated` arrived. Gates the fallback.
    saw_token_usage: bool,
    /// The last non-retryable [`CodexEvent::Notice`], kept so a turn whose
    /// stdout simply closes can still say what went wrong.
    pending_failure: Option<String>,
    /// Set once a terminal frame has been seen, so a duplicate or trailing
    /// terminal cannot end the turn twice.
    finished: bool,
    /// Methods with no arm at all, and how often each arrived.
    unhandled: BTreeMap<String, usize>,
    /// Completed messages whose deltas matched the final text exactly.
    reconciled: usize,
    /// Completed messages whose deltas did **not** reconstruct the final text.
    drifted: usize,
}

impl CodexDecoder {
    /// A decoder for a fresh turn.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one JSON-RPC **notification** and get back what it produced.
    ///
    /// Always returns at least one event; a frame that carries nothing yields
    /// [`CodexEvent::Ignored`] rather than an empty vector, so a caller can
    /// account for every frame it fed in.
    ///
    /// # The reconciliation rule
    ///
    /// `item/completed` for an agent message carries the message's **entire**
    /// final text, and the deltas concatenate to exactly that string — verified
    /// across all 17 recorded vendor runs. So the completed frame is a
    /// *checkpoint*, not a continuation, and appending it after streaming would
    /// print the whole answer twice. This decoder therefore:
    ///
    /// * emits [`CodexEvent::TextComplete`] only when the message never
    ///   streamed (nothing has been shown, so the whole text is new);
    /// * emits nothing when the deltas already reconstruct the final text;
    /// * emits the missing **suffix** as a [`CodexEvent::TextDelta`] when the
    ///   deltas are a strict prefix of it, which repairs a dropped delta without
    ///   ever duplicating text;
    /// * emits nothing, and counts [`Self::drifted`], when the two disagree
    ///   outright. Showing a slightly truncated answer is a smaller failure than
    ///   showing the answer twice, and the divergence is recorded rather than
    ///   papered over.
    ///
    /// Never panics: every field is read through `get`/`as_*`, so a malformed or
    /// unexpectedly-shaped frame degrades to `Ignored` instead of taking the
    /// turn down.
    pub fn push(&mut self, method: &str, params: &Value) -> Vec<CodexEvent> {
        match method {
            "item/agentMessage/delta" => self.agent_message_delta(params),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                Self::reasoning_delta(params)
            }
            "item/started" => self.item(params, CodexItemLifecycle::Started),
            "item/completed" => self.item(params, CodexItemLifecycle::Completed),
            "thread/tokenUsage/updated" => self.token_usage(params),
            "turn/completed" => self.turn_completed(params),
            "turn/failed" => self.turn_failed(params),
            // ⚠ The literal is "error", not "thread/error" — it breaks the
            // slash-separated convention every sibling notification follows, so
            // a match written from the type names alone misses it entirely.
            "error" => self.error_notice(params),
            other => {
                *self.unhandled.entry(other.to_string()).or_insert(0) += 1;
                vec![CodexEvent::Ignored]
            }
        }
    }

    /// The finished agent-message texts, in the order the model wrote them.
    ///
    /// The authoritative `item/completed` text wins; a message that never
    /// completed falls back to whatever streamed, so a turn cut short still
    /// yields what the user already saw. Empty and whitespace-only messages are
    /// dropped — Codex opens every message item with `text: ""`.
    pub fn text(&self) -> Vec<String> {
        self.messages
            .iter()
            .filter_map(|(_, m)| {
                let text = m.final_text.as_deref().unwrap_or(&m.streamed);
                (!text.trim().is_empty()).then(|| text.to_string())
            })
            .collect()
    }

    /// The phase (`commentary` or `final_answer`) Codex assigned an item, if it
    /// announced one.
    ///
    /// Codex streams commentary before it starts working and the real reply
    /// after, both as ordinary agent messages. The phase is carried on
    /// `item/started`, not on the deltas, which is why it is an accessor here
    /// rather than a field on [`CodexEvent::TextDelta`].
    pub fn item_phase(&self, item_id: &str) -> Option<&str> {
        self.index
            .get(item_id)
            .and_then(|i| self.messages.get(*i))
            .and_then(|(_, m)| m.phase.as_deref())
    }

    /// The turn's usage, or `None` if neither source ever reported any.
    pub fn usage(&self) -> Option<&CodexUsage> {
        self.usage.as_ref()
    }

    /// The last non-retryable error the app server reported outside a terminal
    /// frame. A pump whose child's stdout closes without a terminal frame should
    /// use this as the failure reason rather than inventing one.
    pub fn pending_failure(&self) -> Option<&str> {
        self.pending_failure.as_deref()
    }

    /// Whether a terminal frame has been seen.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// How many notifications arrived whose method this decoder has no arm for.
    ///
    /// A vendor upgrade that moves the turn onto a new frame shows up here as a
    /// number, which a test can assert on, instead of as an empty answer nobody
    /// can explain.
    pub fn unhandled(&self) -> usize {
        self.unhandled.values().sum()
    }

    /// The unhandled methods and their counts, for logging a real turn.
    pub fn unhandled_methods(&self) -> &BTreeMap<String, usize> {
        &self.unhandled
    }

    /// Messages whose streamed deltas exactly reconstructed the final text.
    pub fn reconciled(&self) -> usize {
        self.reconciled
    }

    /// Messages whose streamed deltas did not reconstruct the final text.
    ///
    /// Expected to stay zero. A non-zero count means the vendor changed the
    /// delta/completed relationship this decoder is built on, and the streamed
    /// answer may be missing a tail.
    pub fn drifted(&self) -> usize {
        self.drifted
    }

    /// `item/agentMessage/delta` → one [`CodexEvent::TextDelta`].
    fn agent_message_delta(&mut self, params: &Value) -> Vec<CodexEvent> {
        let Some(item_id) = str_at(params, &["itemId"]) else {
            return vec![CodexEvent::Ignored];
        };
        let delta = str_at(params, &["delta"]).unwrap_or_default();
        let slot = self.slot(item_id);
        // Mark the item as streamed even for an empty delta: the consumer is
        // now showing this item, so the completed frame must not re-emit it.
        slot.saw_delta = true;
        if delta.is_empty() {
            return vec![CodexEvent::Ignored];
        }
        slot.streamed.push_str(delta);
        vec![CodexEvent::TextDelta {
            item_id: item_id.to_string(),
            text: delta.to_string(),
        }]
    }

    /// The two reasoning delta methods → [`CodexEvent::ReasoningDelta`].
    ///
    /// ⚠ **Experimental: unobserved on the wire.** Not one of the 17 recorded
    /// vendor runs contains either method, because the recording harness never
    /// asks for a reasoning summary on `turn/start` and the model returned its
    /// reasoning encrypted with an empty summary. So this mapping is written
    /// from the generated schema alone, and whether subscription models ever
    /// emit it is unknown. It is harmless if it never fires.
    fn reasoning_delta(params: &Value) -> Vec<CodexEvent> {
        let Some(item_id) = str_at(params, &["itemId"]) else {
            return vec![CodexEvent::Ignored];
        };
        let delta = str_at(params, &["delta"]).unwrap_or_default();
        if delta.is_empty() {
            return vec![CodexEvent::Ignored];
        }
        vec![CodexEvent::ReasoningDelta {
            item_id: item_id.to_string(),
            text: delta.to_string(),
        }]
    }

    /// `item/started` and `item/completed`, dispatched on the item's type.
    fn item(&mut self, params: &Value, lifecycle: CodexItemLifecycle) -> Vec<CodexEvent> {
        let item = params.get("item").unwrap_or(&Value::Null);
        let kind = str_at(item, &["type"]).unwrap_or_default();
        match kind {
            // The app server spells item types in camelCase and `codex exec` in
            // snake_case; accepting both means a version change on either
            // surface cannot silently swallow the answer.
            "agentMessage" | "agent_message" => self.agent_message_item(item, lifecycle),
            "mcpToolCall" | "mcp_tool_call" => tool_event(item, lifecycle, mcp_kind(item))
                .map_or_else(
                    || vec![CodexEvent::Ignored],
                    |e| vec![CodexEvent::Tool(Box::new(e))],
                ),
            "commandExecution" | "command_execution" => {
                tool_event(item, lifecycle, command_kind(item)).map_or_else(
                    || vec![CodexEvent::Ignored],
                    |e| vec![CodexEvent::Tool(Box::new(e))],
                )
            }
            "fileChange" | "file_change" => tool_event(item, lifecycle, file_change_kind(item))
                .map_or_else(
                    || vec![CodexEvent::Ignored],
                    |e| vec![CodexEvent::Tool(Box::new(e))],
                ),
            // `userMessage` is Codex echoing the prompt back; `reasoning` items
            // close carrying only encrypted content in every recorded run. Both
            // are understood and deliberately dropped, so neither counts as
            // unhandled.
            _ => vec![CodexEvent::Ignored],
        }
    }

    /// The agent-message half of [`Self::item`], where the reconciliation rule
    /// documented on [`Self::push`] lives.
    fn agent_message_item(
        &mut self,
        item: &Value,
        lifecycle: CodexItemLifecycle,
    ) -> Vec<CodexEvent> {
        let Some(item_id) = str_at(item, &["id"]) else {
            return vec![CodexEvent::Ignored];
        };
        let phase = str_at(item, &["phase"]).map(str::to_string);

        if lifecycle == CodexItemLifecycle::Started {
            let slot = self.slot(item_id);
            if phase.is_some() {
                slot.phase = phase;
            }
            // `item/started` carries `text: ""`; there is never anything to show.
            return vec![CodexEvent::Ignored];
        }

        let text = str_at(item, &["text"]).unwrap_or_default().to_string();
        let item_id = item_id.to_string();

        // The per-item bookkeeping and the counters both live on `self`, so the
        // decision is made inside a block that ends the item borrow before the
        // counters are touched. Written this way on purpose: the alternative
        // interleaves a `&mut` field borrow with `&mut self` and only compiles
        // by NLL's liveness analysis, which is a fragile thing to rest on.
        let decision = {
            let slot = self.slot(&item_id);
            if phase.is_some() {
                slot.phase = phase;
            }
            slot.final_text = Some(text.clone());
            if !slot.saw_delta {
                Reconcile::Whole
            } else if slot.streamed == text {
                Reconcile::AlreadySeen
            } else if let Some(suffix) = text.strip_prefix(slot.streamed.as_str()) {
                // A delta went missing. Only the part nobody has seen is new.
                let suffix = suffix.to_string();
                slot.streamed.push_str(&suffix);
                Reconcile::Suffix(suffix)
            } else {
                Reconcile::Drift
            }
        };

        match decision {
            // Nothing was shown for this item, so all of it is new.
            Reconcile::Whole if text.trim().is_empty() => vec![CodexEvent::Ignored],
            Reconcile::Whole => vec![CodexEvent::TextComplete { item_id, text }],
            Reconcile::AlreadySeen => {
                self.reconciled += 1;
                vec![CodexEvent::Ignored]
            }
            Reconcile::Suffix(suffix) => {
                self.reconciled += 1;
                vec![CodexEvent::TextDelta {
                    item_id,
                    text: suffix,
                }]
            }
            // The stream and the final text disagree outright. Re-emitting would
            // print the answer twice, which is a worse failure than a truncated
            // tail, so the divergence is recorded and the frame dropped.
            Reconcile::Drift => {
                self.drifted += 1;
                vec![CodexEvent::Ignored]
            }
        }
    }

    /// `thread/tokenUsage/updated` → the preferred usage snapshot.
    ///
    /// Note the `tokenUsage` key is camelCase, and the breakdown inside it is
    /// too (`inputTokens`, `cachedInputTokens`), which is *not* the spelling the
    /// fallback path uses. Both are read by the same parser.
    ///
    /// A snapshot supersedes the previous one; they are cumulative, so summing
    /// them would multiply the turn's cost by the number of model requests.
    fn token_usage(&mut self, params: &Value) -> Vec<CodexEvent> {
        let Some(token_usage) = params.get("tokenUsage") else {
            return vec![CodexEvent::Ignored];
        };
        // `.total`, not `.last` — see UsageSource::ThreadTokenUsage.
        let Some(total) = token_usage.get("total") else {
            return vec![CodexEvent::Ignored];
        };
        let mut usage = parse_breakdown(total);

        // ⚠ The two figures answer different questions, and `Usage` needs both.
        //
        // The billed buckets (input/output/cache) are cumulative over the turn's
        // model requests, so they come from `total`. But `total_tokens` is the
        // live CONTEXT-OCCUPANCY gauge, and context does not accumulate across
        // requests — it is whatever the last request carried. In the recorded
        // four-request turn `total.totalTokens` is 79,676 while the context at
        // the end is 20,023, so using the cumulative figure would inflate the
        // gauge by roughly one whole context per tool call and show a session
        // with plenty of headroom as nearly full.
        if let Some(last) = token_usage.get("last") {
            if let Some(context) = parse_breakdown(last).total_tokens {
                usage.total_tokens = Some(context);
            }
        }

        let snapshot = CodexUsage {
            usage,
            source: UsageSource::ThreadTokenUsage,
            context_window: i64_at(token_usage, &["modelContextWindow"]).map(|v| v as i32),
        };
        self.saw_token_usage = true;
        self.usage = Some(snapshot.clone());
        vec![CodexEvent::Usage(snapshot)]
    }

    /// `turn/completed` — the normal end of a turn, and one of the two places a
    /// failure can arrive.
    fn turn_completed(&mut self, params: &Value) -> Vec<CodexEvent> {
        let mut events = Vec::new();

        // The fallback usage read, kept alive for the app-server version whose
        // `turn/completed` carries snake_case usage. Only consulted when the
        // preferred source never spoke. Both nestings are tried because the
        // captured 0.147.0 sequence puts it at the top level while the 0.149.0
        // frame nests everything else under `turn`.
        if !self.saw_token_usage {
            if let Some(u) = params
                .get("usage")
                .or_else(|| params.get("turn").and_then(|t| t.get("usage")))
            {
                let snapshot = CodexUsage {
                    usage: parse_breakdown(u),
                    source: UsageSource::TurnCompletedFallback,
                    context_window: None,
                };
                self.usage = Some(snapshot.clone());
                events.push(CodexEvent::Usage(snapshot));
            }
        }

        let turn = params.get("turn");
        // 0.149.0 nests status and error under `turn`; the flat spelling is
        // accepted too so a schema that moves them back does not go unread.
        let status = str_at(params, &["turn", "status"]).or_else(|| str_at(params, &["status"]));
        let status = match status {
            Some("failed") => CodexTurnStatus::Failed,
            Some("interrupted") => CodexTurnStatus::Interrupted,
            Some("completed") => CodexTurnStatus::Completed,
            // No status at all is the older success shape, not an unknown.
            None => CodexTurnStatus::Completed,
            // ⚠ An unrecognised status is NOT success. The v2 enum is
            // completed|interrupted|failed|inProgress, so a bare `_ =>
            // Completed` already swallowed `inProgress`, and would swallow any
            // value a future Codex adds (`cancelled`, `moderated`, `expired`) —
            // fail-open on exactly the vendor drift this module exists to catch.
            // Reported as a failure naming the value, so an unknown state is
            // visible instead of being presented to the user as a finished turn.
            Some(_) => CodexTurnStatus::Failed,
        };
        let unknown_status = matches!(status, CodexTurnStatus::Failed)
            .then(|| str_at(params, &["turn", "status"]).or_else(|| str_at(params, &["status"])))
            .flatten()
            .filter(|s| *s != "failed")
            .map(str::to_string);
        let error = (status == CodexTurnStatus::Failed).then(|| {
            if let Some(unknown) = &unknown_status {
                return format!(
                    "the Codex turn ended with an unrecognised status `{unknown}` — \
                     this build of Biorouter does not know whether that means the \
                     turn succeeded"
                );
            }
            error_message(params.get("turn").and_then(|t| t.get("error")))
                .or_else(|| error_message(params.get("error")))
                // A failure with no message of its own is usually explained by
                // the last non-retryable `error` notification, which arrives
                // first (recorded: the 401 text appears at seq 56 and again in
                // the terminal frame at seq 58).
                .or_else(|| self.pending_failure.clone())
                .unwrap_or_else(|| "the Codex turn failed".to_string())
        });

        self.finished = true;
        events.push(CodexEvent::Terminal(CodexTerminal {
            status,
            error,
            turn_id: turn
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
                .or_else(|| str_at(params, &["turnId"]))
                .map(str::to_string),
        }));
        events
    }

    /// `turn/failed` — the other failure dialect. Retained rather than folded
    /// into `turn/completed` because the previous fold deliberately accepted
    /// both the app-server and the `codex exec` surfaces, and no recording
    /// proves which version emits which.
    fn turn_failed(&mut self, params: &Value) -> Vec<CodexEvent> {
        self.finished = true;
        vec![CodexEvent::Terminal(CodexTerminal {
            status: CodexTurnStatus::Failed,
            error: Some(
                error_message(params.get("error"))
                    .or_else(|| str_at(params, &["message"]).map(str::to_string))
                    .or_else(|| self.pending_failure.clone())
                    .unwrap_or_else(|| "the Codex turn failed".to_string()),
            ),
            turn_id: str_at(params, &["turnId"]).map(str::to_string),
        })]
    }

    /// The standalone `error` notification.
    ///
    /// The recorded 0.149.0 frames put the text at `params.error.message` while
    /// the previous fold read `params.message`; since no recording proves the
    /// other spelling wrong either, **both** are read and the nested one wins.
    /// Swapping one guess for the other would just move the blind spot.
    ///
    /// Not terminal. `willRetry: true` notices precede a *successful* retry —
    /// the recorded auth-failure run emits five of them — so only the
    /// non-retryable one is remembered as the reason a turn failed.
    fn error_notice(&mut self, params: &Value) -> Vec<CodexEvent> {
        let message = error_message(params.get("error"))
            .or_else(|| str_at(params, &["message"]).map(str::to_string))
            .unwrap_or_else(|| "the Codex app server reported an error".to_string());
        let will_retry = params
            .get("willRetry")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !will_retry {
            self.pending_failure = Some(message.clone());
        }
        vec![CodexEvent::Notice {
            message,
            will_retry,
        }]
    }

    /// The bookkeeping slot for an item, created on first sight.
    fn slot(&mut self, item_id: &str) -> &mut AgentMessage {
        let idx = match self.index.get(item_id) {
            Some(i) => *i,
            None => {
                let i = self.messages.len();
                self.messages
                    .push((item_id.to_string(), AgentMessage::default()));
                self.index.insert(item_id.to_string(), i);
                i
            }
        };
        // The index is only ever written alongside the push above, so this
        // cannot miss; the expression avoids an `unwrap` all the same.
        &mut self.messages[idx].1
    }
}

/// Read a nested string, tolerating every missing level.
fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str()
}

/// Read a nested integer, tolerating every missing level.
fn i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_i64()
}

/// Pull a human-readable message out of an error object, accepting the
/// `{message}` object the schema defines, a bare string, and a string that is
/// itself a JSON error envelope.
///
/// ⚠ **The third shape is not hypothetical and it is the one users see.** Codex
/// relays the upstream API's body verbatim as a *string*, so `params.error` came
/// through here as `as_str()` and the whole envelope was rendered in the
/// transcript:
///
/// ```text
/// Ran into this error: Request failed: {"type":"error","status":400,"error":
/// {"type":"invalid_request_error","message":"The 'gpt-6-astra' model requires a
/// newer version of Codex. Please upgrade to the latest app or CLI and try
/// again."}}.
/// ```
///
/// The only actionable sentence in that is buried in the middle of JSON the
/// reader cannot act on. [`unwrap_json_error`] takes it out.
///
/// ⚠ **Unwrapping is best-effort and must stay that way** — see
/// [`super::unwrap_json_error`], which both vendors' decoders share so no two
/// surfaces can disagree about what a failure says. `providers/codex.rs` reads
/// its non-streaming failures through THIS function for the same reason.
pub(crate) fn error_message(error: Option<&Value>) -> Option<String> {
    let error = error?;
    if let Some(s) = error.as_str() {
        return (!s.is_empty()).then(|| explain_disabled_native_tool(&super::unwrap_json_error(s)));
    }
    error
        .get("message")
        .and_then(Value::as_str)
        .map(|s| explain_disabled_native_tool(&super::unwrap_json_error(s)))
}

/// Turn the vendor's internal failure for a tool Biorouter disabled into
/// something a reader can act on.
///
/// Biorouter passes `multi_agent` (and a dozen more) to Codex's `--disable`,
/// because a coding-agent child running its own unmediated tools is exactly
/// what the tool bridge exists to prevent. Codex keeps ADVERTISING those tools
/// anyway, so a model asked to delegate calls `spawn_agent` and gets
/// `no thread with id` back — which names nothing the user did, nothing they
/// can change, and makes a perfectly healthy bridge look broken.
///
/// ⚠ **This is a safety net, not the fix.** The fix is
/// [`coding_agent::native_tools_notice`], which tells the model up front which
/// tools are real; this only catches the case where it reaches for one anyway.
/// It is also best-effort in a second sense: the vendor may keep such a failure
/// inside its own agent loop and only ever report it as prose to the model, in
/// which case nothing reaches here to rewrite. It is matched on a vendor
/// string, so it will stop matching without warning if that string changes —
/// which costs only the explanation, never correctness.
fn explain_disabled_native_tool(message: &str) -> String {
    const VENDOR_MISSING_THREAD: &str = "no thread with id";
    if !message.to_ascii_lowercase().contains(VENDOR_MISSING_THREAD) {
        return message.to_string();
    }
    format!(
        "{message}\n\n\
         This is Codex's own sub-agent machinery, which Biorouter switches off: a \
         coding agent running inside Biorouter uses Biorouter's tools, behind its \
         inspectors and privacy gates, rather than its own. Ask for the work \
         directly instead of asking Codex to delegate it."
    )
}

/// Read an integer field under either the camelCase or the snake_case spelling.
///
/// The two usage sources disagree on casing — `thread/tokenUsage/updated` sends
/// `inputTokens`, the captured `turn/completed` sends `input_tokens` — and one
/// parser reading both is what keeps the fallback path from needing a second
/// copy of the disjointness arithmetic below.
fn token_field(value: &Value, camel: &str, snake: &str) -> Option<i32> {
    value
        .get(camel)
        .or_else(|| value.get(snake))
        .and_then(Value::as_i64)
        .map(|v| v as i32)
}

/// Map a Codex token breakdown onto Biorouter's four **disjoint** buckets.
///
/// Codex follows OpenAI's convention, where the input count is the *total*
/// prompt count and the cached count is a subset of it — confirmed by the
/// recorded numbers, where `totalTokens` 19767 equals `inputTokens` 19682 plus
/// `outputTokens` 85 with `cachedInputTokens` 19200 sitting inside the input
/// figure. [`Usage`] requires its buckets not to overlap, so the cached part is
/// subtracted out here; without that, `billed_total` double-counts every cached
/// token and stops reconciling with a vendor bill.
fn parse_breakdown(value: &Value) -> Usage {
    let total_input = token_field(value, "inputTokens", "input_tokens");
    let cached = token_field(value, "cachedInputTokens", "cached_input_tokens");
    let cache_write = token_field(value, "cacheWriteInputTokens", "cache_write_input_tokens");
    let output = token_field(value, "outputTokens", "output_tokens");

    let fresh_input = match (total_input, cached) {
        (Some(total), Some(c)) => Some((total - c).max(0)),
        (other, _) => other,
    };

    Usage {
        input_tokens: fresh_input,
        output_tokens: output,
        // Context occupancy for the gauge, which includes the cached prefix.
        // Codex reports it directly on the camelCase path; the snake_case
        // fallback has no such field, so it is reconstructed the way the
        // previous parser did.
        total_tokens: token_field(value, "totalTokens", "total_tokens").or_else(|| {
            match (total_input, output) {
                (None, None) => None,
                _ => Some(total_input.unwrap_or(0) + output.unwrap_or(0)),
            }
        }),
        cache_read_input_tokens: cached,
        cache_creation_input_tokens: cache_write,
    }
}

/// The identifying half of an `mcpToolCall` item.
fn mcp_kind(item: &Value) -> Option<CodexToolKind> {
    Some(CodexToolKind::McpToolCall {
        server: str_at(item, &["server"])?.to_string(),
        tool: str_at(item, &["tool"])?.to_string(),
    })
}

/// The identifying half of a `commandExecution` item.
fn command_kind(item: &Value) -> Option<CodexToolKind> {
    Some(CodexToolKind::CommandExecution {
        command: str_at(item, &["command"])?.to_string(),
        cwd: str_at(item, &["cwd"]).map(str::to_string),
    })
}

/// The identifying half of a `fileChange` item.
fn file_change_kind(item: &Value) -> Option<CodexToolKind> {
    Some(CodexToolKind::FileChange {
        changes: item.get("changes").cloned().unwrap_or(Value::Null),
    })
}

/// Assemble a [`CodexToolEvent`] from an item plus its already-decoded kind.
/// Returns `None` when the item has no id or the kind could not be read, which
/// is the only way a malformed tool frame is allowed to fail.
fn tool_event(
    item: &Value,
    lifecycle: CodexItemLifecycle,
    kind: Option<CodexToolKind>,
) -> Option<CodexToolEvent> {
    Some(CodexToolEvent {
        id: str_at(item, &["id"])?.to_string(),
        kind: kind?,
        lifecycle,
        status: str_at(item, &["status"]).map(str::to_string),
        arguments: item.get("arguments").filter(|v| !v.is_null()).cloned(),
        result: item.get("result").filter(|v| !v.is_null()).cloned(),
        error: error_message(item.get("error")),
        aggregated_output: str_at(item, &["aggregatedOutput"]).map(str::to_string),
        exit_code: i64_at(item, &["exitCode"]),
        duration_ms: i64_at(item, &["durationMs"]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every frame below is a verbatim line from a real vendor recording
    /// (`recordings/codex/<cell>/provider→bridge.ndjson`, the `line` field
    /// unwrapped), truncated only where an unread field was enormous. Synthetic
    /// frames would have hidden both bugs this module exists to fix: the wrong
    /// usage field, and the unhandled `turn/completed{status:"failed"}` shape.
    ///
    /// Cell `turn-tools`, seq 37 — a commentary message opening.
    const STARTED_COMMENTARY: &str = r#"{"item":{"type":"agentMessage","id":"msg_0a7eeffd6b544752016a87a72fb32087d0bf94a1dbf766f5a7","text":"","phase":"commentary","memoryCitation":null,"delivery":null},"threadId":"01a021e4-fe50-7c91-abf4-c384358c1d6f","turnId":"01a021e4-ff0a-7bf3-8023-962ba8347adf","startedAtMs":1787275055717}"#;

    /// Cell `turn-tools`, seq 71 — the same message closing with its full text.
    const COMPLETED_COMMENTARY: &str = r#"{"item":{"type":"agentMessage","id":"msg_0a7eeffd6b544752016a87a72fb32087d0bf94a1dbf766f5a7","text":"I’ll inspect and edit `math.js`, then run the exact verification command.","phase":"commentary","memoryCitation":null,"delivery":null},"threadId":"01a021e4-fe50-7c91-abf4-c384358c1d6f","turnId":"01a021e4-ff0a-7bf3-8023-962ba8347adf","completedAtMs":1787275056387}"#;

    /// Cell `turn-tools`, seqs 39-69 — the 16 deltas of that message, in order.
    const COMMENTARY_DELTAS: [&str; 16] = [
        "I",
        "’ll",
        " inspect",
        " and",
        " edit",
        " `",
        "math",
        ".js",
        "`,",
        " then",
        " run",
        " the",
        " exact",
        " verification",
        " command",
        ".",
    ];

    /// The exact string those deltas concatenate to, which is also the text the
    /// completed frame carries. This equality is the whole reconciliation rule.
    const COMMENTARY_TEXT: &str =
        "I’ll inspect and edit `math.js`, then run the exact verification command.";

    const COMMENTARY_ITEM_ID: &str = "msg_0a7eeffd6b544752016a87a72fb32087d0bf94a1dbf766f5a7";

    /// Cell `turn-tools`, seq 81 — the first snapshot, where `total` and `last`
    /// happen to agree because only one model request has happened yet.
    const TOKEN_USAGE_FIRST: &str = r#"{"threadId":"01a021e4-fe50-7c91-abf4-c384358c1d6f","turnId":"01a021e4-ff0a-7bf3-8023-962ba8347adf","tokenUsage":{"total":{"totalTokens":19767,"inputTokens":19682,"cachedInputTokens":19200,"cacheWriteInputTokens":0,"outputTokens":85,"reasoningOutputTokens":0},"last":{"totalTokens":19767,"inputTokens":19682,"cachedInputTokens":19200,"cacheWriteInputTokens":0,"outputTokens":85,"reasoningOutputTokens":0},"modelContextWindow":258400}}"#;

    /// Cell `turn-tools`, seq 94 — after the second model request, where the two
    /// diverge. This frame is the evidence for reading `total`.
    const TOKEN_USAGE_SECOND: &str = r#"{"threadId":"01a021e4-fe50-7c91-abf4-c384358c1d6f","turnId":"01a021e4-ff0a-7bf3-8023-962ba8347adf","tokenUsage":{"total":{"totalTokens":39660,"inputTokens":39487,"cachedInputTokens":38400,"cacheWriteInputTokens":0,"outputTokens":173,"reasoningOutputTokens":0},"last":{"totalTokens":19893,"inputTokens":19805,"cachedInputTokens":19200,"cacheWriteInputTokens":0,"outputTokens":88,"reasoningOutputTokens":0},"modelContextWindow":258400}}"#;

    /// Cell `turn-tools`, seq 128 — a clean completion. Note it carries **no**
    /// `usage` key at all, which is why the fallback below is gated.
    const TURN_COMPLETED_OK: &str = r#"{"threadId":"01a021e4-fe50-7c91-abf4-c384358c1d6f","turn":{"id":"01a021e4-ff0a-7bf3-8023-962ba8347adf","items":[{"type":"agentMessage","id":"msg_0a7eeffd6b544752016a87a73edd5c87d082409fa3559c6f37","text":"2","phase":"final_answer","memoryCitation":null,"delivery":null}],"itemsView":"summary","status":"completed","error":null,"startedAt":1787275050,"completedAt":1787275071,"durationMs":20308}}"#;

    /// Cell `auth-failure`, seq 58 — the failure shape the previous fold did not
    /// handle, which is how a 401 surfaced as "Codex returned an empty response".
    const TURN_COMPLETED_FAILED: &str = r#"{"threadId":"01a0222d-48c6-7ef2-a31f-81ae86c06141","turn":{"id":"01a0222d-4a40-7610-b7bb-a7c51f6b0d2e","items":[],"itemsView":"notLoaded","status":"failed","error":{"message":"unexpected status 401 Unauthorized: Missing bearer or basic authentication in header","codexErrorInfo":"other","additionalDetails":null},"startedAt":1787279788,"completedAt":1787279806,"durationMs":17815}}"#;

    /// Cell `auth-failure`, seq 35 — a retry notice. Fatal-looking, not fatal.
    const ERROR_RETRYABLE: &str = r#"{"error":{"message":"Reconnecting... 2/5","codexErrorInfo":{"responseStreamDisconnected":{"httpStatusCode":401}},"additionalDetails":"unexpected status 401 Unauthorized"},"willRetry":true,"threadId":"01a0222d-48c6-7ef2-a31f-81ae86c06141","turnId":"01a0222d-4a40-7610-b7bb-a7c51f6b0d2e"}"#;

    /// Cell `auth-failure`, seq 56 — the real one. Note the message lives at
    /// `error.message`, not at `message` as the previous fold assumed.
    const ERROR_FATAL: &str = r#"{"error":{"message":"unexpected status 401 Unauthorized: Missing bearer or basic authentication in header","codexErrorInfo":"other","additionalDetails":null},"willRetry":false,"threadId":"01a0222d-48c6-7ef2-a31f-81ae86c06141","turnId":"01a0222d-4a40-7610-b7bb-a7c51f6b0d2e"}"#;

    /// Cell `turn-tools`, seq 103 — a completed shell call.
    const COMMAND_COMPLETED: &str = r#"{"item":{"type":"commandExecution","id":"exec-92692a55-23f9-4f5d-b5bc-adcc985c7fc6","command":"/bin/bash -lc \"node -e 'x'\"","cwd":"/tmp/bb-recording-ws","processId":"13711","status":"completed","aggregatedOutput":"2\n","exitCode":0,"durationMs":0},"threadId":"01a021e4-fe50-7c91-abf4-c384358c1d6f","turnId":"01a021e4-ff0a-7bf3-8023-962ba8347adf","completedAtMs":1787275067926}"#;

    fn frame(raw: &str) -> Value {
        serde_json::from_str(raw).expect("recorded frame must parse")
    }

    fn delta_frame(item_id: &str, delta: &str) -> Value {
        json!({
            "threadId": "01a021e4-fe50-7c91-abf4-c384358c1d6f",
            "turnId": "01a021e4-ff0a-7bf3-8023-962ba8347adf",
            "itemId": item_id,
            "delta": delta,
        })
    }

    /// (a) The deltas reconstruct the completed text exactly, and the completed
    /// frame therefore emits nothing. Appending it would print the answer twice.
    #[test]
    fn deltas_reconstruct_the_completed_text_and_it_is_not_re_emitted() {
        let mut d = CodexDecoder::new();
        d.push("item/started", &frame(STARTED_COMMENTARY));

        let mut streamed = String::new();
        for delta in COMMENTARY_DELTAS {
            let events = d.push(
                "item/agentMessage/delta",
                &delta_frame(COMMENTARY_ITEM_ID, delta),
            );
            match events.as_slice() {
                [CodexEvent::TextDelta { item_id, text }] => {
                    assert_eq!(item_id, COMMENTARY_ITEM_ID);
                    streamed.push_str(text);
                }
                other => panic!("a delta must yield exactly one TextDelta, got {other:?}"),
            }
        }
        assert_eq!(
            streamed, COMMENTARY_TEXT,
            "the recorded deltas must concatenate to the recorded final text"
        );

        let events = d.push("item/completed", &frame(COMPLETED_COMMENTARY));
        assert_eq!(
            events,
            vec![CodexEvent::Ignored],
            "a streamed message must not be emitted a second time on completion"
        );
        assert_eq!(d.reconciled(), 1);
        assert_eq!(d.drifted(), 0);
        // The turn's text is still the whole message, exactly once.
        assert_eq!(d.text(), vec![COMMENTARY_TEXT.to_string()]);
        assert_eq!(d.item_phase(COMMENTARY_ITEM_ID), Some("commentary"));
    }

    /// The vendor's bare `no thread with id` names nothing a reader can act on.
    /// It is what Codex returns for its OWN sub-agent tool, which Biorouter
    /// disables on purpose, so the reader needs to be told that and told what to
    /// do instead.
    #[test]
    fn a_disabled_native_tool_error_explains_itself() {
        let out = super::explain_disabled_native_tool("Error: no thread with id abc123");
        assert!(out.starts_with("Error: no thread with id abc123"), "{out}");
        assert!(
            out.contains("Biorouter switches off"),
            "no explanation: {out}"
        );
        assert!(
            out.contains("Ask for the work directly"),
            "no remedy: {out}"
        );
    }

    /// ⚠ Everything else must pass through UNCHANGED. This rewrite is matched on
    /// a vendor substring, and a rewrite that fires too widely would bury real
    /// errors under advice about a tool that had nothing to do with them.
    #[test]
    fn an_unrelated_error_is_left_exactly_alone() {
        for untouched in [
            "rate limit exceeded",
            "stream closed before the turn completed",
            "Unknown feature flag: skill_search",
        ] {
            assert_eq!(super::explain_disabled_native_tool(untouched), untouched);
        }
    }

    /// The other half of the same rule: a message that never streamed is the
    /// only case where the completed frame carries text the consumer has not
    /// seen, so that is the only case that emits `TextComplete`.
    #[test]
    fn an_unstreamed_message_completes_with_its_full_text() {
        let mut d = CodexDecoder::new();
        let events = d.push("item/completed", &frame(COMPLETED_COMMENTARY));
        assert_eq!(
            events,
            vec![CodexEvent::TextComplete {
                item_id: COMMENTARY_ITEM_ID.to_string(),
                text: COMMENTARY_TEXT.to_string(),
            }]
        );
        assert_eq!(d.text(), vec![COMMENTARY_TEXT.to_string()]);
    }

    /// A dropped delta is repaired by emitting only the unseen suffix, never by
    /// re-emitting the whole message.
    #[test]
    fn a_missing_delta_is_repaired_with_the_suffix_only() {
        let mut d = CodexDecoder::new();
        for &delta in &COMMENTARY_DELTAS[..COMMENTARY_DELTAS.len() - 1] {
            d.push(
                "item/agentMessage/delta",
                &delta_frame(COMMENTARY_ITEM_ID, delta),
            );
        }
        let events = d.push("item/completed", &frame(COMPLETED_COMMENTARY));
        assert_eq!(
            events,
            vec![CodexEvent::TextDelta {
                item_id: COMMENTARY_ITEM_ID.to_string(),
                text: ".".to_string(),
            }],
            "only the part nobody has seen may be emitted"
        );
        assert_eq!(d.drifted(), 0);
    }

    /// (b) THE VERIFIED BUG. Two snapshots arrive; the turn's usage is the
    /// second frame's `total`, not its `last`. `last` is per model request, so
    /// reading it at turn end silently undercounts by every earlier request.
    #[test]
    fn usage_is_the_last_snapshots_total_not_its_last() {
        let mut d = CodexDecoder::new();
        d.push("thread/tokenUsage/updated", &frame(TOKEN_USAGE_FIRST));
        let events = d.push("thread/tokenUsage/updated", &frame(TOKEN_USAGE_SECOND));

        let CodexEvent::Usage(snapshot) = &events[0] else {
            panic!("a tokenUsage frame must yield a Usage event, got {events:?}");
        };
        assert_eq!(snapshot.source, UsageSource::ThreadTokenUsage);

        let usage = &snapshot.usage;
        // `total.totalTokens` = 39660 (cumulative over the turn's model
        // requests). `last.totalTokens` = 19893 (the context the last request
        // carried). The two answer different questions and this snapshot needs
        // both: the billed buckets are cumulative, the occupancy gauge is not.
        assert_eq!(
            usage.total_tokens,
            Some(19893),
            "total_tokens is the live context gauge, so it tracks `last` — using \
             the cumulative figure would inflate it by a whole context per tool \
             call and show a roomy session as nearly full"
        );
        // The other half of the split: the BILLED buckets stay cumulative.
        // `total.outputTokens` is 173; `last.outputTokens` is 88. Reading `last`
        // here would undercount the turn by every earlier model request.
        assert_eq!(
            usage.output_tokens,
            Some(173),
            "the billed buckets come from `total` — 88 would be one request's \
             output, not the turn's"
        );
        // Disjoint buckets: 39487 input is cache-inclusive, 38400 of it cached.
        assert_eq!(usage.input_tokens, Some(39487 - 38400));
        assert_eq!(usage.cache_read_input_tokens, Some(38400));
        assert_eq!(usage.cache_creation_input_tokens, Some(0));
        assert_eq!(usage.output_tokens, Some(173));
        assert_eq!(snapshot.context_window, Some(258400));

        // The clean terminal must not overwrite it with a fallback read.
        d.push("turn/completed", &frame(TURN_COMPLETED_OK));
        assert_eq!(
            d.usage().map(|u| u.source),
            Some(UsageSource::ThreadTokenUsage)
        );
        // `total_tokens` is context occupancy, so it tracks `last` (19,893),
        // while the billed buckets below come from the cumulative `total`.
        assert_eq!(d.usage().and_then(|u| u.usage.total_tokens), Some(19893));
        assert_eq!(
            d.usage().and_then(|u| u.usage.output_tokens),
            Some(173),
            "the billed buckets stay cumulative — 173 is total.outputTokens, not \
             last.outputTokens (88)"
        );
    }

    /// (c) The fallback. With no `thread/tokenUsage/updated` at all, the
    /// snake_case usage on `turn/completed` — the shape captured from a live
    /// 0.147.0 app server — still lands, and reports itself as the fallback.
    /// Deleting this read would zero every usage row on that version.
    #[test]
    fn turn_completed_usage_is_the_fallback_when_no_snapshot_arrived() {
        let mut d = CodexDecoder::new();
        let events = d.push(
            "turn/completed",
            &json!({
                "usage": {
                    "input_tokens": 15317,
                    "cached_input_tokens": 9984,
                    "cache_write_input_tokens": 0,
                    "output_tokens": 7
                }
            }),
        );
        let CodexEvent::Usage(snapshot) = &events[0] else {
            panic!("the fallback must still yield a Usage event, got {events:?}");
        };
        assert_eq!(
            snapshot.source,
            UsageSource::TurnCompletedFallback,
            "a test must be able to tell which source won"
        );
        assert_eq!(snapshot.usage.input_tokens, Some(15317 - 9984));
        assert_eq!(snapshot.usage.cache_read_input_tokens, Some(9984));
        assert_eq!(snapshot.usage.output_tokens, Some(7));
        // No totalTokens on this shape, so it is reconstructed.
        assert_eq!(snapshot.usage.total_tokens, Some(15317 + 7));
        assert!(matches!(events[1], CodexEvent::Terminal(_)));
        assert!(d.finished());
    }

    /// (d.1) `turn/completed{status:"failed"}` — unhandled by the previous fold,
    /// which is exactly why an expired login looked like an empty answer.
    #[test]
    fn turn_completed_failed_is_terminal_and_carries_the_message() {
        let mut d = CodexDecoder::new();
        let events = d.push("turn/completed", &frame(TURN_COMPLETED_FAILED));
        let CodexEvent::Terminal(terminal) = events.last().expect("a terminal event") else {
            panic!("expected a terminal event, got {events:?}");
        };
        assert_eq!(terminal.status, CodexTurnStatus::Failed);
        assert!(
            terminal
                .error
                .as_deref()
                .is_some_and(|e| e.contains("401 Unauthorized")),
            "the reason must survive, got {:?}",
            terminal.error
        );
        assert_eq!(
            terminal.turn_id.as_deref(),
            Some("01a0222d-4a40-7610-b7bb-a7c51f6b0d2e"),
            "phase 5 needs a turn id for turn/interrupt"
        );
    }

    /// (d.2) The `turn/failed` dialect still works. It is kept because the fold
    /// deliberately tolerated both the app-server and `codex exec` surfaces.
    #[test]
    fn turn_failed_is_terminal_and_carries_the_message() {
        let mut d = CodexDecoder::new();
        let events = d.push(
            "turn/failed",
            &json!({"error": {"message": "model unavailable"}, "turnId": "t1"}),
        );
        assert_eq!(
            events,
            vec![CodexEvent::Terminal(CodexTerminal {
                status: CodexTurnStatus::Failed,
                error: Some("model unavailable".to_string()),
                turn_id: Some("t1".to_string()),
            })]
        );
        assert!(d.finished());
    }

    /// A clean completion is terminal without an error.
    /// An unrecognised turn status must NOT be reported as success.
    ///
    /// The v2 enum is completed|interrupted|failed|inProgress, and a catch-all
    /// mapping to `Completed` swallowed `inProgress` already — as well as any
    /// value a future Codex adds. That is fail-open on precisely the vendor
    /// drift this module exists to detect: the user would be shown a turn that
    /// ended, with no indication that Biorouter did not understand how.
    #[test]
    fn an_unrecognised_turn_status_is_not_treated_as_success() {
        for unknown in ["inProgress", "cancelled", "moderated"] {
            let mut decoder = CodexDecoder::new();
            let events = decoder.push(
                "turn/completed",
                &serde_json::json!({
                    "threadId": "t-1",
                    "turn": { "id": "turn-1", "status": unknown }
                }),
            );
            let terminal = events
                .iter()
                .find_map(|e| match e {
                    CodexEvent::Terminal(t) => Some(t),
                    _ => None,
                })
                .expect("a terminal event");
            assert_eq!(
                terminal.status,
                CodexTurnStatus::Failed,
                "`{unknown}` is not a status this build understands, so it must \
                 not be presented as a completed turn"
            );
            let message = terminal.error.as_deref().unwrap_or_default();
            assert!(
                message.contains(unknown),
                "the failure must name the status Biorouter did not understand, \
                 so the drift is diagnosable (got {message:?})"
            );
        }
    }

    #[test]
    fn a_clean_turn_completed_is_terminal_without_an_error() {
        let mut d = CodexDecoder::new();
        let events = d.push("turn/completed", &frame(TURN_COMPLETED_OK));
        assert_eq!(
            events,
            vec![CodexEvent::Terminal(CodexTerminal {
                status: CodexTurnStatus::Completed,
                error: None,
                turn_id: Some("01a021e4-ff0a-7bf3-8023-962ba8347adf".to_string()),
            })]
        );
    }

    /// The retry notices are not fatal; only the `willRetry: false` one is
    /// remembered, and the message is read from `error.message`, where the real
    /// frames put it.
    #[test]
    fn retryable_error_notices_do_not_become_the_failure_reason() {
        let mut d = CodexDecoder::new();
        let events = d.push("error", &frame(ERROR_RETRYABLE));
        assert_eq!(
            events,
            vec![CodexEvent::Notice {
                message: "Reconnecting... 2/5".to_string(),
                will_retry: true,
            }]
        );
        assert!(!d.finished(), "an advisory error must not end the turn");
        assert_eq!(d.pending_failure(), None);

        d.push("error", &frame(ERROR_FATAL));
        assert!(d
            .pending_failure()
            .is_some_and(|m| m.contains("401 Unauthorized")));
    }

    /// The other spelling is read too. No recording contains `params.message`,
    /// so it is unproven rather than wrong, and both are accepted.
    #[test]
    fn the_flat_error_message_spelling_is_also_read() {
        let mut d = CodexDecoder::new();
        let events = d.push("error", &json!({"message": "something broke"}));
        assert_eq!(
            events,
            vec![CodexEvent::Notice {
                message: "something broke".to_string(),
                will_retry: false,
            }]
        );
    }

    /// The vendor relays the upstream API's body verbatim, as a STRING, so
    /// `error.as_str()` used to yield the whole envelope and the transcript read
    /// `Ran into this error: Request failed: {"type":"error","status":400,…}.`
    /// This is the real 0.147.0 payload for an unsupported model.
    #[test]
    fn a_json_envelope_relayed_as_a_string_is_unwrapped_to_its_sentence() {
        const ENVELOPE: &str = r#"{"type":"error","status":400,"error":{"type":"invalid_request_error","message":"The 'gpt-6-astra' model requires a newer version of Codex. Please upgrade to the latest app or CLI and try again."}}"#;
        let expected = "The 'gpt-6-astra' model requires a newer version of Codex. Please \
                        upgrade to the latest app or CLI and try again.";

        let mut d = CodexDecoder::new();
        assert_eq!(
            d.push("error", &json!({ "error": ENVELOPE })),
            vec![CodexEvent::Notice {
                message: expected.to_string(),
                will_retry: false,
            }]
        );
        assert_eq!(d.pending_failure(), Some(expected));

        // …and on the terminal frame, which is the one the provider turns into
        // the `ProviderError` the user reads.
        let mut d = CodexDecoder::new();
        assert_eq!(
            d.push("turn/failed", &json!({ "error": { "message": ENVELOPE } })),
            vec![CodexEvent::Terminal(CodexTerminal {
                status: CodexTurnStatus::Failed,
                error: Some(expected.to_string()),
                turn_id: None,
            })]
        );
    }

    /// (e) An unknown method is counted, not fatal.
    #[test]
    fn an_unknown_method_is_counted_without_panicking() {
        let mut d = CodexDecoder::new();
        assert_eq!(
            d.push("thread/somethingNewInVersion9", &json!({"a": 1})),
            vec![CodexEvent::Ignored]
        );
        d.push("thread/somethingNewInVersion9", &json!({}));
        assert_eq!(d.unhandled(), 2);
        assert_eq!(
            d.unhandled_methods().get("thread/somethingNewInVersion9"),
            Some(&2)
        );
        // A method with an arm is never counted as unhandled, even when its
        // payload is dropped.
        d.push("item/completed", &json!({"item": {"type": "reasoning"}}));
        assert_eq!(d.unhandled(), 2);
    }

    /// Malformed frames degrade; they never panic. Each of these is missing
    /// something the schema marks required.
    #[test]
    fn malformed_frames_are_survived() {
        let mut d = CodexDecoder::new();
        for (method, params) in [
            ("item/agentMessage/delta", json!({})),
            ("item/agentMessage/delta", json!({"itemId": 7})),
            ("item/completed", json!({})),
            ("item/completed", json!({"item": "not an object"})),
            ("item/started", json!({"item": {"type": "mcpToolCall"}})),
            ("thread/tokenUsage/updated", json!({"tokenUsage": {}})),
            ("item/reasoning/textDelta", json!({"delta": "orphan"})),
        ] {
            assert_eq!(
                d.push(method, &params),
                vec![CodexEvent::Ignored],
                "{method} with {params} must degrade, not panic"
            );
        }
        let events = d.push("turn/failed", &json!({}));
        let CodexEvent::Terminal(t) = &events[0] else {
            panic!("expected terminal");
        };
        assert_eq!(t.error.as_deref(), Some("the Codex turn failed"));
    }

    /// Reasoning deltas map through. Experimental: unobserved in every recorded
    /// run, so this test pins the schema's shape rather than a captured frame.
    #[test]
    fn both_reasoning_delta_methods_map_to_reasoning_deltas() {
        let mut d = CodexDecoder::new();
        for method in [
            "item/reasoning/summaryTextDelta",
            "item/reasoning/textDelta",
        ] {
            assert_eq!(
                d.push(
                    method,
                    &json!({
                        "itemId": "rs_1",
                        "delta": "considering",
                        "threadId": "t",
                        "turnId": "u"
                    })
                ),
                vec![CodexEvent::ReasoningDelta {
                    item_id: "rs_1".to_string(),
                    text: "considering".to_string(),
                }],
                "{method} must produce a reasoning delta"
            );
        }
        assert_eq!(d.unhandled(), 0);
    }

    /// Tool items decode into parked events carrying the schema's fields, and
    /// produce no text.
    #[test]
    fn tool_items_are_parked_with_their_fields_decoded() {
        let mut d = CodexDecoder::new();

        let events = d.push(
            "item/started",
            &json!({"item": {
                "type": "mcpToolCall",
                "id": "call_1",
                "server": "biorouter",
                "tool": "developer__shell",
                "arguments": {"command": "ls"},
                "status": "inProgress"
            }}),
        );
        let CodexEvent::Tool(tool) = &events[0] else {
            panic!("expected a tool event, got {events:?}");
        };
        assert_eq!(tool.id, "call_1");
        assert_eq!(tool.lifecycle, CodexItemLifecycle::Started);
        assert_eq!(
            tool.kind,
            CodexToolKind::McpToolCall {
                server: "biorouter".to_string(),
                tool: "developer__shell".to_string(),
            }
        );
        assert_eq!(tool.arguments, Some(json!({"command": "ls"})));
        assert_eq!(tool.status.as_deref(), Some("inProgress"));
        assert_eq!(tool.result, None);

        let events = d.push("item/completed", &frame(COMMAND_COMPLETED));
        let CodexEvent::Tool(tool) = &events[0] else {
            panic!("expected a tool event, got {events:?}");
        };
        assert_eq!(tool.lifecycle, CodexItemLifecycle::Completed);
        assert!(matches!(
            tool.kind,
            CodexToolKind::CommandExecution { ref cwd, .. }
                if cwd.as_deref() == Some("/tmp/bb-recording-ws")
        ));
        assert_eq!(tool.aggregated_output.as_deref(), Some("2\n"));
        assert_eq!(tool.exit_code, Some(0));
        assert_eq!(tool.duration_ms, Some(0));
        assert_eq!(tool.status.as_deref(), Some("completed"));

        assert!(d.text().is_empty(), "tool items carry no assistant text");
        assert_eq!(d.unhandled(), 0);
    }

    /// The whole recorded `turn-tools` shape, end to end: two messages in the
    /// order they were written, tools in between, and the cumulative usage.
    #[test]
    fn a_recorded_turn_yields_its_messages_in_order_with_cumulative_usage() {
        let mut d = CodexDecoder::new();
        d.push("item/started", &frame(STARTED_COMMENTARY));
        for delta in COMMENTARY_DELTAS {
            d.push(
                "item/agentMessage/delta",
                &delta_frame(COMMENTARY_ITEM_ID, delta),
            );
        }
        d.push("item/completed", &frame(COMPLETED_COMMENTARY));
        d.push("item/completed", &frame(COMMAND_COMPLETED));
        d.push("thread/tokenUsage/updated", &frame(TOKEN_USAGE_FIRST));
        d.push("thread/tokenUsage/updated", &frame(TOKEN_USAGE_SECOND));

        // The final answer, a single-delta message (recorded seqs 113-117).
        let answer = "msg_0a7eeffd6b544752016a87a73edd5c87d082409fa3559c6f37";
        d.push(
            "item/started",
            &json!({
                "item": {
                    "type": "agentMessage", "id": answer, "text": "", "phase": "final_answer"
                }
            }),
        );
        d.push("item/agentMessage/delta", &delta_frame(answer, "2"));
        d.push(
            "item/completed",
            &json!({
                "item": {
                    "type": "agentMessage", "id": answer, "text": "2", "phase": "final_answer"
                }
            }),
        );
        d.push("turn/completed", &frame(TURN_COMPLETED_OK));

        assert_eq!(d.text(), vec![COMMENTARY_TEXT.to_string(), "2".to_string()]);
        assert_eq!(d.item_phase(answer), Some("final_answer"));
        // `total_tokens` is context occupancy, so it tracks `last` (19,893),
        // while the billed buckets below come from the cumulative `total`.
        assert_eq!(d.usage().and_then(|u| u.usage.total_tokens), Some(19893));
        assert_eq!(
            d.usage().and_then(|u| u.usage.output_tokens),
            Some(173),
            "the billed buckets stay cumulative — 173 is total.outputTokens, not \
             last.outputTokens (88)"
        );
        assert_eq!(d.reconciled(), 2);
        assert_eq!(d.drifted(), 0);
        assert!(d.finished());
    }

    /// Replay the **recorded** Codex cells through the decoder.
    ///
    /// Until this existed the decoder was tested only against hand-written
    /// `json!` frames — i.e. against this file's own idea of the protocol — and
    /// the four committed Codex fixtures were read by nothing at all. A decoder
    /// that agrees with its author but not with the vendor passes every such
    /// test.
    #[test]
    fn recorded_cells_decode_without_unhandled_drift() {
        for (cell, expect_text, expect_terminal) in [
            ("turn-text", true, true),
            ("turn-tools", true, true),
            ("turn-failed", false, true),
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/coding_agent/codex")
                .join(format!("{cell}.ndjson"));
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));

            let mut decoder = CodexDecoder::new();
            let (mut text, mut terminal, mut tools) = (0usize, 0usize, 0usize);

            for line in body.lines().filter(|l| !l.trim().is_empty()) {
                let frame: Value = serde_json::from_str(line).expect("a vendor frame");
                // JSON-RPC responses carry no method; only notifications drive
                // the decoder.
                let Some(method) = frame.get("method").and_then(Value::as_str) else {
                    continue;
                };
                let params = frame.get("params").cloned().unwrap_or(Value::Null);
                for event in decoder.push(method, &params) {
                    match event {
                        CodexEvent::TextDelta { .. } | CodexEvent::TextComplete { .. } => text += 1,
                        CodexEvent::Terminal(_) => terminal += 1,
                        CodexEvent::Tool(_) => tools += 1,
                        _ => {}
                    }
                }
                // Stop at the first terminal, exactly as the provider's pump
                // does. Some recorded cells hold more than one turn, and
                // replaying past the end of the first would measure something
                // production never sees.
                if terminal > 0 {
                    break;
                }
            }

            assert_eq!(
                terminal,
                usize::from(expect_terminal),
                "{cell}: expected exactly one terminal event; a turn that never \
                 terminates leaves the consumer's stream hanging"
            );
            if expect_text {
                assert!(text > 0, "{cell}: the answer must reach the consumer");
            }
            if cell == "turn-tools" {
                assert!(
                    tools > 0,
                    "turn-tools records real commandExecution items; decoding \
                     none means the cards would never appear"
                );
            }
        }
    }

    /// The recorded two-request usage cell, decoded end to end.
    ///
    /// This is the cell that exists specifically for the total-vs-last trap, and
    /// it was previously read by no test.
    #[test]
    fn the_recorded_usage_cell_splits_billed_from_occupancy() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/coding_agent/codex/turn-usage-two-requests.ndjson");
        let body = std::fs::read_to_string(&path).expect("fixture");

        let mut decoder = CodexDecoder::new();
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            let frame: Value = serde_json::from_str(line).expect("a vendor frame");
            let Some(method) = frame.get("method").and_then(Value::as_str) else {
                continue;
            };
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            decoder.push(method, &params);
        }

        let usage = decoder
            .usage()
            .expect("the cell carries four usage updates");
        assert_eq!(usage.source, UsageSource::ThreadTokenUsage);
        // The last snapshot in the cell: total 79676, last 20023.
        assert_eq!(
            usage.usage.total_tokens,
            Some(20023),
            "occupancy is the last request's context, not the turn's cumulative \
             total — the cumulative figure would read as an almost-full window"
        );
    }
}
