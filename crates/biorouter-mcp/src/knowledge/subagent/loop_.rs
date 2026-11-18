//! Bounded sub-agent loop.
//!
//! The `Completer` trait abstracts the LLM call so that:
//!   - `biorouter-mcp` has no cyclic dependency on `biorouter` (which already
//!     depends on `biorouter-mcp`).
//!   - Tests use a trivial `MockCompleter` without needing a real provider.
//!   - Production callers (in `biorouter` or a macro layer that has access to
//!     both crates) wrap `Arc<dyn Provider>` in an adapter.

use crate::knowledge::subagent::events::{DoneReason, SubAgentEvent};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Message types used inside the loop (provider-agnostic)
// ---------------------------------------------------------------------------

/// One "tool request" emitted by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    /// The unique request-id assigned by the provider.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// JSON arguments (may be Value::Object or Value::Null).
    pub args: serde_json::Value,
}

/// The reply we get back from the LLM after one `complete()` call.
#[derive(Debug, Clone)]
pub struct LlmReply {
    /// The plain-text portion of the assistant's response.
    pub text: String,
    /// Any tool calls embedded in the response (may be empty).
    pub tool_calls: Vec<LlmToolCall>,
}

// ---------------------------------------------------------------------------
// Completer trait  (thin LLM abstraction, loop-internal)
// ---------------------------------------------------------------------------

/// One tool-result part within a compound `LlmMessage::ToolResults` turn.
#[derive(Debug, Clone)]
pub struct ToolResultPart {
    /// Matches `LlmToolCall::id` from the corresponding assistant reply.
    pub request_id: String,
    /// Tool name (for debugging / logging).
    pub name: String,
    /// The string the tool returned.
    pub content: String,
}

/// A conversation turn in the format the Completer expects.
#[derive(Debug, Clone)]
pub enum LlmMessage {
    /// Initial or follow-up user text.
    User(String),
    /// The assistant's previous reply (needed so the provider can see context).
    Assistant(LlmReply),
    /// The result we are feeding back for a specific tool request.
    ///
    /// Kept for backward compatibility (single tool call per turn).  For turns
    /// with multiple tool calls, prefer `ToolResults` so all results are bundled
    /// into a single user message — required by Bedrock's strict validation.
    ToolResult {
        /// Matches `LlmToolCall::id` from the corresponding assistant reply.
        request_id: String,
        /// Tool name (for debugging / logging).
        name: String,
        /// The string the tool returned.
        content: String,
    },
    /// All tool results from a single assistant turn, bundled together.
    ///
    /// Bedrock (and the Anthropic spec) require that when an assistant turn
    /// contains N `tool_use` blocks, ALL N `tool_result` blocks MUST appear
    /// in a SINGLE subsequent user message.  Using this variant instead of N
    /// separate `ToolResult` entries satisfies that constraint.
    ToolResults(Vec<ToolResultPart>),
}

/// One tool call a completer's OWN executor already ran.
///
/// The coding-agent providers are the reason this exists: `claude_code` and
/// `codex` drive a whole agent in a child process, and Biorouter's tools reach
/// them over an MCP bridge the *provider call* establishes. So the child chooses
/// and executes the calls inside one `complete`, and by the time this loop sees
/// a reply the work is done. These are records, not requests.
#[derive(Debug, Clone)]
pub struct ExecutedCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
    /// What the tool returned, as text.
    pub output: String,
    pub is_error: bool,
}

/// One turn, plus whatever the completer ran on its own account.
pub struct CompleterTurn {
    pub reply: LlmReply,
    /// ⚠ **The loop must not dispatch these.** They have already executed. A
    /// `kb_write_page` run twice is a second write, not a redraw.
    pub executed: Vec<ExecutedCall>,
}

impl From<LlmReply> for CompleterTurn {
    fn from(reply: LlmReply) -> Self {
        Self {
            reply,
            executed: Vec::new(),
        }
    }
}

/// Minimal LLM capability needed by the sub-agent loop.
///
/// Callers that hold an `Arc<dyn biorouter::providers::base::Provider>` can
/// provide a thin adapter in their crate.  Inside `biorouter-mcp` we only
/// use the `MockCompleter` in tests.
#[async_trait]
pub trait Completer: Send + Sync {
    /// Send the current conversation (system + messages) to the LLM and
    /// return the next assistant reply.
    async fn complete(
        &self,
        system: &str,
        messages: &[LlmMessage],
        tools: &[Tool],
    ) -> Result<LlmReply>;

    /// The same turn, with this run's dispatcher reachable from *inside* the
    /// completer.
    ///
    /// The default ignores it, which is right for every completer that returns
    /// tool calls for the loop to run. It is overridden by the adapter wrapping
    /// a coding-agent provider (#109): that provider's child executes the tools
    /// itself, over an MCP bridge established for the duration of the provider
    /// call — so the dispatcher has to be reachable *during* the call, not after
    /// it. Without this seam the child was handed a `tools` argument its provider
    /// discards; the model then narrated every call as prose, invented its own
    /// `<tool_response>OK` replies to continue against, and wrote nothing.
    ///
    /// `Arc`, not `&dyn`: the adapter hands the dispatcher to a bridge grant
    /// that lives in a process-global registry for the length of the turn, and a
    /// borrow cannot go there.
    async fn complete_with_dispatch(
        &self,
        system: &str,
        messages: &[LlmMessage],
        tools: &[Tool],
        _dispatch: std::sync::Arc<dyn ToolDispatch>,
    ) -> Result<CompleterTurn> {
        Ok(self.complete(system, messages, tools).await?.into())
    }
}

// ---------------------------------------------------------------------------
// Bounds / result types
// ---------------------------------------------------------------------------

