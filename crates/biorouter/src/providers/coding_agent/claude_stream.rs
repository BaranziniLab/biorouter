//! Routes one line of `claude --output-format stream-json` output to whoever
//! should see it — and, just as importantly, keeps `tool_use` frames away from
//! the Anthropic SSE decoder.
//!
//! # Why this is a pure, synchronous, push-based router
//!
//! Everything interesting about the Claude Code wire format is a *decision*
//! ("does this frame reach the decoder?"), and every decision here was derived
//! from recorded vendor frames. A router that owned the child process could only
//! be tested by spawning `claude`, which needs the vendor CLI installed and
//! signed in — so the tests that matter would never run in CI and would never be
//! the thing that catches a regression. This module therefore does no IO, spawns
//! nothing, and touches no async runtime: `stream()` owns the child and pushes
//! lines in, the tests push recorded lines in, and both exercise identical code.
//!
//! # The frame taxonomy (Claude Code 2.1.235 / 2.1.238)
//!
//! Each stdout line is one JSON object with a `type`:
//!
//! * `system` — `init` (carries `apiKeySource`), `status`, `api_retry`,
//!   `thinking_tokens`, hook and task lifecycle. Never model output.
//! * `stream_event` — wraps a **raw Anthropic Messages-API event** in `.event`.
//!   This is the only frame that carries live deltas, and it appears only when
//!   the child is started with `--include-partial-messages`.
//! * `assistant` / `user` — the vendor's own message-level view. Structurally
//!   duplicated: one `assistant` frame **per content block**, sharing
//!   `message.id`, arriving *before* that block's `content_block_stop`.
//! * `result` — terminal; authoritative usage plus the failure classification.
//! * `control_request` / `control_response` / `command_lifecycle` /
//!   `rate_limit_event` — the bidirectional control protocol and its
//!   book-keeping. Known, and deliberately uninteresting here.
//!
//! # Rule 1 (load-bearing): no `tool_use` event may reach the decoder
//!
//! [`crate::providers::formats::anthropic::response_to_streaming_message`] is
//! not a passive text assembler for tool blocks. Its §6.2b flush
//! (`flush_pending_tool_contents`, `providers/formats/anthropic.rs:546-556`,
//! flushed at `:840-846` and again after the loop at `:958-960`) mints a single
//! batched assistant `Message` full of **unmarked** `ToolRequest`s, and it emits
//! its own `PendingToolCall`s (`:680-689`).
//!
//! An unmarked `ToolRequest` reaching the agent loop is *dispatched*
//! (`agents/agent.rs:7188` `categorize_tools`; `agents/reply_parts.rs:357-361`
//! filters on message content only, never on metadata). With the
//! `mcp__biorouter__` prefix intact that is a "Tool not found" error row per
//! call; with the prefix stripped it is a genuine **second execution** of a call
//! the tool bridge already ran. So the diversion is a correctness requirement
//! from phase 1 onward, long before the phase-3 provider-executed marker exists.
//!
//! The diversion is tracked **by block index**, because the delta and stop
//! frames name only `index` — never the block's type. [`ClaudeStreamRouter`]
//! records which indices opened as `tool_use` and diverts every later frame that
//! names one.
//!
//! Only `tool_use` is diverted, and only `tool_use` needs to be: a census of all
//! 14 recorded cells finds exactly three content-block types on the wire (31
//! `tool_use`, 29 `text`, 4 `thinking`), and the decoder mints a `ToolRequest`
//! for `tool_use` alone. A future server-side block type (`server_tool_use`,
//! `web_search_tool_result`) forwarded here would be ignored by the decoder
//! rather than dispatched — it would show as a rise in nothing, which is why the
//! unknown-*event* counter below exists to catch drift the type list cannot.
//!
//! # Rule 2: block indices are scoped to their message, and must be reset
//!
//! A turn contains several API messages, and each restarts its block numbering
//! at 0. In the recorded `approval-allow` cell the first message has `tool_use`
//! at index 0 and the second has **text** at index 0. A router that never
//! cleared its diverted-index set would silently swallow the whole answer. The
//! set is therefore cleared on every `message_start`.
//!
//! # Rule 3: `message_stop` is never forwarded
//!
//! The decoder's `message_stop` arm **`break`s out of its read loop**
//! (`providers/formats/anthropic.rs:919-943`). One decoder instance therefore
//! decodes exactly one message — but a Claude Code turn emits one
//! `message_start`/`message_stop` pair per API request (three in the recorded
//! `turn-tools` cell). Forwarding `message_stop` would end the decoder at the
//! first tool round-trip and drop every later token. Nothing is lost by
//! swallowing it: in every recording `message_stop` is `{"type":"message_stop"}`
//! with no usage, `message_delta` already yields a running usage snapshot, and
//! the terminal `result` frame carries the authoritative figure.
//!
//! # Rule 4: pick one source of truth per block — the deltas win
//!
//! The `assistant` frame repeats a block the deltas already delivered (verified:
//! concatenated `text_delta`s equal the frame's text, and `result.result` is the
//! same text a third time). Emitting both would double the answer, so a
//! text/thinking `assistant` frame is dropped once deltas for that message have
//! been seen. If deltas were never seen at all — a child started without
//! `--include-partial-messages`, or a turn that failed before streaming — the
//! caller can fall back to [`TerminalFrame::final_text`]; ask
//! [`ClaudeStreamRouter::streamed_any_text`] rather than guessing.
//!
//! # Rule 5: thinking signatures are blanked
//!
//! `signature_delta` frames are dropped, so a thinking block reaches the decoder
//! with an empty signature and the signed-turn persistence branch
//! (`agents/agent.rs:7582-7641`) never engages. This provider's history is
//! flattened to plain text for the next turn anyway
//! (`providers/coding_agent/transcript.rs:51`), so a stored signature could only
//! ever be replayed to an endpoint that never sees it.

use std::collections::HashMap;

use serde_json::Value;

use crate::providers::base::Usage;

/// `system` subtypes this router recognises. A subtype outside this list is
/// counted by [`ClaudeStreamRouter::unhandled`] so vendor drift shows up as a
/// number that went *up*, rather than as silence.
const KNOWN_SYSTEM_SUBTYPES: &[&str] = &[
    "init",
    "status",
    "api_retry",
    "hook_started",
    "hook_response",
    "compact_boundary",
    "thinking_tokens",
    "task_started",
    "task_progress",
    "task_updated",
    "task_notification",
    "tool_progress",
    "tool_use_summary",
    "auth_status",
    "prompt_suggestion",
    "structured_output",
    "mcp_status",
    "user_message_replay",
    "keep_alive",
    // Observed in a real `claude` 2.1.235 capture (the `turn-text` fixture) and
    // added because it was *seen*, not because it was guessed: it closes a turn
    // with a summary Biorouter already has from the `result` frame. It was the
    // one frame in the recorded corpus that tripped the drift counter, which is
    // the counter doing its job.
    "post_turn_summary",
];

/// Top-level frame types that exist and carry nothing this router needs. Listed
/// explicitly so that a genuinely new frame type still increments
/// [`ClaudeStreamRouter::unhandled`].
const KNOWN_INERT_FRAMES: &[&str] = &[
    "control_request",
    "control_response",
    "command_lifecycle",
    "rate_limit_event",
];

/// What the router decided to do with one line of the child's stdout.
///
/// Exactly one of these per line: a stdout line is one frame, and a frame has
/// one destination.
#[derive(Debug)]
pub(crate) enum RoutedFrame {
    /// A raw Anthropic Messages-API event, already re-prefixed with `data: ` and
    /// ready to hand to
    /// [`crate::providers::formats::anthropic::response_to_streaming_message`],
    /// which skips any line that does not start with that prefix
    /// (`providers/formats/anthropic.rs:621-626`).
    AnthropicEvent(String),
    /// A `tool_use` content-block event, diverted **away** from the decoder.
    /// Phase 3 turns these into provider-executed tool cards; phase 1 ignores
    /// them, which is why they are parked in a clearly-typed value rather than
    /// dropped on the floor.
    ///
    /// `#[allow(dead_code)]` on the *variant*: a phase-1 caller matches
    /// `Tool(_)`, which makes the payload an unread field and, under the crate's
    /// `-D warnings`, an error. Measured, not assumed.
    #[allow(dead_code)]
    Tool(ToolBlockEvent),
    /// `system/init`, which reports how the child authenticated.
    Init {
        /// `apiKeySource` as the child reported it — `"none"` under subscription
        /// auth. The caller runs the existing refusal
        /// (`providers/claude_code.rs:283-294`) on it; the router deliberately
        /// does not, so that it stays free of provider errors and stays pure.
        api_key_source: Option<String>,
    },
    /// A user message echoed because the child was started with
    /// `--replay-user-messages`. The provider uses this as the vendor-side
    /// acknowledgement that a live steer left stdin and entered Claude's turn
    /// queue.
    UserReplay(String),
    /// The terminal `result` frame.
    Terminal(TerminalFrame),
    /// Nothing for the caller to do: a known-but-inert frame, a duplicate
    /// `assistant`/`user` block, a swallowed `message_stop`, a dropped
    /// `signature_delta`, or an unparseable line (which is also counted).
    Ignored,
}

