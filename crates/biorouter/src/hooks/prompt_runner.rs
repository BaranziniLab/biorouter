//! Prompt hooks: an LLM judge receives the event payload and decides
//! {"ok": bool, "reason": string}. `ok: false` blocks (for blockable events).

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tracing::debug;

use super::event::HookEvent;
use super::outcome::{HookDecision, HookOutcome};
use crate::conversation::message::Message;
use crate::providers::base::Provider;

const JUDGE_SYSTEM_PROMPT: &str = r#"You are a hook judge for the Biorouter agent. You are given a rule and a JSON event describing what the agent is about to do (or just did). Decide whether the event complies with the rule.

Respond with ONLY a JSON object, no prose, no code fences:
{"ok": true} if the event complies with the rule, or
{"ok": false, "reason": "<short explanation>"} if it violates the rule.

If the rule does not apply to this event, respond {"ok": true}."#;

#[derive(Deserialize)]
struct JudgeVerdict {
    ok: bool,
    #[serde(default)]
    reason: Option<String>,
}

/// Extract the first JSON object from model output, tolerating code fences
/// and surrounding prose.
fn parse_verdict(text: &str) -> Option<JudgeVerdict> {
    let trimmed = text.trim();
    if let Ok(verdict) = serde_json::from_str::<JudgeVerdict>(trimmed) {
        return Some(verdict);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    let candidate = trimmed.get(start..=end)?;
    serde_json::from_str::<JudgeVerdict>(candidate).ok()
}

/// The chat a hook payload names (`HookInput::session_id`), if any.
fn payload_session_id(payload_json: &str) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    payload
        .get("session_id")?
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Run a prompt hook against the given provider. Uses the provider's fast
/// model when one is configured. Failures are non-blocking (failure-open).
///
/// The judge's usage is charged to the chat the payload names, as a side call.
pub async fn run_prompt_hook(
    provider: Arc<dyn Provider>,
    event: HookEvent,
    judge_prompt: &str,
    payload_json: &str,
    timeout: Duration,
) -> HookOutcome {
    let session_id = payload_session_id(payload_json);
    run_prompt_hook_in_session(
        provider,
        session_id.as_deref(),
        event,
        judge_prompt,
        payload_json,
        timeout,
    )
    .await
}

/// [`run_prompt_hook`] charging the judge's usage to `session_id`. With `None`
/// the chat comes from the side-usage scope or `session_context`.
pub async fn run_prompt_hook_in_session(
    provider: Arc<dyn Provider>,
    session_id: Option<&str>,
    event: HookEvent,
    judge_prompt: &str,
    payload_json: &str,
    timeout: Duration,
) -> HookOutcome {
    let system = format!("{JUDGE_SYSTEM_PROMPT}\n\nRule:\n{judge_prompt}");
    let user_message = Message::user().with_text(format!("Event:\n{payload_json}"));

    let completion = tokio::time::timeout(
        timeout,
        provider.complete_fast(&system, &[user_message], &[]),
    )
    .await;

    let response = match completion {
        Ok(Ok((message, usage))) => {
            crate::session::session_manager::side_usage::record_side_call_usage(
                session_id,
                crate::session::session_manager::side_usage::SideCall::PromptHook,
                provider.get_name(),
                &usage,
            )
            .await;
            message.as_concat_text()
        }
        Ok(Err(e)) => {
            return HookOutcome {
                error: Some(format!("prompt hook provider error: {e}")),
                ..Default::default()
            }
        }
        Err(_) => {
            return HookOutcome {
                error: Some(format!("prompt hook timed out after {timeout:?}")),
                ..Default::default()
            }
        }
    };

    debug!("hooks: prompt hook verdict for {}: {}", event, response);
    match parse_verdict(&response) {
        Some(JudgeVerdict { ok: true, .. }) => HookOutcome::default(),
        Some(JudgeVerdict { ok: false, reason }) => HookOutcome {
            decision: Some(HookDecision::Deny {
                reason: reason.unwrap_or_else(|| format!("{event} blocked by prompt hook")),
            }),
            ..Default::default()
        },
        None => HookOutcome {
            error: Some(format!(
                "prompt hook returned unparseable verdict: {}",
                response.chars().take(200).collect::<String>()
            )),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_payload_names_the_chat_to_charge() {
        assert_eq!(
            payload_session_id(r#"{"session_id": "20261110_1", "x": 1}"#).as_deref(),
            Some("20261110_1")
        );
        assert_eq!(payload_session_id(r#"{"transcript_tail": "..."}"#), None);
        assert_eq!(payload_session_id(r#"{"session_id": ""}"#), None);
        assert_eq!(payload_session_id("not json"), None);
    }

    #[test]
    fn parses_plain_verdict() {
        let verdict =
            parse_verdict(r#"{"ok": false, "reason": "writes outside project"}"#).unwrap();
        assert!(!verdict.ok);
        assert_eq!(verdict.reason.as_deref(), Some("writes outside project"));
    }

    #[test]
    fn parses_fenced_verdict() {
        let verdict = parse_verdict("```json\n{\"ok\": true}\n```").unwrap();
        assert!(verdict.ok);
    }

    #[test]
    fn parses_verdict_with_prose() {
        let verdict = parse_verdict(
            "Sure! Here's my answer: {\"ok\": false, \"reason\": \"no\"} hope that helps",
        )
        .unwrap();
        assert!(!verdict.ok);
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_verdict("I think it's fine").is_none());
    }
}