pub struct SubAgentBounds {
    pub max_steps: usize,
    /// Wall clock for the whole run — checked before each step **and** applied
    /// as a timeout to the provider call itself. Only the first of those two
    /// existed for a long time, which meant a single hung request was
    /// completely unbounded: the check between iterations never came round
    /// again, so a run with a 300 s budget could sit on one `await` for as long
    /// as the socket stayed open (DR-16b).
    pub max_wall: Duration,
    /// Ceiling on the conversation the sub-agent may accumulate, counted the
    /// cheap way (see [`estimated_tokens`]).
    ///
    /// This field was declared and read by **nothing**, which is worse than a
    /// missing bound: it reads as a bound that is being enforced. The step
    /// budget does not substitute for it — 30 steps of `kb_read_page` over long
    /// pages is a context overflow the provider rejects, and the rejection
    /// arrives as an opaque provider error rather than as "this run got too big".
    pub max_tokens: u64,
}

impl Default for SubAgentBounds {
    fn default() -> Self {
        Self {
            max_steps: 30,
            max_wall: Duration::from_secs(300),
            max_tokens: 200_000,
        }
    }
}

pub struct SubAgentResult {
    pub final_text: String,
    pub events: Vec<SubAgentEvent>,
    pub reason: DoneReason,
    pub steps_used: usize,
}

// ---------------------------------------------------------------------------
// VocabularyRejection
// ---------------------------------------------------------------------------

/// A tool call was refused because an argument was not a member of that
/// argument's **closed vocabulary** (DR-16).
///
/// ## Why the loop owns this type and not the dispatch
///
/// It changes how the *run* ends, which is a loop-level fact. Every other tool
/// failure is something the model can plausibly fix on the next step — a bad
/// path, a missing argument, a page that is not there — so
/// [`SubAgent::dispatch_turn`] feeds it back as `error: …` and lets the step
/// budget be the thing that stops an unproductive retry. A vocabulary rejection
/// is the one failure where "try again" is *structurally* unlikely to succeed:
/// the model that guessed `treats_disease` will guess again from the same place
/// it guessed the first time, and the budget dies with the run misreported as
/// "the sub-agent ran out of turns".
///
/// So it travels as a typed error inside `anyhow::Error` — recovered with
/// `downcast_ref`, never by matching on message text — and produces
/// [`DoneReason::VocabularyRetriesExhausted`].
///
/// ## It is also the model's error message
///
/// [`Display`](std::fmt::Display) is what gets fed back, and it names the
/// closest legal value, because "invalid predicate" without one is a retry
/// prompt and with one is a fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyRejection {
    /// The argument that was refused, as the tool schema spells it —
    /// `type`, `predicate`, `knowledge_level`, `agent_type`.
    pub field: String,
    /// What the model sent.
    pub value: String,
    /// The nearest legal value, when one is near enough to be a real
    /// suggestion. `None` for a value that resembles nothing in the
    /// vocabulary, where naming an arbitrary member would misdirect.
    pub closest: Option<String>,
    /// How many values the vocabulary has, so the message can say where to
    /// look rather than listing all of them.
    pub legal_count: usize,
    /// The reason this value is not legal, when the vocabulary has more to say
    /// than "not a member" — BioOKF's §6.F non-negatable predicates, for one.
    pub detail: Option<String>,
}

