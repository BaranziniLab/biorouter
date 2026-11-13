//! Owner decision 4 (2026-11-12): when a PRIVATE chat asks for a subagent on a
//! PUBLIC model, DR-19 refuses it, and the person (only) is told the chat can be
//! made public from History. The agent never reads that line.
//!
//! The line is added by the person's surfaces, never carried in the tool result
//! (see [`PUBLIC_CHILD_REFUSAL_OPENING`]). These tests hold the three things that
//! makes true: the refusal it keys on is DR-19's and no other; the desktop keys on
//! the same words and shows the same line; and the refusal the model reads, in
//! every provider format and in the stored message, carries none of it.

use super::*;
use crate::conversation::message::Message;
use crate::privacy::{ModelAffiliation, PrivacyRefusal, ProviderTier};

/// A private parent. It states an affiliation because every real private
/// provider does (the reason `tests::TieredParent` gives), and `Local` so the
/// only refusal a public child can meet is DR-19's.
struct PrivateParent;

#[async_trait::async_trait]
impl crate::providers::base::Provider for PrivateParent {
    fn metadata() -> crate::providers::base::ProviderMetadata {
        crate::providers::base::ProviderMetadata::new(
            "private-parent",
            "Private parent",
            "",
            "private-model",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "private-parent"
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        Some(ModelAffiliation::Local)
    }

    fn get_model_config(&self) -> crate::model::ModelConfig {
        crate::model::ModelConfig::new_or_fail("private-model")
    }

    async fn complete_with_model(
        &self,
        _model_config: &crate::model::ModelConfig,
        _system: &str,
        _messages: &[crate::conversation::message::Message],
        _tools: &[Tool],
    ) -> std::result::Result<
        (
            crate::conversation::message::Message,
            crate::providers::base::ProviderUsage,
        ),
        crate::providers::errors::ProviderError,
    > {
        Err(crate::providers::errors::ProviderError::Authentication(
            "a refused spawn never runs a turn".to_string(),
        ))
    }
}

/// Words that would hand the agent a way to lower the chat, lower-cased. Not a
/// bare "history": the coding-agent transcript wraps earlier turns in a
/// `<conversation_history>` tag, which names no way to lower anything.
const LOWERING_WORDS: [&str; 6] = [
    "from history",
    "in history",
    "make it public",
    "make this chat public",
    "mark it public",
    "declassif",
];

fn assert_no_way_to_lower(surface: &str, text: &str) {
    assert!(
        !text.contains(MAKE_CHAT_PUBLIC_FROM_HISTORY),
        "{surface} carries the person-only line: {text}"
    );
    let lower = text.to_lowercase();
    for word in LOWERING_WORDS {
        assert!(
            !lower.contains(word),
            "{surface} names {word:?}, a way to lower the chat: {text}"
        );
    }
}

/// The line keys on DR-19's refusal and on no other spawn refusal: R4 and DR-31
/// are not cleared by making the chat public, so the line would be false there.
#[test]
fn the_line_keys_on_dr19_and_on_no_other_spawn_refusal() {
    let downgrade = PrivacyRefusal::spawn_downgrade(ProviderTier::Public).to_string();
    assert!(
        downgrade.starts_with(PUBLIC_CHILD_REFUSAL_OPENING),
        "the desktop and the terminal key on these words: {downgrade}"
    );
    assert_eq!(
        person_only_spawn_refusal_hint(&downgrade),
        Some(MAKE_CHAT_PUBLIC_FROM_HISTORY)
    );
    // The refusal the agent reads is unchanged and names no way to lower.
    assert_no_way_to_lower("DR-19's refusal", &downgrade);

    let upgrade = PrivacyRefusal::spawn_upgrade(ProviderTier::Private).to_string();
    let affiliation =
        PrivacyRefusal::spawn_affiliation(Some(ModelAffiliation::Local), None).to_string();
    for other in [upgrade, affiliation] {
        assert!(!other.contains(PUBLIC_CHILD_REFUSAL_OPENING), "{other}");
        assert_eq!(person_only_spawn_refusal_hint(&other), None, "{other}");
    }
}

/// `ui/desktop/src/components/subagent/spawnRefusalHint.ts` holds its own copy
/// of the words, the line and the spawn tool's names, because the renderer
/// cannot import Rust. This reads that file, so the two cannot drift apart.
#[test]
fn the_desktop_keys_on_the_same_words_and_shows_the_same_line() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/desktop/src/components/subagent/spawnRefusalHint.ts");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    for (what, literal) in [
        ("the refusal's opening", PUBLIC_CHILD_REFUSAL_OPENING),
        ("the person-only line", MAKE_CHAT_PUBLIC_FROM_HISTORY),
        ("the prefixed spawn tool", SUBAGENT_TOOL_PREFIXED),
        ("the bare spawn tool", SUBAGENT_TOOL_NAME),
    ] {
        let quoted = format!("'{literal}'");
        assert!(
            source.contains(&quoted),
            "{} must spell {what} exactly as the daemon does: {quoted}",
            path.display()
        );
    }
    // The names the desktop accepts are the names the daemon dispatches as a
    // spawn, and no others.
    assert!(crate::agents::agent::is_spawn_tool_call(
        SUBAGENT_TOOL_PREFIXED
    ));
    assert!(crate::agents::agent::is_spawn_tool_call(SUBAGENT_TOOL_NAME));
}

