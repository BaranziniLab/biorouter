pub(crate) mod agent;
// BR-71 Task 36b: approval routing for agent-created sessions — the relay, the
// single policy point for the two delegation bounds, and the escalation walk.
pub mod approval_relay;
// BR-35: the per-reply wall-clock / token / dollar ceiling. Off unless
// configured; the iteration caps (`max_turns`, `max_tool_calls`) bound how many
// steps a reply takes, this bounds how long it runs and what it costs.
pub mod budget;
// The agent-callable bug reporter: gather the session's own evidence, distil a
// report, and -- only behind a proof-backed approval showing the exact body --
// file it on the public issue tracker.
pub mod bug_report;
pub(crate) mod chatrecall_extension;
pub(crate) mod code_execution_extension;
// BR-48: a deterministic done-ness gate for interactive chat — reuses the
// workflow `SuccessCheck` machinery to keep a turn working until its checks
// pass. Config-gated, default OFF.
pub mod done_gate;
pub mod effort;
pub mod execute_commands;
pub mod extension;
pub mod extension_malware_check;
pub mod extension_manager;
pub mod extension_manager_extension;
pub mod final_output_tool;
pub mod goal;
pub mod knowledge_source_tool;
pub mod knowledge_tool;
mod large_response_handler;
pub mod mcp_client;
pub mod mcp_pool;
pub mod mistakes;
pub mod moim;
// The one approval card `platform__manage_workflow` and `platform__manage_schedule`
// park before they change the user's setup — shared so the two cannot disagree
// about whether to ask (QA 2026-09-10, F1).
pub(crate) mod platform_approval;
pub mod platform_tools;
// BR-47: auto post-edit diagnostics — the config gate, write-detection, and
// corrective-context formatting for the edit->check->fix reflection loop.
pub mod post_edit_diagnostics;
// Stage 0 of the tool-call latency work: opt-in per-phase timing behind
// `BIOROUTER_PHASE_TIMING=1`, free when off.
pub mod phase_timing;
pub mod prompt_manager;
mod recurring;
// BR-12: `pub(crate)` so `context_mgmt::run_eager_compaction` can reuse
// `apply_session_metrics` from the background compaction task.
pub(crate) mod reply_parts;
pub mod resource_refs;
pub mod retry;
pub(crate) mod schedule_tool;
// QA finding F7: every tool call a Code Execution script makes faces the same
// permission decision it would face as a direct call.
pub(crate) mod script_call_gate;
mod session_blob_tool;
// The session-row write for `enabled_extensions.v0`, plus the classifier that
// says which catalog tools require it. `pub` because `agents::agent` is
// `pub(crate)` and `biorouter-server`'s `/agent/call_tool` needs both.
pub mod session_extensions;
// BR-71 decision (c): per-session skill enablement, kept strictly out of the
// machine-wide `skills-config.json`.
pub mod session_skills;
pub mod skill_catalog;
pub mod skill_package;
// Pub so the CLI (`biorouter skill …`) reuses the exact same skill discovery
// roots and frontmatter parsing as this backend extension, instead of keeping
// a drifting duplicate (Codex B2 findings 5+6).
pub mod skills_extension;
// BR-50: an optional, config-gated self-critique pass that re-reads an ordinary
// answer for correctness before it is returned, reusing the goal-judge LLM
// primitive. Default OFF (it costs an extra LLM call per turn).
pub mod self_critique;
// BR-32: the `/goal` stall detector, generalized into a periodic no-progress
// check that runs for every long agentic turn, not just goal sessions.
pub mod stall;
pub mod structured_output;
pub mod subagent_execution_tool;
// BR-40: the async half — a background `subagent` call returns a handle the
// parent waits on with `workspace_watch`, instead of blocking the turn.
pub mod subagent_handle;
pub mod subagent_handler;
pub mod subagent_result;
pub(crate) mod subagent_runtime_profile;
mod subagent_task_config;
pub mod subagent_tool;
pub(crate) mod todo_extension;
pub mod tool_dispatch_limits;
mod tool_execution;
// BR-51: the structured tool-error taxonomy every failed tool result is reduced
// to, so the model and the loop detectors can tell a retryable blip from a hard
// failure.
pub mod tool_errors;
pub mod turn_abort;
pub mod turn_guard;
pub mod types;
pub mod vault_refs;
// The `platform__manage_workflow` handler: the model's hand on the user's saved
// workflows. An Agent tool rather than an extension because `generate` needs the
// agent's own provider — see the module header.
pub mod workflow_tool;
// BR-71: the `workspace` platform extension — the in-process sibling of
// `chatrecall_extension`, whose tools operate the workspace itself (sessions,
// and the GUI's tabs when one is attached).
//
// The plan asked for this beside `chatrecall_extension`; rustfmt's
// `reorder_modules` sorts each contiguous `mod` group, so any such placement is
// undone by `cargo fmt` — and the move strands the comment on whichever module
// takes the vacated line. Sorted position, own comment.
pub mod workspace_extension;
// BR-71 §5: the always-confirm hook for cross-session capability changes.
pub mod workspace_inspector;
pub mod workspace_summary;

pub use agent::{
    Agent, AgentConfig, AgentEvent, ConfirmationOutcome, Drained, ExtensionLoadResult,
    InterruptRefused, PersistedMessage, TurnId,
};
pub use budget::ReplyBudget;
pub use effort::ReasoningEffort;
pub use execute_commands::COMPACT_TRIGGERS;
pub use extension::ExtensionConfig;
pub use extension_manager::{normalize, ExtensionManager};
pub use prompt_manager::PromptManager;
pub use skills_extension::{count_user_skills, reset_to_builtin_skills};
pub use subagent_handle::{BackgroundSubagent, HandleSnapshot, HandleState};
pub use subagent_result::{SubagentResult, SubagentStatus, SubagentTokens};
pub use subagent_runtime_profile::persisted_subagent_extension_projection;
pub use subagent_task_config::TaskConfig;
pub use turn_abort::{exit, TurnAbortCode};
pub use types::{FrontendTool, RetryConfig, SessionConfig, SuccessCheck};
pub use vault_refs::VaultRefs;
