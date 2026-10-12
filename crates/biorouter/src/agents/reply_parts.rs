use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;

use async_stream::try_stream;
use futures::stream::StreamExt;
use serde_json::{json, Value};
use tracing::debug;

use super::super::agents::Agent;
use crate::conversation::message::{Message, MessageContent, ToolRequest};
use crate::conversation::Conversation;
use crate::providers::base::{
    stream_from_single_message, MessageStream, Provider, ProviderSteerReceiver, ProviderUsage,
};
use crate::providers::errors::ProviderError;
use crate::providers::toolshim::{
    convert_tool_messages_to_text, interpret_tool_requests_from_text,
    modify_system_prompt_for_tool_json, OllamaInterpreter, ToolInterpreter,
};

use crate::agents::code_execution_extension::EXTENSION_NAME as CODE_EXECUTION_EXTENSION;
use crate::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME;
use crate::agents::subagent_tool::{SUBAGENT_TOOL_NAME, SUBAGENT_TOOL_PREFIXED};
use crate::session::SessionType;

const SUBAGENT_STEERING_INSTRUCTIONS: &str = "A human can send a new user message directly into this subagent while it is running. At the next safe boundary, the newest direct user message supersedes any conflicting earlier task instruction. Act on it immediately before continuing prior work; do not merely record it or finish the old task first.";
use crate::session::session_manager::UsageLedgerEntry;
use rmcp::model::Tool;

fn attach_skill_inventory(
    extensions_info: &mut [crate::agents::extension::ExtensionInfo],
    inventory: &str,
) -> bool {
    let Some(skills) = extensions_info.iter_mut().find(|info| {
        info.classification == crate::agents::extension::ExtensionClassification::Capability
            && info.name.eq_ignore_ascii_case("skills")
    }) else {
        return false;
    };
    if !inventory.is_empty() {
        skills.instructions.push_str("\n\n");
        skills.instructions.push_str(inventory);
    }
    true
}

pub(super) fn attach_effective_tool_rosters(
    extensions_info: &mut [crate::agents::extension::ExtensionInfo],
    available_tools: &[Tool],
    directly_callable_tools: &[Tool],
) {
    for info in extensions_info {
        info.tool_roster_known = true;
        let prefix = format!("{}__", info.name);
        info.available_tools = available_tools
            .iter()
            .filter_map(|tool| tool.name.strip_prefix(&prefix).map(str::to_string))
            .collect();
        info.directly_callable_tools = directly_callable_tools
            .iter()
            .filter_map(|tool| tool.name.strip_prefix(&prefix).map(str::to_string))
            .collect();
        info.available_tools.sort();
        info.directly_callable_tools.sort();

        // ⚠ **`info.name` is the manager's map KEY, not the extension's declared
        // name.** `get_extensions_info` yields the normalized key, so Workspace
        // arrives as `workspace` while `EXTENSION_NAME` is the literal
        // `"Workspace"` — the comparison could never be true and this whole
        // branch was dead, leaving the full Workspace instructions in the prompt
        // even when only the delegation tools were callable. Code Execution's
        // constant happens to be already-normalized (`code_execution`), which is
        // exactly why the same bug next door does not show.
        if info.classification == crate::agents::extension::ExtensionClassification::Capability
            && info.name
                == crate::config::extensions::name_to_key(
                    crate::agents::workspace_extension::EXTENSION_NAME,
                )
            && !info
                .available_tools
                .iter()
                .any(|tool| tool == "workspace_open")
        {
            info.instructions = "Workspace delegation-only mode. Use only the exact effective roster above to spawn, inspect, steer, watch, or close work. Panel control, opening arbitrary conversations, and changing another conversation's tools are unavailable unless their tools are explicitly listed.".to_string();
        }
        if info.classification == crate::agents::extension::ExtensionClassification::Capability
            && info.name == CODE_EXECUTION_EXTENSION
            && !info
                .available_tools
                .iter()
                .any(|tool| tool == "execute_code")
        {
            info.instructions = "Code Execution is restricted to the effective inspection tools above. Do not run code or hide other direct tools unless `execute_code` is present.".to_string();
        }
        // Same idiom, third capability. Skills' prose names seven operations,
        // and `get_extensions_info` — which is what puts that prose in EVERY
        // turn's system prompt — is filtered by `allowed_extension_keys` and
        // never by the conversation's effective roster. An Agent-Drafter app
        // agent is granted `searchSkills` + `loadSkill` and read instructions
        // for five more that answer with a refusal.
        //
        // Unlike the two above this one does not swap in a fixed sentence: the
        // narrowed prose is generated from the roster, so it stays correct for
        // any subset rather than for the one subset somebody thought of. The
        // full-roster case is rebuilt from the same table and is byte-identical
        // to the static text, so the arm is a no-op wherever nothing was taken
        // away — which is why it is not gated on a marker tool.
        //
        // `available_tools`, not `directly_callable_tools`: Code Execution
        // narrowing is not a loss of capability (the tools stay reachable from
        // a script) and must not rewrite this prose.
        if info.classification == crate::agents::extension::ExtensionClassification::Capability
            && info.name
                == crate::config::extensions::name_to_key(
                    crate::agents::skills_extension::EXTENSION_NAME,
                )
        {
            info.instructions =
                crate::agents::skills_extension::instructions_for_operations(&info.available_tools);
        }
    }
}

pub(super) fn add_core_platform_capability(
    extensions_info: &mut Vec<crate::agents::extension::ExtensionInfo>,
    tools: &[Tool],
) {
    if tools.iter().any(|tool| tool.name.starts_with("platform__"))
        && !extensions_info.iter().any(|info| info.name == "platform")
    {
        extensions_info.push(crate::agents::extension::ExtensionInfo::capability(
            "platform",
            "Core Biorouter operations supplied by the current application session. Use only the exact effective roster; knowledge ingestion appears here only while Knowledge is enabled.",
            false,
        ));
    }
}

fn coerce_value(s: &str, schema: &Value) -> Value {
    let type_str = schema.get("type");

    match type_str {
        Some(Value::String(t)) => match t.as_str() {
            "number" | "integer" => try_coerce_number(s),
            "boolean" => try_coerce_boolean(s),
            _ => Value::String(s.to_string()),
        },
        Some(Value::Array(types)) => {
            // Try each type in order
            for t in types {
                if let Value::String(type_name) = t {
                    match type_name.as_str() {
                        "number" | "integer" if s.parse::<f64>().is_ok() => {
                            return try_coerce_number(s)
                        }
                        "boolean" if matches!(s.to_lowercase().as_str(), "true" | "false") => {
                            return try_coerce_boolean(s)
                        }
                        _ => continue,
                    }
                }
            }
            Value::String(s.to_string())
        }
        _ => Value::String(s.to_string()),
    }
}

fn try_coerce_number(s: &str) -> Value {
    if let Ok(n) = s.parse::<f64>() {
        if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            json!(n as i64)
        } else {
            json!(n)
        }
    } else {
        Value::String(s.to_string())
    }
}

fn try_coerce_boolean(s: &str) -> Value {
    match s.to_lowercase().as_str() {
        "true" => json!(true),
        "false" => json!(false),
        _ => Value::String(s.to_string()),
    }
}

fn coerce_tool_arguments(
    arguments: Option<serde_json::Map<String, Value>>,
    tool_schema: &Value,
) -> Option<serde_json::Map<String, Value>> {
    let args = arguments?;

    let properties = tool_schema.get("properties").and_then(|p| p.as_object())?;

    let mut coerced = serde_json::Map::new();

    for (key, value) in args.iter() {
        let coerced_value =
            if let (Value::String(s), Some(prop_schema)) = (value, properties.get(key)) {
                coerce_value(s, prop_schema)
            } else {
                value.clone()
            };
        coerced.insert(key.clone(), coerced_value);
    }

    Some(coerced)
}

/// Survives the code-execution tool filter?
///
/// When the `code_execution` extension is active the model is meant to reach
/// ordinary tools by *writing code*, so the tool list collapses to
/// `code_execution__*`. The exemptions below are the tools code cannot express.
///
/// ⚠ **The premise of the whole filter is that a dropped tool is still
/// reachable by writing code.** Anything the JS catalogue cannot list must be
/// exempted here or it becomes reachable from NOWHERE. The catalogue is
/// [`ExtensionManager::get_prefixed_tools_excluding`], so the question this
/// predicate has to answer for is "is this tool in **that catalogue**".
///
/// That is NOT the same question as "is this tool in the `ExtensionManager`",
/// and reading it that way is how the class stayed open after issue #141 was
/// closed for one family. Four families are absent from the catalogue, and only
/// one of them for the reason "not in the manager":
///
/// * the `platform__*` tools — dispatched by [`Agent::dispatch_tool_call`] and
///   in no extension at all. Dropping them left `platform.ingest_source`
///   answering `Module not found: platform` with no direct call to fall back to
///   (issue #141): it cost the chat knowledge ingestion, scheduler management,
///   and BR-7's recovery path for an offloaded tool result, whose stub tells the
///   model to call `platform__read_session_blob`.
/// * `workflow__final_output` — pushed onto the roster by
///   [`Agent::list_tools_for`] from the Agent's own `final_output_tool`, and
///   dispatched by the agent loop. `code_execution` is `default_enabled: true`,
///   so without this exemption a workflow armed with `response.json_schema` has
///   no way at all to emit its structured output.
/// * the frontend tools — held in `Agent::frontend_tools`, appended in
///   [`Agent::prepare_model_tool_surface`] and executed by the interface, so
///   they are neither importable nor dispatchable from the sandbox. They are the
///   one dynamic family, which is why this predicate has to be *told* their
///   names rather than recognising them by shape.
/// * `workspace__subagent` — the one family that IS in the manager: the
///   catalogue builder strips it on the way out, for the same reason it is
///   exempted here.
///
/// The remaining `workspace__*` tools are in the catalogue and exempted anyway,
/// because operating the daemon and the GUI from inside the sandbox is not what
/// that surface is for. That is deliberate and safe: an exemption may be a
/// SUPERSET of the rule, and only a shortfall reaches nowhere. The guard that
/// pins the shortfall direction end-to-end is
/// `every_tool_absent_from_the_code_execution_catalogue_stays_directly_callable`
/// below.
///
/// The five Todo tools are the second superset exemption: in the catalogue AND
/// kept, for a reason about models rather than plumbing. A planning tool that
/// is reachable only by writing JavaScript inside `execute_code` is one a model
/// does not reach for — measured in the 2026-09-10 composer QA run, where the
/// collapsed roster had 18 tools, no `todo__*`, and the model built a checklist
/// exactly once, when told to. They are cheap structural calls, the same class
/// as the platform tools, and the planning gate (`agents::planning_gate`)
/// points a multi-step turn at `todo__todo_write` by name, which only works if
/// the model can call it.
///
/// Both name forms of the spawn tool are kept — models strip prefixes.
///
/// [`ExtensionManager::get_prefixed_tools_excluding`]: crate::agents::ExtensionManager::get_prefixed_tools_excluding
/// [`Agent::dispatch_tool_call`]: crate::agents::Agent
/// [`Agent::list_tools_for`]: crate::agents::Agent
/// [`Agent::prepare_model_tool_surface`]: crate::agents::Agent
pub(crate) fn survives_code_execution_filter(
    tool_name: &str,
    code_exec_prefix: &str,
    frontend_tool_names: &HashSet<String>,
) -> bool {
    tool_name.starts_with(code_exec_prefix)
        || crate::security::computer_use::is_computer_use_tool(tool_name)
        || tool_name == SUBAGENT_TOOL_NAME
        || tool_name == SUBAGENT_TOOL_PREFIXED
        // BR-40's separate poll tool used to need its own clause here, so a
        // model that spawned a background child could still collect it. BR-71
        // decision 23 deleted that tool: collecting a background child is now
        // `workspace_watch` / `workspace_read_conversation` / `workspace_close`,
        // which the `workspace__` prefix below already exempts.
        || tool_name.starts_with("workspace__")
        // The predicate, not a second hand-written list: `code_execution`'s
        // refusal path asks the same question, so a fifth platform tool is
        // covered by both the day it is added.
        || crate::agents::platform_tools::is_platform_tool_name(tool_name)
        // The one constant the workflow's structured-output tool is built from,
        // for the same reason.
        || tool_name == FINAL_OUTPUT_TOOL_NAME
        // Sampled by the caller from `Agent::frontend_tools` — there is no name
        // shape to test, because a frontend tool is named by whichever client
        // registered it.
        || frontend_tool_names.contains(tool_name)
        // `kb_delete_base`: a fifth family, and the first one that is absent
        // from the catalogue because the sandbox may not RUN it rather than
        // because the sandbox cannot reach it. Deleting a knowledge base is
        // shown to the user for approval first, and a script call is dispatched
        // with no inspector chain to raise that approval — so the catalogue
        // strips it (`ExtensionManager::get_prefixed_tools_excluding`) and this
        // keeps it directly callable. The same predicate answers both, so the
        // pair cannot drift into "dropped from the catalogue, dropped from the
        // roster" — which is the state that reaches nowhere, and the state this
        // tool shipped in for exactly one live run.
        || crate::security::knowledge_delete::is_knowledge_delete_tool(tool_name)
        // The checklist: exact names, never a `todo__` prefix — see
        // `TODO_TOOL_NAMES` for why the prefix is not the builtin's to claim.
        || crate::agents::todo_extension::is_todo_tool_name(tool_name)
}