/// A tool-block event, held for phase 3.
///
/// `#[allow(dead_code)]` on the payloads: phase 1 routes these correctly and
/// then ignores them, so the fields are written and not read until phase 3
/// consumes them. The alternative — omitting the fields until they are read —
/// would mean re-deriving the wire shape later, which is the part that was
/// expensive to get right.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum ToolBlockEvent {
    /// `content_block_start` whose `content_block.type` is `tool_use`. The id
    /// and name are known here; the arguments are not.
    Opened {
        /// The content-block index this call occupies in the current message.
        index: u64,
        /// The vendor call id (`toolu_…`), which the matching `tool_result`
        /// quotes back as `tool_use_id`.
        id: String,
        /// The tool name as the child sees it (still `mcp__biorouter__`-prefixed
        /// for a bridged tool).
        name: String,
    },
    /// A `content_block_delta` naming an index that opened as `tool_use`.
    ///
    /// Diverted on the **index**, whatever the delta's own type: the frame is
    /// part of a tool block, and the decoder must not see any of it.
    ArgsDelta {
        /// The diverted content-block index.
        index: u64,
        /// The call id recorded when the block opened.
        id: String,
        /// The `partial_json` chunk, or an empty string for a delta shape that
        /// does not carry one.
        partial_json: String,
    },
    /// `content_block_stop` for an index that opened as `tool_use`.
    Closed {
        /// The diverted content-block index.
        index: u64,
        /// The call id recorded when the block opened.
        id: String,
    },
    /// An `assistant` frame repeating one or more complete `tool_use` blocks.
    /// This is where the *complete* arguments arrive; the `input_json_delta`
    /// chunks are only a preview.
    Call {
        /// `message.id`, shared by every `assistant` frame of the same API
        /// message.
        message_id: Option<String>,
        /// Every `tool_use` block the frame carried, in wire order.
        calls: Vec<ToolUseBlock>,
    },
    /// A `user` frame carrying one or more `tool_result` blocks — the result of
    /// a call the tool bridge already executed.
    Result {
        /// Every `tool_result` block the frame carried, in wire order.
        results: Vec<ToolResultBlock>,
    },
}

/// One complete `tool_use` block, as the `assistant` frame states it.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct ToolUseBlock {
    /// The vendor call id (`toolu_…`).
    pub(crate) id: String,
    /// The tool name as the child sees it.
    pub(crate) name: String,
    /// The complete arguments object.
    pub(crate) input: Value,
}

/// One `tool_result` block, as the `user` frame states it.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct ToolResultBlock {
    /// The `tool_use` id this result answers.
    pub(crate) tool_use_id: String,
    /// The result body: a string, or an array of content blocks.
    pub(crate) content: Value,
    /// `is_error` lives on the block itself, not on the frame.
    pub(crate) is_error: bool,
    /// The frame's richer sibling `tool_use_result`, which is an **object** on
    /// success (`stdout`/`stderr`/…) and a **string** on failure.
    pub(crate) detail: Option<Value>,
}

/// The terminal `result` frame, parsed.
///
/// `#[allow(dead_code)]` for the same reason as [`ToolBlockEvent`]: the raw
/// discriminators are parsed here so that a caller diagnosing a failure never
/// has to re-parse the frame, and phase 1 reads only some of them.
#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct TerminalFrame {
    /// `result` — the final answer text, repeated here for the third time.
    /// Useful as a fallback when no partial messages were streamed; see
    /// [`ClaudeStreamRouter::streamed_any_text`].
    pub(crate) final_text: String,
    /// The authoritative usage for the whole turn (the sum over its API calls),
    /// in Biorouter's four disjoint buckets.
    pub(crate) usage: Usage,
    /// `Some` when this turn failed. See [`classify_result`] for why this is not
    /// simply `subtype != "success"`.
    pub(crate) error: Option<TerminalError>,
    /// `subtype`, verbatim. Kept because it is *not* trustworthy on its own and
    /// a caller may want to log what it actually said.
    pub(crate) subtype: Option<String>,
    /// `terminal_reason` — `"completed"` on success, `"api_error"` and friends
    /// otherwise. The most reliable discriminator on the frame.
    pub(crate) terminal_reason: Option<String>,
    /// `api_error_status`, when the CLI attributes the failure to an HTTP status.
    pub(crate) api_error_status: Option<String>,
    /// `stop_reason` (`end_turn`, `tool_use`, `stop_sequence`, …). `null` on a
    /// result the CLI synthesised after a crash.
    pub(crate) stop_reason: Option<String>,
}

/// A classified terminal failure.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct TerminalError {
    /// A category the caller can hand to the provider's existing error mapper
    /// (`providers/claude_code.rs::classify`). Never the literal `"success"` —
    /// see [`classify_result`].
    pub(crate) category: Option<String>,
    /// The user-facing detail, taken from `result` when it says anything.
    pub(crate) detail: String,
}

/// Routes `stream-json` lines. Pure: push a line in, get a decision out.
///
/// State is per-turn, and the parts scoped to one API message are reset on every
/// `message_start` (see the module docs, rule 2).
#[derive(Debug, Default)]
pub(crate) struct ClaudeStreamRouter {
    /// Content-block indices in the **current message** that opened as
    /// `tool_use`, mapped to their call id. This is the whole diversion
    /// mechanism: delta and stop frames name only an index.
    tool_indices: HashMap<u64, String>,
    /// Whether a text or thinking delta has been seen in the current message,
    /// which is what makes a later `assistant` frame a duplicate.
    saw_block_delta: bool,
    /// Whether any text or thinking delta reached the decoder in this turn.
    streamed_any_text: bool,
    /// `apiKeySource` from `system/init`, kept so a caller that missed the
    /// `Init` frame can still ask.
    api_key_source: Option<String>,
    /// The last `system/api_retry` category, used as a fallback classification
    /// when the terminal frame is unhelpful (mirrors
    /// `providers/claude_code.rs:426-437`).
    retry_category: Option<String>,
    /// Frames this router did not recognise. Only ever asserted downward, in the
    /// spirit of bb's `row-counts.json` discipline.
    unhandled: usize,
    /// Duplicate text/thinking blocks dropped under rule 4.
    dropped_duplicate_blocks: usize,
    /// Text/thinking blocks that arrived on an `assistant` frame with no deltas
    /// preceding them — i.e. blocks whose only source of truth was dropped. A
    /// nonzero count in a normal run means `--include-partial-messages` did not
    /// take effect.
    unmirrored_blocks: usize,
    /// Tool events diverted away from the decoder. The number this phase most
    /// wants to be able to prove is nonzero.
    diverted_tool_events: usize,
}

