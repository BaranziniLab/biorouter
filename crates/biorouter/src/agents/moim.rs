use crate::agents::extension_manager::{ExtensionManager, MOIM_CLOSE_TAG, MOIM_OPEN_TAG};
use crate::context_budget::{estimate_tokens, max_moim_tokens, truncate_to_tokens};
use crate::conversation::message::Message;
use crate::conversation::{Conversation, SharedNormalizer};
use rmcp::model::Role;
use std::path::Path;
use tokio_util::sync::CancellationToken;

// Test-only utility. Do not use in production code. No `test` directive due to call outside crate.
thread_local! {
    pub static SKIP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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

/// Inject the MOIM `<info-msg>` block into the conversation handed to the model,
/// returning the (re-normalized) conversation and whether a block was injected.
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
) -> (Conversation, bool) {
    if SKIP.with(|f| f.get()) {
        return (conversation, false);
    }

    if let Some(moim) = extension_manager
        .collect_moim(session_id, working_dir, cancel)
        .await
    {
        let moim = cap_moim_block(moim, max_moim_tokens());
        let mut messages = conversation.messages().clone();
        // Drop any stale MOIM from a prior loop iteration first, so a long
        // multi-tool turn never accumulates several near-identical (and
        // differently-timestamped) `<info-msg>` blocks.
        strip_existing_moim(&mut messages);
        // Insert MOIM after the last assistant message AND any trailing tool
        // responses that follow it. Inserting between an assistant tool_call and
        // its tool_result would put a user message there, which the OpenAI API
        // rejects with a 400 error.
        let after_last_assistant = messages
            .iter()
            .rposition(|m| m.role == Role::Assistant)
            .map(|i| i + 1)
            .unwrap_or(0);
        let idx = messages[after_last_assistant..]
            .iter()
            .position(|m| !m.is_tool_response())
            .map(|offset| after_last_assistant + offset)
            .unwrap_or(messages.len());
        messages.insert(idx, Message::user().with_text(moim));

        let (fixed, issues) = normalizer.normalize(Conversation::new_unvalidated(messages));

        let has_unexpected_issues = issues.iter().any(|issue| {
            !issue.contains("Merged consecutive user messages")
                && !issue.contains("Merged consecutive assistant messages")
        });

        if has_unexpected_issues {
            tracing::warn!("MOIM injection caused unexpected issues: {:?}", issues);
            return (conversation, false);
        }

        return (fixed, true);
    }
    (conversation, false)
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