fn code_execution_mode_is_active(loaded: bool, tools: &[Tool]) -> bool {
    loaded
        && tools
            .iter()
            .any(|tool| tool.name == format!("{CODE_EXECUTION_EXTENSION}__execute_code"))
}

struct PreparedModelToolSurface {
    directly_callable_tools: Vec<Tool>,
    available_tools: Vec<Tool>,
    bridge_plan: Option<crate::agents::agent::CodingAgentBridgePlan>,
    bridge_replaces_tool_surface: bool,
    code_execution_active: bool,
}

/// What the toolshim has seen of one streamed reply.
///
/// OLL-09: the interpreter used to run once PER STREAMED ITEM — one HTTP round
/// trip to a second model per token, each seeing one token — so a call written
/// across many deltas was never seen whole, and the shim could not work on any
/// streaming provider. The reply is now assembled here and interpreted once,
/// after the stream ends.
#[derive(Default)]
struct ToolshimReply {
    text: String,
    /// The streamed message id, so the recovered calls fold into the same
    /// assistant message (`Conversation::push` merges same-id items) exactly as
    /// a natively decoded text-then-call reply does.
    message_id: Option<String>,
    /// The provider already returned a structured call: nothing to interpret.
    native_call: bool,
}

impl ToolshimReply {
    fn observe(&mut self, message: &Message) {
        self.native_call |= message.is_tool_call();
        for content in &message.content {
            if let MessageContent::Text(text) = content {
                self.text.push_str(&text.text);
            }
        }
        if message.id.is_some() {
            self.message_id.clone_from(&message.id);
        }
    }

    /// The recovered calls as one message, or `None` when there are none — or
    /// when the interpreter failed.
    ///
    /// ⚠ A failure here is NOT the reply's failure. This runs after every text
    /// item has been yielded, so the user already holds the whole answer. As an
    /// `Err` it was a recoverable `ExecutionError`, and the mistake ladder
    /// (BR-66) regenerated the ENTIRE reply — minutes on a local 27B — failed
    /// the same way, and ended the turn as failed with the answer shown twice.
    /// The reply is kept as the text it is, which is what a chat without the
    /// shim shows; the misconfigurations that make an interpreter fail every
    /// time are refused before the model is asked
    /// ([`OllamaInterpreter::ensure_ready`]). Logged as metadata only: the
    /// error can quote the reply.
    async fn interpret<T: ToolInterpreter + Sync>(
        &self,
        interpreter: &T,
        toolshim_tools: &[Tool],
    ) -> Option<Message> {
        if self.native_call {
            return None;
        }
        // The requests as the toolshim built them, not re-minted here: a call
        // the text grammar recovered carries the `recovered_from_text` marker
        // in its metadata (G9), and rebuilding it from bare call params, as
        // this did while the toolshim had only an interpreter model, would
        // silently strip it.
        let requests = match interpret_tool_requests_from_text(
            interpreter,
            &self.text,
            toolshim_tools,
        )
        .await
        {
            Ok(requests) => requests,
            Err(e) => {
                tracing::warn!(
                    error_type = e.telemetry_type(),
                    error_kind = ?e.kind(),
                    reply_bytes = self.text.len(),
                    "tool interpreter failed after the reply streamed; the reply is kept as \
                     text and no tool call was recovered from it"
                );
                return None;
            }
        };
        if requests.is_empty() {
            return None;
        }
        let mut message = Message::assistant();
        for request in requests {
            message = message.with_content(request);
        }
        if let Some(id) = &self.message_id {
            message = message.with_id(id.clone());
        }
        Some(message)
    }
}

/// Unsigned thinking held back until the reasoning phase it belongs to ends,
/// then handed on as ONE block.
///
/// The local decoders hand thinking on while it streams — the native Ollama
/// one and the OpenAI-compatible one used for Llama Server each yield a batch
/// every 250 ms (`formats::openai::THINKING_FLUSH_INTERVAL`) — and Codex yields
/// one message per reasoning delta. No client merges those (measured
/// 2026-10-10), and each one made two of them worse:
///
/// - the desktop renders no `thinking` item at all, while every Message frame
///   moves `lastMessageAt`, so the "Thinking · 0:42" elapsed chip (shown from
///   2 s) and the 45 s "still working" nudge never appeared during a minute
///   of thinking that used to show both;
/// - the interactive CLI hides its spinner on every message
///   (`session/mod.rs`), so the first batch, 250 ms in, left the terminal
///   looking hung until the answer; with `BIOROUTER_CLI_SHOW_THINKING` it
///   printed a "Thinking:" header over every 400-byte fragment.
///
/// Held here, the decoders stay live and every client sees what it already
/// understands: one block per phase, just before the answer or tool call it
/// led to. ⚠ Delete this hold once the desktop and the CLI merge same-id
/// thinking items into one block (and stop treating a thinking-only message as
/// the end of "Thinking"); then the batches become live progress with no
/// decoder change.
///
/// Signed thinking is never held: its signature covers exactly one block
/// (Anthropic, Bedrock), and those providers send it whole anyway. Nor is a
/// message that carries anything besides unsigned thinking; it ends the phase.
/// The cost: a stream dropped mid-phase (the turn cancelled, or restarted for
/// a steer) drops the held thinking too, unshown and unstored. Only thinking
/// that had not yet led to anything is lost; a failure the stream reports
/// itself still gets it out first.
#[derive(Default)]
struct HeldThinking {
    message: Option<Message>,
}

impl HeldThinking {
    /// Whether `message` is nothing but unsigned thinking — the only shape held.
    fn holds(message: &Message) -> bool {
        !message.content.is_empty()
            && message.content.iter().all(
                |content| matches!(content, MessageContent::Thinking(t) if t.signature.is_empty()),
            )
    }

    /// Hold `message` (which [`Self::holds`]), joined onto what is already
    /// held for the same reply. A different reply's thinking — Codex gives each
    /// reasoning item its own id — cannot be joined, so the block held for the
    /// earlier one comes back to be yielded first.
    fn hold(&mut self, message: Message) -> Option<Message> {
        let text: String = message
            .content
            .iter()
            .filter_map(|content| match content {
                MessageContent::Thinking(thinking) => Some(thinking.thinking.as_str()),
                _ => None,
            })
            .collect();
        if let Some(held) = self.message.as_mut().filter(|held| held.id == message.id) {
            if let Some(MessageContent::Thinking(block)) = held.content.first_mut() {
                block.thinking.push_str(&text);
            }
            return None;
        }
        let mut fresh = message;
        fresh.content = vec![MessageContent::thinking(text, "")];
        self.message.replace(fresh).and_then(Self::non_empty)
    }

    /// The held block, if any: the phase it belongs to has ended.
    fn release(&mut self) -> Option<Message> {
        self.message.take().and_then(Self::non_empty)
    }

    fn non_empty(message: Message) -> Option<Message> {
        let empty = matches!(
            message.content.first(),
            Some(MessageContent::Thinking(block)) if block.thinking.is_empty()
        );
        (!empty).then_some(message)
    }
}

/// The provider's stream as the agent loop receives it: unsigned thinking held
/// to one block per phase ([`HeldThinking`]), the toolshim watching the reply
/// and interpreting it once at the end, and the EXHAUSTED timing marker.
///
/// Split out of [`Agent::stream_response_from_provider`] so the request
/// plumbing and what is done to each item read separately.
/// Watches a reply for the one shape that leaves a person reading JSON where an
/// answer should be: a WHOLE reply of tool calls written as text, in a turn
/// that offered tools and made no call. Nothing in it ran, and before this
/// nothing said so.
///
/// Diagnostics 20261006_2: llama3.1:8b, in a Code Execution chat, answered
/// three requests with `text_editor`, `shell` and `agent_drafter` calls
/// written as JSON. None names a tool that chat offered, so the provider
/// neither recovered them nor asked the model again, and each turn ended on the
/// JSON. The person gave up and asked for code to run by hand. The calls are
/// never run from here either; the person is only told.
#[derive(Default)]
struct UnrunCallWatch {
    text: String,
    called: bool,
}

impl UnrunCallWatch {
    fn observe(&mut self, message: &Message) {
        self.called |= message.content.iter().any(|content| {
            matches!(
                content,
                MessageContent::ToolRequest(_) | MessageContent::FrontendToolRequest(_)
            )
        });
        for content in &message.content {
            if let MessageContent::Text(text) = content {
                self.text.push_str(&text.text);
            }
        }
    }

    /// The notice for a finished reply, when it needs one.
    fn notice(&self) -> Option<Message> {
        if self.called {
            return None;
        }
        let names = crate::providers::formats::text_tool_calls::whole_reply_call_names(&self.text)?;
        Some(unrun_call_notice(&names))
    }
}

/// What the person reads under a reply that is only tool calls written as
/// text. User-only: the model never sees it.
fn unrun_call_notice(names: &[String]) -> Message {
    const SHOWN: usize = 3;
    let mut quoted: Vec<String> = names
        .iter()
        .take(SHOWN)
        .map(|name| format!("'{name}'"))
        .collect();
    if names.len() > SHOWN {
        quoted.push(format!("{} more", names.len() - SHOWN));
    }
    let what = match quoted.as_slice() {
        [one] => format!("a call to {one}"),
        [first, second] => format!("calls to {first} and {second}"),
        [init @ .., last] => format!("calls to {} and {last}", init.join(", ")),
        [] => "a tool call".to_string(),
    };
    Message::assistant()
        .with_system_notification(
            crate::conversation::message::SystemNotificationType::InlineMessage,
            format!(
                "Nothing ran: this reply is {what} written as text, not a tool call Biorouter \
                 can run. Smaller local models often do this. A larger model, or fewer \
                 extensions turned on in this chat, may help."
            ),
        )
        .user_only()
}