impl ClaudeStreamRouter {
    /// A router for one child process. Stream-json steering can put several
    /// vendor turns in that process; per-message state resets on each
    /// `message_start`, while the counters intentionally describe the whole run.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Route one line of the child's stdout.
    ///
    /// Never panics and never errors: a malformed or unknown line is counted and
    /// skipped, because a vendor that adds a frame type mid-release must not be
    /// able to kill a turn.
    pub(crate) fn push_line(&mut self, line: &str) -> RoutedFrame {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return RoutedFrame::Ignored;
        }
        let Ok(frame) = serde_json::from_str::<Value>(trimmed) else {
            // Not JSON at all. The CLI writes diagnostics to stderr, so this is
            // either truncation or drift; either way it is data, not a crash.
            self.unhandled += 1;
            return RoutedFrame::Ignored;
        };
        match frame.get("type").and_then(Value::as_str) {
            Some("stream_event") => self.route_stream_event(&frame),
            Some("assistant") => self.route_assistant(&frame),
            Some("user") => self.route_user(&frame),
            Some("system") => self.route_system(&frame),
            Some("result") => self.route_result(&frame),
            Some(other) if KNOWN_INERT_FRAMES.contains(&other) => RoutedFrame::Ignored,
            _ => {
                self.unhandled += 1;
                RoutedFrame::Ignored
            }
        }
    }

    /// Re-serialise an Anthropic event and prefix it for the decoder.
    ///
    /// Re-serialising rather than slicing the original line keeps this honest
    /// about what the decoder receives: exactly `.event`, nothing of the Claude
    /// Code envelope (`session_id`, `uuid`, `parent_tool_use_id`, `ttft_ms`).
    fn forward(&mut self, event: &Value) -> RoutedFrame {
        match serde_json::to_string(event) {
            Ok(json) => RoutedFrame::AnthropicEvent(format!("data: {json}")),
            Err(_) => {
                self.unhandled += 1;
                RoutedFrame::Ignored
            }
        }
    }

    /// Record a diverted tool event.
    fn divert(&mut self, event: ToolBlockEvent) -> RoutedFrame {
        self.diverted_tool_events += 1;
        RoutedFrame::Tool(event)
    }

    /// Unwrap a `stream_event` and route the Anthropic event inside it.
    fn route_stream_event(&mut self, frame: &Value) -> RoutedFrame {
        let Some(event) = frame.get("event") else {
            self.unhandled += 1;
            return RoutedFrame::Ignored;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.begin_message();
                self.forward(event)
            }
            Some("content_block_start") => self.route_block_start(event),
            Some("content_block_delta") => self.route_block_delta(event),
            Some("content_block_stop") => self.route_block_stop(event),
            Some("message_delta") => self.forward(event),
            // Rule 3: forwarding this ends the decoder at the first message.
            Some("message_stop") => RoutedFrame::Ignored,
            Some("ping") => RoutedFrame::Ignored,
            _ => {
                self.unhandled += 1;
                RoutedFrame::Ignored
            }
        }
    }

    /// Reset the per-message state (rule 2).
    fn begin_message(&mut self) {
        self.tool_indices.clear();
        self.saw_block_delta = false;
    }

    /// `content_block_start`: the only frame that says what a block *is*, and
    /// therefore the only place an index can be marked for diversion.
    fn route_block_start(&mut self, event: &Value) -> RoutedFrame {
        let index = block_index(event);
        let Some(block) = event.get("content_block") else {
            return self.forward(event);
        };
        if type_field(block) != "tool_use" {
            return self.forward(event);
        }
        // Even a malformed tool block must be diverted — reaching the decoder is
        // what causes the double execution, and an id-less block would still
        // arrive there as a `ToolRequest`.
        let id = string_field(block, "id");
        let name = string_field(block, "name");
        self.tool_indices.insert(index, id.clone());
        self.divert(ToolBlockEvent::Opened { index, id, name })
    }

    /// `content_block_delta`: diverted on index, otherwise forwarded — except
    /// `signature_delta`, which is dropped so thinking arrives unsigned (rule 5).
    fn route_block_delta(&mut self, event: &Value) -> RoutedFrame {
        let index = block_index(event);
        if let Some(id) = self.tool_indices.get(&index).cloned() {
            let partial_json = event
                .get("delta")
                .map(|d| string_field(d, "partial_json"))
                .unwrap_or_default();
            return self.divert(ToolBlockEvent::ArgsDelta {
                index,
                id,
                partial_json,
            });
        }
        let delta_type = event.get("delta").map(type_field).unwrap_or_default();
        if delta_type == "signature_delta" {
            return RoutedFrame::Ignored;
        }
        if matches!(delta_type, "text_delta" | "thinking_delta") {
            self.saw_block_delta = true;
            self.streamed_any_text = true;
        }
        self.forward(event)
    }

    /// `content_block_stop`: diverted on index. The index stays in the diverted
    /// set until the next `message_start` — a repeated or late stop for a
    /// settled tool block must not suddenly become a decoder event.
    fn route_block_stop(&mut self, event: &Value) -> RoutedFrame {
        let index = block_index(event);
        if let Some(id) = self.tool_indices.get(&index).cloned() {
            return self.divert(ToolBlockEvent::Closed { index, id });
        }
        self.forward(event)
    }

    /// An `assistant` frame: tool blocks are parked, text/thinking blocks are
    /// duplication (rule 4).
    fn route_assistant(&mut self, frame: &Value) -> RoutedFrame {
        if is_subagent(frame) {
            // Subagent output is the child's own inner loop. Biorouter runs with
            // `--tools ""` so it should never appear; filtered defensively
            // because attributing it to this turn would be wrong.
            return RoutedFrame::Ignored;
        }
        let Some(blocks) = content_blocks(frame) else {
            return RoutedFrame::Ignored;
        };
        let calls: Vec<ToolUseBlock> = blocks
            .iter()
            .filter(|b| type_field(b) == "tool_use")
            .map(|b| ToolUseBlock {
                id: string_field(b, "id"),
                name: string_field(b, "name"),
                input: b.get("input").cloned().unwrap_or(Value::Null),
            })
            .collect();
        if !calls.is_empty() {
            let message_id = frame.get("message").and_then(|m| opt_string(m, "id"));
            return self.divert(ToolBlockEvent::Call { message_id, calls });
        }
        let text_like = blocks
            .iter()
            .filter(|b| matches!(type_field(b), "text" | "thinking" | "redacted_thinking"))
            .count();
        if self.saw_block_delta {
            self.dropped_duplicate_blocks += text_like;
        } else {
            self.unmirrored_blocks += text_like;
        }
        RoutedFrame::Ignored
    }

    /// A `user` frame: the `tool_result` half of a call the bridge already ran.
    fn route_user(&mut self, frame: &Value) -> RoutedFrame {
        if is_subagent(frame) {
            return RoutedFrame::Ignored;
        }
        if let Some(text) = frame
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
        {
            return RoutedFrame::UserReplay(text.to_string());
        }
        let Some(blocks) = content_blocks(frame) else {
            return RoutedFrame::Ignored;
        };
        let results: Vec<ToolResultBlock> = blocks
            .iter()
            .filter(|b| type_field(b) == "tool_result")
            .map(|b| ToolResultBlock {
                tool_use_id: string_field(b, "tool_use_id"),
                content: b.get("content").cloned().unwrap_or(Value::Null),
                // `is_error` lives on the block, not on the frame.
                is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                detail: frame.get("tool_use_result").cloned(),
            })
            .collect();
        if results.is_empty() {
            return RoutedFrame::Ignored;
        }
        self.divert(ToolBlockEvent::Result { results })
    }

    /// A `system` frame. Only `init` and `api_retry` say anything a turn needs.
    fn route_system(&mut self, frame: &Value) -> RoutedFrame {
        match frame.get("subtype").and_then(Value::as_str) {
            Some("init") => {
                let api_key_source = opt_string(frame, "apiKeySource");
                self.api_key_source.clone_from(&api_key_source);
                RoutedFrame::Init { api_key_source }
            }
            Some("api_retry") => {
                self.retry_category = opt_string(frame, "error");
                RoutedFrame::Ignored
            }
            Some(other) if KNOWN_SYSTEM_SUBTYPES.contains(&other) => RoutedFrame::Ignored,
            _ => {
                self.unhandled += 1;
                RoutedFrame::Ignored
            }
        }
    }

    /// The terminal `result` frame.
    fn route_result(&mut self, frame: &Value) -> RoutedFrame {
        let terminal = TerminalFrame {
            final_text: string_field(frame, "result"),
            usage: parse_usage(frame.get("usage")),
            error: classify_result(frame, self.retry_category.as_deref()),
            subtype: opt_string(frame, "subtype"),
            terminal_reason: opt_string(frame, "terminal_reason"),
            api_error_status: opt_string(frame, "api_error_status"),
            stop_reason: opt_string(frame, "stop_reason"),
        };
        RoutedFrame::Terminal(terminal)
    }
}