/// The refused spawn, through the real tool entry point, then through what the
/// agent loop does to every result before it is stored and sent (the guardrail,
/// which leaves an `Err` alone, and the error taxonomy's annotation).
async fn refused_spawn(background: bool) -> ErrorData {
    let temp = tempfile::TempDir::new().unwrap();
    let sm = std::sync::Arc::new(crate::session::SessionManager::new(
        temp.path().to_path_buf(),
    ));
    let config = AgentConfig::new(
        sm,
        crate::config::permission::PermissionManager::instance(),
        None,
        crate::config::BioRouterMode::Auto,
    );
    let provider: std::sync::Arc<dyn crate::providers::base::Provider> =
        std::sync::Arc::new(PrivateParent);
    let task_config = TaskConfig::new(provider, "parent-1", temp.path(), vec![]);
    // `ollama` resolves Public off this machine (see `tests::ask_params`).
    let overrides = HashMap::from([
        (
            "OLLAMA_HOST".to_string(),
            "https://ollama.example.com".to_string(),
        ),
        (
            "BIOROUTER_SUBAGENT_BACKGROUND".to_string(),
            background.to_string(),
        ),
    ]);
    let refused = crate::config::with_config_overrides(overrides, async {
        handle_subagent_tool(
            &config,
            json!({
                "instructions": "summarise the cohort",
                "settings": { "provider": "ollama" },
                "background": background,
            }),
            task_config,
            HashMap::new(),
            temp.path().to_path_buf(),
            None,
        )
        .result
        .await
    })
    .await;
    let (guarded, _) = crate::guardrails::tool_output::guard_tool_result(
        refused,
        Some(SUBAGENT_TOOL_PREFIXED),
        crate::guardrails::tool_output::ToolOutputGuardrailMode::default(),
    );
    let (annotated, _) = crate::agents::tool_errors::annotate_tool_result(
        guarded,
        crate::agents::tool_errors::ToolErrorTaxonomyConfig::default(),
    );
    annotated.expect_err("DR-19 refuses a private chat's public subagent")
}

/// What the model reads of the refused spawn, in each request shape Biorouter
/// sends. Every one must carry the refusal (so the check is not vacuous) and none
/// may name a way to lower the chat.
fn model_facing_renderings(messages: &[Message]) -> Vec<(&'static str, String)> {
    use crate::providers::formats;
    #[cfg_attr(not(feature = "aws-providers"), allow(unused_mut))]
    let mut out = vec![
        (
            "openai",
            serde_json::to_string(&formats::openai::format_messages(
                messages,
                &crate::providers::utils::ImageFormat::OpenAi,
            ))
            .unwrap(),
        ),
        (
            "anthropic",
            serde_json::to_string(&formats::anthropic::format_messages(messages)).unwrap(),
        ),
        (
            "google",
            serde_json::to_string(&formats::google::format_messages(messages)).unwrap(),
        ),
        (
            "ollama",
            serde_json::to_string(&formats::ollama::format_messages(messages, false, false))
                .unwrap(),
        ),
        (
            "openai responses",
            serde_json::to_string(
                &formats::openai_responses::create_responses_request(
                    &crate::model::ModelConfig::new_or_fail("gpt-test"),
                    "",
                    messages,
                    &[],
                )
                .unwrap(),
            )
            .unwrap(),
        ),
        (
            "coding agent transcript",
            crate::providers::coding_agent::transcript::flatten(messages).unwrap_or_default(),
        ),
    ];
    #[cfg(feature = "aws-providers")]
    out.push((
        "bedrock",
        format!(
            "{:?}",
            formats::bedrock::to_bedrock_messages("bedrock-test", messages).unwrap()
        ),
    ));
    out
}

/// The heart of the decision: the agent reads the refusal exactly as before, and
/// nothing on the path to any model, nor in the stored message every surface and
/// every later turn reads back, says the chat can be made public.
#[tokio::test]
async fn the_model_never_reads_the_person_only_line() {
    for background in [false, true] {
        let refusal = refused_spawn(background).await;
        // The tool result is the refusal the agent always read, unchanged.
        assert!(
            refusal.message.contains(PUBLIC_CHILD_REFUSAL_OPENING),
            "background={background}: {refusal:?}"
        );
        let display = refusal.to_string();
        assert_no_way_to_lower("the error's display", &display);
        assert_no_way_to_lower("the error's data", &format!("{:?}", refusal.data));

        let call_id = "call-refused-spawn";
        let messages = vec![
            Message::user().with_text("Delegate the cohort summary."),
            Message::assistant().with_tool_request(
                call_id,
                Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    name: SUBAGENT_TOOL_PREFIXED.into(),
                    arguments: json!({ "instructions": "summarise the cohort" })
                        .as_object()
                        .cloned(),
                    meta: None,
                }),
            ),
            Message::user().with_tool_response(call_id, Err(refusal.clone())),
        ];

        // What the session store keeps and the desktop, the CLI and every later
        // turn read back.
        let stored = serde_json::to_string(&messages[2]).unwrap();
        assert!(stored.contains(PUBLIC_CHILD_REFUSAL_OPENING), "{stored}");
        assert_no_way_to_lower("the stored message", &stored);

        for (format, rendered) in model_facing_renderings(&messages) {
            assert!(
                rendered.contains(PUBLIC_CHILD_REFUSAL_OPENING),
                "background={background}: {format} did not render the refusal, so this \
                 check would pass vacuously: {rendered}"
            );
            assert_no_way_to_lower(format, &rendered);
        }
    }
}
