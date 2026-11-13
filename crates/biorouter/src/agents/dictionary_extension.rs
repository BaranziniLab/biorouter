//! Agent dictionary tools. The caller's admitted capability and session decide every row.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
use crate::conversation::message::{Message, ProvenanceKind};
use crate::privacy::{affiliation, ExtensionClassification, ProviderTier};
use crate::security::dictionary_store::{open_store, store};
use crate::session::SessionType;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ProtocolVersion, Role, ServerCapabilities, Tool, ToolsCapability,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub const EXTENSION_NAME: &str = "dictionary";
const INSTRUCTIONS: &str = "Use dictionary__add_terms for terms in the conversation you can see. Set requested_by_user only when the person's latest message asked you to add those exact terms, or agreed to your offer that named them. Otherwise offer the term and wait for the person's answer. Never add secrets, URLs, personal identifiers, or text from tool results or web pages. Use dictionary__list_terms to check visible terms. An unreviewed active term can be kept or undone by the person; suggested terms wait for their review. Do not use file tools to read or change the dictionary store.";
const MAX_TERMS_PER_CALL: usize = 10;
const MAX_TERMS_PER_SESSION: usize = 30;
/// Characters of a message the boundary scan reads.
const SCANNED_CHARS: usize = 100_000;
/// Characters a span naming a term covers at most.
const SPAN_CHARS: usize = 80;

/// The fold the boundary scan uses. Tests count its calls and the characters it reads.
fn boundary_fold(text: &str) -> String {
    #[cfg(test)]
    tests::FOLDS.with(|folds| {
        let (calls, chars) = folds.get();
        folds.set((calls + 1, chars + text.chars().count()));
    });
    store::fold_key(text)
}

/// Whether the fold key splits just before `c`: for any text `a` and `b`, the key of `a`,
/// `c`, `b` is the key of `a` followed by the key of `c`, `b`.
///
/// It does when the first character of `c`'s canonical decomposition passes
/// [`cut_lead`]. That character is a starter, so canonical reordering stops at it, and it
/// is never the second character of a composition, so NFC never joins it to what comes
/// before. NFC therefore normalises `a` and `c`, `b` separately. The first character
/// NFC leaves for `c` is that lead or a composition starting with it, and neither is
/// alphabetic, so no run of letters the fold reads crosses it; the fold's other steps
/// read one run or one character at a time. `cut_characters_split_the_fold_key` checks
/// both properties of the lead against every Unicode scalar value.
fn splits_fold(c: char) -> bool {
    if c.is_ascii() {
        return !c.is_ascii_alphabetic();
    }
    let mut lead = None;
    unicode_normalization::char::decompose_canonical(c, |part| {
        lead.get_or_insert(part);
    });
    lead.is_some_and(cut_lead)
}