fn agent_facing_stream(
    mut stream: MessageStream,
    toolshim_interpreter: Option<OllamaInterpreter>,
    toolshim_tools: Vec<Tool>,
    tools_offered: bool,
    streaming: bool,
    stream_open_start: std::time::Instant,
) -> MessageStream {
    Box::pin(try_stream! {
        let mut toolshim_reply = ToolshimReply::default();
        let mut held_thinking = HeldThinking::default();
        let mut unrun_calls = UnrunCallWatch::default();
        while let Some(result) = stream.next().await {
            let (message, usage, pending) = match result {
                Ok(item) => item,
                Err(e) => {
                    // What the model thought before the failure is still
                    // shown, as it was while the decoders' batches went
                    // straight through.
                    if let Some(thinking) = held_thinking.release() {
                        yield (Some(thinking), None, None);
                    }
                    Err(e)?
                }
            };

            // Store the model information in the global store
            if let Some(usage) = usage.as_ref() {
                crate::providers::base::set_current_model(&usage.model);
            }

            // The toolshim only WATCHES while the reply streams; the text
            // still reaches the user token by token.
            if let (Some(message), Some(_)) = (message.as_ref(), toolshim_interpreter.as_ref()) {
                toolshim_reply.observe(message);
            }
            if let (Some(message), true) = (message.as_ref(), tools_offered) {
                unrun_calls.observe(message);
            }

            // Thinking waits for the end of its phase; see `HeldThinking`.
            let message = match message {
                Some(thinking) if HeldThinking::holds(&thinking) => {
                    if let Some(earlier) = held_thinking.hold(thinking) {
                        yield (Some(earlier), None, None);
                    }
                    if usage.is_none() && pending.is_none() {
                        continue;
                    }
                    None
                }
                other => other,
            };
            // Anything else ends the phase: the held block goes first, so
            // it still lands before the output it led to.
            if let Some(thinking) = held_thinking.release() {
                yield (Some(thinking), None, None);
            }

            // `pending` is a display-only tool-call notification. It travels
            // its own slot, never becoming a `Message`, so it can never be
            // dispatched. Forwarded verbatim to the agent loop.
            yield (message, usage, pending);
        }
        // A reply that only thought.
        if let Some(thinking) = held_thinking.release() {
            yield (Some(thinking), None, None);
        }

        // One interpretation of the whole reply, after it ended. The calls
        // arrive as one more message under the reply's id — the shape a
        // provider's own text-then-call stream has.
        if let Some(interpreter) = toolshim_interpreter.as_ref() {
            if let Some(calls) = toolshim_reply.interpret(interpreter, &toolshim_tools).await {
                unrun_calls.observe(&calls);
                yield (Some(calls), None, None);
            }
        }

        // Last, after any call the toolshim found: a reply that is nothing but
        // calls written as text, none of which ran, gets one line saying so.
        if let Some(notice) = unrun_calls.notice() {
            yield (Some(notice), None, None);
        }

        // The stream is drained: generation is genuinely finished. This is
        // the marker the old instrumentation lacked entirely, which is why
        // generation time was invisible and prior investigations attributed
        // the whole model wait to the (tiny) stream-open span.
        //
        // Emitted on BOTH paths — on the non-streaming path this is a
        // one-element stream and the total duplicates FULL_GENERATION_END.
        // `streaming` is what makes the two distinguishable; it also lets a
        // report count OPENED vs EXHAUSTED per path, which is how a turn
        // aborted before the stream drained (no EXHAUSTED) shows up.
        debug!(
            streaming,
            total_ms = stream_open_start.elapsed().as_millis() as u64,
            "WAITING_LLM_STREAM_EXHAUSTED"
        );
    })
}

impl Agent {
    async fn prepare_model_tool_surface(
        &self,
        session_id: &str,
        provider: &Arc<dyn Provider>,
    ) -> PreparedModelToolSurface {
        let mut tools = self.list_tools(session_id, None).await;

        let frontend_tools = self.frontend_tools.lock().await;
        tools.extend(
            frontend_tools
                .values()
                .map(|frontend| frontend.tool.clone()),
        );
        // Sampled from the SAME lock guard that just supplied the tools, so the
        // Code Execution filter below can never disagree with the list it is
        // filtering about which names are frontend ones.
        let frontend_tool_names: HashSet<String> = frontend_tools.keys().cloned().collect();
        drop(frontend_tools);

        let code_execution_loaded = self
            .extension_manager
            .is_extension_enabled(CODE_EXECUTION_EXTENSION)
            .await;
        let available_tools = tools.clone();
        let bridge_plan = self
            .prepare_coding_agent_bridge_plan(provider, &available_tools)
            .await;
        let bridge_replaces_tool_surface = provider.uses_tool_bridge_for_tool_surface();
        let active_bridge_plan = bridge_replaces_tool_surface.then(|| {
            bridge_plan
                .as_ref()
                .expect("a bridge-backed provider always has a bridge plan")
        });
        let code_execution_active = !bridge_replaces_tool_surface
            && code_execution_mode_is_active(code_execution_loaded, &available_tools);
        if let Some(plan) = active_bridge_plan {
            tools.clone_from(&plan.tools);
        } else if code_execution_active {
            let code_exec_prefix = format!("{CODE_EXECUTION_EXTENSION}__");
            tools.retain(|tool| {
                survives_code_execution_filter(&tool.name, &code_exec_prefix, &frontend_tool_names)
            });
        }

        // Stable ordering preserves multi-session prompt caching.
        tools.sort_by(|a, b| a.name.cmp(&b.name));

        PreparedModelToolSurface {
            directly_callable_tools: tools,
            available_tools,
            bridge_plan,
            bridge_replaces_tool_surface,
            code_execution_active,
        }
    }

    /// Number of tools advertised directly to the model bound to this session.
    ///
    /// This is the final model-facing surface: privacy filtering, frontend
    /// tools, Code Execution narrowing, and coding-agent bridge replacement have
    /// all been applied. The permission editor intentionally uses a different,
    /// unfiltered route so a human can still administer hidden private tools.
    pub async fn callable_tool_count(&self, session_id: &str) -> Result<usize> {
        let provider = self.provider().await?;
        let surface = self.prepare_model_tool_surface(session_id, &provider).await;
        Ok(surface.directly_callable_tools.len())
    }

    pub async fn prepare_tools_and_prompt(
        &self,
        session_id: &str,
        working_dir: &std::path::Path,
    ) -> Result<(Vec<Tool>, Vec<Tool>, String)> {
        let provider = self.provider().await?;
        let (tools, toolshim_tools, system_prompt, _bridge_plan) = self
            .prepare_tools_and_prompt_for_provider(session_id, working_dir, &provider)
            .await?;
        Ok((tools, toolshim_tools, system_prompt))
    }

    pub(super) async fn prepare_tools_and_prompt_for_provider(
        &self,
        session_id: &str,
        working_dir: &std::path::Path,
        provider: &Arc<dyn Provider>,
    ) -> Result<(
        Vec<Tool>,
        Vec<Tool>,
        String,
        Option<crate::agents::agent::CodingAgentBridgePlan>,
    )> {
        let surface = self.prepare_model_tool_surface(session_id, provider).await;
        let mut tools = surface.directly_callable_tools;
        let effective_tools = surface.available_tools;
        let bridge_plan = surface.bridge_plan;
        let bridge_replaces_tool_surface = surface.bridge_replaces_tool_surface;
        let code_execution_active = surface.code_execution_active;
        // Only actual turn preparation may replace shared execution policy.
        // Read-only consumers such as the callable-count route must not change
        // how an already-running turn grades or approves a tool call.
        self.tool_risks.refresh_from_tools(&tools);
        let active_bridge_plan = if bridge_replaces_tool_surface {
            Some(
                bridge_plan
                    .as_ref()
                    .expect("a bridge-backed provider always has a bridge plan"),
            )
        } else {
            None
        };

        // Prepare system prompt
        let mut extensions_info = self.extension_manager.get_extensions_info().await;
        let prompt_tools = active_bridge_plan
            .map_or_else(|| effective_tools.as_slice(), |plan| plan.tools.as_slice());
        add_core_platform_capability(&mut extensions_info, prompt_tools);
        attach_effective_tool_rosters(&mut extensions_info, prompt_tools, &tools);
        if extensions_info.iter().any(|info| {
            info.classification == crate::agents::extension::ExtensionClassification::Capability
                && info.name.eq_ignore_ascii_case("skills")
                && !info.available_tools.is_empty()
        }) {
            let inventory = crate::agents::skills_extension::session_skill_inventory_instructions(
                self.config.session_manager.as_ref(),
                session_id,
            )
            .await?;
            attach_skill_inventory(&mut extensions_info, &inventory);
        }

        let model_config = provider.get_model_config();

        // BR-3: pick the system-prompt variant for this provider/model so small
        // local models get extra scaffolding while strong models stay lean.
        // The instance's own answer to "does inference run on this machine",
        // not its registry name: a declarative `ollama_compatible` provider
        // registers under any name and must be prompted like `ollama` (OLL-20).
        let runs_on_this_machine = crate::providers::utils::runs_on_this_machine(provider.as_ref());
        let prompt_variant = crate::agents::prompt_manager::PromptVariant::select(
            provider.get_name(),
            &model_config.model_name,
            runs_on_this_machine,
        );
        // A local engine re-reads the whole prompt when its prefix changes, so
        // its clock is rendered by the day (`prompt_manager::PromptClock::Day`).
        let local_engine = crate::agents::prompt_manager::is_local_engine(
            provider.get_name(),
            runs_on_this_machine,
        );

        let is_subagent = matches!(
            self.config
                .session_manager
                .get_session(session_id, false)
                .await
                .ok()
                .map(|session| session.session_type),
            Some(SessionType::SubAgent)
        );
        // The same predicate, over the same roster, that decides whether the
        // turn enforces anything (`Agent::begin_planning_turn`) — so the prompt
        // can never describe a gate this turn does not run. Read before the
        // toolshim branch below empties `tools`.
        let checklist_enforcement = crate::agents::planning_gate::enforcement_applies(
            self.config.biorouter_mode,
            is_subagent,
            bridge_replaces_tool_surface,
            tools.iter().map(|tool| tool.name.as_ref()),
        );

        let prompt_manager = self.prompt_manager.lock().await;
        let enable_subagents = match active_bridge_plan {
            Some(plan) => plan.delegation_available,
            None => self.subagents_enabled(session_id).await,
        };
        let crew_scoped = crate::crew::manager()?.is_scoped_session(session_id).await;
        let builder = prompt_manager
            .builder()
            .with_extensions(extensions_info.into_iter())
            .with_frontend_instructions(self.frontend_instructions.lock().await.clone())
            .with_code_execution_mode(code_execution_active)
            .with_checklist_enforcement(checklist_enforcement)
            .with_enable_subagents(enable_subagents)
            .with_prompt_variant(prompt_variant)
            .with_local_engine(local_engine);
        let mut system_prompt = if crew_scoped {
            builder.build()
        } else {
            builder.with_hints(working_dir).build()
        };

        if is_subagent {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(SUBAGENT_STEERING_INSTRUCTIONS);
        }

        // Handle toolshim if enabled
        let mut toolshim_tools = vec![];
        if model_config.toolshim {
            // If tool interpretation is enabled, modify the system prompt
            system_prompt = modify_system_prompt_for_tool_json(&system_prompt, &tools);
            // Make a copy of tools before emptying
            toolshim_tools = tools.clone();
            // Empty the tools vector for provider completion
            tools = vec![];
        }

        Ok((tools, toolshim_tools, system_prompt, bridge_plan))
    }