impl std::fmt::Display for VocabularyRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` is not a legal `{}`: this base's vocabulary is closed and has {} value(s), \
             declared as the `enum` on this argument in the tool schema",
            self.value, self.field, self.legal_count
        )?;
        if let Some(detail) = &self.detail {
            write!(f, ". {detail}")?;
        }
        match &self.closest {
            Some(closest) => write!(
                f,
                ". Closest legal value: `{closest}` — use it, or pick another value from the \
                 enum; do not send `{}` again",
                self.value
            ),
            None => write!(
                f,
                ". Nothing in the vocabulary is close to it, so re-read the enum on this \
                 argument and choose from it rather than guessing"
            ),
        }
    }
}

impl std::error::Error for VocabularyRejection {}

impl VocabularyRejection {
    /// True when `err` is (or wraps) a vocabulary rejection.
    ///
    /// `downcast_ref` and not a string match: the message is prose written for
    /// a model and will be reworded, and a terminal reason that depended on its
    /// wording would go quietly wrong the first time it was.
    pub fn is_one(err: &anyhow::Error) -> bool {
        err.downcast_ref::<Self>().is_some()
    }
}

// ---------------------------------------------------------------------------
// ToolDispatch trait
// ---------------------------------------------------------------------------

/// Trait the caller implements: given a tool name + JSON args, run it and
/// return a string result.
#[async_trait]
pub trait ToolDispatch: Send + Sync {
    async fn call(&self, name: &str, args: serde_json::Value) -> Result<String>;
}

// ---------------------------------------------------------------------------
// SubAgent
// ---------------------------------------------------------------------------

pub struct SubAgent {
    pub completer: Box<dyn Completer>,
    pub tools: Vec<Tool>,
    pub system_prompt: String,
    pub bounds: SubAgentBounds,
}

impl SubAgent {
    #[allow(clippy::too_many_lines)]
    pub async fn run(
        &self,
        user_message: &str,
        dispatch: std::sync::Arc<dyn ToolDispatch>,
        cancel: Option<&CancellationToken>,
        event_sink: Option<&tokio::sync::mpsc::UnboundedSender<SubAgentEvent>>,
    ) -> Result<SubAgentResult> {
        let mut events: Vec<SubAgentEvent> = Vec::new();
        let mut messages: Vec<LlmMessage> = vec![LlmMessage::User(user_message.to_string())];
        let started = Instant::now();
        let mut steps = 0usize;
        // Whether the most recent turn that dispatched anything was refused for
        // a closed-vocabulary value. Read only when a budget stops the run —
        // see `budget_reason`.
        let mut retrying_vocabulary = false;

        loop {
            // --- Bound checks before calling the LLM ---
            if let Some(reason) = self.stop_before_step(steps, started, &messages, cancel) {
                let reason = budget_reason(reason, retrying_vocabulary);
                let text = stop_text(&reason);
                return Ok(make_result(events, reason, text, steps));
            }

            // --- LLM call, under the remaining wall-clock budget ---
            //
            // The timeout is the same budget the loop checks between steps, not
            // a second one: a per-call cap would be a number nobody could
            // reconcile with `max_wall`, and the question a caller asks is "how
            // long may this run take", once.
            let remaining = self.bounds.max_wall.saturating_sub(started.elapsed());
            // #109: the dispatcher goes IN, so a completer whose child executes
            // the tools itself can reach it during the provider call.
            let call = self.completer.complete_with_dispatch(
                &self.system_prompt,
                &messages,
                &self.tools,
                std::sync::Arc::clone(&dispatch),
            );
            let timed_call = tokio::time::timeout(remaining, call);
            let turn = match await_or_cancel(timed_call, cancel).await {
                AwaitOrCancel::Cancelled => {
                    return Ok(make_result(
                        events,
                        DoneReason::Cancelled,
                        stop_text(&DoneReason::Cancelled),
                        steps,
                    ));
                }
                AwaitOrCancel::Completed(Ok(turn)) => turn?,
                AwaitOrCancel::Completed(Err(_elapsed)) => {
                    let reason = budget_reason(DoneReason::TimeBudgetReached, retrying_vocabulary);
                    let text = if reason == DoneReason::TimeBudgetReached {
                        "time budget reached while waiting for the model"
                    } else {
                        stop_text(&reason)
                    };
                    return Ok(make_result(events, reason, text, steps));
                }
            };
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Ok(make_result(
                    events,
                    DoneReason::Cancelled,
                    stop_text(&DoneReason::Cancelled),
                    steps,
                ));
            }

            let CompleterTurn { reply, executed } = turn;

            let step_ev = SubAgentEvent::Step {
                index: steps,
                assistant_text: reply.text.clone(),
            };
            if let Some(tx) = event_sink {
                let _ = tx.send(step_ev.clone());
            }
            events.push(step_ev);

            // #109: calls the completer's child already ran. Recorded as events
            // so the run log reads the same whichever provider drove it, and
            // deliberately NOT fed to `dispatch_turn` — they have executed.
            record_executed(&executed, &mut events, event_sink);

            if reply.tool_calls.is_empty() {
                if cancel.is_some_and(CancellationToken::is_cancelled) {
                    return Ok(make_result(
                        events,
                        DoneReason::Cancelled,
                        stop_text(&DoneReason::Cancelled),
                        steps,
                    ));
                }
                return Ok(make_result(
                    events,
                    DoneReason::NoMoreToolCalls,
                    &reply.text,
                    steps,
                ));
            }

            // The `complete()` sentinel ends the run — but only AFTER the calls
            // it arrived beside have actually run.
            //
            // This check used to sit here and `return` immediately, above both
            // the assistant push and the dispatch loop, so every other tool call
            // in the same turn was discarded undispatched. That was rare while
            // the procedures ended with a lone `complete()` step; under typed
            // extraction the natural model output is N writes followed by
            // `complete` in one turn, which is exactly the losing shape.
            //
            // And it loses *silently*: nothing under `knowledge/` changed, so
            // `txn_wrote_knowledge_pages` returns false, the txn aborts, and the
            // ingest fails with "wrote no knowledge pages" — which points the
            // investigator at the model's authoring rather than at this loop
            // (DR-15).
            let complete_requested = reply.tool_calls.iter().any(|t| t.name == "complete");

            // Store the assistant turn in the conversation
            messages.push(LlmMessage::Assistant(reply.clone()));

            let dispatch_result = self
                .dispatch_turn(
                    &reply.tool_calls,
                    dispatch.as_ref(),
                    &mut events,
                    cancel,
                    event_sink,
                )
                .await;
            if dispatch_result.cancelled {
                return Ok(make_result(
                    events,
                    DoneReason::Cancelled,
                    stop_text(&DoneReason::Cancelled),
                    steps,
                ));
            }
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Ok(make_result(
                    events,
                    DoneReason::Cancelled,
                    stop_text(&DoneReason::Cancelled),
                    steps,
                ));
            }
            // Only a turn that actually dispatched something updates the flag:
            // a turn of pure prose is not evidence that the model stopped
            // retrying, and clearing it there would hide the diagnosis behind
            // one apologetic message.
            if !dispatch_result.result_parts.is_empty() {
                retrying_vocabulary = dispatch_result.rejected_vocabulary;
            }
            // Bundle all results into one message so Bedrock sees a single user
            // turn paired against the assistant turn above.
            if !dispatch_result.result_parts.is_empty() {
                messages.push(LlmMessage::ToolResults(dispatch_result.result_parts));
            }

            if complete_requested {
                return Ok(make_result(
                    events,
                    DoneReason::CompleteSentinel,
                    &reply.text,
                    steps,
                ));
            }
            steps += 1;
        }
    }

    /// Every reason to stop *before* spending another provider call, in the
    /// order they are cheapest to check.
    ///
    /// One function rather than four inline `if`s so that adding a bound is a
    /// change in one place — the token budget below was declared in
    /// [`SubAgentBounds`] and enforced nowhere, and a list of checks that is
    /// hard to see the end of is how that happens.
    fn stop_before_step(
        &self,
        steps: usize,
        started: Instant,
        messages: &[LlmMessage],
        cancel: Option<&CancellationToken>,
    ) -> Option<DoneReason> {
        if steps >= self.bounds.max_steps {
            return Some(DoneReason::StepBudgetReached);
        }
        if started.elapsed() > self.bounds.max_wall {
            return Some(DoneReason::TimeBudgetReached);
        }
        if estimated_tokens(&self.system_prompt, messages) > self.bounds.max_tokens {
            return Some(DoneReason::TokenBudgetReached);
        }
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Some(DoneReason::Cancelled);
        }
        None
    }

    /// Dispatch every tool call in one assistant turn, in order, recording an
    /// event per call and per result.
    ///
    /// Results come back as parts of a SINGLE `LlmMessage::ToolResults`: Bedrock
    /// (and the Anthropic spec) require that when an assistant turn contains N
    /// `tool_use` blocks, ALL N `tool_result` blocks appear in one subsequent
    /// user message, and emitting separate messages causes a ValidationException.
    ///
    /// `complete` is skipped rather than dispatched — it is a loop sentinel, not
    /// a tool anyone implements, so dispatching it would produce an "unknown
    /// tool" error event on a turn that in fact succeeded.
    ///
    /// A failing tool is fed back to the model as `error: …` rather than ending
    /// the run: a bad argument is something the model can fix on the next step,
    /// and `max_steps` is what stops it retrying forever.
    ///
    /// The returned flag says whether any of those failures was a
    /// [`VocabularyRejection`] — the one class of failure where retrying is not
    /// a fix, and which therefore renames the run's terminal reason if a budget
    /// is what ends it.
    async fn dispatch_turn(
        &self,
        calls: &[LlmToolCall],
        dispatch: &dyn ToolDispatch,
        events: &mut Vec<SubAgentEvent>,
        cancel: Option<&CancellationToken>,
        event_sink: Option<&tokio::sync::mpsc::UnboundedSender<SubAgentEvent>>,
    ) -> DispatchTurn {
        let mut result_parts: Vec<ToolResultPart> = Vec::new();
        let mut rejected_vocabulary = false;
        for call in calls.iter().filter(|c| c.name != "complete") {
            let call_ev = SubAgentEvent::ToolCall {
                name: call.name.clone(),
                args: call.args.clone(),
            };
            if let Some(tx) = event_sink {
                let _ = tx.send(call_ev.clone());
            }
            events.push(call_ev);

            let outcome =
                await_or_cancel(dispatch.call(&call.name, call.args.clone()), cancel).await;
            let AwaitOrCancel::Completed(outcome) = outcome else {
                return DispatchTurn {
                    result_parts,
                    rejected_vocabulary,
                    cancelled: true,
                };
            };
            let (ok, summary, content) = match outcome {
                Ok(s) => (true, s.chars().take(120).collect(), s),
                Err(e) => {
                    rejected_vocabulary |= VocabularyRejection::is_one(&e);
                    let msg = e.to_string();
                    (false, msg.clone(), format!("error: {msg}"))
                }
            };
            let result_ev = SubAgentEvent::ToolResult {
                name: call.name.clone(),
                ok,
                summary,
            };
            if let Some(tx) = event_sink {
                let _ = tx.send(result_ev.clone());
            }
            events.push(result_ev);

            result_parts.push(ToolResultPart {
                request_id: call.id.clone(),
                name: call.name.clone(),
                content,
            });
        }
        DispatchTurn {
            result_parts,
            rejected_vocabulary,
            cancelled: false,
        }
    }
}

struct DispatchTurn {
    result_parts: Vec<ToolResultPart>,
    rejected_vocabulary: bool,
    cancelled: bool,
}

/// Write a completer-executed call into the run log, as the pair the loop's own
/// dispatch would have written.
///
/// The point is that a reader of a run cannot tell — and should not have to —
/// whether the tool was run by this loop or by a child agent behind the bridge.
/// Both went through the same gate stack; only the process differs.
fn record_executed(
    executed: &[ExecutedCall],
    events: &mut Vec<SubAgentEvent>,
    event_sink: Option<&tokio::sync::mpsc::UnboundedSender<SubAgentEvent>>,
) {
    for call in executed {
        for event in [
            SubAgentEvent::ToolCall {
                name: call.name.clone(),
                args: call.args.clone(),
            },
            SubAgentEvent::ToolResult {
                name: call.name.clone(),
                ok: !call.is_error,
                summary: call.output.chars().take(120).collect(),
            },
        ] {
            if let Some(tx) = event_sink {
                let _ = tx.send(event.clone());
            }
            events.push(event);
        }
    }
}

/// Rename a budget stop when the run was, at the moment it ran out, still
/// retrying a rejected controlled-vocabulary value (DR-16).
///
/// All three budgets and not only the step budget: a run that spends its wall
/// clock or fills its context re-guessing a predicate died of the same thing the
/// step-budget one did, and telling a caller "time budget reached" sends them to
/// look at provider latency.
///
/// Nothing else is renamed. `CompleteSentinel`, `NoMoreToolCalls` and
/// `Cancelled` are decisions somebody made, and a rejected argument earlier in
/// the run does not make them something else.
fn budget_reason(reason: DoneReason, retrying_vocabulary: bool) -> DoneReason {
    if !retrying_vocabulary {
        return reason;
    }
    match reason {
        DoneReason::StepBudgetReached
        | DoneReason::TimeBudgetReached
        | DoneReason::TokenBudgetReached => DoneReason::VocabularyRetriesExhausted,
        other => other,
    }
}

/// The final text that goes with a stop decided before the model was asked.
fn stop_text(reason: &DoneReason) -> &'static str {
    match reason {
        DoneReason::StepBudgetReached => "step budget reached",
        DoneReason::TimeBudgetReached => "time budget reached",
        DoneReason::TokenBudgetReached => "token budget reached",
        DoneReason::VocabularyRetriesExhausted => {
            "the budget ran out while the sub-agent was still retrying a value that is not in \
             this base's controlled vocabulary; the tool's rejection names the closest legal \
             value, and the full list is the `enum` on that argument"
        }
        DoneReason::Cancelled => "cancelled",
        // Not reachable from `stop_before_step`, which only returns budget and
        // cancellation reasons; spelled out rather than caught by a wildcard so
        // a new variant is a compile error here and gets its own sentence.
        DoneReason::CompleteSentinel | DoneReason::NoMoreToolCalls | DoneReason::Error => "stopped",
    }
}

/// A deliberately cheap size estimate for the conversation so far: four
/// characters to the token.
///
/// Not a tokenizer, and it must not become one here. The real counter
/// (`tiktoken-rs`, in `context_mgmt`) lives in `biorouter`, which depends on
/// this crate — reaching for it would be the circular dependency the whole
/// `Completer` abstraction exists to avoid. What this bound is for is stopping
/// a run whose context has grown to a size the provider is about to refuse, and
/// for that a ratio that is right to within a factor of two, applied to a
/// 200_000 default, does the job. It over-counts JSON tool arguments (which
/// tokenize densely) and under-counts nothing, so it errs toward stopping early.
fn estimated_tokens(system: &str, messages: &[LlmMessage]) -> u64 {
    let chars: usize = system.chars().count() + messages.iter().map(message_chars).sum::<usize>();
    (chars / 4) as u64
}

fn message_chars(message: &LlmMessage) -> usize {
    match message {
        LlmMessage::User(text) => text.chars().count(),
        LlmMessage::Assistant(reply) => {
            reply.text.chars().count()
                + reply
                    .tool_calls
                    .iter()
                    .map(|c| c.name.chars().count() + c.args.to_string().chars().count())
                    .sum::<usize>()
        }
        LlmMessage::ToolResult { content, .. } => content.chars().count(),
        LlmMessage::ToolResults(parts) => parts
            .iter()
            .map(|p| p.content.chars().count())
            .sum::<usize>(),
    }
}

enum AwaitOrCancel<T> {
    Completed(T),
    Cancelled,
}

async fn await_or_cancel<F>(
    future: F,
    cancel: Option<&CancellationToken>,
) -> AwaitOrCancel<F::Output>
where
    F: std::future::Future,
{
    let Some(cancel) = cancel else {
        return AwaitOrCancel::Completed(future.await);
    };
    tokio::select! {
        biased;
        () = cancel.cancelled() => AwaitOrCancel::Cancelled,
        output = future => AwaitOrCancel::Completed(output),
    }
}

fn make_result(
    events: Vec<SubAgentEvent>,
    reason: DoneReason,
    text: &str,
    steps: usize,
) -> SubAgentResult {
    SubAgentResult {
        final_text: text.to_string(),
        events,
        reason,
        steps_used: steps,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{Mutex, Notify};

    /// A MockCompleter that pops canned `LlmReply` values from a queue.
    struct MockCompleter {
        replies: Mutex<Vec<LlmReply>>,
    }

    impl MockCompleter {
        fn new(replies: Vec<LlmReply>) -> Self {
            Self {
                replies: Mutex::new(replies),
            }
        }
    }

    #[async_trait]
    impl Completer for MockCompleter {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[LlmMessage],
            _tools: &[Tool],
        ) -> Result<LlmReply> {
            let mut q = self.replies.lock().await;
            if q.is_empty() {
                panic!("MockCompleter ran out of canned replies");
            }
            Ok(q.remove(0))
        }
    }

    /// A dispatcher that always succeeds.
    struct EchoDispatch;

    #[async_trait]
    impl ToolDispatch for EchoDispatch {
        async fn call(&self, _name: &str, _args: serde_json::Value) -> Result<String> {
            Ok("ok".to_string())
        }
    }

    fn tool_call_reply(tool_name: &str) -> LlmReply {
        LlmReply {
            text: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "req-1".into(),
                name: tool_name.to_string(),
                args: serde_json::Value::Object(Default::default()),
            }],
        }
    }

    fn two_tool_call_reply() -> LlmReply {
        LlmReply {
            text: String::new(),
            tool_calls: vec![
                LlmToolCall {
                    id: "tc-1".into(),
                    name: "kb_search".to_string(),
                    args: serde_json::Value::Object(Default::default()),
                },
                LlmToolCall {
                    id: "tc-2".into(),
                    name: "kb_read_page".to_string(),
                    args: serde_json::Value::Object(Default::default()),
                },
            ],
        }
    }

    fn text_reply(text: &str) -> LlmReply {
        LlmReply {
            text: text.to_string(),
            tool_calls: vec![],
        }
    }

    fn make_agent(completer: MockCompleter, max_steps: usize) -> SubAgent {
        SubAgent {
            completer: Box::new(completer),
            tools: vec![],
            system_prompt: "sys".into(),
            bounds: SubAgentBounds {
                max_steps,
                ..Default::default()
            },
        }
    }

    /// A MockCompleter that also records the `messages` slice it receives each call.
    ///
    /// The `received` field is an `Arc<Mutex<…>>` so the test can clone a reference
    /// before moving the completer into `Box<dyn Completer>` and inspect recordings
    /// after the run completes.
    struct RecordingCompleter {
        replies: Mutex<Vec<LlmReply>>,
        received: Arc<Mutex<Vec<Vec<LlmMessage>>>>,
    }

    #[async_trait]
    impl Completer for RecordingCompleter {
        async fn complete(
            &self,
            _system: &str,
            messages: &[LlmMessage],
            _tools: &[Tool],
        ) -> Result<LlmReply> {
            self.received.lock().await.push(messages.to_vec());
            let mut q = self.replies.lock().await;
            if q.is_empty() {
                panic!("RecordingCompleter ran out of canned replies");
            }
            Ok(q.remove(0))
        }
    }

    /// Test 1: one tool call then a text-only reply → Done(NoMoreToolCalls) in 1 iteration.
    #[tokio::test]
    async fn two_step_happy_path() {
        let completer = MockCompleter::new(vec![
            tool_call_reply("kb_search"), // step 0: issues a tool call
            text_reply("all done"),       // step 1: no tool calls → done
        ]);
        let agent = make_agent(completer, 10);
        let result = agent
            .run("hello", std::sync::Arc::new(EchoDispatch), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::NoMoreToolCalls);
        // steps_used reflects the loop counter at the time of Done, which
        // is 1 (incremented after the first tool-dispatch round)
        assert_eq!(result.steps_used, 1);
        assert_eq!(result.final_text, "all done");
    }

    /// Test 2: provider always returns a tool call → hits max_steps.
    #[tokio::test]
    async fn step_budget_exceeded() {
        // 35 replies, but max_steps = 5 so only 5 are consumed.
        let replies: Vec<LlmReply> = (0..35).map(|_| tool_call_reply("kb_search")).collect();
        let completer = MockCompleter::new(replies);
        let agent = make_agent(completer, 5);
        let result = agent
            .run("hello", std::sync::Arc::new(EchoDispatch), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::StepBudgetReached);
        assert!(result.steps_used <= 5);
    }

    // ── DR-16: a budget that ran out re-guessing a closed vocabulary ────────

    /// A dispatcher that refuses every call the way the typed BioOKF writer
    /// refuses an invented predicate.
    struct VocabularyRefusing;

    #[async_trait]
    impl ToolDispatch for VocabularyRefusing {
        async fn call(&self, _name: &str, _args: serde_json::Value) -> Result<String> {
            Err(VocabularyRejection {
                field: "predicate".into(),
                value: "treats_disease".into(),
                closest: Some("treats".into()),
                legal_count: 35,
                detail: None,
            }
            .into())
        }
    }

    /// A dispatcher that fails for an ordinary reason.
    struct AlwaysFailing;

    #[async_trait]
    impl ToolDispatch for AlwaysFailing {
        async fn call(&self, _name: &str, _args: serde_json::Value) -> Result<String> {
            Err(anyhow::anyhow!("no such page"))
        }
    }

    /// The failure Stage 5 exists to make legible. Today's run reports
    /// `StepBudgetReached`, ingest then aborts the txn with *"wrote no knowledge
    /// pages"*, and both sentences send the investigator to look at the model's
    /// page authoring — which is the one thing that was not wrong.
    #[tokio::test]
    async fn a_budget_spent_retrying_an_invalid_vocabulary_says_that_and_not_step_budget() {
        let replies: Vec<LlmReply> = (0..35)
            .map(|_| tool_call_reply("kb_write_concept"))
            .collect();
        let agent = make_agent(MockCompleter::new(replies), 5);
        let result = agent
            .run("hello", std::sync::Arc::new(VocabularyRefusing), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::VocabularyRetriesExhausted);
        assert!(
            result.final_text.contains("controlled vocabulary"),
            "the terminal text has to be readable on its own: {}",
            result.final_text
        );
    }

    /// The negative half, and the one that keeps the new reason honest: an
    /// ordinary tool failure is still a step budget, because retrying a bad path
    /// really can succeed and a vocabulary guess really cannot.
    #[tokio::test]
    async fn an_ordinary_tool_failure_still_reports_the_step_budget() {
        let replies: Vec<LlmReply> = (0..35).map(|_| tool_call_reply("kb_read_page")).collect();
        let agent = make_agent(MockCompleter::new(replies), 5);
        let result = agent
            .run("hello", std::sync::Arc::new(AlwaysFailing), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::StepBudgetReached);
    }

    /// A run that hit a rejection and then recovered ends normally. Without
    /// this the reason would be "was there ever a bad predicate", which would
    /// relabel every successful run that took one wrong turn.
    #[tokio::test]
    async fn recovering_from_a_rejection_does_not_relabel_the_run() {
        struct RefuseOnce(std::sync::atomic::AtomicBool);
        #[async_trait]
        impl ToolDispatch for RefuseOnce {
            async fn call(&self, _name: &str, _args: serde_json::Value) -> Result<String> {
                if self.0.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    return Err(VocabularyRejection {
                        field: "type".into(),
                        value: "Diseases".into(),
                        closest: Some("Disease".into()),
                        legal_count: 28,
                        detail: None,
                    }
                    .into());
                }
                Ok("ok".to_string())
            }
        }
        let replies: Vec<LlmReply> = (0..35)
            .map(|_| tool_call_reply("kb_write_concept"))
            .collect();
        let agent = make_agent(MockCompleter::new(replies), 5);
        let result = agent
            .run(
                "hello",
                std::sync::Arc::new(RefuseOnce(std::sync::atomic::AtomicBool::new(true))),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::StepBudgetReached);
    }

    /// DR-16b. `max_tokens` was declared and read by nothing, which reads as a
    /// bound that is being enforced.
    ///
    /// The step budget is deliberately left wide open here: a run can blow its
    /// context in three steps if the pages are big, and a test that could also
    /// have been satisfied by `max_steps` would prove nothing about this bound.
    #[tokio::test]
    async fn a_conversation_that_outgrows_max_tokens_stops_and_says_so() {
        let replies: Vec<LlmReply> = (0..10).map(|_| tool_call_reply("kb_read_page")).collect();
        let agent = SubAgent {
            completer: Box::new(MockCompleter::new(replies)),
            tools: vec![],
            system_prompt: "sys".into(),
            bounds: SubAgentBounds {
                max_steps: 10,
                max_tokens: 100, // ≈400 characters
                ..Default::default()
            },
        };
        // One oversized page result per step, so the second check sees a
        // conversation past the budget.
        struct BigPages;
        #[async_trait]
        impl ToolDispatch for BigPages {
            async fn call(&self, _name: &str, _args: serde_json::Value) -> Result<String> {
                Ok("x".repeat(2000))
            }
        }

        let result = agent
            .run("hello", std::sync::Arc::new(BigPages), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::TokenBudgetReached);
        assert!(
            result.steps_used < 10,
            "the token bound must bite before the step bound, got {} steps",
            result.steps_used
        );
    }

    #[test]
    fn zero_elapsed_at_zero_wall_budget_stops_before_the_first_step() {
        let mut agent = make_agent(MockCompleter::new(vec![]), 1);
        agent.bounds.max_wall = Duration::ZERO;
        // Saturated elapsed time models the controlled same-tick clock boundary.
        let started = Instant::now() + Duration::from_secs(86_400);
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert_eq!(
            agent.stop_before_step(0, started, &[], None),
            Some(DoneReason::TimeBudgetReached)
        );
    }

    #[tokio::test]
    async fn zero_wall_budget_never_calls_a_ready_completer() {
        let received = Arc::new(Mutex::new(vec![]));
        let agent = SubAgent {
            completer: Box::new(RecordingCompleter {
                replies: Mutex::new(vec![text_reply("must not be requested")]),
                received: Arc::clone(&received),
            }),
            tools: vec![],
            system_prompt: "sys".into(),
            bounds: SubAgentBounds {
                max_wall: Duration::ZERO,
                ..Default::default()
            },
        };
        let result = agent
            .run("hello", Arc::new(EchoDispatch), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::TimeBudgetReached);
        assert_eq!(result.steps_used, 0);
        assert!(received.lock().await.is_empty());
    }

    /// The other half of DR-16b: `max_wall` was checked only *between*
    /// iterations, so one hung provider call was unbounded — a 300 s budget
    /// could sit on a single `await` for as long as the socket stayed open,
    /// and the ingest UI shows a run that never finishes and never fails.
    #[tokio::test]
    async fn a_hung_provider_call_is_cut_off_at_the_wall_clock() {
        /// Never answers within any budget a test would set. A real one of
        /// these is a socket the far end forgot.
        struct NeverAnswers;

        #[async_trait]
        impl Completer for NeverAnswers {
            async fn complete(
                &self,
                _system: &str,
                _messages: &[LlmMessage],
                _tools: &[Tool],
            ) -> Result<LlmReply> {
                std::future::pending().await
            }
        }

        // A real (tiny) budget rather than tokio's paused clock, which needs the
        // `test-util` feature this workspace does not enable. The run ends when
        // the budget does, so the test costs exactly this much wall time — and
        // the budget is generous enough that the between-steps check cannot win
        // the race and report the same reason for the wrong cause.
        let agent = SubAgent {
            completer: Box::new(NeverAnswers),
            tools: vec![],
            system_prompt: "sys".into(),
            bounds: SubAgentBounds {
                max_wall: Duration::from_millis(250),
                ..Default::default()
            },
        };
        let result = agent
            .run("hello", std::sync::Arc::new(EchoDispatch), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::TimeBudgetReached);
        assert!(
            result.final_text.contains("waiting for the model"),
            "the reason must distinguish a hung call from a long run, got: {}",
            result.final_text
        );
    }

    /// A level-triggered token cannot lose a cancellation that arrives before
    /// the loop starts.
    #[tokio::test]
    async fn cancellation_before_the_loop_starts_is_observed() {
        let cancel = CancellationToken::new();
        cancel.cancel();

        let completer = MockCompleter::new(vec![text_reply("never reached")]);
        let agent = make_agent(completer, 30);
        let result = agent
            .run(
                "hello",
                std::sync::Arc::new(EchoDispatch),
                Some(&cancel),
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::Cancelled);
    }

    struct SetOnDrop(Arc<std::sync::atomic::AtomicBool>);

    impl Drop for SetOnDrop {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    struct BlockingCompleter {
        started: Arc<Notify>,
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl Completer for BlockingCompleter {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[LlmMessage],
            _tools: &[Tool],
        ) -> Result<LlmReply> {
            let _drop_probe = SetOnDrop(Arc::clone(&self.dropped));
            self.started.notify_one();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn cancelling_a_blocked_provider_drops_its_future() {
        let started = Arc::new(Notify::new());
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = CancellationToken::new();
        let agent = SubAgent {
            completer: Box::new(BlockingCompleter {
                started: Arc::clone(&started),
                dropped: Arc::clone(&dropped),
            }),
            tools: vec![],
            system_prompt: "sys".into(),
            bounds: SubAgentBounds::default(),
        };
        let run_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            agent
                .run(
                    "hello",
                    std::sync::Arc::new(EchoDispatch),
                    Some(&run_cancel),
                    None,
                )
                .await
                .unwrap()
        });

        started.notified().await;
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("cancellation must not wait for the provider wall-clock bound")
            .unwrap();

        assert_eq!(result.reason, DoneReason::Cancelled);
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "the blocked provider future remained alive after cancellation"
        );
    }

    struct BlockingMutation {
        started: Arc<Notify>,
        dropped: Arc<std::sync::atomic::AtomicBool>,
        mutations: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl ToolDispatch for BlockingMutation {
        async fn call(&self, _name: &str, _args: serde_json::Value) -> Result<String> {
            let _drop_probe = SetOnDrop(Arc::clone(&self.dropped));
            self.started.notify_one();
            std::future::pending::<()>().await;
            self.mutations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok("mutated".into())
        }
    }

    #[tokio::test]
    async fn cancelling_a_blocked_tool_prevents_the_mutation_from_continuing() {
        let started = Arc::new(Notify::new());
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mutations = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = CancellationToken::new();
        let agent = make_agent(
            MockCompleter::new(vec![tool_call_reply("kb_write_page")]),
            30,
        );
        let dispatch = Arc::new(BlockingMutation {
            started: Arc::clone(&started),
            dropped: Arc::clone(&dropped),
            mutations: Arc::clone(&mutations),
        });
        let run_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            agent
                .run("hello", dispatch, Some(&run_cancel), None)
                .await
                .unwrap()
        });

        started.notified().await;
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("cancellation must not wait for a blocked tool")
            .unwrap();

        assert_eq!(result.reason, DoneReason::Cancelled);
        assert!(
            dropped.load(std::sync::atomic::Ordering::SeqCst),
            "the blocked tool future remained alive after cancellation"
        );
        assert_eq!(
            mutations.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a cancelled tool continued into its mutation"
        );
    }

    /// Test: when the assistant returns 2 tool calls in one turn, the loop must
    /// push exactly ONE `LlmMessage::ToolResults` (not two separate `ToolResult`
    /// entries) so that Bedrock sees a single user message with both results.
    #[tokio::test]
    async fn loop_bundles_multiple_tool_calls_into_one_tool_results_message() {
        // Shared recording store so we can inspect what the completer saw after run().
        let received: Arc<Mutex<Vec<Vec<LlmMessage>>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();

        let recording = RecordingCompleter {
            replies: Mutex::new(vec![
                two_tool_call_reply(), // step 0: assistant issues 2 tool calls
                text_reply("done"),    // step 1: no more tool calls → loop exits
            ]),
            received: received_clone,
        };

        let agent = SubAgent {
            completer: Box::new(recording),
            tools: vec![],
            system_prompt: "sys".into(),
            bounds: SubAgentBounds {
                max_steps: 10,
                ..Default::default()
            },
        };
        let result = agent
            .run("go", std::sync::Arc::new(EchoDispatch), None, None)
            .await
            .unwrap();
        assert_eq!(result.reason, DoneReason::NoMoreToolCalls);

        // calls[0] = first complete() call = [User]  (initial)
        // calls[1] = second complete() call = [User, Assistant(2 tool calls), ToolResults(2 parts)]
        let calls = received.lock().await;
        assert!(
            calls.len() >= 2,
            "expected at least 2 completer calls, got {}",
            calls.len()
        );
        let msgs_after_first_turn = &calls[1];

        // The message right after the Assistant turn must be a single ToolResults,
        // not two separate ToolResult entries.
        let tool_result_msgs: Vec<&LlmMessage> = msgs_after_first_turn
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    LlmMessage::ToolResults(_) | LlmMessage::ToolResult { .. }
                )
            })
            .collect();

        assert_eq!(
            tool_result_msgs.len(),
            1,
            "all tool results from one turn must collapse into exactly ONE message; got {}",
            tool_result_msgs.len()
        );

        // And that one message must be the ToolResults compound variant.
        assert!(
            matches!(tool_result_msgs[0], LlmMessage::ToolResults(parts) if parts.len() == 2),
            "the single tool-result message must be LlmMessage::ToolResults with 2 parts"
        );
    }

    /// A dispatcher that records every call it is handed, so a test can assert
    /// what actually ran rather than what the loop said it did.
    #[derive(Default)]
    struct RecordingDispatch {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ToolDispatch for RecordingDispatch {
        async fn call(&self, name: &str, _args: serde_json::Value) -> Result<String> {
            self.calls.lock().await.push(name.to_string());
            Ok("ok".to_string())
        }
    }

    /// DR-15. The natural shape of a typed-extraction turn: N writes and then
    /// `complete`, in one assistant reply.
    ///
    /// The loop used to return on seeing the sentinel *before* the dispatch
    /// loop, so the write never ran — and the failure surfaced far away and
    /// wearing a disguise: nothing under `knowledge/` changed, so
    /// `txn_wrote_knowledge_pages` returned false, the txn aborted, and the
    /// ingest reported "wrote no knowledge pages", which reads as the model
    /// having authored nothing.
    #[tokio::test]
    async fn a_write_beside_the_complete_sentinel_is_still_dispatched() {
        let reply = LlmReply {
            text: "filed".into(),
            tool_calls: vec![
                LlmToolCall {
                    id: "tc-1".into(),
                    name: "kb_write_page".to_string(),
                    args: serde_json::Value::Object(Default::default()),
                },
                LlmToolCall {
                    id: "tc-2".into(),
                    name: "complete".to_string(),
                    args: serde_json::Value::Object(Default::default()),
                },
            ],
        };
        let dispatch = RecordingDispatch::default();
        let seen = dispatch.calls.clone();

        let agent = make_agent(MockCompleter::new(vec![reply]), 10);
        let result = agent
            .run("go", std::sync::Arc::new(dispatch), None, None)
            .await
            .unwrap();

        assert_eq!(
            *seen.lock().await,
            vec!["kb_write_page".to_string()],
            "the sibling write must run, and the sentinel must not be dispatched \
             as if it were a tool"
        );
        assert_eq!(
            result.reason,
            DoneReason::CompleteSentinel,
            "the sentinel is still honoured, just not before its siblings"
        );
        assert_eq!(result.final_text, "filed");
    }

    /// Test 4: with an event_sink, events arrive in the channel as the loop runs.
    #[tokio::test]
    async fn run_emits_events_to_sink_live() {
        use tokio::sync::mpsc;

        let (tx, mut rx) = mpsc::unbounded_channel::<SubAgentEvent>();

        // Two-step run: one tool call, then a text-only reply → NoMoreToolCalls.
        let completer = MockCompleter::new(vec![tool_call_reply("kb_search"), text_reply("done")]);
        let agent = make_agent(completer, 10);

        let _ = agent
            .run("test", std::sync::Arc::new(EchoDispatch), None, Some(&tx))
            .await
            .unwrap();

        // Close the sender so rx drains to exhaustion.
        drop(tx);

        let mut count = 0usize;
        while rx.recv().await.is_some() {
            count += 1;
        }
        assert!(
            count > 0,
            "event sink must have received at least one event"
        );
    }
}