/// Read-only observers: what the router noticed while routing.
///
/// A separate `impl` block so that one `#[allow(dead_code)]` can cover the whole
/// group with one explanation. These are the diagnostic surface — the counters a
/// drift test asserts on, and the two fallbacks a caller needs only in the
/// unhappy path — so several of them have no caller in the lib build until the
/// later phases land. Deliberately NOT extended to [`ClaudeStreamRouter::new`]
/// and [`ClaudeStreamRouter::push_line`]: if *those* ever go unused, the module
/// is not wired in and the warning is the report.
#[allow(dead_code)]
impl ClaudeStreamRouter {
    /// Frames this router did not recognise, over the whole turn.
    ///
    /// The drift signal. In the spirit of bb's `row-counts.json` discipline, a
    /// test should only ever be allowed to lower this number: a vendor release
    /// that adds a frame type shows up as a count that went up, rather than as
    /// output that quietly stopped being routed.
    pub(crate) fn unhandled(&self) -> usize {
        self.unhandled
    }

    /// Whether any text or thinking delta was forwarded to the decoder.
    ///
    /// `false` at the end of a turn means nothing was streamed and the caller
    /// should fall back to [`TerminalFrame::final_text`] rather than yielding an
    /// empty answer (see rule 4 in the module docs).
    pub(crate) fn streamed_any_text(&self) -> bool {
        self.streamed_any_text
    }

    /// `apiKeySource` as reported by `system/init`, if it has arrived.
    pub(crate) fn api_key_source(&self) -> Option<&str> {
        self.api_key_source.as_deref()
    }

    /// The last `system/api_retry` error category, for classifying a turn whose
    /// terminal frame never arrived.
    pub(crate) fn retry_category(&self) -> Option<&str> {
        self.retry_category.as_deref()
    }

    /// Duplicate text/thinking blocks dropped under rule 4.
    pub(crate) fn dropped_duplicate_blocks(&self) -> usize {
        self.dropped_duplicate_blocks
    }

    /// Text/thinking blocks whose deltas never arrived (see
    /// [`Self::streamed_any_text`]).
    pub(crate) fn unmirrored_blocks(&self) -> usize {
        self.unmirrored_blocks
    }

    /// Tool events kept away from the Anthropic decoder in this turn.
    pub(crate) fn diverted_tool_events(&self) -> usize {
        self.diverted_tool_events
    }
}

/// The content-block index a `content_block_*` event names.
///
/// Defaults to 0 rather than erroring: an event with no index is malformed, and
/// treating it as block 0 keeps a diverted tool block diverted, which is the
/// failure that matters.
fn block_index(event: &Value) -> u64 {
    event.get("index").and_then(Value::as_u64).unwrap_or(0)
}

/// A frame produced by one of the child's own subagents, which
/// `parent_tool_use_id` identifies (it is `null` for the main session).
fn is_subagent(frame: &Value) -> bool {
    frame
        .get("parent_tool_use_id")
        .is_some_and(|v| !v.is_null())
}