    /// Stream a response from the LLM provider.
    /// Handles toolshim transformations if needed
    pub(crate) async fn stream_response_from_provider(
        provider: Arc<dyn Provider>,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
        toolshim_tools: &[Tool],
        steering: Option<ProviderSteerReceiver>,
    ) -> Result<MessageStream, ProviderError> {
        let config = provider.get_model_config();

        // The toolshim interpreter is built and checked BEFORE the model is
        // asked anything. Building it is where a non-local OLLAMA_HOST or an
        // Ollama cloud model is refused (OLL-10), and `ensure_ready` is where a
        // missing Ollama or an unpulled interpreter model is: refused here, the
        // misconfiguration costs a sentence rather than a whole generation
        // regenerated and thrown away at the end.
        let toolshim_interpreter = if config.toolshim {
            let ready = match OllamaInterpreter::for_model(&config) {
                Ok(interpreter) => interpreter.ensure_ready().await.map(|()| interpreter),
                Err(e) => Err(e),
            };
            match ready {
                Ok(interpreter) => Some(interpreter),
                // In the stream, like every other creation-time failure below,
                // so the agent's own error handling reports it.
                Err(e) => return Ok(Box::pin(futures::stream::once(async move { Err(e) }))),
            }
        } else {
            None
        };

        // Convert tool messages to text if toolshim is enabled
        let messages_for_provider = if config.toolshim {
            convert_tool_messages_to_text(messages)
        } else {
            Conversation::new_unvalidated(messages.to_vec())
        };

        // Clone owned data to move into the async stream
        let system_prompt = system_prompt.to_owned();
        let tools = tools.to_owned();
        let toolshim_tools = toolshim_tools.to_owned();
        let provider = provider.clone();

        // Capture errors during stream creation and return them as part of the stream
        // so they can be handled by the existing error handling logic in the agent
        // Marker semantics (deliberately explicit — the previous names were
        // actively misleading, see the Stage 0 note in
        // `docs/history/streaming-tool-call-ui-2026-07/tool-call-ui-latency-investigation.md` §4.4).
        //
        // `provider.stream()` returns as soon as the response *headers* arrive;
        // token generation happens later, while the returned stream is polled.
        // So the OPEN -> OPENED span is request-serialize + connect + TTFB
        // ONLY, and is NOT comparable to the non-streaming pair below, which
        // brackets an entire blocking generation. Three distinct spans:
        //
        //   OPEN      -> OPENED     : connect + TTFB
        //   OPENED    -> EXHAUSTED  : generation (tokens streaming back)
        //   OPEN      -> EXHAUSTED  : total
        //
        // The OPENED -> EXHAUSTED == generation identity holds ONLY on the
        // streaming path. The non-streaming branch below funnels its single
        // completed message through `stream_from_single_message` into the same
        // `try_stream!`, so it also emits EXHAUSTED — but never an OPENED, and
        // its EXHAUSTED total is the same span FULL_GENERATION_END already
        // reported. Every marker therefore carries `streaming=<bool>` so a log
        // that mixes both paths (which a single session does — see the
        // investigation, §0) can be partitioned without inferring anything from
        // line adjacency. Pair OPENED with EXHAUSTED only within streaming=true.
        let streaming = provider.supports_streaming();
        let stream_open_start = std::time::Instant::now();
        let stream_result = if streaming {
            debug!(streaming, "WAITING_LLM_STREAM_OPEN");
            let result = match steering {
                Some(steering) => {
                    provider
                        .stream_with_steering(
                            system_prompt.as_str(),
                            messages_for_provider.messages(),
                            &tools,
                            steering,
                        )
                        .await
                }
                None => {
                    provider
                        .stream(
                            system_prompt.as_str(),
                            messages_for_provider.messages(),
                            &tools,
                        )
                        .await
                }
            };
            debug!(
                streaming,
                open_ms = stream_open_start.elapsed().as_millis() as u64,
                "WAITING_LLM_STREAM_OPENED"
            );
            result
        } else {
            // Non-streaming: this pair brackets the FULL generation (request +
            // connect + all tokens), unlike the streaming OPEN/OPENED pair.
            debug!(streaming, "WAITING_LLM_FULL_GENERATION_START");
            let complete_result = provider
                .complete(
                    system_prompt.as_str(),
                    messages_for_provider.messages(),
                    &tools,
                )
                .await;
            debug!(
                streaming,
                total_ms = stream_open_start.elapsed().as_millis() as u64,
                "WAITING_LLM_FULL_GENERATION_END"
            );

            match complete_result {
                Ok((message, usage)) => Ok(stream_from_single_message(message, usage)),
                Err(e) => Err(e),
            }
        };

        // If there was an error creating the stream, return a stream that yields that error
        let stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                // Return a stream that immediately yields the error
                // This allows the error to be caught by existing error handling in agent.rs
                return Ok(Box::pin(try_stream! {
                    yield Err(e)?;
                }));
            }
        };

        let tools_offered = !tools.is_empty() || !toolshim_tools.is_empty();
        Ok(agent_facing_stream(
            stream,
            toolshim_interpreter,
            toolshim_tools,
            tools_offered,
            streaming,
            stream_open_start,
        ))
    }

    /// Categorize tool requests from the response into different types
    /// Returns:
    /// - frontend_requests: Tool requests that should be handled by the frontend
    /// - other_requests: All other tool requests (including requests to enable extensions)
    /// - filtered_message: The original message with frontend tool requests removed
    pub(crate) async fn categorize_tool_requests(
        &self,
        response: &Message,
        tools: &[Tool],
    ) -> (Vec<ToolRequest>, Vec<ToolRequest>, Message) {
        // First collect all tool requests with coercion applied
        let tool_requests: Vec<ToolRequest> = response
            .content
            .iter()
            .filter_map(|content| {
                if let MessageContent::ToolRequest(req) = content {
                    let mut coerced_req = req.clone();

                    if let Ok(ref mut tool_call) = coerced_req.tool_call {
                        if let Some(tool) = tools.iter().find(|t| t.name == tool_call.name) {
                            let schema_value = Value::Object(tool.input_schema.as_ref().clone());
                            tool_call.arguments =
                                coerce_tool_arguments(tool_call.arguments.clone(), &schema_value);

                            if let Some(ref meta) = tool.meta {
                                coerced_req.tool_meta = serde_json::to_value(meta).ok();
                            }
                        }
                    }

                    Some(coerced_req)
                } else {
                    None
                }
            })
            .collect();

        // Create a filtered message with frontend tool requests removed
        let mut filtered_content = Vec::new();
        let mut tool_request_index = 0;

        for content in &response.content {
            match content {
                MessageContent::ToolRequest(_) => {
                    if tool_request_index < tool_requests.len() {
                        let coerced_req = &tool_requests[tool_request_index];
                        tool_request_index += 1;

                        let should_include = if let Ok(tool_call) = &coerced_req.tool_call {
                            !self.is_frontend_tool(&tool_call.name).await
                        } else {
                            true
                        };

                        if should_include {
                            filtered_content.push(MessageContent::ToolRequest(coerced_req.clone()));
                        }
                    }
                }
                _ => {
                    filtered_content.push(content.clone());
                }
            }
        }

        let mut filtered_message =
            Message::new(response.role.clone(), response.created, filtered_content);

        // Preserve the ID if it exists
        if let Some(id) = response.id.clone() {
            filtered_message = filtered_message.with_id(id);
        }
        // And who the message is for: this copy is the one the client is
        // shown live, and a user-only notice (`UnrunCallWatch`) streamed as
        // visible to the model while its stored row said otherwise.
        filtered_message.metadata = response.metadata.clone();

        // Categorize tool requests
        let mut frontend_requests = Vec::new();
        let mut other_requests = Vec::new();

        for request in tool_requests {
            if let Ok(tool_call) = &request.tool_call {
                if self.is_frontend_tool(&tool_call.name).await {
                    frontend_requests.push(request);
                } else {
                    other_requests.push(request);
                }
            } else {
                // If there's an error in the tool call, add it to other_requests
                other_requests.push(request);
            }
        }

        (frontend_requests, other_requests, filtered_message)
    }

    pub(crate) async fn update_session_metrics(
        &self,
        session_config: &crate::agents::types::SessionConfig,
        usage: &ProviderUsage,
        is_compaction_usage: bool,
        event_key: &str,
    ) -> Result<()> {
        let provider_name = match usage.provider.clone() {
            Some(provider) => Some(provider),
            None => self
                .provider()
                .await
                .ok()
                .map(|provider| provider.get_name().to_string()),
        };
        apply_session_metrics(
            &self.config.session_manager,
            session_config,
            usage,
            is_compaction_usage,
            event_key,
            provider_name,
        )
        .await
    }
}

