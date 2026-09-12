//! BRSDK content guardrails — a deny-by-default safety layer run around the
//! agent loop for apps that declare it.
//!
//! Phase 2a (this commit) ships the **local** PII/PHI detector ([`pii`]): it
//! runs entirely on-device (no provider, no network), which is what makes it
//! safe to apply to clinical/biomedical text. The LLM-judge checks
//! (prompt-injection / groundedness / moderation), the staged pipeline, and the
//! wiring into the agent loop land in Phase 2b.

pub mod pii;
pub mod run_state;
/// Credential redaction in tool output — the second line behind
/// `SecretGuard`'s argument scan (H1). Applied at the dispatch choke point, on
/// every tool result, in every chat.
pub mod secret_output;
/// Main-loop tool-*output* guardrail: scans returned tool content for
/// prompt-injection markers and PII/PHI and (by default) annotates it as
/// untrusted data before it re-enters the model context. This is the first
/// implementation of a [`GuardrailStage::ToolOutput`] on the CLI/GUI loop.
pub mod tool_output;
/// The inverse of [`tool_output`]'s frame, for the surfaces where a **human**
/// reads tool text or where it becomes durable content. Display only: nothing
/// in it may run on the path to a model. Kept out of [`tool_output`] so that
/// making a view prettier is never a reason to edit the control.
pub mod tool_output_display;

/// Where in a turn's lifecycle a guardrail runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardrailStage {
    /// Before the model sees the user input (e.g. PII masking of the prompt).
    PreFlight,
    /// The user input, alongside the model call.
    Input,
    /// A tool's arguments, before the tool runs.
    ToolInput,
    /// A tool's result, before it re-enters the conversation.
    ToolOutput,
    /// The final answer, before it is returned.
    Output,
}

/// The outcome of evaluating a guardrail against some text/args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardrailVerdict {
    /// Allowed unchanged.
    Pass,
    /// Allowed after rewriting (e.g. PII masked); `finding` summarizes what.
    Mask {
        masked_text: String,
        finding: String,
    },
    /// Tripped. `blocked` drives block-vs-warn behavior at the call site.
    Trip { reason: String, blocked: bool },
}