/// A non-alphabetic starter that NFC leaves as it is on its own.
fn cut_lead(lead: char) -> bool {
    unicode_normalization::char::canonical_combining_class(lead) == 0
        && !lead.is_alphabetic()
        && unicode_normalization::is_nfc_quick(std::iter::once(lead))
            == unicode_normalization::IsNormalized::Yes
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    terms: Vec<AddTerm>,
    #[serde(default)]
    requested_by_user: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTerm {
    written: String,
    kind: Option<store::TermKind>,
    reason: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    query: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveArgs {
    ids: Vec<String>,
}

pub struct DictionaryClient {
    info: InitializeResult,
    sessions: Arc<crate::session::SessionManager>,
}

impl DictionaryClient {
    pub fn new(context: PlatformExtensionContext) -> Self {
        Self {
            info: InitializeResult {
                protocol_version: ProtocolVersion::V_2025_03_26,
                capabilities: ServerCapabilities {
                    tools: Some(ToolsCapability {
                        list_changed: Some(false),
                    }),
                    ..ServerCapabilities::default()
                },
                server_info: Implementation {
                    name: EXTENSION_NAME.into(),
                    title: Some("Dictionary".into()),
                    version: "1.0.0".into(),
                    icons: None,
                    website_url: None,
                },
                instructions: Some(INSTRUCTIONS.into()),
            },
            sessions: context.session_manager,
        }
    }

    fn tools() -> Vec<Tool> {
        let schema = |value| serde_json::from_value::<JsonObject>(value).expect("static schema");
        vec![
            Tool::new("add_terms", "Suggest one to ten terms from this conversation. Terms the person requested may become active under their setting.", schema(json!({
                "type":"object", "required":["terms"], "properties":{
                    "terms":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"object","required":["written"],"properties":{
                        "written":{"type":"string","minLength":1,"maxLength":64},"kind":{"type":"string","enum":["gene_symbol","acronym","name","term"]},"reason":{"type":"string","maxLength":140}
                    },"additionalProperties":false}},
                    "requested_by_user":{"type":"boolean"}
                },"additionalProperties":false
            }))),
            Tool::new("list_terms", "List only dictionary terms this chat may see, without a total count.", schema(json!({
                "type":"object", "properties":{
                    "query":{"type":"string"},"status":{"type":"string","enum":["active","suggested","all"]},
                    "limit":{"type":"integer","minimum":1,"maximum":50},"cursor":{"type":"string"}
                },"additionalProperties":false
            }))),
            Tool::new("remove_added_terms", "Remove only agent terms this chat may see that are suggested or still unreviewed.", schema(json!({
                "type":"object","required":["ids"],"properties":{"ids":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":10}},"additionalProperties":false
            }))),
        ]
    }

    async fn allowed_session(&self, session_id: &str) -> Result<crate::session::Session> {
        let session = self.sessions.get_session(session_id, true).await?;
        if session.session_type != SessionType::User {
            return Err(anyhow!(
                "The Dictionary capability is unavailable in this session."
            ));
        }
        // Fails closed like `Agent::dispatch_tool_call`: when the Crew manager cannot be
        // read, the chat cannot be shown to be outside a Crew scope.
        let crew_scoped = match crate::crew::manager() {
            Ok(crew) => crew.is_scoped_session(session_id).await,
            Err(_) => true,
        };
        if crew_scoped {
            return Err(anyhow!(
                "The Dictionary capability is unavailable in a Crew scoped chat."
            ));
        }
        Ok(session)
    }

    fn visible(entry: &store::Entry, cap: crate::privacy::CallCapability) -> bool {
        if entry.tier == store::EntryTier::Private && !cap.tier().is_private() {
            return false;
        }
        let stored = entry
            .affiliation
            .as_ref()
            .map(|row| row.institutions.as_slice());
        affiliation::dictionary_row_allowlists(stored)
            .into_iter()
            .all(|allowlist| {
                cap.cross_affiliation_warning(
                    "dictionary",
                    &ExtensionClassification {
                        tier: ProviderTier::Private,
                        affiliation: allowlist,
                    },
                )
                .is_none()
            })
    }

    /// Whether `text` names `term` (owner decision O-34): within its first 100,000
    /// characters, a span of at most 80 that starts and ends at a word boundary has the
    /// term's fold key.
    ///
    /// A span's key is put together from the keys of the pieces between cut characters
    /// ([`splits_fold`]) instead of folding every span whole. A piece that starts at a cut
    /// is folded once for the whole scan, and so is the stretch from the last cut before
    /// an end to that end. A start stops at the first piece that leaves the term's key,
    /// because every longer span's key begins with the pieces already joined. So a start
    /// folds only its own text up to the next cut, and a message of words between spaces
    /// costs about one fold per word.
    fn named_at_boundary(text: &str, term: &str) -> bool {
        let folded = boundary_fold(term);
        if folded.is_empty() {
            return false;
        }
        let text = text.chars().take(SCANNED_CHARS).collect::<String>();
        let chars = text.char_indices().collect::<Vec<_>>();
        let len = chars.len();
        let alnum = chars
            .iter()
            .map(|(_, c)| c.is_alphanumeric())
            .collect::<Vec<_>>();
        let cut = chars
            .iter()
            .map(|(_, c)| splits_fold(*c))
            .collect::<Vec<_>>();
        let is_cut = |at: usize| cut.get(at).copied().unwrap_or(false);
        let byte = |at: usize| chars.get(at).map_or(text.len(), |(offset, _)| *offset);
        // Both offsets come from `char_indices`, so the slice always exists.
        let span = |from: usize, to: usize| {
            boundary_fold(text.get(byte(from)..byte(to)).unwrap_or_default())
        };
        // Keyed by position: the key from a cut to the next cut, and the key from the
        // last cut before an end to that end. Neither depends on where a span starts.
        let mut pieces = HashMap::<usize, String>::new();
        let mut tails = HashMap::<usize, String>::new();
        for start in 0..len {
            if start > 0 && alnum[start - 1] {
                continue;
            }
            // The key from `start` to `head`, which is `start` or the last cut passed,
            // is `folded[..matched]`.
            let mut head = start;
            let mut matched = 0;
            // The key from a start that is not a cut to the first cut after it.
            let mut own = None::<String>;
            for end in start + 1..=len.min(start + SPAN_CHARS) {
                let last = end - 1;
                if last > start && is_cut(last) {
                    // `last` is the first cut after `head`.
                    let piece = if is_cut(head) {
                        pieces.entry(head).or_insert_with(|| span(head, last))
                    } else {
                        own.get_or_insert_with(|| span(head, last))
                    };
                    match folded.get(matched..) {
                        Some(rest) if rest.starts_with(piece.as_str()) => matched += piece.len(),
                        _ => break,
                    }
                    head = last;
                }
                // `end == len` reads as no character, so not alphanumeric.
                if alnum.get(end).copied().unwrap_or(false) {
                    continue;
                }
                let direct;
                let tail = if is_cut(end) {
                    // `end` is the first cut after `head`, so the tail is `head`'s piece.
                    if is_cut(head) {
                        pieces.entry(head).or_insert_with(|| span(head, end))
                    } else {
                        own.get_or_insert_with(|| span(head, end))
                    }
                    .as_str()
                } else if is_cut(head) {
                    // `head` is the last cut before `end`, whatever the start.
                    tails.entry(end).or_insert_with(|| span(head, end)).as_str()
                } else {
                    direct = span(start, end);
                    direct.as_str()
                };
                if folded.get(matched..) == Some(tail) {
                    return true;
                }
            }
        }
        false
    }

    fn person_named_term(session: &crate::session::Session, term: &str) -> bool {
        session.conversation.as_ref().is_some_and(|conversation| {
            Self::person_named_term_in_messages(conversation.messages(), term)
        })
    }

    fn person_named_term_in_messages(messages: &[Message], term: &str) -> bool {
        // Tool results are stored with the user role, and so are rows only the model
        // reads (hook context, post-edit diagnostics, loop warnings, nudges, compaction
        // summaries), which can echo text from files or the web. The person's latest
        // message is the newest user message the person can see that carries no tool
        // result, and the offer it answers is the newest message before it that the
        // person can see.
        let Some((index, latest)) = messages.iter().enumerate().rev().find(|(_, message)| {
            message.role == Role::User && message.is_user_visible() && !message.is_tool_response()
        }) else {
            return false;
        };
        if latest.metadata.provenance.as_ref().is_some_and(|stamp| {
            matches!(
                stamp.kind,
                ProvenanceKind::AgentInjection | ProvenanceKind::SpawnContext
            )
        }) {
            return false;
        }
        if Self::named_at_boundary(&latest.as_concat_text(), term) {
            return true;
        }
        messages
            .get(..index)
            .and_then(|before| {
                before
                    .iter()
                    .rev()
                    .find(|message| message.is_user_visible())
            })
            .filter(|offer| offer.role == Role::Assistant)
            .is_some_and(|offer| Self::named_at_boundary(&offer.as_concat_text(), term))
    }

    fn should_activate(mode: store::AgentWrites, asked: bool, session_count: usize) -> bool {
        !crate::dictation::activation::suggest_only_for_process()
            && session_count < MAX_TERMS_PER_SESSION
            && match mode {
                store::AgentWrites::Suggest => false,
                store::AgentWrites::OnRequest => asked,
                store::AgentWrites::Add => true,
            }
    }

    fn rejects_content(terms: &[AddTerm]) -> bool {
        let pii = crate::guardrails::pii::PiiDetector::new();
        terms.iter().any(|term| {
            !pii.scan(&term.written).is_empty()
                || term
                    .reason
                    .as_deref()
                    .is_some_and(|reason| !pii.scan(reason).is_empty())
                || term.written.contains("://")
                || term
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("://"))
                || crate::guardrails::secret_output::redact_text(&term.written).is_some()
                || term.reason.as_deref().is_some_and(|reason| {
                    crate::guardrails::secret_output::redact_text(reason).is_some()
                })
        })
    }

    fn rejects_term_count(count: usize) -> bool {
        count == 0 || count > MAX_TERMS_PER_CALL
    }

    async fn add_terms(&self, args: Value, meta: &McpMeta) -> Result<Vec<Content>> {
        let args: AddArgs = serde_json::from_value(args)?;
        if Self::rejects_term_count(args.terms.len()) {
            return Err(anyhow!("Add one to ten terms per call."));
        }
        let session = self.allowed_session(&meta.session_id).await?;
        if Self::rejects_content(&args.terms) {
            return Err(anyhow!(
                "Dictionary terms cannot contain personal data or URLs."
            ));
        }
        let private = meta.capability.tier().is_private();
        let tier = if private {
            store::EntryTier::Private
        } else {
            store::EntryTier::Public
        };
        let reach = if private {
            store::Reach::All
        } else {
            store::Reach::Public
        };
        let touched = self.sessions.session_affiliations(&meta.session_id).await;
        let affiliation = if private {
            affiliation::dictionary_row_institutions(
                affiliation::WriterModel::Known(meta.capability.affiliation()),
                touched,
            )
            .stored_ids()
            .map(|institutions| store::RowAffiliation { institutions })
        } else {
            None
        };
        let requested = args
            .terms
            .iter()
            .map(|term| args.requested_by_user && Self::person_named_term(&session, &term.written))
            .collect::<Vec<_>>();
        let now = store::schema::now();
        let session_id = meta.session_id.clone();
        let cap = meta.capability;
        let outcomes = open_store().write(|file| {
            let mode = file.settings.agent_writes;
            let mut session_count = file
                .entries
                .iter()
                .filter(|entry| {
                    entry.source == store::EntrySource::Agent
                        && entry.suggested_in.as_deref() == Some(&session_id)
                })
                .count();
            let mut answers = Vec::with_capacity(args.terms.len());
            for (term, asked) in args.terms.into_iter().zip(requested) {
                let active = Self::should_activate(mode, asked, session_count);
                let mut new = store::NewTerm::person(term.written, tier);
                new.source = store::EntrySource::Agent;
                new.status = if active {
                    store::EntryStatus::Active
                } else {
                    store::EntryStatus::Suggested
                };
                new.match_policy = Some(store::MatchPolicy::Exact);
                new.kind = term.kind;
                new.reason = term.reason;
                new.suggested_in = Some(session_id.clone());
                new.affiliation = affiliation.clone();
                let outcome = file.add(new, reach, now)?;
                if matches!(outcome, store::AddOutcome::Created { .. }) {
                    session_count += 1;
                }
                let (outcome, row) = match outcome {
                    store::AddOutcome::Created { ref id, .. } => {
                        let row = file.find(id).cloned();
                        (outcome, row)
                    }
                    store::AddOutcome::Exists { ref id } => {
                        let row = file
                            .find(id)
                            .filter(|entry| Self::visible(entry, cap))
                            .cloned();
                        if row.is_some() {
                            (outcome, row)
                        } else {
                            (store::AddOutcome::Skipped, None)
                        }
                    }
                    store::AddOutcome::Skipped => (store::AddOutcome::Skipped, None),
                };
                answers.push((outcome, row));
            }
            Ok(answers)
        })?;
        Ok(Self::add_result(
            outcomes,
            crate::dictation::activation::process_surface(),
        ))
    }

    fn add_result(
        outcomes: Vec<(store::AddOutcome, Option<store::Entry>)>,
        surface: crate::dictation::activation::Surface,
    ) -> Vec<Content> {
        let mut added = 0;
        let mut suggested = 0;
        let mut existing = 0;
        let mut unchanged = false;
        let mut rows = Vec::new();
        for (outcome, row) in outcomes {
            match outcome {
                store::AddOutcome::Created { status, .. } => {
                    if status == store::EntryStatus::Active {
                        added += 1;
                    } else {
                        suggested += 1;
                    }
                }
                store::AddOutcome::Exists { .. } => existing += 1,
                store::AddOutcome::Skipped => unchanged = true,
            }
            if let Some(row) = row {
                rows.push(
                    json!({"id": row.id.as_str(), "rev": row.rev, "written": row.written,
                    "status": row.status.as_str(), "reviewed": row.reviewed,
                    "source": row.source.as_str()}),
                );
            }
        }
        let mut messages = Vec::new();
        if added > 0 {
            messages.push(format!(
                "Added {added} terms to the dictionary. They are marked for the person's review."
            ));
        }
        if suggested > 0 {
            messages.push(format!("Recorded {suggested} suggestions for the person's review in Dictionary. Nothing changes in dictation until they accept."));
        }
        if existing > 0 {
            messages.push(if existing == 1 {
                "1 term was already in Dictionary.".into()
            } else {
                format!("{existing} terms were already in Dictionary.")
            });
        }
        if unchanged {
            messages.push("Some terms could not be recorded.".into());
        }
        if matches!(
            surface,
            crate::dictation::activation::Surface::Cli | crate::dictation::activation::Surface::Acp
        ) {
            messages.push("Review them with `biorouter dictionary review` on this computer, or in Dictionary in Biorouter.".into());
        }
        vec![
            Content::text(messages.join("\n")),
            Content::text(json!({"dictionary_rows": rows}).to_string())
                .with_audience(vec![Role::User]),
        ]
    }

    fn listed_row(entry: &store::Entry) -> Value {
        json!({
            "id": entry.id.as_str(), "written": entry.written, "tier": entry.tier.as_str(),
            "status": entry.status.as_str(), "kind": entry.kind,
            "reviewed": entry.reviewed, "source": entry.source.as_str(),
        })
    }

    async fn list_terms(&self, args: Value, meta: &McpMeta) -> Result<Value> {
        let args: ListArgs = serde_json::from_value(args)?;
        let _session = self.allowed_session(&meta.session_id).await?;
        let limit = args.limit.unwrap_or(20);
        if args
            .status
            .as_deref()
            .is_some_and(|value| !matches!(value, "active" | "suggested" | "all"))
        {
            return Err(anyhow!("status must be active, suggested, or all"));
        }
        if limit == 0 || limit > 50 {
            return Err(anyhow!("limit must be between 1 and 50"));
        }
        let view = open_store().view()?;
        let query = args.query.map(|q| store::fold_key(&q));
        let mut rows = view
            .entries()
            .iter()
            .filter(|entry| Self::visible(entry, meta.capability))
            .filter(|entry| {
                args.status
                    .as_deref()
                    .is_none_or(|status| status == "all" || entry.status.as_str() == status)
            })
            .filter(|entry| query.as_ref().is_none_or(|q| entry.fold_key().contains(q)))
            .filter(|entry| {
                args.cursor
                    .as_ref()
                    .is_none_or(|cursor| entry.id.as_str() > cursor.as_str())
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        let next_cursor = rows
            .get(limit)
            .map(|_| rows[limit - 1].id.as_str().to_string());
        Ok(
            json!({"terms": rows.into_iter().take(limit).map(Self::listed_row).collect::<Vec<_>>(),"next_cursor":next_cursor}),
        )
    }

    async fn remove_added_terms(&self, args: Value, meta: &McpMeta) -> Result<Value> {
        let args: RemoveArgs = serde_json::from_value(args)?;
        if args.ids.is_empty() || args.ids.len() > MAX_TERMS_PER_CALL {
            return Err(anyhow!("Remove one to ten terms per call."));
        }
        let _session = self.allowed_session(&meta.session_id).await?;
        let ids = args
            .ids
            .iter()
            .map(|raw| store::EntryId::parse(raw).ok_or_else(|| anyhow!(store::MSG_NOT_FOUND)))
            .collect::<Result<Vec<_>>>()?;
        let cap = meta.capability;
        let removed = open_store().write(|file| {
            for id in &ids {
                let entry = file.find(id).ok_or(store::StoreError::NotFound)?;
                if !Self::visible(entry, cap)
                    || entry.source != store::EntrySource::Agent
                    || (entry.status != store::EntryStatus::Suggested && entry.reviewed)
                {
                    return Err(store::StoreError::NotFound);
                }
            }
            file.entries.retain(|entry| !ids.contains(&entry.id));
            Ok(ids.len())
        })?;
        Ok(json!({"removed":removed}))
    }
}

#[async_trait]
impl McpClientTrait for DictionaryClient {
    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        meta: McpMeta,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let args = Value::Object(arguments.unwrap_or_default());
        let result: Result<Vec<Content>> = match name {
            "add_terms" => self.add_terms(args, &meta).await,
            "list_terms" => self
                .list_terms(args, &meta)
                .await
                .map(|value| vec![Content::text(value.to_string())]),
            "remove_added_terms" => self
                .remove_added_terms(args, &meta)
                .await
                .map(|value| vec![Content::text(value.to_string())]),
            _ => Err(anyhow!("Unknown dictionary tool")),
        };
        Ok(match result {
            Ok(content) => CallToolResult::success(content),
            Err(error) => CallToolResult::error(vec![Content::text(error.to_string())]),
        })
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Calls of [`boundary_fold`] on this thread, and the characters they read.
        pub(super) static FOLDS: std::cell::Cell<(usize, usize)> =
            const { std::cell::Cell::new((0, 0)) };
    }

    /// The scan before it was bounded: every span folded whole. The new scan must agree
    /// with it on every input.
    fn named_at_boundary_reference(text: &str, term: &str) -> bool {
        let folded = store::fold_key(term);
        if folded.is_empty() {
            return false;
        }
        let text = text.chars().take(100_000).collect::<String>();
        for (start, _) in text.char_indices() {
            if text
                .get(..start)
                .and_then(|before| before.chars().next_back())
                .is_some_and(char::is_alphanumeric)
            {
                continue;
            }
            let Some(rest) = text.get(start..) else {
                continue;
            };
            for (offset, ch) in rest.char_indices().take(80) {
                let end = start + offset + ch.len_utf8();
                if text
                    .get(end..)
                    .and_then(|after| after.chars().next())
                    .is_some_and(char::is_alphanumeric)
                {
                    continue;
                }
                if text
                    .get(start..end)
                    .is_some_and(|candidate| store::fold_key(candidate) == folded)
                {
                    return true;
                }
            }
        }
        false
    }

    /// The spans the reference folds for `text`, and the characters it folds, counted
    /// without folding.
    fn reference_work(text: &str) -> (usize, usize) {
        let chars = text.chars().take(100_000).collect::<Vec<_>>();
        let mut work = (0, 0);
        for start in 0..chars.len() {
            if start > 0 && chars[start - 1].is_alphanumeric() {
                continue;
            }
            for end in start + 1..=chars.len().min(start + 80) {
                if end == chars.len() || !chars[end].is_alphanumeric() {
                    work = (work.0 + 1, work.1 + end - start);
                }
            }
        }
        work
    }

    fn agree(text: &str, term: &str) -> bool {
        let expected = named_at_boundary_reference(text, term);
        assert_eq!(
            DictionaryClient::named_at_boundary(text, term),
            expected,
            "text {text:?} term {term:?}"
        );
        expected
    }

    #[test]
    fn the_bounded_scan_agrees_with_the_reference_on_tricky_inputs() {
        let i_dots = "\u{130}".repeat(50);
        let i_dot_pairs = "i\u{307}".repeat(50);
        let eighty = "a".repeat(80);
        let eighty_one = "a".repeat(81);
        let cases: &[(&str, &str, bool)] = &[
            ("Please add EPAS1 and HIF2A", "EPAS1", true),
            ("Please add XEPAS1Y", "EPAS1", false),
            ("EPAS1 is a gene", "EPAS1", true),
            ("the gene EPAS1", "EPAS1", true),
            ("epas1", "EPAS1", true),
            ("(EPAS1)", "EPAS1", true),
            ("EPAS1.", "EPAS1", true),
            ("EPAS1,2", "EPAS1", true),
            ("x-EPAS1", "EPAS1", true),
            ("EPAS12", "EPAS1", false),
            ("1EPAS1", "EPAS1", false),
            ("E-PAS-1 is here", "EPAS 1", true),
            ("HIF\u{2013}2", "HIF2", true),
            ("HIF-2alpha", "HIF2\u{3b1}", true),
            ("TNFalpha.", "TNF-\u{3b1}", true),
            ("NFkappaB", "NF\u{3ba}B", false),
            ("Philadelphia", "\u{3c6}ladel\u{3c6}a", false),
            ("Philadelphia", "Philadelphia", true),
            ("Cafe\u{301} au lait", "Caf\u{e9}", true),
            ("Cafe\u{301}", "Cafe", true),
            ("e\u{301}\u{323} x", "\u{1eb9}\u{301}", true),
            ("x <\u{338} y", "\u{226e}", true),
            ("x <\u{338}", "<", true),
            (
                "\u{39f}\u{394}\u{39f}\u{3a3}",
                "\u{3bf}\u{3b4}\u{3bf}\u{3c2}",
                true,
            ),
            ("\u{212a}EAP", "KEAP", true),
            ("A\u{387}B", "AB", true),
            ("EPAS1\u{37e}", "EPAS1", true),
            ("Bio\u{a0}router", "Biorouter", true),
            ("\u{1100}\u{1161} x", "\u{ac00}", true),
            (
                "\u{4e2d}\u{6587}\u{ff0c}\u{6d4b}\u{8bd5}\u{3002}",
                "\u{6d4b}\u{8bd5}",
                true,
            ),
            (
                "\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}",
                "\u{6d4b}\u{8bd5}",
                false,
            ),
            ("\u{301}\u{301}\u{301}", "\u{301}\u{301}", true),
            (&i_dots, &i_dot_pairs, true),
            (&eighty, &eighty, true),
            (&eighty_one, &eighty_one, false),
            ("anything", "", false),
            ("a - b", " -. ", false),
            ("", "EPAS1", false),
        ];
        for (text, term, expected) in cases {
            assert_eq!(agree(text, term), *expected, "text {text:?} term {term:?}");
        }
    }

    #[test]
    fn the_scan_reads_only_the_first_hundred_thousand_characters() {
        let at_the_limit = format!("{} EPAS1X", "x".repeat(99_994));
        assert!(agree(&at_the_limit, "EPAS1"));
        let past_the_limit = format!("{} EPAS1", "x".repeat(99_995));
        assert!(!agree(&past_the_limit, "EPAS1"));
    }

    struct Seeded(u64);

    impl Seeded {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }
    }

    #[test]
    fn the_bounded_scan_agrees_with_the_reference_on_generated_inputs() {
        // Letters, Greek names, digits, cuts, combining marks and characters NFC or the
        // fold change, one token between each pair of bars.
        let tokens = "EPAS|epas|1|2|HIF|alpha|Alpha|eta|beta|TNF|a|e|E|x| |  |-|.|,|(|)|<|=|\
            \u{301}|\u{323}|\u{338}|\u{e9}|\u{c9}|\u{3b1}|\u{3a3}|\u{3c2}|\u{3c3}|\u{130}|\
            i\u{307}|\u{212a}|\u{37e}|\u{387}|\u{b7}|\u{2013}|\u{a0}|\u{4e2d}|\u{ff0c}|\
            \u{3002}|\u{1f600}|\u{200d}|\u{663}|\u{1c5}|\u{df}|\u{345}|\u{94d}|\u{915}|\
            \u{1100}|\u{1161}|\u{ac00}"
            .split('|')
            .collect::<Vec<_>>();
        assert_eq!(tokens.len(), 54);
        let mut seeded = Seeded(0x5eed_d1c7_0a11_0001);
        let mut named = 0;
        let cases = 3_000;
        for case in 0..cases {
            // One text in ten is long enough to pass the 80 character span limit.
            let count = if case % 10 == 0 {
                20 + seeded.below(30)
            } else {
                seeded.below(12)
            };
            let text = (0..count)
                .map(|_| tokens[seeded.below(tokens.len())])
                .collect::<String>();
            let chars = text.chars().collect::<Vec<_>>();
            let term = if !chars.is_empty() && seeded.below(2) == 0 {
                let from = seeded.below(chars.len());
                let to = from + 1 + seeded.below(chars.len() - from);
                let piece = chars[from..to].iter().collect::<String>();
                match seeded.below(3) {
                    0 => piece.to_uppercase(),
                    1 => piece.to_lowercase(),
                    _ => piece,
                }
            } else {
                (0..1 + seeded.below(3))
                    .map(|_| tokens[seeded.below(tokens.len())])
                    .collect::<String>()
            };
            if agree(&text, &term) {
                named += 1;
            }
        }
        // Both answers occur often enough to test: about one input in seven is named.
        assert!(
            named > cases / 20 && named < cases / 2,
            "{named} of {cases} named"
        );
    }

    #[test]
    fn cut_characters_split_the_fold_key() {
        use unicode_normalization::char::decompose_canonical;
        for c in (0..=0x7f_u8).map(char::from) {
            assert_eq!(splits_fold(c), cut_lead(c), "{c:?}");
        }
        let mut later = std::collections::HashSet::new();
        for composite in (0..=0x10_ffff).filter_map(char::from_u32) {
            let mut parts = Vec::new();
            decompose_canonical(composite, |part| parts.push(part));
            later.extend(parts.iter().skip(1).copied());
            if parts.len() > 1 && cut_lead(parts[0]) {
                assert!(
                    !composite.is_alphabetic(),
                    "{composite:?} is a letter composed from the cut {:?}",
                    parts[0]
                );
            }
        }
        for c in later {
            assert!(
                !cut_lead(c),
                "{c:?} can join the character before it, so it cannot cut"
            );
        }
        let befores = [
            "", "e", "alph", "TNF", "<", "e\u{301}", "\u{3a3}", "\u{130}",
        ];
        let afters = [
            "",
            "\u{338}",
            "\u{301}a",
            "alpha",
            "\u{323}\u{301}",
            "\u{3c2}",
            "x",
        ];
        let cuts =
            " -.<1\u{ff0c}\u{3002}\u{2014}\u{a0}\u{387}\u{37e}\u{2329}\u{1f600}\u{663}\u{200d}";
        for cut in cuts.chars() {
            assert!(splits_fold(cut), "{cut:?}");
            for before in befores {
                for after in afters {
                    assert_eq!(
                        store::fold_key(&format!("{before}{cut}{after}")),
                        store::fold_key(before) + &store::fold_key(&format!("{cut}{after}")),
                        "{before:?} {cut:?} {after:?}"
                    );
                }
            }
        }
        for joins in "aE\u{301}\u{338}\u{1161}\u{3b1}\u{e9}".chars() {
            assert!(!splits_fold(joins), "{joins:?}");
        }
    }

    /// Measured on this message: 26,829 folds reading 120,722 characters, where the
    /// reference folds 410,829 spans reading 16,674,864. Words such as `HIF-2alpha`
    /// and `(n=12)` hold several pieces, so the count is about two folds per word.
    #[test]
    fn a_long_message_without_the_term_costs_a_few_folds_per_word() {
        let sentence = "Patients with HIF-2alpha variants (n=12) showed raised EPAS2 levels; \
            Cafe\u{301} \u{4e2d}\u{6587}\u{ff0c}\u{6d4b}\u{8bd5}\u{3002} ";
        let message = sentence.chars().cycle().take(100_000).collect::<String>();
        FOLDS.with(|folds| folds.set((0, 0)));
        assert!(!DictionaryClient::named_at_boundary(&message, "EPAS1"));
        let (calls, chars) = FOLDS.with(|folds| folds.get());
        let words = message.split_whitespace().count();
        let (spans, span_chars) = reference_work(&message);
        assert!(calls <= 3 * words, "{calls} folds for {words} words");
        assert!(chars <= 2 * 100_000, "{chars} characters folded");
        assert!(calls * 10 <= spans, "{calls} folds against {spans} spans");
        assert!(
            chars * 100 <= span_chars,
            "{chars} characters folded against {span_chars}"
        );
    }

    /// A message of plain words between spaces costs one fold per word start and one
    /// for the term.
    #[test]
    fn plain_prose_costs_one_fold_per_word() {
        let message = "the quick brown fox jumps over a lazy dog "
            .chars()
            .cycle()
            .take(100_000)
            .collect::<String>();
        FOLDS.with(|folds| folds.set((0, 0)));
        assert!(!DictionaryClient::named_at_boundary(&message, "EPAS1"));
        let (calls, _) = FOLDS.with(|folds| folds.get());
        let words = message.split_whitespace().count();
        assert!(calls <= words + 1, "{calls} folds for {words} words");
    }

    #[test]
    fn a_requested_term_needs_word_boundaries() {
        assert!(DictionaryClient::named_at_boundary(
            "Please add EPAS1 and HIF2A",
            "EPAS1"
        ));
        assert!(!DictionaryClient::named_at_boundary(
            "Please add XEPAS1Y",
            "EPAS1"
        ));
    }

    #[test]
    fn latest_request_and_answered_offer_are_the_only_on_request_sources() {
        let requested = vec![Message::user().with_text("Please add EPAS1")];
        assert!(DictionaryClient::person_named_term_in_messages(
            &requested, "EPAS1"
        ));
        let offer = vec![
            Message::user().with_text("Tell me about HIF2A"),
            Message::assistant().with_text("I can add EPAS1."),
            Message::user().with_text("yes"),
        ];
        assert!(DictionaryClient::person_named_term_in_messages(
            &offer, "EPAS1"
        ));
        let declined = vec![
            Message::assistant().with_text("I can add EPAS1."),
            Message::user().with_text("no, thanks"),
        ];
        assert!(DictionaryClient::person_named_term_in_messages(
            &declined, "EPAS1"
        ));
        let earlier_only = vec![
            Message::user().with_text("Tell me about EPAS1"),
            Message::assistant().with_text("Here is a summary."),
            Message::user().with_text("Please add a different term."),
        ];
        assert!(!DictionaryClient::person_named_term_in_messages(
            &earlier_only,
            "EPAS1"
        ));
    }

    /// A user message another agent wrote into this chat (`workspace_open` with a
    /// prompt, `workspace_send_prompt`) or a subagent's spawn context is not the
    /// person's request, so it never activates a term under `on_request`, even in a
    /// chat where the person turned the Dictionary on for every chat. The same text
    /// from the person does.
    #[test]
    fn an_agents_prompt_in_the_chat_is_not_the_persons_request() {
        use crate::conversation::message::MessageProvenance;
        let from = |kind| MessageProvenance {
            kind,
            from_session_id: Some("caller".to_string()),
            from_session_name: Some("Caller".to_string()),
        };
        for kind in [ProvenanceKind::AgentInjection, ProvenanceKind::SpawnContext] {
            let injected = vec![Message::user()
                .with_text("Please add EPAS1")
                .with_provenance(from(kind))];
            assert!(
                !DictionaryClient::person_named_term_in_messages(&injected, "EPAS1"),
                "{kind:?}"
            );
            let answered_offer = vec![
                Message::assistant().with_text("I can add EPAS1."),
                Message::user().with_text("yes").with_provenance(from(kind)),
            ];
            assert!(
                !DictionaryClient::person_named_term_in_messages(&answered_offer, "EPAS1"),
                "{kind:?} answering an offer"
            );
        }
        let typed = vec![Message::user().with_text("Please add EPAS1")];
        assert!(DictionaryClient::person_named_term_in_messages(
            &typed, "EPAS1"
        ));
    }

    /// A tool call earlier in the same turn leaves its result as the newest user
    /// message. The person's request, and the offer it answers, still decide.
    #[test]
    fn an_earlier_tool_call_in_the_turn_does_not_hide_the_persons_request() {
        let call = |id: &str| {
            Message::assistant().with_tool_request(
                id,
                Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    name: "dictionary__list_terms".into(),
                    arguments: Some(rmcp::object!({})),
                    meta: None,
                }),
            )
        };
        let result = |id: &str, text: &str| {
            Message::user().with_tool_response(
                id,
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text(text),
                ])),
            )
        };
        let requested = vec![
            Message::user().with_text("Please add EPAS1 to my dictionary"),
            call("a"),
            result("a", "[]"),
        ];
        assert!(DictionaryClient::person_named_term_in_messages(
            &requested, "EPAS1"
        ));
        let answered_offer = vec![
            Message::assistant().with_text("I can add EPAS1."),
            Message::user().with_text("yes"),
            call("b"),
            result("b", "[]"),
            call("c"),
            result("c", "[]"),
        ];
        assert!(DictionaryClient::person_named_term_in_messages(
            &answered_offer,
            "EPAS1"
        ));
        let only_in_a_tool_result = vec![
            Message::user().with_text("Please add a term"),
            call("d"),
            result("d", "EPAS1"),
        ];
        assert!(!DictionaryClient::person_named_term_in_messages(
            &only_in_a_tool_result,
            "EPAS1"
        ));
    }

    /// Rows only the model reads (hook context, post-edit diagnostics, loop warnings,
    /// nudges) are stored with the user role. They never stand in for the person's
    /// request, and an assistant message they follow is not an offer the person
    /// answered.
    #[test]
    fn a_row_only_the_model_reads_is_not_the_persons_request() {
        let model_only = |text: &str| Message::user().with_text(text).with_visibility(false, true);
        let after_a_tool_result = vec![
            Message::user().with_text("Please fix the build"),
            Message::assistant().with_text("Editing the file."),
            Message::user().with_tool_response(
                "a",
                Ok(rmcp::model::CallToolResult::success(vec![
                    rmcp::model::Content::text("done"),
                ])),
            ),
            model_only("Diagnostics mention EPAS1"),
        ];
        assert!(!DictionaryClient::person_named_term_in_messages(
            &after_a_tool_result,
            "EPAS1"
        ));
        let after_an_assistant_offer = vec![
            Message::user().with_text("Tell me about HIF2A"),
            Message::assistant().with_text("I can add EPAS1."),
            model_only("Keep going."),
        ];
        assert!(!DictionaryClient::person_named_term_in_messages(
            &after_an_assistant_offer,
            "EPAS1"
        ));
        let reply_after_a_hidden_row = vec![
            Message::assistant().with_text("I can add EPAS1."),
            model_only("Hook context."),
            Message::user().with_text("yes"),
        ];
        assert!(DictionaryClient::person_named_term_in_messages(
            &reply_after_a_hidden_row,
            "EPAS1"
        ));
    }

    #[test]
    fn the_add_schema_refuses_a_tier_argument() {
        let parsed = serde_json::from_value::<AddArgs>(json!({
            "terms": [{"written": "EPAS1"}], "tier": "private"
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn write_modes_and_session_cap_choose_status_before_the_store_caps() {
        use store::AgentWrites::{Add, OnRequest, Suggest};
        assert!(!DictionaryClient::should_activate(Suggest, true, 0));
        assert!(!DictionaryClient::should_activate(OnRequest, false, 0));
        assert!(DictionaryClient::should_activate(OnRequest, true, 0));
        assert!(DictionaryClient::should_activate(Add, false, 29));
        assert!(!DictionaryClient::should_activate(Add, true, 30));
    }

    #[tokio::test]
    async fn call_cap_and_alias_fields_are_rejected() {
        let eleven = (0..11)
            .map(|n| json!({"written":format!("Term{n}")}))
            .collect::<Vec<_>>();
        let data = tempfile::tempdir().unwrap();
        let client = DictionaryClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::new(crate::session::SessionManager::new(
                data.path().to_path_buf(),
            )),
        });
        let meta = McpMeta::new(
            "nonexistent",
            crate::privacy::CallCapability::for_test_restricted(),
        );
        assert!(client
            .add_terms(json!({"terms": eleven}), &meta)
            .await
            .unwrap_err()
            .to_string()
            .contains("one to ten"));
        assert!(serde_json::from_value::<AddTerm>(
            json!({"written":"EPAS1", "spoken_forms":["epas one"]})
        )
        .is_err());
        assert!(serde_json::from_value::<AddArgs>(
            json!({"terms":[{"written":"EPAS1"}],"requested_by_user":true,"tier":"private"})
        )
        .is_err());
    }

    #[test]
    fn a_url_in_a_reason_is_refused_before_any_store_write() {
        let terms = vec![AddTerm {
            written: "EPAS1".into(),
            kind: None,
            reason: Some("copied from https://example.test/source".into()),
        }];
        assert!(DictionaryClient::rejects_content(&terms));
    }

    #[test]
    fn add_result_keeps_row_identifiers_in_user_audience_only() {
        let mut file = store::StoreFile::default();
        let mut term = store::NewTerm::person("EPAS1", store::EntryTier::Public);
        term.source = store::EntrySource::Agent;
        let created = file
            .add(term, store::Reach::All, store::schema::now())
            .unwrap();
        let id = match &created {
            store::AddOutcome::Created { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        let content = DictionaryClient::add_result(
            vec![(created, Some(file.entries[0].clone()))],
            crate::dictation::activation::Surface::Unspecified,
        );
        let serialized = content
            .iter()
            .map(|part| serde_json::to_value(part).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            serialized[0]["text"],
            "Added 1 terms to the dictionary. They are marked for the person's review."
        );
        assert!(!serialized[0].to_string().contains(id.as_str()));
        assert_eq!(serialized[1]["annotations"]["audience"], json!(["user"]));
        assert!(serialized[1].to_string().contains(id.as_str()));
    }

    #[test]
    fn cli_and_acp_get_a_review_hint_while_unspecified_hosts_get_only_metadata() {
        let outcomes = || {
            let mut file = store::StoreFile::default();
            let mut term = store::NewTerm::person("EPAS1", store::EntryTier::Public);
            term.source = store::EntrySource::Agent;
            term.status = store::EntryStatus::Suggested;
            let created = file
                .add(term, store::Reach::All, store::schema::now())
                .unwrap();
            vec![(created, Some(file.entries[0].clone()))]
        };
        for surface in [
            crate::dictation::activation::Surface::Cli,
            crate::dictation::activation::Surface::Acp,
        ] {
            let parts = DictionaryClient::add_result(outcomes(), surface);
            let text = serde_json::to_value(&parts[0]).unwrap()["text"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(text.contains("biorouter dictionary review"));
        }
        let parts = DictionaryClient::add_result(
            outcomes(),
            crate::dictation::activation::Surface::Unspecified,
        );
        let text = serde_json::to_value(&parts[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!text.contains("biorouter dictionary review"));
        assert_eq!(
            serde_json::to_value(&parts[1]).unwrap()["annotations"]["audience"],
            json!(["user"])
        );
    }

    #[test]
    fn listed_rows_include_the_tier_without_a_withheld_count() {
        let mut file = store::StoreFile::default();
        let result = file
            .add(
                store::NewTerm::person("EPAS1", store::EntryTier::Public),
                store::Reach::All,
                store::schema::now(),
            )
            .unwrap();
        assert!(matches!(result, store::AddOutcome::Created { .. }));
        let row = DictionaryClient::listed_row(&file.entries[0]);
        assert_eq!(row["tier"], "public");
        assert!(row.get("withheld").is_none());
    }
    #[test]
    fn duplicate_result_reports_unchanged_row_and_never_claims_an_add() {
        let mut file = store::StoreFile::default();
        let created = file
            .add(
                store::NewTerm::person("EPAS1", store::EntryTier::Public),
                store::Reach::All,
                store::schema::now(),
            )
            .unwrap();
        let id = match created {
            store::AddOutcome::Created { id, .. } => id,
            _ => unreachable!(),
        };
        let parts = DictionaryClient::add_result(
            vec![(
                store::AddOutcome::Exists { id: id.clone() },
                Some(file.entries[0].clone()),
            )],
            crate::dictation::activation::Surface::Unspecified,
        );
        let message = serde_json::to_value(&parts[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(message, "1 term was already in Dictionary.");
        assert!(!message.contains("Added"));
        let user_part = serde_json::to_value(&parts[1]).unwrap();
        let rows = user_part["text"].as_str().unwrap();
        assert!(rows.contains(id.as_str()));
        assert!(rows.contains("\"source\":\"user\""));
    }

    #[test]
    fn advertised_add_schema_has_the_store_input_bounds() {
        let tool = DictionaryClient::tools()
            .into_iter()
            .find(|tool| tool.name.as_ref() == "add_terms")
            .unwrap();
        let schema = serde_json::to_value(tool.input_schema).unwrap();
        let item = &schema["properties"]["terms"]["items"]["properties"];
        assert_eq!(item["written"]["minLength"], 1);
        assert_eq!(item["written"]["maxLength"], 64);
        assert_eq!(item["reason"]["maxLength"], 140);
    }

    #[tokio::test]
    async fn client_stamps_private_affiliation_filters_duplicates_and_obeys_session_cap() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = sessions
            .create_session(
                root.path().to_path_buf(),
                "dictionary test".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        let ucsf = crate::privacy::affiliation::InstitutionId::new("ucsf");
        let stanford = crate::privacy::affiliation::InstitutionId::new("stanford");
        sessions
            .record_session_affiliation(&session.id, ucsf)
            .await
            .unwrap();
        let client = DictionaryClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::clone(&sessions),
        });
        let private_local = McpMeta::new(
            &session.id,
            crate::privacy::CallCapability::for_test(ProviderTier::Private, true),
        );
        client
            .add_terms(json!({"terms":[{"written":"EPAS1"}]}), &private_local)
            .await
            .unwrap();
        let view = open_store().view().unwrap();
        let row = view
            .entries()
            .iter()
            .find(|entry| entry.written == "EPAS1")
            .unwrap();
        assert_eq!(row.tier, store::EntryTier::Private);
        assert_eq!(
            row.affiliation.as_ref().unwrap().institutions,
            vec!["ucsf".to_string()]
        );
        assert_eq!(row.status, store::EntryStatus::Suggested);

        let public = McpMeta::new(
            &session.id,
            crate::privacy::CallCapability::for_test_restricted(),
        );
        assert_eq!(
            client.list_terms(json!({}), &public).await.unwrap()["terms"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        let private_stanford = McpMeta::new(
            &session.id,
            crate::privacy::CallCapability::for_test_affiliated(
                ProviderTier::Private,
                true,
                Some(crate::privacy::ModelAffiliation::institution(stanford)),
            ),
        );
        let listed = client
            .list_terms(json!({}), &private_stanford)
            .await
            .unwrap();
        assert!(listed["terms"].as_array().unwrap().is_empty());
        assert!(listed.get("total").is_none());

        let restricted = client
            .add_terms(json!({"terms":[{"written":"EPAS1"}]}), &private_stanford)
            .await
            .unwrap();
        let public_text = serde_json::to_value(&restricted[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        let user_rows = serde_json::to_value(&restricted[1]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(public_text, "Some terms could not be recorded.");
        assert_eq!(user_rows, "{\"dictionary_rows\":[]}");
        assert!(!public_text.contains("already"));
        assert!(!user_rows.contains(row.id.as_str()));

        let cap_session = sessions
            .create_session(
                root.path().to_path_buf(),
                "cap session".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        let cap_meta = McpMeta::new(
            &cap_session.id,
            crate::privacy::CallCapability::for_test_restricted(),
        );

        open_store()
            .write(|file| {
                file.settings.agent_writes = store::AgentWrites::Add;
                Ok(())
            })
            .unwrap();
        for batch in 0..3 {
            let terms = (batch * 10..batch * 10 + 10)
                .map(|n| json!({"written":format!("SessionTerm{n}")}))
                .collect::<Vec<_>>();
            client
                .add_terms(json!({"terms":terms}), &cap_meta)
                .await
                .unwrap();
        }
        client
            .add_terms(json!({"terms":[{"written":"SessionTerm30"}]}), &cap_meta)
            .await
            .unwrap();
        let view = open_store().view().unwrap();
        assert_eq!(
            view.entries()
                .iter()
                .filter(|entry| entry.source == store::EntrySource::Agent
                    && entry.status == store::EntryStatus::Active
                    && entry.suggested_in.as_deref() == Some(cap_session.id.as_str()))
                .count(),
            30
        );
        assert_eq!(
            view.entries()
                .iter()
                .find(|entry| entry.written == "SessionTerm29")
                .unwrap()
                .status,
            store::EntryStatus::Active
        );
        assert_eq!(
            view.entries()
                .iter()
                .find(|entry| entry.written == "SessionTerm30")
                .unwrap()
                .status,
            store::EntryStatus::Suggested
        );
        drop(view);
    }

    #[tokio::test]
    async fn a_fresh_session_at_the_tier_cap_downgrades_only_the_201st_agent_row() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = sessions
            .create_session(
                root.path().to_path_buf(),
                "tier cap".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        let client = DictionaryClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: sessions,
        });
        open_store()
            .write(|file| {
                file.settings.agent_writes = store::AgentWrites::Add;
                for n in 0..200 {
                    let mut term =
                        store::NewTerm::person(format!("PublicTerm{n}"), store::EntryTier::Public);
                    term.source = store::EntrySource::Agent;
                    file.add(term, store::Reach::All, store::schema::now())?;
                }
                Ok(())
            })
            .unwrap();
        let view = open_store().view().unwrap();
        assert_eq!(
            view.entries()
                .iter()
                .filter(|entry| entry.tier == store::EntryTier::Public
                    && entry.status == store::EntryStatus::Active
                    && entry.source == store::EntrySource::Agent)
                .count(),
            200
        );
        assert_eq!(
            view.entries()
                .iter()
                .find(|entry| entry.written == "PublicTerm199")
                .unwrap()
                .status,
            store::EntryStatus::Active
        );
        drop(view);
        let meta = McpMeta::new(
            &session.id,
            crate::privacy::CallCapability::for_test_restricted(),
        );
        client
            .add_terms(json!({"terms":[{"written":"PublicTerm200"}]}), &meta)
            .await
            .unwrap();
        let view = open_store().view().unwrap();
        assert_eq!(
            view.entries()
                .iter()
                .find(|entry| entry.written == "PublicTerm200")
                .unwrap()
                .status,
            store::EntryStatus::Suggested
        );
        assert_eq!(
            view.entries()
                .iter()
                .filter(|entry| entry.tier == store::EntryTier::Public
                    && entry.status == store::EntryStatus::Active
                    && entry.source == store::EntrySource::Agent)
                .count(),
            200
        );
    }

    /// A person's chat in `sessions` whose stored conversation is `messages`, in order.
    async fn stored_chat(
        sessions: &crate::session::SessionManager,
        root: &std::path::Path,
        messages: &[Message],
    ) -> String {
        let session = sessions
            .create_session(root.to_path_buf(), "stored chat".into(), SessionType::User)
            .await
            .unwrap();
        for message in messages {
            sessions.add_message(&session.id, message).await.unwrap();
        }
        session.id
    }

    fn client_over(sessions: &Arc<crate::session::SessionManager>) -> DictionaryClient {
        DictionaryClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::clone(sessions),
        })
    }

    fn row_named(written: &str) -> Option<store::Entry> {
        open_store()
            .view()
            .unwrap()
            .entries()
            .iter()
            .find(|entry| entry.written == written)
            .cloned()
    }

    fn public_meta(session_id: &str) -> McpMeta {
        McpMeta::new(
            session_id,
            crate::privacy::CallCapability::for_test_restricted(),
        )
    }

    fn requested(term: &str) -> Value {
        json!({"terms": [{"written": term}], "requested_by_user": true})
    }

    /// The rows the tool row reads, from the part only the person sees.
    fn user_rows(content: &[Content]) -> Vec<Value> {
        let part = serde_json::to_value(&content[1]).unwrap();
        assert_eq!(part["annotations"]["audience"], json!(["user"]));
        let rows: Value = serde_json::from_str(part["text"].as_str().unwrap()).unwrap();
        rows["dictionary_rows"].as_array().unwrap().clone()
    }

    fn model_text(content: &[Content]) -> String {
        serde_json::to_value(&content[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn created_id(outcome: store::AddOutcome) -> store::EntryId {
        match outcome {
            store::AddOutcome::Created { id, .. } => id,
            other => panic!("expected a new row, got {other:?}"),
        }
    }

    /// The stamp follows the caller's capability: a public chat writes a public row with
    /// no institutions, even from a chat whose record holds one.
    #[tokio::test]
    async fn a_public_callers_term_is_stamped_public_with_no_institutions() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = stored_chat(&sessions, root.path(), &[]).await;
        sessions
            .record_session_affiliation(
                &session,
                crate::privacy::affiliation::InstitutionId::new("ucsf"),
            )
            .await
            .unwrap();
        let client = client_over(&sessions);

        client
            .add_terms(
                json!({"terms":[{"written":"EPAS1"}]}),
                &public_meta(&session),
            )
            .await
            .unwrap();

        let row = row_named("EPAS1").expect("the public caller's term was recorded");
        assert_eq!(row.tier, store::EntryTier::Public);
        assert_eq!(
            row.affiliation, None,
            "a public row carries no institutions, whatever the chat's record holds"
        );
        assert_eq!(row.source, store::EntrySource::Agent);
        assert_eq!(row.suggested_in.as_deref(), Some(session.as_str()));
        let listed = client
            .list_terms(json!({}), &public_meta(&session))
            .await
            .unwrap();
        assert_eq!(listed["terms"][0]["written"], "EPAS1");
        assert_eq!(listed["terms"][0]["tier"], "public");
    }

    /// O-35 through the tool: a private chat whose `session_affiliations` record cannot be
    /// read stamps its row `{"institutions": []}`, never `null`, while a readable empty
    /// record on the same local model stamps `null`.
    #[tokio::test]
    async fn a_chat_whose_affiliation_record_cannot_be_read_stamps_an_empty_institution_set() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let unreadable = stored_chat(&sessions, root.path(), &[]).await;
        let readable = stored_chat(&sessions, root.path(), &[]).await;
        let pool = sessions.storage().pool().await.unwrap();
        sqlx::query("UPDATE sessions SET session_affiliations = ?1 WHERE id = ?2")
            .bind("not-json")
            .bind(&unreadable)
            .execute(pool)
            .await
            .unwrap();
        assert!(
            sessions.session_affiliations(&unreadable).await.is_err(),
            "precondition: this chat's record cannot be read"
        );
        assert!(sessions
            .session_affiliations(&readable)
            .await
            .unwrap()
            .is_empty());
        let client = client_over(&sessions);
        let local = crate::privacy::CallCapability::for_test(ProviderTier::Private, true);

        client
            .add_terms(
                json!({"terms":[{"written":"EPAS1"}]}),
                &McpMeta::new(&unreadable, local),
            )
            .await
            .unwrap();
        client
            .add_terms(
                json!({"terms":[{"written":"HIF2A"}]}),
                &McpMeta::new(&readable, local),
            )
            .await
            .unwrap();

        let unknown = row_named("EPAS1").expect("the term from the unreadable chat");
        assert_eq!(unknown.tier, store::EntryTier::Private);
        assert_eq!(
            unknown.affiliation,
            Some(store::RowAffiliation {
                institutions: Vec::new()
            })
        );
        let unclaimed = row_named("HIF2A").expect("the term from the readable chat");
        assert_eq!(unclaimed.tier, store::EntryTier::Private);
        assert_eq!(unclaimed.affiliation, None);
    }

    /// `on_request`, the new store's setting: the assistant offers EPAS1, the person
    /// answers "yes", and `add_terms` with `requested_by_user` lands EPAS1 active and
    /// unreviewed, which is the row the tool row shows **Keep** and **Undo** for.
    #[tokio::test]
    async fn a_yes_to_an_offer_naming_epas1_lands_it_active_and_unreviewed() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = stored_chat(
            &sessions,
            root.path(),
            &[
                Message::user().with_text("Which genes matter for living at high altitude?"),
                Message::assistant()
                    .with_text("EPAS1 is one of them. Should I add EPAS1 to your dictionary?"),
                Message::user().with_text("yes"),
            ],
        )
        .await;
        assert_eq!(
            open_store().view().unwrap().settings().agent_writes,
            store::AgentWrites::OnRequest,
            "a new store answers on request"
        );
        let client = client_over(&sessions);

        let content = client
            .add_terms(requested("EPAS1"), &public_meta(&session))
            .await
            .unwrap();

        let row = row_named("EPAS1").expect("EPAS1 was recorded");
        assert_eq!(row.status, store::EntryStatus::Active);
        assert!(!row.reviewed);
        assert_eq!(row.source, store::EntrySource::Agent);
        assert_eq!(
            model_text(&content),
            "Added 1 terms to the dictionary. They are marked for the person's review."
        );
        let rows = user_rows(&content);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], row.id.as_str());
        assert_eq!(rows[0]["source"], "agent");
        assert_eq!(rows[0]["status"], "active");
        assert_eq!(rows[0]["reviewed"], false);
    }

    /// O-34: the reply to the offer is not checked for agreement, so "no, thanks" to an
    /// offer naming EPAS1 lands it active and unreviewed too.
    #[tokio::test]
    async fn a_no_thanks_to_an_offer_naming_epas1_also_lands_it_active_and_unreviewed() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = stored_chat(
            &sessions,
            root.path(),
            &[
                Message::user().with_text("Which genes matter for living at high altitude?"),
                Message::assistant()
                    .with_text("EPAS1 is one of them. Should I add EPAS1 to your dictionary?"),
                Message::user().with_text("no, thanks"),
            ],
        )
        .await;
        let client = client_over(&sessions);

        let content = client
            .add_terms(requested("EPAS1"), &public_meta(&session))
            .await
            .unwrap();

        let row = row_named("EPAS1").expect("EPAS1 was recorded");
        assert_eq!(row.status, store::EntryStatus::Active);
        assert!(!row.reviewed);
        let rows = user_rows(&content);
        assert_eq!(rows[0]["status"], "active");
        assert_eq!(rows[0]["reviewed"], false);
    }

    /// `on_request` activates only a term in the person's latest message or in the offer
    /// that message answers. A term named only earlier in the conversation, or named
    /// nowhere, lands suggested even with `requested_by_user`; and without the flag an
    /// answered offer lands suggested too.
    #[tokio::test]
    async fn a_term_named_only_earlier_or_nowhere_lands_suggested_even_when_requested() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let earlier = stored_chat(
            &sessions,
            root.path(),
            &[
                Message::user().with_text("Tell me about EPAS1"),
                Message::assistant().with_text("Here is a summary of that gene."),
                Message::user().with_text("Please add that gene to my dictionary."),
            ],
        )
        .await;
        let nowhere = stored_chat(
            &sessions,
            root.path(),
            &[
                Message::assistant().with_text("Should I add HIF2A to your dictionary?"),
                Message::user().with_text("yes"),
            ],
        )
        .await;
        let unflagged = stored_chat(
            &sessions,
            root.path(),
            &[
                Message::assistant().with_text("Should I add EGLN1 to your dictionary?"),
                Message::user().with_text("yes"),
            ],
        )
        .await;
        let client = client_over(&sessions);

        let content = client
            .add_terms(requested("EPAS1"), &public_meta(&earlier))
            .await
            .unwrap();
        assert_eq!(
            row_named("EPAS1").unwrap().status,
            store::EntryStatus::Suggested,
            "a term found only earlier in the conversation was activated"
        );
        assert_eq!(user_rows(&content)[0]["status"], "suggested");
        assert!(model_text(&content).starts_with("Recorded 1 suggestions"));

        client
            .add_terms(requested("VHL"), &public_meta(&nowhere))
            .await
            .unwrap();
        assert_eq!(
            row_named("VHL").unwrap().status,
            store::EntryStatus::Suggested,
            "a term in neither the latest message nor the offer was activated"
        );

        client
            .add_terms(
                json!({"terms":[{"written":"EGLN1"}]}),
                &public_meta(&unflagged),
            )
            .await
            .unwrap();
        assert_eq!(
            row_named("EGLN1").unwrap().status,
            store::EntryStatus::Suggested,
            "an answered offer without requested_by_user was activated"
        );
    }

    /// ACP stdio always suggests: with the process surface set the way `biorouter acp`
    /// sets it, a term the person agreed to lands suggested in `on_request`, and so does
    /// any term in `add`.
    #[tokio::test]
    async fn acp_stdio_records_a_requested_term_as_a_suggestion() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        crate::dictation::activation::set_process_surface(
            crate::dictation::activation::Surface::Acp,
        );
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = stored_chat(
            &sessions,
            root.path(),
            &[
                Message::assistant().with_text("Should I add EPAS1 to your dictionary?"),
                Message::user().with_text("yes, please add EPAS1 and HIF2A"),
            ],
        )
        .await;
        let client = client_over(&sessions);
        assert_eq!(
            open_store().view().unwrap().settings().agent_writes,
            store::AgentWrites::OnRequest
        );

        let content = client
            .add_terms(requested("EPAS1"), &public_meta(&session))
            .await
            .unwrap();
        let row = row_named("EPAS1").expect("EPAS1 was recorded");
        assert_eq!(row.status, store::EntryStatus::Suggested);
        let text = model_text(&content);
        assert!(text.starts_with("Recorded 1 suggestions"), "{text}");
        assert!(text.contains("biorouter dictionary review"), "{text}");

        open_store()
            .write(|file| {
                file.settings.agent_writes = store::AgentWrites::Add;
                Ok(())
            })
            .unwrap();
        client
            .add_terms(requested("HIF2A"), &public_meta(&session))
            .await
            .unwrap();
        assert_eq!(
            row_named("HIF2A").unwrap().status,
            store::EntryStatus::Suggested,
            "ACP stdio activated a term in add mode"
        );
    }

    /// `remove_added_terms` cannot touch a person's row: named alone or beside the agent's
    /// own suggestion, the call is refused and the store is unchanged; the suggestion
    /// alone is removed.
    #[tokio::test]
    async fn remove_added_terms_cannot_remove_a_persons_row() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = stored_chat(&sessions, root.path(), &[]).await;
        let client = client_over(&sessions);
        let (person_id, agent_id) = open_store()
            .write(|file| {
                let now = store::schema::now();
                let person = file.add(
                    store::NewTerm::person("EPAS1", store::EntryTier::Public),
                    store::Reach::All,
                    now,
                )?;
                let mut suggestion = store::NewTerm::person("HIF2A", store::EntryTier::Public);
                suggestion.source = store::EntrySource::Agent;
                suggestion.status = store::EntryStatus::Suggested;
                let suggestion = file.add(suggestion, store::Reach::All, now)?;
                Ok((created_id(person), created_id(suggestion)))
            })
            .unwrap();
        let persons_row = row_named("EPAS1").unwrap();
        assert_eq!(persons_row.source, store::EntrySource::User);
        let entries_before = open_store().view().unwrap().entries().to_vec();
        let meta = public_meta(&session);

        let alone = client
            .remove_added_terms(json!({"ids":[person_id.as_str()]}), &meta)
            .await
            .unwrap_err();
        assert_eq!(alone.to_string(), store::MSG_NOT_FOUND);
        let beside = client
            .remove_added_terms(
                json!({"ids":[agent_id.as_str(), person_id.as_str()]}),
                &meta,
            )
            .await
            .unwrap_err();
        assert_eq!(beside.to_string(), store::MSG_NOT_FOUND);
        let through_the_tool = client
            .call_tool(
                "remove_added_terms",
                Some(
                    json!({"ids":[person_id.as_str()]})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                meta.clone(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(through_the_tool.is_error, Some(true));
        let private = McpMeta::new(
            &session,
            crate::privacy::CallCapability::for_test(ProviderTier::Private, true),
        );
        assert!(client
            .remove_added_terms(json!({"ids":[person_id.as_str()]}), &private)
            .await
            .is_err());
        assert_eq!(
            open_store().view().unwrap().entries().to_vec(),
            entries_before,
            "a refused removal changed the store"
        );

        let removed = client
            .remove_added_terms(json!({"ids":[agent_id.as_str()]}), &meta)
            .await
            .unwrap();
        assert_eq!(removed, json!({"removed": 1}));
        assert_eq!(row_named("HIF2A"), None);
        assert_eq!(row_named("EPAS1"), Some(persons_row));
    }

    /// The instructions are fixed text: no term reaches them, even once the store holds
    /// terms of both tiers and the client has added one.
    #[tokio::test]
    async fn the_instructions_hold_no_term_text_after_terms_exist() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
        let sessions = Arc::new(crate::session::SessionManager::new(
            root.path().join("sessions"),
        ));
        let session = stored_chat(&sessions, root.path(), &[]).await;
        open_store()
            .write(|file| {
                let now = store::schema::now();
                file.add(
                    store::NewTerm::person("Tachyzoite", store::EntryTier::Public),
                    store::Reach::All,
                    now,
                )?;
                file.add(
                    store::NewTerm::person("Xylophagy", store::EntryTier::Private),
                    store::Reach::All,
                    now,
                )?;
                Ok(())
            })
            .unwrap();
        let client = client_over(&sessions);
        client
            .add_terms(
                json!({"terms":[{"written":"EPAS1"}]}),
                &public_meta(&session),
            )
            .await
            .unwrap();
        assert_eq!(open_store().view().unwrap().entries().len(), 3);

        let instructions = client
            .get_info()
            .and_then(|info| info.instructions.clone())
            .expect("the capability carries instructions");
        for term in ["Tachyzoite", "Xylophagy", "EPAS1"] {
            assert!(
                !instructions.contains(term),
                "the instructions name {term}: {instructions}"
            );
        }
        assert_eq!(instructions, INSTRUCTIONS);
    }
}