pub(crate) async fn apply_session_metrics(
    manager: &crate::session::SessionManager,
    session_config: &crate::agents::types::SessionConfig,
    usage: &ProviderUsage,
    is_compaction_usage: bool,
    event_key: &str,
    provider_name: Option<String>,
) -> Result<()> {
    let session_id = session_config.id.as_str();
    let (current_total, current_input, current_output) = if is_compaction_usage {
        let new_input = usage.usage.output_tokens;
        (new_input, new_input, None)
    } else {
        (
            usage.usage.total_tokens,
            usage.usage.input_tokens,
            usage.usage.output_tokens,
        )
    };

    let billed_total = usage.usage.billed_total();
    if billed_total.is_some() || usage.usage.total_tokens.is_some() {
        manager
            .apply_usage_event(UsageLedgerEntry {
                event_key: event_key.to_string(),
                session_id: session_id.to_string(),
                schedule_id: session_config.schedule_id.clone(),
                current_total_tokens: current_total,
                current_input_tokens: current_input,
                current_output_tokens: current_output,
                billed_total_tokens: billed_total,
                input_tokens: usage.usage.input_tokens,
                output_tokens: usage.usage.output_tokens,
                model_id: Some(usage.model.clone()),
                provider: provider_name,
                cache_read_tokens: Some(usage.usage.cache_read_input_tokens.unwrap_or(0)),
                cache_creation_tokens: Some(usage.usage.cache_creation_input_tokens.unwrap_or(0)),
            })
            .await?;
    } else {
        manager
            .update(session_id)
            .schedule_id(session_config.schedule_id.clone())
            .apply()
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use async_trait::async_trait;
    use rmcp::{
        handler::server::router::tool::ToolRouter, object, tool, tool_handler, tool_router,
    };

    #[derive(Clone)]
    struct MockProvider {
        model_config: ModelConfig,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn metadata() -> crate::providers::base::ProviderMetadata {
            crate::providers::base::ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "mock"
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> anyhow::Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text("ok"),
                ProviderUsage::new("mock".to_string(), Usage::default()),
            ))
        }
    }

    #[tokio::test]
    async fn crew_prompt_omits_local_hints_while_ordinary_prompt_keeps_them() -> anyhow::Result<()>
    {
        let working_dir = tempfile::tempdir()?;
        let canary = format!("prompt-local-context-canary-{}", uuid::Uuid::new_v4());
        std::fs::write(working_dir.path().join("AGENTS.md"), &canary)?;
        let store = tempfile::tempdir()?;
        let session_manager = Arc::new(crate::session::SessionManager::new(
            store.path().to_path_buf(),
        ));
        let crew_session = session_manager
            .create_session(
                working_dir.path().to_path_buf(),
                "crew-prompt-context".into(),
                SessionType::User,
            )
            .await?;
        let ordinary_session = session_manager
            .create_session(
                working_dir.path().to_path_buf(),
                "ordinary-prompt-context".into(),
                SessionType::User,
            )
            .await?;
        let provider: Arc<dyn Provider> = Arc::new(MockProvider {
            model_config: ModelConfig::new("test-model")?,
        });
        let manager =
            crate::crew::install_test_scope(&crew_session.id, Some(provider.as_ref())).await;
        let agent = Agent::with_config(crate::agents::AgentConfig::new(
            session_manager,
            Arc::new(crate::config::permission::PermissionManager::new(
                store.path().to_path_buf(),
            )),
            None,
            crate::config::BioRouterMode::Auto,
        ));

        let (_tools, _toolshim_tools, crew_prompt, _bridge_plan) = agent
            .prepare_tools_and_prompt_for_provider(&crew_session.id, working_dir.path(), &provider)
            .await?;
        assert!(
            !crew_prompt.contains(&canary),
            "Crew prompt leaked AGENTS.md: {crew_prompt}"
        );

        crate::crew::remove_test_scope(&manager, &crew_session.id).await;
        let (_tools, _toolshim_tools, ordinary_prompt, _bridge_plan) = agent
            .prepare_tools_and_prompt_for_provider(
                &ordinary_session.id,
                working_dir.path(),
                &provider,
            )
            .await?;
        assert!(
            ordinary_prompt.contains(&canary),
            "ordinary prompt lost AGENTS.md: {ordinary_prompt}"
        );
        Ok(())
    }

    #[test]
    fn live_skill_inventory_attaches_only_to_the_enabled_skills_capability() {
        use crate::agents::extension::{ExtensionClassification, ExtensionInfo};

        let mut enabled = vec![ExtensionInfo::capability("skills", "static", false)];
        assert!(attach_skill_inventory(&mut enabled, "live generation 2"));
        assert_eq!(enabled[0].instructions, "static\n\nlive generation 2");

        let mut installed_extension = vec![ExtensionInfo::classified(
            "skills",
            "third party",
            false,
            ExtensionClassification::Extension,
        )];
        assert!(!attach_skill_inventory(
            &mut installed_extension,
            "must not attach"
        ));
        assert_eq!(installed_extension[0].instructions, "third party");
    }

    #[test]
    fn effective_tool_rosters_distinguish_module_access_from_direct_calls() {
        use crate::agents::extension::ExtensionInfo;

        let mut entries = vec![ExtensionInfo::capability("developer", "guidance", false)];
        let available = vec![
            Tool::new("developer__shell", "shell", object!({"type": "object"})),
            Tool::new(
                "developer__text_editor",
                "editor",
                object!({"type": "object"}),
            ),
        ];
        let direct = vec![available[0].clone()];

        attach_effective_tool_rosters(&mut entries, &available, &direct);
        assert_eq!(entries[0].available_tools, ["shell", "text_editor"]);
        assert_eq!(entries[0].directly_callable_tools, ["shell"]);

        let platform = vec![Tool::new(
            "platform__ingest_conversation".to_string(),
            "ingest".to_string(),
            object!({"type": "object"}),
        )];
        add_core_platform_capability(&mut entries, &platform);
        attach_effective_tool_rosters(&mut entries, &platform, &platform);
        let core = entries.iter().find(|info| info.name == "platform").unwrap();
        assert_eq!(core.available_tools, ["ingest_conversation"]);
        assert_eq!(core.directly_callable_tools, ["ingest_conversation"]);
    }

    /// The narrowed instruction blocks fire on the manager's map KEY, which is
    /// not the extension's declared name. Workspace declares `"Workspace"` and
    /// is keyed `workspace`, so the comparison was never true and the full
    /// instructions shipped even with only the delegation tools callable.
    ///
    /// Asserted for BOTH capabilities that narrow, and in both directions —
    /// Code Execution's constant is already normalized, so testing only it
    /// would have passed while Workspace stayed dead.
    #[test]
    fn a_narrowed_capability_is_matched_by_its_map_key_not_its_declared_name() {
        use crate::agents::extension::ExtensionInfo;

        let narrowed = |key: &str, tool: &str| {
            let mut entries = vec![ExtensionInfo::capability(key, "FULL GUIDANCE", false)];
            let tools = vec![Tool::new(
                format!("{key}__{tool}"),
                "t",
                object!({"type": "object"}),
            )];
            attach_effective_tool_rosters(&mut entries, &tools, &tools);
            entries.remove(0).instructions
        };

        // `workspace_open` absent -> delegation-only.
        assert!(narrowed("workspace", "subagent").contains("delegation-only"));
        // `workspace_open` present -> the full block survives.
        assert_eq!(narrowed("workspace", "workspace_open"), "FULL GUIDANCE");

        assert!(narrowed("code_execution", "read_module").contains("restricted to the effective"));
        assert_eq!(narrowed("code_execution", "execute_code"), "FULL GUIDANCE");

        // And the declared name is genuinely not the key, so the guard is not
        // vacuous.
        assert_ne!(
            crate::agents::workspace_extension::EXTENSION_NAME,
            crate::config::extensions::name_to_key(
                crate::agents::workspace_extension::EXTENSION_NAME
            )
        );
    }

    /// The Skills capability's prose is narrowed to the conversation's own
    /// roster.
    ///
    /// `get_extensions_info` — which is what puts that prose in the system
    /// prompt of every turn — is filtered by `allowed_extension_keys` and never
    /// by `available_tools`, so an Agent-Drafter app agent granted
    /// `searchSkills` + `loadSkill` read instructions for five more operations
    /// and called tools that answer with a refusal. Nothing else catches it:
    /// the roster is correct, the dispatch is correct, and only the prompt lies.
    #[test]
    fn the_skills_capability_advertises_only_the_operations_this_conversation_can_call() {
        use crate::agents::extension::ExtensionInfo;

        const ALL: [&str; 7] = [
            "searchSkills",
            "loadSkill",
            "searchMarketplaceSkills",
            "installMarketplaceSkill",
            "importSkillPackage",
            "removeSkillPackage",
            "setSkillEnabled",
        ];
        let prose = |granted: &[&str]| {
            let mut entries = vec![ExtensionInfo::capability("skills", "FULL GUIDANCE", false)];
            let tools: Vec<Tool> = granted
                .iter()
                .map(|name| {
                    Tool::new(
                        format!("skills__{name}"),
                        "t",
                        object!({ "type": "object" }),
                    )
                })
                .collect();
            attach_effective_tool_rosters(&mut entries, &tools, &tools);
            entries.remove(0).instructions
        };

        // The app-agent grant (`APP_SKILLS_TOOLS`).
        let app = prose(&["searchSkills", "loadSkill"]);
        assert!(app.contains("searchSkills"), "{app}");
        assert!(app.contains("loadSkill"), "{app}");
        for refused in &ALL[2..] {
            assert!(
                !app.contains(refused),
                "the prompt must not teach `{refused}`, which this conversation cannot call: {app}"
            );
        }
        assert!(
            !app.contains("desktop approval"),
            "the approval sentence describes a click no granted operation can reach: {app}"
        );

        // Nothing taken away: the full prose survives intact, so the arm cannot
        // quietly shrink an ordinary chat's instructions.
        let full = prose(&ALL);
        for op in ALL {
            assert!(
                full.contains(op),
                "the full roster must keep `{op}`: {full}"
            );
        }
        assert!(full.contains("desktop approval"), "{full}");
        assert!(full.contains("about-biorouter"), "{full}");

        // And a roster with nothing on it says so rather than listing seven
        // operations that all refuse.
        let none = prose(&[]);
        for op in ALL {
            assert!(!none.contains(op), "{none}");
        }
        assert!(
            none.contains("none of its operations are callable"),
            "{none}"
        );
    }

    /// Two routes describe ONE model-facing surface: the read-only count
    /// (`callable_tool_count`, behind `GET /agent/callable_tool_count`) and the
    /// turn's own preparation. They must agree — and the count route must not
    /// grade, because grading replaces the execution policy a running turn
    /// approves against.
    ///
    /// ⚠ Both reads resolve the platform gates independently
    /// (`Agent::platform_tool_gates`), and one of those gates —
    /// `session_blobs` — is `message_blobs::lazy_load_enabled()`, a LIVE read
    /// of the config layer, which resolves `BIOROUTER_SESSION_BLOB_LAZY_LOAD`
    /// from the process environment. So this test's surface used to be
    /// whatever the binary's other tests had left in the environment at each of
    /// the two instants, and CI caught it: `left: 4, right: 5`, the fifth tool
    /// being `platform__read_session_blob`, which appeared between the two
    /// reads because a concurrent `#[serial]` test had just parked the flag.
    ///
    /// The gate is therefore an INPUT here, stated once as a task-local config
    /// override — invisible to every other test, so it needs no lock and gives
    /// none — and the expected roster is derived from a single sample of the
    /// same gates rather than from a hard-coded number. A tool that appears
    /// from somewhere else now names itself instead of moving a count by one.
    #[tokio::test]
    async fn callable_count_is_pure_while_turn_prep_grades_sorted_frontend_tools(
    ) -> anyhow::Result<()> {
        let overrides = std::collections::HashMap::from([(
            "BIOROUTER_SESSION_BLOB_LAZY_LOAD".to_string(),
            "true".to_string(),
        )]);
        crate::config::with_config_overrides(overrides, async {
            let agent = crate::agents::Agent::new();

            let session = agent
                .config
                .session_manager
                .create_session(
                    std::path::PathBuf::default(),
                    "test-prepare-tools".to_string(),
                    SessionType::Hidden,
                )
                .await?;

            let model_config = ModelConfig::new("test-model").unwrap();
            let provider = std::sync::Arc::new(MockProvider { model_config });
            agent.update_provider(provider, &session.id).await?;

            // Add unsorted frontend tools
            let frontend_tools = vec![
                Tool::new(
                    "frontend__z_tool".to_string(),
                    "Z tool".to_string(),
                    object!({ "type": "object", "properties": { } }),
                ),
                Tool::new(
                    "frontend__a_tool".to_string(),
                    "A tool".to_string(),
                    object!({ "type": "object", "properties": { } }),
                ),
            ];

            agent
                .add_extension(crate::agents::extension::ExtensionConfig::Frontend {
                    name: "frontend".to_string(),
                    description: "desc".to_string(),
                    tools: frontend_tools,
                    instructions: None,
                    bundled: None,
                    available_tools: vec![],
                })
                .await
                .unwrap();

            // The whole roster this agent may advertise, stated rather than
            // inherited: the two frontend tools registered above, plus whatever the
            // platform gates open — sampled ONCE, here, from the same accessor both
            // routes use.
            let mut expected: Vec<String> = agent
                .platform_tool_gates()
                .await
                .tools(None)
                .into_iter()
                .map(|tool| tool.name.to_string())
                .chain([
                    "frontend__a_tool".to_string(),
                    "frontend__z_tool".to_string(),
                ])
                .collect();
            expected.sort();
            assert!(
                expected.contains(&"platform__read_session_blob".to_string()),
                "the override above must reach the gates, else this test is back to \
             reading the ambient environment: {expected:?}"
            );

            let working_dir = std::env::current_dir()?;
            assert!(agent.tool_risks.is_empty());
            let callable_count = agent.callable_tool_count(&session.id).await?;
            assert_eq!(
                callable_count,
                expected.len(),
                "the count route must report the model-facing surface"
            );
            assert!(
                agent.tool_risks.is_empty(),
                "a read-only count request mutated the shared risk registry"
            );

            let (tools, _toolshim_tools, _system_prompt) = agent
                .prepare_tools_and_prompt(&session.id, &working_dir)
                .await?;
            assert_eq!(callable_count, tools.len());
            assert_eq!(agent.tool_risks.len(), tools.len());

            // Sorted ascending, and exactly the roster named above — a stable order
            // is what preserves multi-session prompt caching.
            let names: Vec<String> = tools.iter().map(|t| t.name.clone().into_owned()).collect();
            assert_eq!(names, expected);

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn subagent_prompt_makes_the_latest_direct_human_message_authoritative(
    ) -> anyhow::Result<()> {
        let agent = crate::agents::Agent::new();
        let session = agent
            .config
            .session_manager
            .create_session(
                std::path::PathBuf::default(),
                format!("steerable-subagent-{}", uuid::Uuid::new_v4()),
                SessionType::SubAgent,
            )
            .await?;
        agent
            .update_provider(
                Arc::new(MockProvider {
                    model_config: ModelConfig::new("test-model").unwrap(),
                }),
                &session.id,
            )
            .await?;

        let (_, _, system_prompt) = agent
            .prepare_tools_and_prompt(&session.id, std::path::Path::new("."))
            .await?;

        assert!(system_prompt.contains(SUBAGENT_STEERING_INSTRUCTIONS));
        assert!(system_prompt.contains("supersedes any conflicting earlier task instruction"));
        Ok(())
    }

    #[test]
    fn the_code_execution_filter_keeps_the_prefixed_spawn_tool_and_workspace_tools() {
        let prefix = format!("{CODE_EXECUTION_EXTENSION}__");
        let frontend: HashSet<String> = ["ui__pick_a_file".to_string()].into_iter().collect();
        let no_frontend = HashSet::new();
        // Kept: the sandbox itself …
        assert!(survives_code_execution_filter(
            &format!("{prefix}execute_code"),
            &prefix,
            &no_frontend
        ));
        // … both spellings of the spawn tool — a filter that knows only the
        // bare name deletes delegation from every default session …
        assert!(survives_code_execution_filter(
            "subagent",
            &prefix,
            &no_frontend
        ));
        assert!(survives_code_execution_filter(
            "workspace__subagent",
            &prefix,
            &no_frontend
        ));
        // … and the whole workspace surface, or enabling Workspace Control
        // silently does nothing in the default configuration.
        assert!(survives_code_execution_filter(
            "workspace__workspace_list",
            &prefix,
            &no_frontend
        ));
        assert!(survives_code_execution_filter(
            "workspace__workspace_send_prompt",
            &prefix,
            &no_frontend
        ));
        // … every Agent-dispatched platform tool, which no script can import
        // (issue #141). Asserted HERE, beside the rest of the filter, so an
        // editor of this predicate sees the case rather than having to know
        // that `agent.rs` holds a separate test about it.
        for name in crate::agents::platform_tools::PLATFORM_TOOL_NAMES {
            assert!(
                survives_code_execution_filter(name, &prefix, &no_frontend),
                "{name} is dispatched by the agent loop and is in no importable module"
            );
        }
        // … the workflow structured-output tool, or a workflow with a
        // `response.json_schema` has no way to emit its result at all …
        assert!(survives_code_execution_filter(
            FINAL_OUTPUT_TOOL_NAME,
            &prefix,
            &no_frontend
        ));
        // … and whatever the interface registered as a frontend tool, which is
        // the one family with no name shape to test.
        assert!(survives_code_execution_filter(
            "ui__pick_a_file",
            &prefix,
            &frontend
        ));
        assert!(!survives_code_execution_filter(
            "ui__pick_a_file",
            &prefix,
            &no_frontend
        ));
        // Dropped: everything the model is supposed to reach through code.
        assert!(!survives_code_execution_filter(
            "developer__shell",
            &prefix,
            &no_frontend
        ));
        assert!(!survives_code_execution_filter(
            "memory__remember",
            &prefix,
            &no_frontend
        ));
    }

    /// The checklist stays a direct call in Code Execution mode. Before this,
    /// `todo__*` collapsed into the JS catalogue with everything else and a
    /// model reached it only by scripting — which it did once, when told to.
    #[test]
    fn the_code_execution_filter_keeps_every_todo_tool_directly_callable() {
        let prefix = format!("{CODE_EXECUTION_EXTENSION}__");
        let no_frontend = HashSet::new();
        for name in crate::agents::todo_extension::TODO_TOOL_NAMES {
            assert!(
                survives_code_execution_filter(name, &prefix, &no_frontend),
                "{name} is a cheap structural call the planning gate points at by name, so \
                 Code Execution mode must keep it directly callable"
            );
        }
        // Exact names, not a prefix: a third-party server keyed `todo` gets no
        // exemption from sharing the builtin's key.
        assert!(!survives_code_execution_filter(
            "todo__some_other_tool",
            &prefix,
            &no_frontend
        ));
    }

    #[test]
    fn computer_use_actions_stay_directly_callable_without_a_javascript_approval_wrapper() {
        let frontend = HashSet::new();
        for tool in crate::security::computer_use::TOOLS {
            assert!(survives_code_execution_filter(
                &format!("computercontroller__{tool}"),
                "code_execution__",
                &frontend
            ));
        }
        assert!(!survives_code_execution_filter(
            "developer__shell",
            "code_execution__",
            &frontend
        ));
        assert!(!survives_code_execution_filter(
            "agentdrafter__list_apps",
            "code_execution__",
            &frontend
        ));
    }

    /// The CLASS guard, and the thing that was missing while issue #141 was
    /// closed for exactly one of the families it applies to.
    ///
    /// Code Execution mode's whole premise is "a dropped tool is still
    /// reachable by writing code", and the only thing that makes that true is
    /// membership of `ExtensionManager::get_prefixed_tools_excluding` — the
    /// importable-module catalogue. So the invariant is one implication, in one
    /// direction:
    ///
    /// > a tool in `available_tools` that is NOT in the catalogue must survive
    /// > [`survives_code_execution_filter`].
    ///
    /// Both lists are in hand inside [`Agent::prepare_model_tool_surface`],
    /// which is why this drives a real Agent instead of asserting over a list of
    /// names: a name list is a statement about the families somebody remembered,
    /// and the three that were still unexempted after #141
    /// (`workflow__final_output`, the frontend tools, and — in the catalogue's
    /// eyes — `workspace__subagent`) are precisely the ones nobody did.
    ///
    /// The non-vacuity assertions below matter as much as the loop: with an
    /// empty catalogue, or with nothing outside it, the implication holds for
    /// free.
    #[tokio::test]
    #[serial_test::serial]
    async fn every_tool_absent_from_the_code_execution_catalogue_stays_directly_callable() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let path_root = tempfile::TempDir::new().expect("an isolated capability root");
        let path_root_value = path_root.path().to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root(path_root_value.as_str());

        // ⚠ `BIOROUTER_SESSION_BLOB_LAZY_LOAD` is set as a TASK-LOCAL config
        // override, never in the process environment. It decides whether
        // `platform__read_session_blob` reaches the model-facing roster
        // (`PlatformToolGates::session_blobs`), and every agent in this binary
        // reads it live through `Config::get_param` without asking for
        // `env_lock` — so parking it in the environment silently changed the
        // callable-tool count of every concurrently running test. It did:
        // `callable_count_is_pure_while_turn_prep_grades_sorted_frontend_tools`
        // below failed on CI with `left: 4, right: 5`, the missing tool being
        // this one, because the flag flipped between its two reads.
        // `model::tests::no_test_parks_a_shared_setting_in_the_process_environment`
        // is the standing guard.
        let overrides = std::collections::HashMap::from([(
            "BIOROUTER_SESSION_BLOB_LAZY_LOAD".to_string(),
            "true".to_string(),
        )]);
        crate::config::with_config_overrides(overrides, async {
            let store = tempfile::TempDir::new().expect("an isolated session store");
            let session_manager = Arc::new(crate::session::SessionManager::new(
                store.path().to_path_buf(),
            ));
            let session = session_manager
                .create_session(
                    store.path().to_path_buf(),
                    "code-execution-catalogue".to_string(),
                    SessionType::User,
                )
                .await
                .expect("create the session under test");
            let agent = crate::agents::Agent::with_config(crate::agents::AgentConfig::new(
                Arc::clone(&session_manager),
                crate::config::permission::PermissionManager::instance(),
                None,
                crate::config::BioRouterMode::Auto,
            ));

            // One ordinary extension, so the catalogue is not empty …
            agent
                .add_extension(crate::agents::extension::ExtensionConfig::Platform {
                    name: "todo".into(),
                    description: "todo".into(),
                    bundled: Some(true),
                    available_tools: vec![],
                })
                .await
                .expect("add the ordinary extension");
            // … Knowledge, which is what opens two of the platform tools …
            for name in ["knowledge", "code_execution"] {
                let target = crate::agents::extension_manager::resolve_bundled_extension(name)
                    .unwrap_or_else(|| panic!("{name} must resolve as a bundled capability"));
                agent
                    .add_extension(target.into_config(format!("{name} for the catalogue guard")))
                    .await
                    .unwrap_or_else(|error| panic!("enable {name}: {error}"));
            }
            // … a frontend tool, which lives only in `Agent::frontend_tools` …
            agent
                .add_extension(crate::agents::extension::ExtensionConfig::Frontend {
                    name: "frontend".to_string(),
                    description: "desc".to_string(),
                    tools: vec![Tool::new(
                        "frontend__pick_a_file".to_string(),
                        "Ask the interface for a file".to_string(),
                        object!({ "type": "object", "properties": { } }),
                    )],
                    instructions: None,
                    bundled: None,
                    available_tools: vec![],
                })
                .await
                .expect("register the frontend tool");
            // … and an armed structured-output response, which is the whole reason
            // a workflow ever has `workflow__final_output` on its roster.
            agent
                .add_final_output_tool(crate::workflow::Response {
                    json_schema: Some(json!({ "type": "object" })),
                })
                .await;

            let provider: Arc<dyn Provider> = Arc::new(MockProvider {
                model_config: ModelConfig::new("test-model").unwrap(),
            });
            agent
                .update_provider(Arc::clone(&provider), &session.id)
                .await
                .expect("bind the mock provider");

            let surface = agent
                .prepare_model_tool_surface(&session.id, &provider)
                .await;
            assert!(
                surface.code_execution_active,
                "precondition: Code Execution narrowing must be in force, else this proves nothing"
            );

            let catalogue: HashSet<String> = agent
                .extension_manager
                .get_prefixed_tools_excluding(CODE_EXECUTION_EXTENSION, None)
                .await
                .expect("build the importable-module catalogue")
                .into_iter()
                .map(|tool| tool.name.to_string())
                .collect();
            assert!(
                !catalogue.is_empty(),
                "an empty catalogue satisfies the implication for free"
            );

            let code_exec_prefix = format!("{CODE_EXECUTION_EXTENSION}__");
            let uncatalogued: Vec<String> = surface
                .available_tools
                .iter()
                .map(|tool| tool.name.to_string())
                .filter(|name| !catalogue.contains(name))
                .filter(|name| !name.starts_with(&code_exec_prefix))
                .collect();

            // Non-vacuity, family by family: each of these is in the roster and NOT
            // in the catalogue, so each is a real obligation rather than a name the
            // loop below never sees.
            for expected in [
                crate::agents::platform_tools::PLATFORM_INGEST_SOURCE_TOOL_NAME,
                crate::agents::platform_tools::PLATFORM_INGEST_CONVERSATION_TOOL_NAME,
                crate::agents::platform_tools::PLATFORM_READ_SESSION_BLOB_TOOL_NAME,
                FINAL_OUTPUT_TOOL_NAME,
                "frontend__pick_a_file",
                // The fifth family, and the one whose absence from the catalogue is
                // a POLICY rather than a plumbing fact: the sandbox could dispatch
                // `kb_delete_base` perfectly well, and must not, because the
                // approval it is contracted to show cannot be raised from there.
                // Listed here so removing the catalogue strip cannot quietly retire
                // the obligation below along with it.
                "knowledge__kb_delete_base",
            ] {
                assert!(
                    uncatalogued.iter().any(|name| name == expected),
                    "{expected} must be in the roster and outside the catalogue for this \
                 guard to mean anything: {uncatalogued:?}"
                );
            }

            let frontend_tool_names: HashSet<String> =
                agent.frontend_tools.lock().await.keys().cloned().collect();
            let callable: HashSet<String> = surface
                .directly_callable_tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect();
            for name in &uncatalogued {
                assert!(
                    survives_code_execution_filter(name, &code_exec_prefix, &frontend_tool_names),
                    "`{name}` is in the model's roster and in no importable module, so Code \
                 Execution mode would leave it reachable from NOWHERE"
                );
                // The same statement one layer out: the exemption has to actually
                // reach the surface the model is handed.
                assert!(
                    callable.contains(name),
                    "`{name}` survives the predicate but is missing from the prepared roster"
                );
            }
        })
        .await;
    }

    #[test]
    fn code_execution_presence_without_executor_does_not_hide_direct_tools() {
        let inspection_only = vec![Tool::new(
            "code_execution__read_module".to_string(),
            "read".to_string(),
            object!({ "type": "object", "properties": {} }),
        )];
        assert!(!code_execution_mode_is_active(true, &inspection_only));

        let executor = vec![Tool::new(
            "code_execution__execute_code".to_string(),
            "execute".to_string(),
            object!({ "type": "object", "properties": {} }),
        )];
        assert!(code_execution_mode_is_active(true, &executor));
        assert!(!code_execution_mode_is_active(false, &executor));
    }

    // ------------------------------------------------------------------
    // Issue #56 Gate F2: a private server's own INSTRUCTIONS.
    //
    // `get_extensions_info` hands every installed extension's instruction
    // text to the prompt builder, and the result is the system prompt of
    // EVERY turn. Gate E filters `filter_tools`, a different function on a
    // different path, so a private connector's tool names could be hidden
    // while its prose — table names, cohort semantics, credential scope —
    // still shipped to a public model on every request.
    // ------------------------------------------------------------------

    /// One of the two extensions the compiled-in BAAM baseline calls private.
    const PRIVATE_EXTENSION: &str = "ucsfomopagent";
    const SENTINEL: &str = "SENTINEL-INSTRUCTIONS";

    /// An in-process MCP server whose only distinguishing feature is the
    /// instruction text the manager hands the prompt builder. Injected through
    /// the real admission point, so the tier under test is the one
    /// `classify_extension` resolves rather than one poked into the record.
    #[derive(Clone)]
    struct InstructedServer {
        tool_router: ToolRouter<Self>,
    }

    #[tool_router(router = tool_router)]
    impl InstructedServer {
        fn new() -> Self {
            Self {
                tool_router: Self::tool_router(),
            }
        }

        #[tool(description = "Private extension fixture tool")]
        fn fixture_tool(&self) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::Content::text("ok"),
            ]))
        }
    }

    #[tool_handler(router = self.tool_router)]
    impl rmcp::ServerHandler for InstructedServer {
        fn get_info(&self) -> rmcp::model::ServerInfo {
            rmcp::model::ServerInfo {
                capabilities: rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
                instructions: Some(SENTINEL.to_string()),
                ..Default::default()
            }
        }
    }

    struct TieredProvider {
        tier: crate::privacy::ProviderTier,
    }

    #[async_trait]
    impl Provider for TieredProvider {
        fn metadata() -> crate::providers::base::ProviderMetadata {
            crate::providers::base::ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "plain"
        }

        fn tier(&self) -> crate::privacy::ProviderTier {
            self.tier
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("plain-model")
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> anyhow::Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text("ok"),
                ProviderUsage::new("plain-model".to_string(), Usage::default()),
            ))
        }
    }

    /// Bind a model of `tier` and render the system prompt the way a turn
    /// does. The assertion is on the RENDERED PROMPT, not on the tool list:
    /// asserting on the tools is the wrong-implementation trap, because Gate E
    /// already hides those and the instructions still ship.
    async fn system_prompt_under(
        agent: &crate::agents::Agent,
        session_id: &str,
        working_dir: &std::path::Path,
        tier: crate::privacy::ProviderTier,
    ) -> String {
        agent
            .update_provider(Arc::new(TieredProvider { tier }), session_id)
            .await
            .expect("binding a model of this tier is legal on a public session");
        let (_tools, _toolshim_tools, system_prompt) = agent
            .prepare_tools_and_prompt(session_id, working_dir)
            .await
            .expect("the real system-prompt path");
        system_prompt
    }

    #[tokio::test]
    async fn a_private_servers_instructions_do_not_reach_a_public_system_prompt() {
        use crate::privacy::ProviderTier::{Private, Public};

        let dir = tempfile::tempdir().unwrap();
        let agent = crate::agents::Agent::with_config(crate::agents::AgentConfig::new(
            Arc::new(crate::session::SessionManager::new(
                dir.path().to_path_buf(),
            )),
            Arc::new(crate::config::permission::PermissionManager::new(
                dir.path().to_path_buf(),
            )),
            None,
            crate::config::BioRouterMode::Auto,
        ));
        let session = agent
            .config
            .session_manager
            .create_session(
                dir.path().to_path_buf(),
                "gate-f".to_string(),
                SessionType::Hidden,
            )
            .await
            .unwrap();
        agent
            .extension_manager
            .add_inprocess_server(PRIVATE_EXTENSION, InstructedServer::new())
            .await
            .expect("inject the private extension");

        // Public first: the session is public, so both binds below are legal
        // and the order cannot be reversed.
        let public_prompt = system_prompt_under(&agent, &session.id, dir.path(), Public).await;
        assert!(
            !public_prompt.contains(SENTINEL),
            "a private server's instructions reached a public model's system prompt"
        );

        let private_prompt = system_prompt_under(&agent, &session.id, dir.path(), Private).await;
        assert!(
            private_prompt.contains(SENTINEL),
            "the model entitled to the extension must still get its instructions"
        );
    }

    #[tokio::test]
    async fn test_stream_error_propagation() {
        use futures::StreamExt;

        type StreamItem = Result<(Option<Message>, Option<ProviderUsage>), ProviderError>;
        let stream = futures::stream::iter(vec![
            Ok((Some(Message::assistant().with_text("chunk1")), None)),
            Ok((Some(Message::assistant().with_text("chunk2")), None)),
            Err(ProviderError::RequestFailed(
                "simulated stream error".to_string(),
            )),
        ] as Vec<StreamItem>);

        let mut pinned = Box::pin(stream);
        let mut results = Vec::new();
        let mut error_seen = false;

        while let Some(result) = pinned.next().await {
            match result {
                Ok((message, _usage)) => {
                    if let Some(msg) = message {
                        results.push(msg.as_concat_text());
                    }
                }
                Err(_e) => {
                    error_seen = true;
                    break;
                }
            }
        }

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], "chunk1");
        assert_eq!(results[1], "chunk2");
        assert!(
            error_seen,
            "Error should have been propagated, not silently ignored"
        );
    }
    /// An interpreter that answers with fixed calls and counts how often, and
    /// on what text, it was asked.
    struct ScriptedInterpreter {
        answer: Vec<&'static str>,
        asked: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ToolInterpreter for ScriptedInterpreter {
        async fn interpret_to_tool_calls(
            &self,
            content: &str,
            _tools: &[Tool],
        ) -> Result<Vec<rmcp::model::CallToolRequestParams>, ProviderError> {
            self.asked.lock().unwrap().push(content.to_string());
            Ok(self
                .answer
                .iter()
                .map(|name| rmcp::model::CallToolRequestParams {
                    task: None,
                    name: name.to_string().into(),
                    arguments: Some(object!({"command": "ls"})),
                    meta: None,
                })
                .collect())
        }
    }

    fn scripted(answer: Vec<&'static str>) -> ScriptedInterpreter {
        ScriptedInterpreter {
            answer,
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn shell_tool() -> Tool {
        Tool::new(
            "developer__shell",
            "run a command",
            object!({"type": "object", "properties": {"command": {"type": "string"}}}),
        )
    }

    /// OLL-09: a reply streamed across many deltas is interpreted ONCE, on the
    /// whole reply, and the calls arrive under the reply's own id so the agent
    /// folds them into the same assistant message. Prose the text grammar
    /// cannot read is what still reaches the interpreter model.
    #[tokio::test]
    async fn toolshim_interprets_a_streamed_reply_once_after_it_ends() {
        let mut reply = ToolshimReply::default();
        for piece in ["I will list ", "the files with ", "the shell."] {
            reply.observe(&Message::assistant().with_text(piece).with_id("chatcmpl-7"));
        }
        let interpreter = scripted(vec!["developer__shell", "shell"]);
        let calls = reply
            .interpret(&interpreter, &[shell_tool()])
            .await
            .expect("the offered tool is recovered");

        assert_eq!(
            *interpreter.asked.lock().unwrap(),
            ["I will list the files with the shell."],
            "one interpretation, of the whole text"
        );
        assert_eq!(calls.id.as_deref(), Some("chatcmpl-7"));
        let names: Vec<String> = calls
            .content
            .iter()
            .filter_map(|c| c.as_tool_request())
            .map(|r| r.tool_call.as_ref().unwrap().name.to_string())
            .collect();
        assert_eq!(names, ["developer__shell"], "`shell` was never offered");
    }

    /// A JSON call streamed across deltas is read by the toolshim's text
    /// grammar on the whole reply — no second model — and keeps the
    /// `recovered_from_text` marker (G9) on its way into the reply. This path
    /// rebuilt every call from bare params, which dropped the marker.
    #[tokio::test]
    async fn toolshim_keeps_the_recovered_from_text_marker_on_a_streamed_call() {
        let mut reply = ToolshimReply::default();
        for piece in [
            "{\"name\": \"developer",
            "__shell\", ",
            "\"arguments\": {\"command\": \"ls\"}}",
        ] {
            reply.observe(&Message::assistant().with_text(piece).with_id("chatcmpl-8"));
        }
        let interpreter = scripted(vec!["developer__shell"]);
        let calls = reply
            .interpret(&interpreter, &[shell_tool()])
            .await
            .expect("the call is recovered");

        assert!(
            interpreter.asked.lock().unwrap().is_empty(),
            "the grammar answered; the interpreter model was never asked"
        );
        assert_eq!(calls.id.as_deref(), Some("chatcmpl-8"));
        let requests: Vec<_> = calls
            .content
            .iter()
            .filter_map(|c| c.as_tool_request())
            .collect();
        assert_eq!(requests.len(), 1);
        assert!(
            crate::providers::formats::text_tool_calls::is_recovered(requests[0]),
            "{:?}",
            requests[0]
        );
        let call = requests[0].tool_call.as_ref().unwrap();
        assert_eq!(call.name, "developer__shell");
        assert_eq!(call.arguments.as_ref().unwrap()["command"], "ls");
    }

    /// A reply that already carries a structured call, or that names no
    /// offered tool, adds nothing — and a native call is never re-interpreted.
    #[tokio::test]
    async fn toolshim_adds_nothing_to_a_native_call_or_an_unrecognised_reply() {
        let mut native = ToolshimReply::default();
        native.observe(&Message::assistant().with_text("calling"));
        native.observe(&Message::assistant().with_tool_request(
            "call-1",
            Ok(rmcp::model::CallToolRequestParams {
                task: None,
                name: "developer__shell".into(),
                arguments: Some(object!({"command": "ls"})),
                meta: None,
            }),
        ));
        let interpreter = scripted(vec!["developer__shell"]);
        assert!(native
            .interpret(&interpreter, &[shell_tool()])
            .await
            .is_none());
        assert!(interpreter.asked.lock().unwrap().is_empty());

        let mut prose = ToolshimReply::default();
        prose.observe(&Message::assistant().with_text("The answer is 5."));
        let interpreter = scripted(vec!["noop"]);
        assert!(prose
            .interpret(&interpreter, &[shell_tool()])
            .await
            .is_none());
    }

    /// An interpreter that cannot be reached.
    struct UnreachableInterpreter;

    #[async_trait]
    impl ToolInterpreter for UnreachableInterpreter {
        async fn interpret_to_tool_calls(
            &self,
            _content: &str,
            _tools: &[Tool],
        ) -> Result<Vec<rmcp::model::CallToolRequestParams>, ProviderError> {
            Err(ProviderError::RequestFailed(
                "error sending request for url (http://localhost:11434/api/chat)".into(),
            ))
        }
    }

    /// The interpreter runs after the whole reply has streamed, so its failure
    /// must not fail the reply. As an error it was recoverable, and BR-66
    /// regenerated the entire reply, failed the same way, and ended the turn as
    /// failed with the answer shown twice.
    #[tokio::test]
    async fn toolshim_keeps_a_delivered_reply_when_the_interpreter_fails() {
        let mut reply = ToolshimReply::default();
        // Prose, so the interpreter model is reached: a call the text grammar
        // reads never asks it.
        reply.observe(
            &Message::assistant()
                .with_text("I will list the files with the shell.")
                .with_id("chatcmpl-9"),
        );
        let failed: Option<Message> = reply
            .interpret(&UnreachableInterpreter, &[shell_tool()])
            .await;
        assert!(failed.is_none(), "no call recovered, and no error raised");
    }

    /// A provider that counts the requests it was asked to answer.
    struct CountingToolshimProvider {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl Provider for CountingToolshimProvider {
        fn metadata() -> crate::providers::base::ProviderMetadata {
            crate::providers::base::ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "counting"
        }
        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new_or_fail("qwen3.8:27b")
                .with_toolshim(true)
                .with_toolshim_model(Some("local-interpreter".into()))
        }
        async fn complete_with_model(
            &self,
            _model_config: &crate::model::ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok((
                Message::assistant().with_text("a long local reply"),
                ProviderUsage::new(
                    "qwen3.8:27b".into(),
                    crate::providers::base::Usage::new(Some(10), Some(5), Some(15)),
                ),
            ))
        }
    }

    /// The misconfiguration that makes the interpreter fail EVERY time — no
    /// Ollama on `OLLAMA_HOST` — is refused before the chat model is asked,
    /// so it costs one loopback round trip instead of a generation.
    #[tokio::test]
    async fn toolshim_refuses_a_missing_interpreter_before_the_model_is_asked() {
        // A port that was free a moment ago: nothing answers on it.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let host = format!("http://127.0.0.1:{port}");
        let _env = env_lock::lock_env([("OLLAMA_HOST", Some(host.as_str()))]);

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let provider: Arc<dyn Provider> = Arc::new(CountingToolshimProvider {
            calls: Arc::clone(&calls),
        });
        let mut stream = Agent::stream_response_from_provider(
            provider,
            "system",
            &[Message::user().with_text("list the files")],
            &[],
            &[shell_tool()],
            None,
        )
        .await
        .unwrap();
        let first = stream.next().await.expect("the stream says why");
        let error = first.expect_err("refused, not answered").to_string();
        assert!(error.contains("none answered"), "{error}");
        assert!(stream.next().await.is_none());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the chat model must not be asked"
        );
    }

    /// A streaming provider that plays back a fixed list of stream items.
    struct ScriptedStreamProvider {
        items: std::sync::Mutex<
            Option<Vec<Result<crate::providers::base::ProviderStreamItem, ProviderError>>>,
        >,
    }

    #[async_trait]
    impl Provider for ScriptedStreamProvider {
        fn metadata() -> crate::providers::base::ProviderMetadata {
            crate::providers::base::ProviderMetadata::empty()
        }
        fn get_name(&self) -> &str {
            "scripted"
        }
        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("qwen3.8:27b")
        }
        fn supports_streaming(&self) -> bool {
            true
        }
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            unreachable!("the scripted provider only streams")
        }
        async fn stream(
            &self,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<MessageStream, ProviderError> {
            let items = self.items.lock().unwrap().take().expect("streamed once");
            Ok(Box::pin(futures::stream::iter(items)))
        }
    }

    /// What `stream_response_from_provider` hands the agent loop for `items`,
    /// one entry per yielded item.
    async fn through_the_agent_stream(
        items: Vec<Result<crate::providers::base::ProviderStreamItem, ProviderError>>,
    ) -> Vec<Result<crate::providers::base::ProviderStreamItem, ProviderError>> {
        through_the_agent_stream_offering(items, &[]).await
    }

    /// [`through_the_agent_stream`] for a request that offered `tools`.
    async fn through_the_agent_stream_offering(
        items: Vec<Result<crate::providers::base::ProviderStreamItem, ProviderError>>,
        tools: &[Tool],
    ) -> Vec<Result<crate::providers::base::ProviderStreamItem, ProviderError>> {
        let provider: Arc<dyn Provider> = Arc::new(ScriptedStreamProvider {
            items: std::sync::Mutex::new(Some(items)),
        });
        Agent::stream_response_from_provider(
            provider,
            "system",
            &[Message::user().with_text("how many rows?")],
            tools,
            &[],
            None,
        )
        .await
        .unwrap()
        .collect()
        .await
    }

    fn thinking_batch(
        text: &str,
        id: &str,
    ) -> Result<crate::providers::base::ProviderStreamItem, ProviderError> {
        Ok((
            Some(
                Message::assistant()
                    .with_content(MessageContent::thinking(text, ""))
                    .with_id(id),
            ),
            None,
            None,
        ))
    }

    /// The thinking text and id of an item that must be one unsigned block.
    fn one_thinking_block(
        item: &Result<crate::providers::base::ProviderStreamItem, ProviderError>,
    ) -> (String, Option<String>) {
        let (Some(message), None, None) = item.as_ref().expect("an item, not an error") else {
            panic!("expected a lone message: {item:?}");
        };
        let [MessageContent::Thinking(block)] = message.content.as_slice() else {
            panic!("expected one thinking block: {message:?}");
        };
        assert!(block.signature.is_empty());
        (block.thinking.clone(), message.id.clone())
    }

    fn usage_item() -> Result<crate::providers::base::ProviderStreamItem, ProviderError> {
        Ok((
            None,
            Some(ProviderUsage::new(
                "qwen3.8:27b".into(),
                Usage::new(Some(10), Some(5), Some(15)),
            )),
            None,
        ))
    }

    /// The local decoders hand thinking on in 250 ms batches, and no client
    /// merges them: the desktop's elapsed timer restarted at every batch and
    /// the CLI hid its spinner at the first one. The agent loop now receives
    /// one block per phase, before the answer it led to, under the reply's id.
    #[tokio::test]
    async fn thinking_batches_reach_the_agent_as_one_block_before_the_answer() {
        let out = through_the_agent_stream(vec![
            thinking_batch("Let me ", "r1"),
            thinking_batch("count ", "r1"),
            thinking_batch("the rows.", "r1"),
            Ok((
                Some(Message::assistant().with_text("Five").with_id("r1")),
                None,
                None,
            )),
            usage_item(),
        ])
        .await;

        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(
            one_thinking_block(&out[0]),
            ("Let me count the rows.".to_string(), Some("r1".to_string()))
        );
        let (Some(answer), None, None) = out[1].as_ref().unwrap() else {
            panic!("the answer second: {:?}", out[1]);
        };
        assert_eq!(answer.as_concat_text(), "Five");
        assert!(matches!(out[2].as_ref().unwrap(), (None, Some(_), None)));
    }

    /// The phase also ends at a tool call's first notice, at a failure, at the
    /// end of a reply that only thought, and at usage that rode in with the
    /// last batch. The held block comes out first each time, so it never
    /// lands after what it led to, and nothing the model thought is lost.
    #[tokio::test]
    async fn held_thinking_comes_out_before_whatever_ends_the_phase() {
        let pending = crate::providers::base::PendingToolCall {
            id: "call-1".into(),
            name: "developer__shell".into(),
            partial_args: None,
        };
        let out = through_the_agent_stream(vec![
            thinking_batch("a", "r"),
            thinking_batch("b", "r"),
            Ok((None, None, Some(pending))),
        ])
        .await;
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(one_thinking_block(&out[0]).0, "ab");
        assert!(matches!(out[1].as_ref().unwrap(), (None, None, Some(_))));

        let out = through_the_agent_stream(vec![
            thinking_batch("half a thought", "r"),
            Err(ProviderError::ServerError("connection reset".into())),
        ])
        .await;
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(one_thinking_block(&out[0]).0, "half a thought");
        assert!(out[1].is_err());

        let out = through_the_agent_stream(vec![
            thinking_batch("only ", "r"),
            thinking_batch("this", "r"),
        ])
        .await;
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(one_thinking_block(&out[0]).0, "only this");

        let (_, usage, _) = usage_item().unwrap();
        let out = through_the_agent_stream(vec![
            thinking_batch("x", "r"),
            Ok((
                Some(
                    Message::assistant()
                        .with_content(MessageContent::thinking("y", ""))
                        .with_id("r"),
                ),
                usage,
                None,
            )),
        ])
        .await;
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(one_thinking_block(&out[0]).0, "xy");
        assert!(matches!(out[1].as_ref().unwrap(), (None, Some(_), None)));
    }

    /// Signed thinking passes straight through: a signature covers exactly one
    /// block. Thinking under different ids (Codex gives each reasoning item its
    /// own) stays two blocks, in order.
    #[tokio::test]
    async fn signed_thinking_and_separate_replies_are_never_merged() {
        let signed = |text: &str| {
            Ok((
                Some(
                    Message::assistant()
                        .with_content(MessageContent::thinking(text, "sig"))
                        .with_id("m"),
                ),
                None,
                None,
            ))
        };
        let out = through_the_agent_stream(vec![signed("one"), signed("two")]).await;
        assert_eq!(out.len(), 2, "{out:?}");
        for (item, want) in out.iter().zip(["one", "two"]) {
            let (Some(message), None, None) = item.as_ref().unwrap() else {
                panic!("{item:?}");
            };
            assert!(matches!(
                message.content.as_slice(),
                [MessageContent::Thinking(t)] if t.thinking == want && t.signature == "sig"
            ));
        }

        let out = through_the_agent_stream(vec![
            thinking_batch("first ", "rs_1"),
            thinking_batch("item", "rs_1"),
            thinking_batch("second ", "rs_2"),
            thinking_batch("item", "rs_2"),
            Ok((
                Some(Message::assistant().with_text("Done").with_id("msg_3")),
                None,
                None,
            )),
        ])
        .await;
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(
            one_thinking_block(&out[0]),
            ("first item".to_string(), Some("rs_1".to_string()))
        );
        assert_eq!(
            one_thinking_block(&out[1]),
            ("second item".to_string(), Some("rs_2".to_string()))
        );
    }

    /// Diagnostics 20261006_2: one of the reporter's exact assistant replies.
    fn reporter_reply(index: u64) -> String {
        let doc: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/ollama/diagnostics_20261006_2/replies.json"
        ))
        .unwrap();
        doc["replies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|reply| reply["index"] == index)
            .and_then(|reply| reply["reply"].as_str())
            .unwrap_or_else(|| panic!("no reply {index} in the fixture"))
            .to_string()
    }

    fn reply_item(text: &str) -> Result<crate::providers::base::ProviderStreamItem, ProviderError> {
        Ok((
            Some(Message::assistant().with_text(text).with_id("r1")),
            None,
            None,
        ))
    }

    /// The inline notice text of an item, when it is one.
    fn notice_text(
        item: &Result<crate::providers::base::ProviderStreamItem, ProviderError>,
    ) -> Option<String> {
        let (Some(message), None, None) = item.as_ref().ok()? else {
            return None;
        };
        match message.content.as_slice() {
            [MessageContent::SystemNotification(note)]
                if note.notification_type
                    == crate::conversation::message::SystemNotificationType::InlineMessage =>
            {
                assert!(
                    !message.is_agent_visible() && message.is_user_visible(),
                    "the notice is for the person only: {message:?}"
                );
                assert_ne!(message.id.as_deref(), Some("r1"), "its own row");
                Some(note.msg.clone())
            }
            _ => None,
        }
    }

    /// Diagnostics 20261006_2: llama3.1:8b, in a chat with Code Execution on,
    /// answered three requests with a call written as JSON to `text_editor`,
    /// `shell` and `agent_drafter`, none of them a tool that chat offered.
    /// Nothing ran and nothing said so; the reporter read the JSON as the
    /// answer and gave up. The reply still shows as it came, and one line
    /// after it, for the person only, says that nothing ran.
    #[tokio::test]
    async fn a_reply_of_calls_written_as_text_says_that_nothing_ran() {
        for (index, name) in [
            (1, "'text_editor'"),
            (7, "'shell'"),
            (15, "'agent_drafter'"),
        ] {
            let reply = reporter_reply(index);
            let out = through_the_agent_stream_offering(
                vec![reply_item(&reply), usage_item()],
                &[shell_tool()],
            )
            .await;
            assert_eq!(out.len(), 3, "reply {index}: {out:?}");
            let (Some(shown), None, None) = out[0].as_ref().unwrap() else {
                panic!("reply {index}: the reply first: {:?}", out[0]);
            };
            assert_eq!(
                shown.as_concat_text(),
                reply,
                "the reply is shown as it came"
            );
            assert!(matches!(out[1].as_ref().unwrap(), (None, Some(_), None)));
            let notice = notice_text(&out[2])
                .unwrap_or_else(|| panic!("reply {index}: a notice last: {:?}", out[2]));
            assert!(
                notice.starts_with("Nothing ran: this reply is a call to ")
                    && notice.contains(name)
                    && notice.contains("written as text"),
                "{notice}"
            );
        }

        // Reply 11, two calls in one reply.
        let out = through_the_agent_stream_offering(
            vec![reply_item(&reporter_reply(11)), usage_item()],
            &[shell_tool()],
        )
        .await;
        let notice = out.iter().find_map(notice_text).expect("a notice");
        assert!(
            notice.contains("calls to 'workspace_read_conversation' and 'workspace_read_panel'"),
            "{notice}"
        );
    }

    /// Answers are never noticed: the reporter's prose, its code for the person
    /// to run, and the JSON it was asked for. Nor is a reply in a turn that
    /// offered no tools, nor one whose call was made after all (Ollama's one
    /// corrective retry turning reply 9 into a real `search_modules` call).
    #[tokio::test]
    async fn answers_and_made_calls_get_no_notice() {
        for index in [3, 5, 13, 17, 19] {
            let out = through_the_agent_stream_offering(
                vec![reply_item(&reporter_reply(index)), usage_item()],
                &[shell_tool()],
            )
            .await;
            assert!(
                out.iter().all(|item| notice_text(item).is_none()),
                "reply {index}: {out:?}"
            );
        }

        let out =
            through_the_agent_stream(vec![reply_item(&reporter_reply(7)), usage_item()]).await;
        assert!(out.iter().all(|item| notice_text(item).is_none()));

        let call = Message::assistant()
            .with_tool_request(
                "call-1",
                Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    name: "code_execution__search_modules".into(),
                    arguments: Some(object!({"terms": "llama"})),
                    meta: None,
                }),
            )
            .with_id("r1");
        let out = through_the_agent_stream_offering(
            vec![
                reply_item(&reporter_reply(9)),
                Ok((Some(call), None, None)),
                usage_item(),
            ],
            &[shell_tool()],
        )
        .await;
        assert!(
            out.iter().all(|item| notice_text(item).is_none()),
            "{out:?}"
        );
    }
}
