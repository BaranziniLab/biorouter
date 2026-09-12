//! Chat-side handler for the `platform__ingest_source` tool (issue #108).
//!
//! Its sibling [`crate::agents::knowledge_tool`] folds *conversations* into a
//! knowledge base; this one folds *documents* — files, folders, archives, URLs
//! and pasted text — through the very same `macros::ingest` pipeline the
//! desktop's dropzone drives over `POST /knowledge/bases/{id}/ingest`.
//!
//! The three things it owns, and nothing else:
//!
//! 1. **Reading what a person named.** [`parse_sources`] turns the tool's
//!    arguments into [`SourceSpec`]s. Pure, and unit-tested, because "did the
//!    model's `sources` array mean a path or a URL" is a decision that should
//!    not require a filesystem to re-check.
//! 2. **Choosing a provider without ever substituting one.** The chat's own
//!    model runs the ingest unless the caller explicitly names another in
//!    `model`; ordinary providers use request tool calls and coding agents use
//!    the provider-driven bridge. A model with neither mechanism is refused by
//!    name. See [`source_ingest::tool_capability_refusal`].
//! 3. **Answering truthfully.** The report names the model, gives per-source
//!    status, and cannot read as success when only raw sources were staged.
//!
//! Everything transactional — the git txn, the abort, the privacy ratchet, the
//! graph rebuild, the post-commit verification — belongs to the macro.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rmcp::model::Content;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::knowledge_tool::{build_model_ref_provider, kb_caller, resolve_target_kb};
use super::Agent;
use crate::knowledge::source_ingest::{
    self, ingest_sources, CompleterFactory, SourceIngestArgs, SourceSpec,
};
use crate::knowledge::ProviderCompleter;
use crate::mcp_utils::ToolResult;
use crate::privacy::ProviderTier;
use crate::providers::base::Provider;
use crate::session::session_manager::Session;
use biorouter_mcp::knowledge::service::KnowledgeService;
use biorouter_mcp::knowledge::subagent::loop_::SubAgentBounds;
use biorouter_mcp::knowledge::types::ModelRef;

/// The provider that will run the ingest, resolved and reported.
struct ChosenModel {
    completer: CompleterFactory,
    capability: ProviderTier,
    affiliation: Option<crate::privacy::affiliation::ModelAffiliation>,
    /// `provider/model`, for the report. Issue #108: the provider choice must be
    /// visible in the answer, not inferred from context.
    label: String,
}

impl Agent {
    pub async fn handle_ingest_source(
        &self,
        arguments: Value,
        session: &Session,
        cancel: Option<CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let chat_provider = self.provider().await.ok();
        let pinned_provider = Arc::new(tokio::sync::Mutex::new(chat_provider.clone()));
        let chat_capability = crate::privacy::CallCapability::sample(&pinned_provider).await;
        handle_ingest_source_with_provider(
            arguments,
            session,
            cancel,
            chat_capability,
            chat_provider,
        )
        .await
    }
}

/// Run the chat-side transactional ingest with a provider already pinned to the
/// current turn. Coding-agent bridge calls use this entry point so the tool does
/// not need a second, weaker implementation outside the ordinary handler.
pub(crate) async fn handle_ingest_source_with_provider(
    arguments: Value,
    session: &Session,
    cancel: Option<CancellationToken>,
    chat_capability: crate::privacy::CallCapability,
    chat_provider: Option<Arc<dyn Provider>>,
) -> ToolResult<Vec<Content>> {
    let svc = KnowledgeService::new_default().map_err(internal)?;

    let sources = parse_sources(&arguments, &session.working_dir).map_err(invalid_params)?;

    // Issue #56. The identity of the model *in this chat* — the audience of
    // the candidate list `resolve_target_kb` may put in its no-target error.
    // The bridge hands in the same once-per-call capability its dispatcher and
    // privacy gates use, so this path cannot re-sample a different model.
    let kb_id = resolve_target_kb(&svc, &arguments, &session.id, &kb_caller(chat_capability))
        .map_err(invalid_params)?;

    let chosen = choose_ingest_model(&arguments, session, cancel.clone(), chat_provider).await?;

    let report = ingest_sources(
        &svc,
        SourceIngestArgs {
            kb_id,
            // Issue #56. The tier and affiliation of the provider that will
            // actually run the sub-agent — from `paired_factory`, so they and
            // every completer in the batch come off one binding.
            caller_capability: chosen.capability,
            caller_affiliation: chosen.affiliation,
            sources,
            completer: chosen.completer,
            focus: arguments
                .get("focus")
                .and_then(Value::as_str)
                .map(str::to_string),
            bounds: ingest_bounds(),
            event_sink: None,
            cancel,
            model_label: chosen.label,
        },
    )
    .await
    .map_err(internal)?;

    Ok(vec![Content::text(report.summary())])
}