/// The `type` of a content block or a delta, or `""` when it has none.
fn type_field(value: &Value) -> &str {
    value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The `message.content` array of an `assistant` or `user` frame.
///
/// `None` when `content` is not an array — which happens on the post-compaction
/// continuation frame, the one place in the protocol where `content` is a plain
/// string (`system/compact_boundary` precedes it).
fn content_blocks(frame: &Value) -> Option<&Vec<Value>> {
    frame
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
}

/// Read an optional string field.
fn opt_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Read a string field, defaulting to empty rather than dropping the block.
fn string_field(value: &Value, key: &str) -> String {
    opt_string(value, key).unwrap_or_default()
}

/// Decide whether a terminal frame reports a failure, and under what category.
///
/// **`subtype` alone is not the answer.** The recorded auth-failure turn ends
/// with `is_error:true`, `terminal_reason:"api_error"` and `subtype:"success"` —
/// classifying on `subtype` would report that a "Not logged in" turn succeeded,
/// and using it as the error *category* would hand the provider's error mapper
/// the literal `"success"`. So the three fields are consulted together:
/// `is_error`, a `terminal_reason` that is anything other than `"completed"`,
/// and a non-null `api_error_status` each independently mean failure.
fn classify_result(frame: &Value, retry_category: Option<&str>) -> Option<TerminalError> {
    let is_error = frame.get("is_error").and_then(Value::as_bool) == Some(true);
    let terminal_reason = frame.get("terminal_reason").and_then(Value::as_str);
    let bad_terminal_reason = terminal_reason.is_some_and(|r| !r.is_empty() && r != "completed");
    let api_error_status = frame
        .get("api_error_status")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if !is_error && !bad_terminal_reason && api_error_status.is_none() {
        return None;
    }

    // Ordered by how much each field is worth: `terminal_reason` is the
    // discriminator the CLI actually maintains, an `error_*` subtype is the
    // documented category, and the retry category is what the blocking parser
    // already falls back to (`providers/claude_code.rs:426-437`).
    let subtype = frame
        .get("subtype")
        .and_then(Value::as_str)
        .filter(|s| s.starts_with("error"));
    let category = terminal_reason
        .filter(|r| !r.is_empty() && *r != "completed")
        .or(subtype)
        .or(api_error_status)
        .or(retry_category)
        .map(str::to_string);

    // Through `unwrap_json_error`: `result` carries the vendor's own text, which
    // is usually a sentence (`API Error: 400 Claude Code 2.1.235 does not
    // support this model…`) but is sometimes the upstream API's JSON body
    // verbatim. Unwrapping is a no-op on the sentence and is what keeps the
    // envelope out of the transcript.
    let detail = frame
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(super::unwrap_json_error)
        .unwrap_or_else(|| "`claude` reported an error".to_string());

    Some(TerminalError { category, detail })
}

/// Pull Biorouter's four disjoint token buckets out of the terminal frame.
///
/// Deliberately identical to `providers/claude_code.rs::parse_usage`, which is
/// private to that module: Claude Code reports the Anthropic API's shape, where
/// `input_tokens` already **excludes** both cache buckets — exactly the
/// invariant [`Usage`] documents — so nothing is subtracted here. `total_tokens`
/// is context occupancy for the live gauge (cache included), not the billed sum.
fn parse_usage(usage: Option<&Value>) -> Usage {
    let Some(u) = usage else {
        return Usage::default();
    };
    let get = |k: &str| -> Option<i32> {
        let raw = u.get(k).and_then(Value::as_i64)?;
        // Saturating rather than wrapping: a bogus count must not turn into a
        // negative token figure that the usage ledger then sums.
        Some(i32::try_from(raw).unwrap_or(i32::MAX))
    };

    let input = get("input_tokens");
    let output = get("output_tokens");
    let cache_read = get("cache_read_input_tokens");
    let cache_creation = get("cache_creation_input_tokens");

    Usage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: match (input, output) {
            (None, None) => None,
            _ => Some(
                input.unwrap_or(0)
                    + output.unwrap_or(0)
                    + cache_read.unwrap_or(0)
                    + cache_creation.unwrap_or(0),
            ),
        },
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: cache_creation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // Real recorded frames.
    //
    // Every literal below is a verbatim line from bb's vendor recordings
    // (`packages/provider-bridge-protocol/recordings/claude-code/<cell>/
    // provider→bridge.ndjson`, Claude Code 2.1.238), taken from the `line`
    // field of the NDJSON envelope. Three are trimmed, each marked TRIMMED:
    // `system/init` and the two `result` frames drop long arrays that no code
    // path here reads, and the thinking signatures are truncated. Nothing that
    // the router keys on was altered.
    //
    // They are inline consts rather than files on disk because a unit test that
    // reads a path is a test that can fail for a reason unrelated to the code.
    // ---------------------------------------------------------------------

    /// approval-allow line 9. TRIMMED: `tools`, `mcp_servers`, `slash_commands`,
    /// `agents`, `skills`, `plugins`, `capabilities`, `memory_paths` elided.
    const INIT: &str = r#"{"type":"system","subtype":"init","cwd":"/tmp/bb-recording-ws","session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","model":"claude-opus-5[1m]","permissionMode":"acceptEdits","apiKeySource":"none","claude_code_version":"2.1.238","output_style":"default","uuid":"d3924fd0-9275-499d-8d5f-75ae46d04e32","fast_mode_state":"off"}"#;

    /// approval-allow line 10.
    const STATUS: &str = r#"{"type":"system","subtype":"status","status":"requesting","uuid":"12eaba5f-6e01-471e-a331-211b8d06a145","session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade"}"#;

    /// approval-allow line 24.
    const RATE_LIMIT: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1787288400,"rateLimitType":"five_hour","overageStatus":"allowed","overageResetsAt":1788220800,"isUsingOverage":false},"uuid":"e6dbda08-889d-4a99-a261-a39b4eed46c8","session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade"}"#;

    // --- The tool round-trip: approval-allow lines 11-26. ------------------

    /// approval-allow line 11 — the tool message opens.
    const TOOL_MESSAGE_START: &str = r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-opus-5","id":"msg_011CeF2Cvx5RGdqcuKQjJqLv","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"stop_details":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":11300,"cache_read_input_tokens":17489,"output_tokens":17,"service_tier":"standard"},"diagnostics":null}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"186121e2-19b8-46ee-bc0e-5a0e8d2a2387","ttft_ms":1269}"#;

    /// approval-allow line 12 — `tool_use` at index 0.
    const TOOL_BLOCK_START: &str = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01DS65QaNoiWEyuRBSzMgdvT","name":"Bash","input":{},"caller":{"type":"direct"}}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"21d80f32-8ce2-4b0b-98e8-0aa54e2b2d92"}"#;

    /// approval-allow line 13 — the first chunk is empty, and names only `index`.
    const TOOL_ARGS_DELTA_EMPTY: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"bd2011d9-5c46-43c7-a281-59895dbbc892"}"#;

    /// approval-allow line 14.
    const TOOL_ARGS_DELTA_BODY: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"curl -sI https://example.com | head -n 1"}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"8ea3cb38-199c-4213-8465-d1c41212f702"}"#;

    /// approval-allow line 16.
    const TOOL_ARGS_DELTA_TAIL: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"}"}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"0039f112-6a9d-4961-bdf3-c0cb062f5418"}"#;

    /// approval-allow line 17 — the complete call, arriving BEFORE its stop.
    const TOOL_ASSISTANT_FRAME: &str = r#"{"type":"assistant","message":{"model":"claude-opus-5","id":"msg_011CeF2Cvx5RGdqcuKQjJqLv","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_01DS65QaNoiWEyuRBSzMgdvT","name":"Bash","input":{"command":"curl -sI https://example.com | head -n 1","description":"Fetch HTTP status line from example.com"},"caller":{"type":"direct"}}],"stop_reason":null,"usage":{"input_tokens":2,"output_tokens":17}},"parent_tool_use_id":null,"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","uuid":"6fa8c6c9-11d1-404c-923a-86011a1c553b","timestamp":"2026-08-21T01:25:07.446Z","request_id":"req_011CeF2CuynHwtYFs11dJxTY"}"#;

    /// approval-allow line 19 — names only `index`.
    const TOOL_BLOCK_STOP: &str = r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"2f4d0884-f2c4-4cf0-afb5-167ac04a958a"}"#;

    /// approval-allow line 22.
    const TOOL_MESSAGE_DELTA: &str = r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null,"stop_details":null},"usage":{"input_tokens":2,"cache_creation_input_tokens":11300,"cache_read_input_tokens":17489,"output_tokens":96}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"ec147f2c-b522-4b71-9518-427084d8925d"}"#;

    /// approval-allow line 23.
    const TOOL_MESSAGE_STOP: &str = r#"{"type":"stream_event","event":{"type":"message_stop"},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"2a987f9f-2ed9-4845-8855-dece14d18e8d"}"#;

    /// approval-allow line 26 — the result, keyed by `tool_use_id`.
    const TOOL_RESULT_FRAME: &str = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_01DS65QaNoiWEyuRBSzMgdvT","type":"tool_result","content":"HTTP/2 200","is_error":false}]},"parent_tool_use_id":null,"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","uuid":"c980f43a-9e7c-42e9-bac9-a1090698d6c9","timestamp":"2026-08-21T01:27:38.733Z","tool_use_result":{"stdout":"HTTP/2 200","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false}}"#;

    // --- The text message that follows it: approval-allow lines 28-35. -----

    /// approval-allow line 28 — a NEW message; block numbering restarts at 0.
    const TEXT_MESSAGE_START: &str = r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-opus-5","id":"msg_011CeF2QGT9T5rrYZZJBoqi8","type":"message","role":"assistant","content":[],"stop_reason":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":1234,"cache_read_input_tokens":28789,"output_tokens":1}},"diagnostics":null},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"8bd1a2c0-fe08-43db-ac66-9c703b731da4","ttft_ms":959}"#;

    /// approval-allow line 29 — `text` at index 0, the same index the tool used.
    const TEXT_BLOCK_START: &str = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"f1657c54-d620-439b-9572-7c6c66a379e7"}"#;

    /// approval-allow line 30.
    const TEXT_DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"bad4e3a6-1cff-46a4-abae-3d590bc268d9"}"#;

    /// approval-allow line 31 — the same "done" a second time.
    const TEXT_ASSISTANT_FRAME: &str = r#"{"type":"assistant","message":{"model":"claude-opus-5","id":"msg_011CeF2QGT9T5rrYZZJBoqi8","type":"message","role":"assistant","content":[{"type":"text","text":"done"}],"stop_reason":null,"usage":{"input_tokens":2,"output_tokens":1}},"parent_tool_use_id":null,"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","uuid":"8f7e1215-4f9a-4ca7-9d09-548fe4323196","timestamp":"2026-08-21T01:27:39.771Z","request_id":"req_011CeF2QEpAgxBT3KUkwWArE"}"#;

    /// approval-allow line 32.
    const TEXT_BLOCK_STOP: &str = r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"9a027b61-6756-44d6-8044-40461ef11629"}"#;

    /// approval-allow line 33.
    const TEXT_MESSAGE_DELTA: &str = r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null,"stop_details":null},"usage":{"input_tokens":2,"cache_creation_input_tokens":1234,"cache_read_input_tokens":28789,"output_tokens":3}},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"1836998d-520a-4d31-b12b-042235d2347c"}"#;

    /// approval-allow line 34.
    const TEXT_MESSAGE_STOP: &str = r#"{"type":"stream_event","event":{"type":"message_stop"},"session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","parent_tool_use_id":null,"uuid":"1896cff2-4cc0-4a76-9bd7-0802af5c77bf"}"#;

    /// approval-allow line 35. TRIMMED: `modelUsage`, `subagent_stats`,
    /// `permission_denials` and `usage.iterations` elided.
    const RESULT_SUCCESS: &str = r#"{"is_error":false,"duration_api_ms":3498,"num_turns":2,"stop_reason":"end_turn","session_id":"0e70de90-9b5d-47f6-96b1-1d2711dbcade","total_cost_usd":0.150974,"usage":{"input_tokens":4,"cache_creation_input_tokens":12534,"cache_read_input_tokens":46278,"output_tokens":99,"output_tokens_details":{"thinking_tokens":0},"service_tier":"standard"},"terminal_reason":"completed","subtype":"success","api_error_status":null,"result":"done","type":"result","duration_ms":154793,"uuid":"96e3cc49-c0fd-4a25-b2c0-a7c1bc0feb93"}"#;

    /// auth-failure line 10. TRIMMED as above. `is_error:true` WITH
    /// `subtype:"success"` — the frame this router's classifier exists for.
    const RESULT_AUTH_FAILURE: &str = r#"{"is_error":true,"duration_api_ms":0,"num_turns":1,"stop_reason":"stop_sequence","session_id":"54386450-a88e-4d9e-b4e1-8893a0c47239","total_cost_usd":0,"usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0},"terminal_reason":"api_error","subtype":"success","api_error_status":null,"result":"Not logged in · Please run /login","type":"result","duration_ms":80,"uuid":"43e6aa78-7336-4803-a034-ab74e042e620"}"#;

    // --- A mixed message: plan-mode lines 25-42 (thinking @0, tool_use @1). --

    /// plan-mode line 25.
    const PM_MESSAGE_START: &str = r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-opus-5","id":"msg_011CeF6vcuWRjAkz9NP8Z8rq","type":"message","role":"assistant","content":[],"stop_reason":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":150,"cache_read_input_tokens":31048,"output_tokens":2}}},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"e7d29394-3c16-4721-907f-0c4915f2d5c7","ttft_ms":722}"#;

    /// plan-mode line 26.
    const PM_THINKING_START: &str = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"5d50fbfd-aa17-4d29-8920-8fc7d07fc0be"}"#;

    /// plan-mode line 27 — interleaved between thinking deltas.
    const PM_THINKING_TOKENS: &str = r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":1,"estimated_tokens_delta":1,"uuid":"33a67abe-dc3a-43d4-a3bf-15abd37f4396","session_id":"712e74da-9f7d-46ff-8b4b-1da157824717"}"#;

    /// plan-mode line 28.
    const PM_THINKING_DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"It","estimated_tokens":null}},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"927532ee-aea1-46a6-8a58-2c7057a2199a"}"#;

    /// plan-mode line 32. TRIMMED: the base64 signature is truncated — the test
    /// asserts the frame is dropped, so its bytes are irrelevant.
    const PM_SIGNATURE_DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"CAISygIKowEIERgCKkCDHpau"}},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"1aa8996c-295e-4e4c-8ba5-2c1dd2536377"}"#;

    /// plan-mode line 34 — the thinking block's stop, at index 0.
    const PM_THINKING_STOP: &str = r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"a4e0d684-85c2-4144-9d4f-d9e8e7981688"}"#;

    /// plan-mode line 35 — `tool_use` at index **1**, in the same message.
    const PM_TOOL_START: &str = r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_019F7agwZAPdwfcZFG8YaFUu","name":"Bash","input":{},"caller":{"type":"direct"}}},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"0bde313d-edd9-43b1-914c-b049a24f5867"}"#;

    /// plan-mode line 37.
    const PM_TOOL_ARGS_DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\": \"cd /tmp/bb-recording-ws && git diff && ls"}},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"943a9aba-a5c7-41eb-a853-b12276fba3e6"}"#;

    /// plan-mode line 42.
    const PM_TOOL_STOP: &str = r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1},"session_id":"712e74da-9f7d-46ff-8b4b-1da157824717","parent_tool_use_id":null,"uuid":"fc190fb1-4521-4eed-9a8f-7cea4fbb6d22"}"#;

    /// The `type` of the Anthropic event inside a forwarded line, or `None` for
    /// anything the router did not forward.
    fn forwarded_event_type(routed: &RoutedFrame) -> Option<String> {
        let RoutedFrame::AnthropicEvent(line) = routed else {
            return None;
        };
        let payload = line
            .strip_prefix("data: ")
            .expect("the decoder skips any line without the `data: ` prefix");
        let event: Value = serde_json::from_str(payload).expect("forwarded a non-JSON payload");
        Some(event["type"].as_str().unwrap_or_default().to_string())
    }

    /// The content-block index a diverted per-block tool event names, or `None`
    /// for anything that is not one.
    fn diverted_index(routed: &RoutedFrame) -> Option<u64> {
        match routed {
            RoutedFrame::Tool(
                ToolBlockEvent::Opened { index, .. }
                | ToolBlockEvent::ArgsDelta { index, .. }
                | ToolBlockEvent::Closed { index, .. },
            ) => Some(*index),
            _ => None,
        }
    }

    /// Push a whole recorded turn through one router.
    fn route_all(lines: &[&str]) -> (ClaudeStreamRouter, Vec<RoutedFrame>) {
        let mut router = ClaudeStreamRouter::new();
        let routed = lines.iter().map(|l| router.push_line(l)).collect();
        (router, routed)
    }

    /// (a) A text turn streams its deltas and ends with the terminal frame.
    ///
    /// The message-level frames (`message_start`, `content_block_start`, the
    /// delta, `content_block_stop`, `message_delta`) all reach the decoder,
    /// because the stable `message_start` id is what keeps every text chunk on
    /// one persisted row rather than one row per delta (`agents/agent.rs:290`).
    #[test]
    fn a_text_turn_forwards_its_deltas_and_ends_with_a_terminal_frame() {
        let (router, routed) = route_all(&[
            INIT,
            STATUS,
            TEXT_MESSAGE_START,
            TEXT_BLOCK_START,
            TEXT_DELTA,
            TEXT_ASSISTANT_FRAME,
            TEXT_BLOCK_STOP,
            TEXT_MESSAGE_DELTA,
            TEXT_MESSAGE_STOP,
            RESULT_SUCCESS,
        ]);

        let RoutedFrame::Init { api_key_source } = &routed[0] else {
            panic!(
                "system/init must surface apiKeySource so the caller can run the \
                 subscription refusal: {:?}",
                routed[0]
            );
        };
        assert_eq!(api_key_source.as_deref(), Some("none"));
        let forwarded: Vec<String> = routed.iter().filter_map(forwarded_event_type).collect();
        assert_eq!(
            forwarded,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
            ],
            "the text path must reach the Anthropic decoder intact"
        );

        let RoutedFrame::Terminal(terminal) = routed.last().expect("no frames routed") else {
            panic!(
                "the result frame must route to Terminal: {:?}",
                routed.last()
            );
        };
        assert_eq!(terminal.final_text, "done");
        assert!(terminal.error.is_none(), "a completed turn is not an error");
        assert_eq!(terminal.usage.input_tokens, Some(4));
        assert_eq!(terminal.usage.output_tokens, Some(99));
        assert_eq!(terminal.usage.cache_read_input_tokens, Some(46278));
        assert_eq!(terminal.usage.cache_creation_input_tokens, Some(12534));
        // Context occupancy for the gauge: all four buckets, not the billed sum.
        assert_eq!(terminal.usage.total_tokens, Some(4 + 99 + 46278 + 12534));
        assert!(router.streamed_any_text());
        assert_eq!(router.unhandled(), 0, "every recorded frame is understood");
    }

    /// (b) THE PHASE CONTRACT: a tool block reaches the decoder ZERO times.
    ///
    /// Every one of `content_block_start{tool_use}`, its `input_json_delta`s and
    /// its `content_block_stop` must route to `Tool` — including the delta and
    /// stop frames, which name only `index` and would otherwise be
    /// indistinguishable from text. A single one of them reaching
    /// `response_to_streaming_message` mints an unmarked `ToolRequest` that the
    /// agent loop dispatches, which is either a "Tool not found" row or a second
    /// execution of a call the bridge already ran.
    #[test]
    fn no_tool_use_frame_ever_reaches_the_anthropic_decoder() {
        let (router, routed) = route_all(&[
            INIT,
            STATUS,
            TOOL_MESSAGE_START,
            TOOL_BLOCK_START,
            TOOL_ARGS_DELTA_EMPTY,
            TOOL_ARGS_DELTA_BODY,
            TOOL_ARGS_DELTA_TAIL,
            TOOL_ASSISTANT_FRAME,
            TOOL_BLOCK_STOP,
            TOOL_MESSAGE_DELTA,
            TOOL_MESSAGE_STOP,
            RATE_LIMIT,
            TOOL_RESULT_FRAME,
        ]);

        // Only the message-level frames may be forwarded; no content_block_* at
        // all, because every content block in this message is the tool call.
        let forwarded: Vec<String> = routed.iter().filter_map(forwarded_event_type).collect();
        assert_eq!(
            forwarded,
            vec!["message_start", "message_delta"],
            "a tool turn must forward no content-block event whatsoever"
        );

        let tool_events: Vec<&ToolBlockEvent> = routed
            .iter()
            .filter_map(|r| match r {
                RoutedFrame::Tool(event) => Some(event),
                _ => None,
            })
            .collect();
        assert_eq!(
            tool_events.len(),
            7,
            "start + 3 arg deltas + the assistant frame + stop + the result: {tool_events:?}"
        );
        assert!(matches!(
            tool_events[0],
            ToolBlockEvent::Opened { index: 0, .. }
        ));
        assert!(matches!(
            tool_events[1],
            ToolBlockEvent::ArgsDelta { index: 0, .. }
        ));
        assert!(matches!(
            tool_events[5],
            ToolBlockEvent::Closed { index: 0, .. }
        ));

        // The complete arguments arrive on the `assistant` frame, not on the
        // deltas — phase 3 builds the card from this one.
        let ToolBlockEvent::Call { calls, .. } = tool_events[4] else {
            panic!("the assistant tool_use frame must park as a Call: {tool_events:?}");
        };
        assert_eq!(calls[0].id, "toolu_01DS65QaNoiWEyuRBSzMgdvT");
        assert_eq!(calls[0].name, "Bash");
        assert_eq!(
            calls[0].input["command"],
            Value::from("curl -sI https://example.com | head -n 1")
        );

        let ToolBlockEvent::Result { results } = tool_events[6] else {
            panic!("the user tool_result frame must park as a Result: {tool_events:?}");
        };
        assert_eq!(results[0].tool_use_id, calls[0].id);
        assert_eq!(results[0].content, Value::from("HTTP/2 200"));
        assert!(!results[0].is_error);
        assert!(
            results[0].detail.is_some(),
            "the richer `tool_use_result` sibling is what phase 3 shows on the card"
        );
        assert_eq!(router.diverted_tool_events(), 7);
        assert_eq!(router.unhandled(), 0);
    }

    /// Block indices restart at 0 in every message, so a diverted index must not
    /// outlive its message. In this exact recorded pair, message A has
    /// `tool_use` at index 0 and message B has **text** at index 0: a router
    /// that never cleared the set would swallow the whole answer while looking
    /// like it was correctly protecting the decoder.
    #[test]
    fn a_diverted_index_does_not_leak_into_the_next_message() {
        let (_router, routed) = route_all(&[
            TOOL_MESSAGE_START,
            TOOL_BLOCK_START,
            TOOL_ARGS_DELTA_BODY,
            TOOL_BLOCK_STOP,
            TOOL_MESSAGE_STOP,
            TEXT_MESSAGE_START,
            TEXT_BLOCK_START,
            TEXT_DELTA,
            TEXT_BLOCK_STOP,
        ]);

        assert!(
            matches!(routed[6], RoutedFrame::AnthropicEvent(_)),
            "text at index 0 of the NEXT message must reach the decoder: {:?}",
            routed[6]
        );
        assert!(
            matches!(routed[7], RoutedFrame::AnthropicEvent(_)),
            "the text delta must reach the decoder: {:?}",
            routed[7]
        );
        assert!(matches!(routed[8], RoutedFrame::AnthropicEvent(_)));
    }

    /// One message, thinking at index 0 and a tool call at index 1: only the
    /// tool index is diverted, and the `signature_delta` is dropped so the
    /// thinking block reaches the decoder unsigned (rule 5).
    #[test]
    fn a_mixed_message_diverts_only_the_tool_index() {
        let (router, routed) = route_all(&[
            PM_MESSAGE_START,
            PM_THINKING_START,
            PM_THINKING_TOKENS,
            PM_THINKING_DELTA,
            PM_SIGNATURE_DELTA,
            PM_THINKING_STOP,
            PM_TOOL_START,
            PM_TOOL_ARGS_DELTA,
            PM_TOOL_STOP,
        ]);

        let forwarded: Vec<String> = routed.iter().filter_map(forwarded_event_type).collect();
        assert_eq!(
            forwarded,
            vec![
                "message_start",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
            ],
            "index 0 is thinking and must stream; index 1 is a tool call and must not"
        );
        assert!(
            matches!(routed[4], RoutedFrame::Ignored),
            "signature_delta is dropped so the signed-turn persistence branch \
             never engages for a provider whose history is flattened to text"
        );
        assert_eq!(diverted_index(&routed[6]), Some(1), "the tool block opens");
        assert_eq!(diverted_index(&routed[7]), Some(1), "its args delta");
        assert_eq!(diverted_index(&routed[8]), Some(1), "its stop");
        assert_eq!(router.unhandled(), 0);
    }

    /// (c) The `assistant` frame repeats a block the deltas already delivered,
    /// and arrives *before* that block's stop. Emitting both would render the
    /// answer twice, so the frame is dropped once deltas have been seen.
    #[test]
    fn a_duplicate_assistant_text_frame_is_dropped_after_its_deltas() {
        let (router, routed) = route_all(&[
            TEXT_MESSAGE_START,
            TEXT_BLOCK_START,
            TEXT_DELTA,
            TEXT_ASSISTANT_FRAME,
        ]);

        assert!(
            matches!(routed[3], RoutedFrame::Ignored),
            "the deltas are the source of truth for a text block: {:?}",
            routed[3]
        );
        assert_eq!(router.dropped_duplicate_blocks(), 1);
        assert_eq!(
            router.unmirrored_blocks(),
            0,
            "the block WAS mirrored by deltas, so it is duplication, not loss"
        );
    }

    /// The other half of rule 4: an `assistant` text frame with no deltas before
    /// it is the *only* source for that block. It is still not forwarded (the
    /// router never synthesises Anthropic events), but it is counted, and
    /// `streamed_any_text()` stays false so the caller knows to fall back to the
    /// terminal frame's text instead of yielding an empty answer.
    #[test]
    fn an_unmirrored_assistant_text_frame_is_counted_not_silently_lost() {
        let (router, routed) = route_all(&[TEXT_MESSAGE_START, TEXT_ASSISTANT_FRAME]);

        assert!(matches!(routed[1], RoutedFrame::Ignored));
        assert_eq!(router.unmirrored_blocks(), 1);
        assert_eq!(router.dropped_duplicate_blocks(), 0);
        assert!(!router.streamed_any_text());
    }

    /// (d) `is_error:true` arrives with `subtype:"success"` in the recorded
    /// auth-failure turn. Classifying on `subtype` would call that turn a
    /// success; using `subtype` as the error *category* would hand the provider's
    /// error mapper the literal `"success"`.
    #[test]
    fn an_error_result_with_subtype_success_is_still_an_error() {
        let (_router, routed) = route_all(&[RESULT_AUTH_FAILURE]);

        let RoutedFrame::Terminal(terminal) = &routed[0] else {
            panic!("the result frame must route to Terminal: {:?}", routed[0]);
        };
        assert_eq!(terminal.subtype.as_deref(), Some("success"));
        let error = terminal
            .error
            .as_ref()
            .expect("is_error:true is an error whatever the subtype says");
        assert_eq!(
            error.category.as_deref(),
            Some("api_error"),
            "the category comes from terminal_reason, never from the useless subtype"
        );
        assert!(error.detail.contains("Not logged in"));
    }

    /// A successful terminal frame is not classified as an error just because it
    /// carries a `terminal_reason`. The negative half of the same rule.
    #[test]
    fn a_completed_result_is_not_classified_as_an_error() {
        let (_router, routed) = route_all(&[RESULT_SUCCESS]);
        let RoutedFrame::Terminal(terminal) = &routed[0] else {
            panic!("expected Terminal");
        };
        assert!(terminal.error.is_none());
        assert_eq!(terminal.terminal_reason.as_deref(), Some("completed"));
    }

    /// Rule 3, pinned: the decoder `break`s on `message_stop`
    /// (`providers/formats/anthropic.rs:919-943`), so forwarding it would end
    /// decoding at the first tool round-trip and drop every later token. A turn
    /// has one `message_stop` per API request — three in the recorded
    /// `turn-tools` cell.
    #[test]
    fn message_stop_is_never_forwarded_because_the_decoder_breaks_on_it() {
        let (_router, routed) = route_all(&[TOOL_MESSAGE_STOP, TEXT_MESSAGE_STOP]);
        for (i, frame) in routed.iter().enumerate() {
            assert!(
                matches!(frame, RoutedFrame::Ignored),
                "message_stop #{i} must not reach the decoder: {frame:?}"
            );
        }
    }

    /// (e) Unknown and malformed frames are counted and skipped. The counter is
    /// the drift signal — assert it only ever goes DOWN, in the spirit of bb's
    /// `row-counts.json` discipline. A vendor that adds a frame type mid-release
    /// must not be able to kill a turn.
    #[test]
    fn unknown_frames_are_counted_and_never_panic() {
        let mut router = ClaudeStreamRouter::new();

        // Known-but-inert frames are NOT drift; they must not inflate the count.
        assert!(matches!(router.push_line(STATUS), RoutedFrame::Ignored));
        assert!(matches!(router.push_line(RATE_LIMIT), RoutedFrame::Ignored));
        assert!(matches!(router.push_line(""), RoutedFrame::Ignored));
        assert_eq!(router.unhandled(), 0);

        // A brand-new top-level frame type.
        assert!(matches!(
            router.push_line(r#"{"type":"tool_progress","tool_use_id":"toolu_01","progress":0.5}"#),
            RoutedFrame::Ignored
        ));
        // A brand-new `system` subtype.
        assert!(matches!(
            router.push_line(r#"{"type":"system","subtype":"quantum_boundary","x":1}"#),
            RoutedFrame::Ignored
        ));
        // A brand-new Anthropic event inside a stream_event: NOT forwarded,
        // because a future event type could carry tool content.
        assert!(matches!(
            router.push_line(
                r#"{"type":"stream_event","event":{"type":"content_block_reopen","index":0}}"#
            ),
            RoutedFrame::Ignored
        ));
        // Truncated output, which a killed child really does produce.
        assert!(matches!(
            router.push_line(r#"{"type":"assistant","message":{"content":[{"type":"te"#),
            RoutedFrame::Ignored
        ));
        // A frame with no `type` at all.
        assert!(matches!(router.push_line("{}"), RoutedFrame::Ignored));

        assert_eq!(router.unhandled(), 5);
    }

    /// `system/api_retry` is not model output, but its category is the fallback
    /// classification when a turn dies without a usable terminal frame — the
    /// same fallback the blocking parser already uses
    /// (`providers/claude_code.rs:426-437`).
    #[test]
    fn an_api_retry_category_survives_to_classify_a_bare_failure() {
        let mut router = ClaudeStreamRouter::new();
        router.push_line(r#"{"type":"system","subtype":"api_retry","error":"overloaded_error"}"#);
        assert_eq!(router.retry_category(), Some("overloaded_error"));

        // A terminal frame that says only "is_error" borrows the retry category.
        let routed = router.push_line(r#"{"type":"result","is_error":true,"result":""}"#);
        let RoutedFrame::Terminal(terminal) = routed else {
            panic!("expected Terminal");
        };
        let error = terminal.error.expect("is_error:true is an error");
        assert_eq!(error.category.as_deref(), Some("overloaded_error"));
        assert_eq!(error.detail, "`claude` reported an error");
    }

    /// A keyed child must be refusable: the router surfaces `apiKeySource`
    /// verbatim so the caller runs the existing refusal
    /// (`providers/claude_code.rs:283-294`) rather than a second copy of it.
    #[test]
    fn a_keyed_child_is_reported_verbatim_for_the_caller_to_refuse() {
        let mut router = ClaudeStreamRouter::new();
        let routed = router.push_line(
            r#"{"type":"system","subtype":"init","apiKeySource":"ANTHROPIC_API_KEY","model":"claude-opus-5"}"#,
        );
        let RoutedFrame::Init { api_key_source } = routed else {
            panic!("expected Init");
        };
        assert_eq!(api_key_source.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert_eq!(router.api_key_source(), Some("ANTHROPIC_API_KEY"));
    }

    /// Subagent frames carry a non-null `parent_tool_use_id`. Biorouter runs the
    /// child with `--tools ""` so none should appear, but attributing one to
    /// this turn would show a tool card for a call the bridge never saw.
    #[test]
    fn subagent_frames_are_not_attributed_to_this_turn() {
        let mut router = ClaudeStreamRouter::new();
        let routed = router.push_line(
            r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_sub","type":"tool_result","content":"x","is_error":false}]},"parent_tool_use_id":"toolu_01RNa8","session_id":"s"}"#,
        );
        assert!(matches!(routed, RoutedFrame::Ignored));
        assert_eq!(router.diverted_tool_events(), 0);
    }

    /// A `tool_result` marked `is_error` still parks as a result — the failure
    /// is data on the block, and phase 3 renders it as a red card rather than
    /// dropping the pair and leaving a skeleton stuck.
    #[test]
    fn a_failed_tool_result_parks_with_its_error_flag() {
        let mut router = ClaudeStreamRouter::new();
        // approval-deny line 28, trimmed to the block that carries the flag.
        let routed = router.push_line(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Permission request denied","is_error":true,"tool_use_id":"toolu_01RYGr"}]},"parent_tool_use_id":null,"tool_use_result":"Error: Permission request denied"}"#,
        );
        let RoutedFrame::Tool(ToolBlockEvent::Result { results }) = routed else {
            panic!("expected a parked tool result");
        };
        assert!(results[0].is_error);
        assert_eq!(results[0].tool_use_id, "toolu_01RYGr");
    }

    /// The forwarded line must be exactly the Anthropic event, stripped of the
    /// Claude Code envelope: the decoder parses the payload into a struct with a
    /// flattened `data: Value`, and `session_id`/`uuid`/`ttft_ms` would ride
    /// along into it.
    #[test]
    fn a_forwarded_line_carries_the_event_and_nothing_of_the_envelope() {
        let mut router = ClaudeStreamRouter::new();
        let RoutedFrame::AnthropicEvent(line) = router.push_line(TEXT_DELTA) else {
            panic!("a text delta must be forwarded");
        };
        assert!(line.starts_with("data: "));
        let event: Value = serde_json::from_str(line.trim_start_matches("data: ")).unwrap();
        assert_eq!(event["type"], Value::from("content_block_delta"));
        assert_eq!(event["delta"]["text"], Value::from("done"));
        assert!(event.get("session_id").is_none());
        assert!(event.get("uuid").is_none());
        assert!(event.get("parent_tool_use_id").is_none());
    }

    /// Replay each recorded Claude cell through the router and pin what it yields.
    ///
    /// The counts are the bb `row-counts.json` discipline: `unhandled` may only go
    /// down. A vendor frame Biorouter starts needing shows up here as a number that
    /// moved, rather than as output quietly going missing in production.
    #[test]
    fn recorded_cells_route_without_unhandled_drift() {
        // (cell, at least this many forwarded text/thinking events, tool events)
        let expectations = [
            ("turn-text", 5usize, 0usize),
            ("turn-thinking", 5, 0),
            ("turn-tools", 1, 1),
            ("turn-tool-error", 1, 1),
        ];

        for (cell, min_forwarded, min_tool) in expectations {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/coding_agent/claude")
                .join(format!("{cell}.ndjson"));
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));

            let mut router = ClaudeStreamRouter::new();
            let (mut forwarded, mut tool_events) = (0usize, 0usize);
            for line in body.lines().filter(|l| !l.trim().is_empty()) {
                match router.push_line(line) {
                    RoutedFrame::AnthropicEvent(_) => forwarded += 1,
                    RoutedFrame::Tool(_) => tool_events += 1,
                    _ => {}
                }
            }

            assert_eq!(
                router.unhandled(),
                0,
                "{cell}: {} frame(s) the router did not understand. This count may \
                 only go DOWN — an increase means the vendor changed something and \
                 output is being dropped silently",
                router.unhandled()
            );
            assert!(
                forwarded >= min_forwarded,
                "{cell}: only {forwarded} event(s) reached the text decoder (expected \
                 at least {min_forwarded}) — the answer would arrive truncated"
            );
            assert!(
                tool_events >= min_tool,
                "{cell}: only {tool_events} tool event(s) were diverted (expected at \
                 least {min_tool})"
            );
        }
    }

    /// **The invariant that keeps the whole design safe**, asserted directly on the
    /// recorded frames: not one `tool_use` event may reach the Anthropic decoder.
    ///
    /// If one did, the decoder's §6.2b flush would mint an *unmarked* `ToolRequest`
    /// batch, and the agent loop would dispatch it — really re-running a shell
    /// command the child had already run. The mirrored pair is emitted separately
    /// and marked; this is the other half of that guarantee.
    #[test]
    fn no_tool_use_event_reaches_the_text_decoder_in_any_recorded_cell() {
        for cell in [
            "turn-text",
            "turn-thinking",
            "turn-tools",
            "turn-tool-error",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/coding_agent/claude")
                .join(format!("{cell}.ndjson"));
            let body = std::fs::read_to_string(&path).expect("fixture");

            let mut router = ClaudeStreamRouter::new();
            for line in body.lines().filter(|l| !l.trim().is_empty()) {
                if let RoutedFrame::AnthropicEvent(data) = router.push_line(line) {
                    // Parse rather than substring-match: `stop_reason: "tool_use"`
                    // rides a perfectly legitimate `message_delta` — the one carrying
                    // the turn's usage — and rejecting that would reject a frame the
                    // decoder must see. What must never arrive is a tool *content
                    // block* or its argument deltas.
                    let payload = data.strip_prefix("data: ").unwrap_or(&data);
                    let event: serde_json::Value =
                        serde_json::from_str(payload).expect("a forwarded line is one event");
                    let kind = event.get("type").and_then(Value::as_str);
                    if kind == Some("content_block_start") {
                        assert_ne!(
                            event
                                .get("content_block")
                                .and_then(|b| b.get("type"))
                                .and_then(Value::as_str),
                            Some("tool_use"),
                            "{cell}: a tool_use content block reached the text decoder. It \
                             would mint an unmarked ToolRequest and the loop would execute \
                             the call a second time."
                        );
                    }
                    if kind == Some("content_block_delta") {
                        assert_ne!(
                            event
                                .get("delta")
                                .and_then(|d| d.get("type"))
                                .and_then(Value::as_str),
                            Some("input_json_delta"),
                            "{cell}: tool-call arguments reached the text decoder, which \
                             buffers them into a dispatchable ToolRequest."
                        );
                    }
                }
            }
        }
    }
}