/// Which model runs this ingest — **never** a silent substitution.
///
/// Order: an explicitly named `model` (an alternate provider, so Gate H
/// applies), otherwise the model pinned to this chat turn. Either way the
/// provider is asked whether it can drive a Biorouter-run tool loop before any
/// source is touched, so a mismatch costs nothing and is reported rather than
/// discovered from a run that quietly wrote nothing.
async fn choose_ingest_model(
    arguments: &Value,
    session: &Session,
    cancel: Option<CancellationToken>,
    chat_provider: Option<Arc<dyn Provider>>,
) -> Result<ChosenModel, rmcp::model::ErrorData> {
    if biorouter_mcp::knowledge::test_mode::env_enabled() {
        // The third of the named test-mode exemptions (the HTTP route's
        // `build_completer` and `build_model_ref_completer` are the others).
        // No provider is constructed, so there is no instance to read a tier
        // from, and nothing leaves the process for a gate to refuse.
        return Ok(ChosenModel {
            completer: Box::new(|| {
                Box::new(biorouter_mcp::knowledge::test_mode::TestModeCompleter)
            }),
            capability: ProviderTier::Public,
            affiliation: None,
            label: "test-mode".to_string(),
        });
    }

    let (provider, label) = match parse_model_ref(arguments) {
        Some(model) => {
            let label = format!("{}/{}", model.provider, model.model);
            let provider = build_model_ref_provider(
                &model,
                session.privacy_tier,
                "ingesting these sources",
                "this tool's `model` argument",
            )
            .await
            .map_err(invalid_params)?;
            (provider, label)
        }
        None => {
            let provider = chat_provider
                .ok_or_else(|| internal("a model provider is required to ingest documents"))?;
            let label = format!(
                "{}/{}",
                provider.get_name(),
                provider.get_active_model_name()
            );
            (provider, label)
        }
    };

    if let Some(refusal) = source_ingest::tool_capability_refusal(provider.as_ref()) {
        return Err(invalid_params(refusal));
    }

    let (completer, capability, affiliation) =
        ProviderCompleter::paired_factory(provider, Some(session.id.clone()), cancel);
    Ok(ChosenModel {
        completer,
        capability,
        affiliation,
        label,
    })
}

/// The same bounds the HTTP ingest route uses. Stated here rather than shared
/// because that one is route-private; if they ever need to differ, they can.
fn ingest_bounds() -> SubAgentBounds {
    SubAgentBounds {
        max_steps: 60,
        max_wall: std::time::Duration::from_secs(900),
        max_tokens: 200_000,
    }
}

/// `{"provider": …, "model": …}`, or `None` when the caller named no model.
///
/// A half-filled object is `None` — an alternate provider is an explicit choice,
/// and half of one is not a choice at all. The chat's own model then runs the
/// ingest, which is the outcome the caller gets by saying nothing.
fn parse_model_ref(arguments: &Value) -> Option<ModelRef> {
    let obj = arguments.get("model")?;
    let provider = obj.get("provider")?.as_str()?.trim();
    let model = obj.get("model")?.as_str()?.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(ModelRef {
        provider: provider.to_string(),
        model: model.to_string(),
    })
}

/// Turn the tool's arguments into the sources to ingest.
///
/// Accepts a batch (`sources`) and the one-source shorthands (`path` / `url` /
/// `text`), and both may be given together — a model that writes one of each is
/// asking for both, and refusing that combination would be a rule with no
/// purpose.
///
/// A **bare string** in `sources` is a URL when it carries an `http://` or
/// `https://` scheme and a local path otherwise. Scheme-first rather than
/// "does it exist on disk", because the disk test makes the meaning of an
/// argument depend on the machine's state at that instant: the same string would
/// be a path on one run and a URL on the next, and the failure would be a
/// confusing fetch rather than a missing file.
///
/// A relative path resolves against the session's working directory, and `~`
/// expands, so what the user typed in chat means what it means in their shell.
pub(crate) fn parse_sources(
    arguments: &Value,
    working_dir: &Path,
) -> anyhow::Result<Vec<SourceSpec>> {
    let mut out = Vec::new();

    if let Some(items) = arguments.get("sources").and_then(Value::as_array) {
        for (i, item) in items.iter().enumerate() {
            match item {
                Value::String(s) => out.push(spec_from_str(s, working_dir)?),
                Value::Object(_) => out.push(spec_from_object(item, working_dir, i)?),
                other => anyhow::bail!(
                    "sources[{i}] must be a path or URL string, or an object with `path`, `url` \
                     or `text`; got {other}"
                ),
            }
        }
    }

    if let Some(path) = non_empty(arguments, "path") {
        out.push(SourceSpec::Path(resolve_path(&path, working_dir)));
    }
    if let Some(url) = non_empty(arguments, "url") {
        out.push(SourceSpec::Url(url));
    }
    if let Some(text) = non_empty(arguments, "text") {
        out.push(SourceSpec::Text {
            text,
            title: non_empty(arguments, "title"),
        });
    }

    if out.is_empty() {
        anyhow::bail!(
            "no sources given: pass `sources` (a list of file paths, folders or URLs), or one of \
             `path` / `url` / `text`"
        );
    }
    Ok(out)
}

fn spec_from_str(raw: &str, working_dir: &Path) -> anyhow::Result<SourceSpec> {
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("a source cannot be an empty string");
    }
    if is_http_url(raw) {
        return Ok(SourceSpec::Url(raw.to_string()));
    }
    Ok(SourceSpec::Path(resolve_path(raw, working_dir)))
}

fn spec_from_object(item: &Value, working_dir: &Path, index: usize) -> anyhow::Result<SourceSpec> {
    let path = non_empty(item, "path");
    let url = non_empty(item, "url");
    let text = non_empty(item, "text");
    match (path, url, text) {
        (Some(p), None, None) => Ok(SourceSpec::Path(resolve_path(&p, working_dir))),
        (None, Some(u), None) => Ok(SourceSpec::Url(u)),
        (None, None, Some(t)) => Ok(SourceSpec::Text {
            text: t,
            title: non_empty(item, "title"),
        }),
        (None, None, None) => anyhow::bail!(
            "sources[{index}] has none of `path`, `url` or `text` — one of them names the source"
        ),
        _ => anyhow::bail!(
            "sources[{index}] sets more than one of `path`, `url` and `text`; give one source per \
             entry so each is reported separately"
        ),
    }
}

fn is_http_url(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// `~` expands and a relative path resolves against the chat's working
/// directory, so a path the user typed means what it means in their shell.
fn resolve_path(raw: &str, working_dir: &Path) -> PathBuf {
    let expanded = PathBuf::from(shellexpand::tilde(raw).as_ref());
    if expanded.is_absolute() {
        expanded
    } else {
        working_dir.join(expanded)
    }
}

fn non_empty(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn internal(e: impl std::fmt::Display) -> rmcp::model::ErrorData {
    rmcp::model::ErrorData::new(rmcp::model::ErrorCode::INTERNAL_ERROR, e.to_string(), None)
}

fn invalid_params(e: impl std::fmt::Display) -> rmcp::model::ErrorData {
    rmcp::model::ErrorData::new(rmcp::model::ErrorCode::INVALID_PARAMS, e.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wd() -> PathBuf {
        PathBuf::from("/home/me/proj")
    }

    #[test]
    fn a_batch_of_bare_strings_splits_into_paths_and_urls() {
        let specs = parse_sources(
            &json!({"sources": ["papers/a.pdf", "https://example.org/b.pdf", "/tmp/c.md"]}),
            &wd(),
        )
        .unwrap();
        assert_eq!(
            specs,
            vec![
                SourceSpec::Path(PathBuf::from("/home/me/proj/papers/a.pdf")),
                SourceSpec::Url("https://example.org/b.pdf".into()),
                SourceSpec::Path(PathBuf::from("/tmp/c.md")),
            ]
        );
    }

    /// The scheme decides, never the filesystem: `parse_sources` must give the
    /// same answer on a machine where the file happens to exist and one where it
    /// does not.
    #[test]
    fn a_bare_string_is_classified_by_scheme_not_by_what_is_on_disk() {
        let here = std::env::current_dir().unwrap();
        let real = here.join("Cargo.toml");
        let specs = parse_sources(&json!({"sources": [real.to_string_lossy()]}), &wd()).unwrap();
        assert_eq!(specs, vec![SourceSpec::Path(real)]);

        let specs = parse_sources(
            &json!({"sources": ["https://example.org/definitely-not-a-file"]}),
            &wd(),
        )
        .unwrap();
        assert!(matches!(specs[0], SourceSpec::Url(_)));
    }

    #[test]
    fn object_entries_name_exactly_one_source_each() {
        let specs = parse_sources(
            &json!({"sources": [
                {"path": "~/Downloads/x.pdf"},
                {"url": "https://example.org/y"},
                {"text": "hello", "title": "Note"}
            ]}),
            &wd(),
        )
        .unwrap();
        assert!(matches!(specs[0], SourceSpec::Path(_)));
        assert!(matches!(specs[1], SourceSpec::Url(_)));
        assert_eq!(
            specs[2],
            SourceSpec::Text {
                text: "hello".into(),
                title: Some("Note".into())
            }
        );
        // `~` expanded rather than being taken literally.
        let SourceSpec::Path(p) = &specs[0] else {
            panic!("expected a path")
        };
        assert!(!p.to_string_lossy().starts_with('~'), "got {}", p.display());
    }

    #[test]
    fn an_entry_that_names_two_sources_is_refused_by_index() {
        let err = parse_sources(
            &json!({"sources": [{"path": "a.pdf", "url": "https://example.org/b"}]}),
            &wd(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("sources[0]"), "got: {err}");
        assert!(err.contains("more than one"), "got: {err}");
    }

    #[test]
    fn an_entry_that_names_no_source_is_refused_by_index() {
        let err = parse_sources(&json!({"sources": [{"title": "orphan"}]}), &wd())
            .unwrap_err()
            .to_string();
        assert!(err.contains("sources[0]"), "got: {err}");
    }

    #[test]
    fn the_single_source_shorthands_work_and_compose_with_a_batch() {
        let specs = parse_sources(
            &json!({"sources": ["a.pdf"], "url": "https://example.org/b", "text": "note"}),
            &wd(),
        )
        .unwrap();
        assert_eq!(specs.len(), 3);
    }

    #[test]
    fn no_source_at_all_names_every_argument_that_would_have_worked() {
        let err = parse_sources(&json!({"kb_id": "ms"}), &wd())
            .unwrap_err()
            .to_string();
        for arg in ["sources", "path", "url", "text"] {
            assert!(err.contains(arg), "the error must name `{arg}`: {err}");
        }
    }

    // -----------------------------------------------------------------------
    // Gate H: the alternate provider a MODEL names
    // -----------------------------------------------------------------------

    fn public_session(id: &str) -> Session {
        use crate::session::session_manager::SessionType;
        Session {
            id: id.to_string(),
            working_dir: PathBuf::from("."),
            name: "Gate H".to_string(),
            user_set_name: false,
            session_type: SessionType::User,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            extension_data: Default::default(),
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            accumulated_total_tokens: None,
            accumulated_input_tokens: None,
            accumulated_output_tokens: None,
            schedule_id: None,
            workflow: None,
            user_workflow_values: None,
            conversation: None,
            message_count: 0,
            provider_name: None,
            model_config: None,
            diverged_from: None,
            branch_point_msg_uid: None,
            parent_session_id: None,
            privacy_tier: crate::privacy::SessionClassification::Public,
            privacy_reason: None,
        }
    }

    /// `ollama` resolved against a host, so `providers::create` builds a REAL
    /// provider whose `tier()` is the production implementation reading the
    /// resolved base URL. 127.0.0.1 is loopback, so it reads PRIVATE; port 1
    /// refuses instantly, so nothing here waits on a model.
    fn ollama_at(host: &str) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::from([("OLLAMA_HOST".to_string(), host.to_string())])
    }
    const A_PRIVATE_HOST: &str = "http://127.0.0.1:1";
    const A_PUBLIC_HOST: &str = "https://api.example-saas.invalid";

    async fn ingest_on(kb: &str, session: &Session, host: &str) -> ToolResult<Vec<Content>> {
        crate::config::with_config_overrides(
            ollama_at(host),
            handle_ingest_source_with_provider(
                serde_json::json!({
                    "kb_id": kb,
                    "text": "Ordinary public notes, ingested from a public chat.",
                    "title": "note",
                    "model": {"provider": "ollama", "model": "qwen3"},
                }),
                session,
                None,
                crate::privacy::CallCapability::for_test(ProviderTier::Public, true),
                None,
            ),
        )
        .await
    }

    /// **The finding, driven through the real handler.** A PUBLIC chat names a
    /// PRIVATE model in `platform__ingest_source`'s `model` argument. The chosen
    /// provider's tier — not the session's classification — is what
    /// `SourceIngestArgs::caller_capability` carries into the knowledge base's
    /// permanent ratchet, so before the fix this call privatised the base and
    /// locked the chat that owns it out of its own notes.
    ///
    /// Measured on this branch before the gate changed, with exactly this test:
    ///
    /// ```text
    /// --- handler outcome: Ok([… "Curated 0 of 1 source(s) into knowledge base
    ///     'gate-h-exploit' on ollama/qwen3. …"])
    /// --- tier::is_private after the call: true
    /// --- a public caller can still reach it: false
    /// ```
    ///
    /// i.e. the tool answered with an ordinary per-source report — not an error —
    /// and the base was gone. Note the ratchet fires in `prepare_ingest_base`,
    /// **before** the sub-agent runs, so a curation that failed outright still
    /// cost the user the base.
    #[tokio::test]
    async fn a_public_chat_may_not_privatise_a_base_by_naming_a_private_model() {
        use biorouter_mcp::knowledge::caller::KbCaller;
        use biorouter_mcp::knowledge::tier;

        let svc = KnowledgeService::new_default().expect("the lib binary's sandboxed root");
        let kb = "gate-h-private-model-from-public-chat";
        svc.create_base(kb, "Gate H", None).expect("create");
        assert!(
            !tier::is_private(svc.root(), kb) && KbCaller::restricted().can_reach(svc.root(), kb),
            "precondition: the base starts public and its public owner can reach it"
        );

        let session = public_session("gate-h-public");
        let err = ingest_on(kb, &session, A_PRIVATE_HOST)
            .await
            .expect_err("a public chat may not run an ingest on a private model")
            .message
            .to_string();
        assert!(err.contains("private model"), "{err}");
        assert!(
            err.contains("`model` argument"),
            "the refusal must name the knob that fixes it: {err}"
        );
        assert!(
            err.contains("ollama"),
            "a refusal names what it refused: {err}"
        );

        // The ratchet is permanent, so a refusal that had already written would be
        // unrecoverable. It did not write, and the owner still has its base.
        assert!(
            !tier::is_private(svc.root(), kb),
            "a refused ingest ratcheted the base to PRIVATE anyway"
        );
        assert!(
            KbCaller::restricted().can_reach(svc.root(), kb),
            "the public chat that owns this base can no longer read it"
        );
    }

    /// The other direction, or the gate is not a barrier but an outage: the same
    /// public chat naming a PUBLIC alternate model still runs, and the base stays
    /// public.
    ///
    /// The ingest itself fails — the endpoint is unroutable — and that is the
    /// point: it fails *past* the gate, with the per-source report the tool
    /// answers with, having reached `prepare_ingest_base` and left the base
    /// public.
    #[tokio::test]
    async fn the_same_chat_may_still_name_a_public_alternate_model() {
        use biorouter_mcp::knowledge::caller::KbCaller;
        use biorouter_mcp::knowledge::tier;

        let svc = KnowledgeService::new_default().expect("the lib binary's sandboxed root");
        let kb = "gate-h-public-model-from-public-chat";
        svc.create_base(kb, "Gate H", None).expect("create");

        let session = public_session("gate-h-public-sideways");
        let report = ingest_on(kb, &session, A_PUBLIC_HOST)
            .await
            .expect("a public chat on a public alternate model is a sideways choice");
        let text = format!("{report:?}");
        assert!(
            text.contains("ollama/qwen3"),
            "the report must name the model that ran it: {text}"
        );
        assert!(!tier::is_private(svc.root(), kb));
        assert!(KbCaller::restricted().can_reach(svc.root(), kb));
    }

    /// Half a model reference is not a choice, so the chat's own model runs the
    /// ingest — never a partially-resolved provider.
    #[test]
    fn a_half_filled_model_reference_selects_no_alternate_provider() {
        assert!(parse_model_ref(&json!({"model": {"provider": "anthropic"}})).is_none());
        assert!(parse_model_ref(&json!({"model": {"model": "claude-opus-5"}})).is_none());
        assert!(parse_model_ref(&json!({"model": {"provider": " ", "model": "x"}})).is_none());
        assert!(parse_model_ref(&json!({})).is_none());
        let m = parse_model_ref(&json!({"model": {"provider": "versa", "model": "opus"}})).unwrap();
        assert_eq!(m.provider, "versa");
        assert_eq!(m.model, "opus");
    }
}
