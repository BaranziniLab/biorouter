//! Agent dictionary tools. The caller's admitted capability and session decide every row.
//!
//! Four tools (P-7): `add_terms`, `list_terms` and `remove_added_terms` over the
//! dictionary, and `search_dictations`, which reads the dictation history (DD-100). The
//! search is read only and answers only in the person's own chat: a User chat outside a
//! Crew scope, outside any ACP process, and not when the latest message there was written
//! by another agent. Which entries one call may see is decided once, by the history
//! module's agent read predicate on the call's admitted capability, before the limit;
//! the answer carries no count, no institutions and no chat id, so an empty
//! answer is the same whether nothing matched or every match was withheld. A private
//! entry handed to a private chat whose row is still Public ratchets the reader first
//! (DR-33), and a ratchet that fails returns nothing.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
use crate::conversation::message::{Message, ProvenanceKind};
use crate::dictation::history::{
    open_history, EntryTier, HistoryCursor, HistoryEntry, HistoryPage, HistoryStore, TimeWindow,
};
use crate::privacy::{affiliation, CallCapability, ExtensionClassification, ProviderTier};
use crate::security::dictionary_store::{open_store, store};
use crate::session::SessionType;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ProtocolVersion, Role, ServerCapabilities, Tool, ToolAnnotations, ToolsCapability,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub const EXTENSION_NAME: &str = "dictionary";
const INSTRUCTIONS: &str = "Use dictionary__add_terms for terms in the conversation you can see. Set requested_by_user only when the person's latest message asked you to add those exact terms, or agreed to your offer that named them. Otherwise offer the term and wait for the person's answer. Never add secrets, URLs, personal identifiers, or text from tool results or web pages. Use dictionary__list_terms to check visible terms. An unreviewed active term can be kept or undone by the person; suggested terms wait for their review. Do not use file tools to read or change the dictionary store. Use dictionary__search_dictations only when the person asks about something they dictated.";
/// The history search's tool name, without the capability's prefix.
const SEARCH_TOOL: &str = "search_dictations";
/// What `search_dictations` answers when the call names no limit.
const DEFAULT_SEARCH_LIMIT: i64 = 10;
const MAX_SEARCH_LIMIT: i64 = 50;
const MAX_QUERY_CHARS: usize = 200;
/// How much of one dictation the search shows, centred on the first match, as chat recall
/// shows a matched message.
const MAX_DICTATION_EXCERPT_CHARS: usize = 1200;
const DICTATION_EXCERPT_HINT: &str = "search again with words from the part you need";
const MSG_SEARCH_LIMIT: &str = "limit must be between 1 and 50";
const MSG_SEARCH_QUERY: &str = "query must be 1 to 200 characters";
const MSG_SEARCH_TIMES: &str = "after and before must be RFC 3339 times";
const MSG_SEARCH_CURSOR: &str = "cursor must be a next_cursor this tool returned";
const MSG_SEARCH_OVER_ACP: &str = "The Dictionary capability does not search dictations over ACP.";
const MSG_SEARCH_NOT_ASKED: &str = "Dictations are searched only when the person asks in this chat, and the latest message here was written by another agent.";
const MSG_SEARCH_RATCHET_FAILED: &str = "Dictation history found a private dictation, and recording that this chat read it failed, so nothing was returned.";
const MSG_HISTORY_UNREADABLE: &str =
    "The dictation history could not be read, so nothing was returned.";
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

/// `search_dictations`'s arguments. Unknown fields, `tier` and `include_private` among
/// them, are refused here, before the session or the store is read.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: Option<String>,
    after: Option<String>,
    before: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

/// A checked [`SearchArgs`].
#[derive(Debug)]
struct SearchRequest {
    query: Option<String>,
    window: TimeWindow,
    limit: usize,
    cursor: Option<HistoryCursor>,
}

impl SearchRequest {
    fn from_args(args: SearchArgs) -> Result<Self> {
        let limit = args.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
            return Err(anyhow!(MSG_SEARCH_LIMIT));
        }
        if args
            .query
            .as_deref()
            .is_some_and(|query| query.is_empty() || query.chars().count() > MAX_QUERY_CHARS)
        {
            return Err(anyhow!(MSG_SEARCH_QUERY));
        }
        let instant = |raw: Option<String>| {
            raw.map(|text| {
                DateTime::parse_from_rfc3339(&text)
                    .map(|at| at.with_timezone(&Utc))
                    .map_err(|_| anyhow!(MSG_SEARCH_TIMES))
            })
            .transpose()
        };
        let after = instant(args.after)?;
        // Both bounds are inclusive. Entries are stamped in whole seconds and the store's
        // upper bound is exclusive, so the bound handed down is the next whole second.
        let before = instant(args.before)?.and_then(|at| {
            at.with_nanosecond(0)
                .unwrap_or(at)
                .checked_add_signed(chrono::Duration::seconds(1))
        });
        let cursor = args
            .cursor
            .map(|token| {
                HistoryCursor::from_token(&token).ok_or_else(|| anyhow!(MSG_SEARCH_CURSOR))
            })
            .transpose()?;
        Ok(Self {
            query: args.query,
            window: TimeWindow { after, before },
            limit: usize::try_from(limit).map_err(|_| anyhow!(MSG_SEARCH_LIMIT))?,
            cursor,
        })
    }
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
            Tool::new(SEARCH_TOOL, "Find what the person said with Biorouter dictation, newest first. Use only when the person asks you to find or recall something they dictated. Returns only the dictations this chat may see, each clipped to an excerpt, without a total count.", schema(json!({
                "type":"object", "properties":{
                    "query":{"type":"string","minLength":1,"maxLength":200,"description":"Words the dictation holds. Case and accents do not matter. Omit it to list the newest dictations."},
                    "after":{"type":"string","description":"Only dictations at or after this RFC 3339 instant."},
                    "before":{"type":"string","description":"Only dictations at or before this RFC 3339 instant."},
                    "limit":{"type":"integer","minimum":1,"maximum":50,"description":"At most this many dictations, 10 when omitted."},
                    "cursor":{"type":"string","description":"The next_cursor of an earlier answer, for the dictations after it."}
                },"additionalProperties":false
            }))).annotate(ToolAnnotations {
                title: Some("Search dictations".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
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

    /// The person's latest message and its position. Tool results are stored with the user
    /// role, and so are rows only the model reads (hook context, post-edit diagnostics,
    /// loop warnings, nudges, compaction summaries), which can echo text from files or the
    /// web. The person's latest message is the newest user message the person can see that
    /// carries no tool result.
    fn latest_person_message(messages: &[Message]) -> Option<(usize, &Message)> {
        messages.iter().enumerate().rev().find(|(_, message)| {
            message.role == Role::User && message.is_user_visible() && !message.is_tool_response()
        })
    }

    /// Whether another agent wrote `message` into this chat (`workspace_open` with a
    /// prompt, `workspace_send_prompt`) or it is a subagent's spawn context, so it is not
    /// the person's request.
    fn written_by_an_agent(message: &Message) -> bool {
        message.metadata.provenance.as_ref().is_some_and(|stamp| {
            matches!(
                stamp.kind,
                ProvenanceKind::AgentInjection | ProvenanceKind::SpawnContext
            )
        })
    }

    /// Whether the chat's latest message was put there by another agent, which never
    /// starts a history search (the person asks, an agent never does).
    fn an_agent_wrote_the_latest_message(messages: &[Message]) -> bool {
        Self::latest_person_message(messages)
            .is_some_and(|(_, latest)| Self::written_by_an_agent(latest))
    }

    fn person_named_term_in_messages(messages: &[Message], term: &str) -> bool {
        // The offer the person's latest message answers is the newest message before it
        // that the person can see.
        let Some((index, latest)) = Self::latest_person_message(messages) else {
            return false;
        };
        if Self::written_by_an_agent(latest) {
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

    async fn search_dictations(&self, args: Value, meta: &McpMeta) -> Result<Value> {
        self.search_dictations_in(&open_history(), args, meta).await
    }

    /// `search_dictations` over `history`. Arguments are checked first, then the session,
    /// and only then is the store read. One visibility decision, on the admitted
    /// capability, serves the cursor and the page, and it runs before the limit, so
    /// entries this call may not see never use up its results.
    async fn search_dictations_in(
        &self,
        history: &HistoryStore,
        args: Value,
        meta: &McpMeta,
    ) -> Result<Value> {
        let request = SearchRequest::from_args(serde_json::from_value(args)?)?;
        let session = self.allowed_session(&meta.session_id).await?;
        // ACP stdio keeps the capability for suggestions, and a client such as the Jupyter
        // AI persona delivers other people's messages as user messages.
        if crate::dictation::activation::process_surface()
            == crate::dictation::activation::Surface::Acp
        {
            return Err(anyhow!(MSG_SEARCH_OVER_ACP));
        }
        if session.conversation.as_ref().is_some_and(|conversation| {
            Self::an_agent_wrote_the_latest_message(conversation.messages())
        }) {
            return Err(anyhow!(MSG_SEARCH_NOT_ASKED));
        }
        let cap = meta.capability;
        let visible = move |entry: &HistoryEntry| {
            crate::dictation::history::agent_reads_dictation(entry, cap)
        };
        let cursor = request
            .cursor
            .map(|cursor| Self::resume_point(history, cursor, &visible));
        let page = history
            .search(
                request.query.as_deref(),
                request.window,
                cursor.as_ref(),
                request.limit,
                &visible,
            )
            .map_err(|_| anyhow!(MSG_HISTORY_UNREADABLE))?;
        if let Some(private) = page
            .entries
            .iter()
            .find(|entry| entry.tier == EntryTier::Private)
        {
            self.ratchet_reader(&meta.session_id, cap, &private.id)
                .await?;
        }
        Ok(Self::search_answer(&page, request.query.as_deref()))
    }

    /// Where a page after `cursor` starts. A cursor naming an entry this call may not see
    /// is read as one naming no entry, so a made up cursor tells the caller nothing about
    /// entries it cannot see.
    fn resume_point(
        history: &HistoryStore,
        cursor: HistoryCursor,
        visible: &dyn Fn(&HistoryEntry) -> bool,
    ) -> HistoryCursor {
        match history.get(&cursor.id) {
            Ok(Some(entry)) if visible(&entry) => cursor,
            _ => HistoryCursor {
                at: cursor.at,
                id: crate::dictation::history::store::new_entry_id(),
            },
        }
    }

    /// The reader ratchet (DR-33): a private dictation is about to reach this chat, so the
    /// chat is private from now on, recorded before anything is returned. Only an enforced,
    /// private capability can see a private entry, so only such a call is asked, and a
    /// reader whose row is already private changes nothing. `Err` withholds the whole
    /// answer, because a read this process cannot record is one it must not perform.
    async fn ratchet_reader(
        &self,
        reader: &str,
        cap: CallCapability,
        entry_id: &str,
    ) -> Result<()> {
        if !cap.enforced() || !cap.tier().is_private() {
            return Ok(());
        }
        if self
            .sessions
            .get_session(reader, false)
            .await
            .is_ok_and(|row| row.privacy_tier.is_private())
        {
            return Ok(());
        }
        crate::agents::extension_manager::raise_reader_privacy(
            &self.sessions,
            reader,
            &format!("dictation:{entry_id}"),
        )
        .await
        .map_err(|_| anyhow!(MSG_SEARCH_RATCHET_FAILED))
    }

    /// The answer: each entry's id, time, surface word, tier and an excerpt, and the next
    /// page's cursor. No count, no institutions, no chat id.
    fn search_answer(page: &HistoryPage, query: Option<&str>) -> Value {
        let dictations = page
            .entries
            .iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "at": entry.created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
                    "where": entry.surface.as_str(),
                    "tier": entry.tier.as_str(),
                    "text": crate::agents::chatrecall_extension::excerpt_around(
                        &entry.text,
                        query.unwrap_or_default(),
                        MAX_DICTATION_EXCERPT_CHARS,
                        DICTATION_EXCERPT_HINT,
                    ),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "dictations": dictations,
            "next_cursor": page.next_cursor.as_ref().map(HistoryCursor::to_token),
        })
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
            SEARCH_TOOL => self
                .search_dictations(args, &meta)
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

    /// `dictionary__search_dictations` (DD-100): the dictation history, searched by a
    /// person's own chat under the call's admitted capability.
    mod dictation_search {
        use super::*;
        use crate::conversation::message::MessageProvenance;
        use crate::dictation::history::{store as history_store, Surface};
        use crate::privacy::{CallCapability, ModelAffiliation, SessionClassification};
        use crate::session::SessionManager;

        const EMPTY_ANSWER: &str = r#"{"dictations":[],"next_cursor":null}"#;

        fn at(seconds: i64) -> DateTime<Utc> {
            DateTime::from_timestamp(1_795_707_552 + seconds, 0).unwrap()
        }

        fn dictation(seconds: i64, text: &str, tier: EntryTier) -> HistoryEntry {
            HistoryEntry {
                id: history_store::new_entry_id(),
                created_at: at(seconds),
                text: text.to_string(),
                tier,
                institutions: None,
                surface: Surface::Composer,
                duration_ms: 1_000,
                model: "whisper-small.en-q8_0".to_string(),
                private_terms: false,
                session_id: None,
                session_incarnation: None,
                truncated: None,
            }
        }

        fn public_cap() -> CallCapability {
            CallCapability::for_test(ProviderTier::Public, true)
        }

        fn private_cap() -> CallCapability {
            CallCapability::for_test(ProviderTier::Private, true)
        }

        fn affiliated(name: &str) -> CallCapability {
            CallCapability::for_test_affiliated(
                ProviderTier::Private,
                true,
                Some(ModelAffiliation::institution(
                    crate::privacy::affiliation::InstitutionId::new(name),
                )),
            )
        }

        /// One isolated session store, one isolated history, the extension under test and
        /// a person's chat to search from. Nothing here resolves `Paths`, so these tests
        /// share the test process.
        struct Searcher {
            _temp: tempfile::TempDir,
            sessions: Arc<SessionManager>,
            history: HistoryStore,
            client: DictionaryClient,
            chat: String,
        }

        impl Searcher {
            async fn new() -> Self {
                let temp = tempfile::TempDir::new().unwrap();
                let sessions = Arc::new(SessionManager::new(temp.path().join("sessions")));
                let history = HistoryStore::open(temp.path().join("history"));
                let client = client_over(&sessions);
                let chat = sessions
                    .create_session(
                        temp.path().to_path_buf(),
                        "person".into(),
                        SessionType::User,
                    )
                    .await
                    .unwrap()
                    .id;
                Self {
                    _temp: temp,
                    sessions,
                    history,
                    client,
                    chat,
                }
            }

            fn seed(&self, entry: HistoryEntry) -> String {
                let id = entry.id.clone();
                self.history.append(entry).unwrap();
                id
            }

            async fn search(&self, cap: CallCapability, args: Value) -> Result<Value> {
                self.search_from(&self.chat, cap, args).await
            }

            async fn search_from(
                &self,
                chat: &str,
                cap: CallCapability,
                args: Value,
            ) -> Result<Value> {
                self.client
                    .search_dictations_in(&self.history, args, &McpMeta::new(chat, cap))
                    .await
            }

            async fn chat_of(&self, kind: SessionType, messages: &[Message]) -> String {
                let session = self
                    .sessions
                    .create_session(self._temp.path().to_path_buf(), format!("{kind}"), kind)
                    .await
                    .unwrap();
                for message in messages {
                    self.sessions
                        .add_message(&session.id, message)
                        .await
                        .unwrap();
                }
                session.id
            }

            async fn row(&self) -> crate::session::Session {
                self.sessions.get_session(&self.chat, false).await.unwrap()
            }
        }

        fn rows(answer: &Value) -> Vec<Value> {
            answer["dictations"].as_array().unwrap().clone()
        }

        fn texts(answer: &Value) -> Vec<String> {
            rows(answer)
                .iter()
                .map(|row| row["text"].as_str().unwrap().to_string())
                .collect()
        }

        /// The filter runs before the limit: sixty private dictations newer than ten
        /// public ones never use up a public chat's results, and paging walks the public
        /// ones newest first with nothing private in any page.
        #[tokio::test]
        async fn a_public_chat_gets_its_full_quota_when_private_dictations_are_newer() {
            let s = Searcher::new().await;
            for i in 0..10 {
                s.seed(dictation(
                    i,
                    &format!("EPAS1 public note {i}"),
                    EntryTier::Public,
                ));
            }
            for i in 0..60 {
                s.seed(dictation(
                    100 + i,
                    &format!("EPAS1 PRIVATESENTINEL {i}"),
                    EntryTier::Private,
                ));
            }

            let answer = s
                .search(public_cap(), json!({"query": "epas1", "limit": 10}))
                .await
                .unwrap();
            let found = rows(&answer);
            assert_eq!(found.len(), 10, "{answer}");
            assert!(found.iter().all(|row| row["tier"] == "public"), "{answer}");
            assert_eq!(found[0]["text"], "EPAS1 public note 9", "newest first");
            assert_eq!(answer["next_cursor"], Value::Null, "{answer}");
            assert!(!answer.to_string().contains("PRIVATESENTINEL"));
            assert!(answer.get("total").is_none(), "no count is returned");

            let mut seen = Vec::new();
            let mut cursor = Value::Null;
            for expected in [4, 4, 2] {
                let mut args = json!({"query": "EPAS1", "limit": 4});
                if let Some(token) = cursor.as_str() {
                    args["cursor"] = json!(token);
                }
                let page = s.search(public_cap(), args).await.unwrap();
                assert_eq!(rows(&page).len(), expected, "{page}");
                assert!(!page.to_string().contains("PRIVATESENTINEL"), "{page}");
                seen.extend(texts(&page));
                cursor = page["next_cursor"].clone();
            }
            assert_eq!(cursor, Value::Null, "the last page has no cursor");
            let expected: Vec<String> = (0..10)
                .rev()
                .map(|i| format!("EPAS1 public note {i}"))
                .collect();
            assert_eq!(seen, expected);
        }

        /// P-6: an empty answer is the same, byte for byte, whether nothing matched or
        /// every match was withheld, and whether the history exists at all.
        #[tokio::test]
        async fn an_empty_answer_says_nothing_about_withheld_dictations() {
            let withheld = Searcher::new().await;
            withheld.seed(dictation(0, "EPAS1 PRIVATESENTINEL", EntryTier::Private));
            withheld.seed(dictation(1, "HIF2A in the clinic", EntryTier::Private));
            let unmatched = Searcher::new().await;
            unmatched.seed(dictation(0, "HIF2A in the clinic", EntryTier::Public));
            let empty = Searcher::new().await;

            let mut answers = Vec::new();
            for searcher in [&withheld, &unmatched, &empty] {
                let answer = searcher
                    .search(public_cap(), json!({"query": "EPAS1"}))
                    .await
                    .unwrap();
                answers.push(answer.to_string());
            }
            for answer in &answers {
                assert_eq!(answer, EMPTY_ANSWER);
            }
            // Without a query too: withheld entries leave no trace in an empty page.
            let listed = withheld.search(public_cap(), json!({})).await.unwrap();
            assert_eq!(listed.to_string(), EMPTY_ANSWER);
            assert!(
                !withheld.row().await.privacy_tier.is_private(),
                "nothing was returned, so nothing ratcheted"
            );
        }

        /// A private chat sees both tiers, each private dictation also passing its
        /// institutions under P-16: one from Stanford's model only reaches a model every
        /// listed institution admits, here Stanford's or one on this computer.
        #[tokio::test]
        async fn a_private_chat_sees_both_tiers() {
            let s = Searcher::new().await;
            s.seed(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public));
            s.seed(dictation(1, "EPAS1 PRIVATESENTINEL", EntryTier::Private));
            let mut stanford_only = dictation(2, "EPAS1 STANFORDSENTINEL", EntryTier::Private);
            stanford_only.institutions = Some(vec!["stanford".to_string()]);
            s.seed(stanford_only);

            let both = s
                .search(private_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap()
                .to_string();
            for sentinel in ["PUBLICSENTINEL", "PRIVATESENTINEL", "STANFORDSENTINEL"] {
                assert!(both.contains(sentinel), "{sentinel}: {both}");
            }
            let ucsf = s
                .search(affiliated("ucsf"), json!({"query": "EPAS1"}))
                .await
                .unwrap()
                .to_string();
            assert!(ucsf.contains("PUBLICSENTINEL") && ucsf.contains("PRIVATESENTINEL"));
            assert!(
                !ucsf.contains("STANFORDSENTINEL"),
                "a Stanford dictation reached a UCSF model: {ucsf}"
            );
            let stanford = s
                .search(affiliated("stanford"), json!({"query": "EPAS1"}))
                .await
                .unwrap()
                .to_string();
            assert!(stanford.contains("STANFORDSENTINEL"), "{stanford}");
            let public = s
                .search(public_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap()
                .to_string();
            assert!(public.contains("PUBLICSENTINEL"));
            assert!(!public.contains("PRIVATESENTINEL") && !public.contains("STANFORDSENTINEL"));
        }

        /// DR-33: a private dictation handed to a private chat whose row is still Public
        /// ratchets the row first, with the dictation's id as the reason.
        #[tokio::test]
        async fn a_private_dictation_ratchets_a_public_reader_before_it_is_returned() {
            let s = Searcher::new().await;
            // Appended in time order, as the daemon records them.
            s.seed(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public));
            s.seed(dictation(1, "EPAS1 older private", EntryTier::Private));
            let first_private = s.seed(dictation(2, "EPAS1 PRIVATESENTINEL", EntryTier::Private));
            assert!(!s.row().await.privacy_tier.is_private(), "precondition");

            let answer = s
                .search(private_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap();
            assert!(answer.to_string().contains("PRIVATESENTINEL"));
            let row = s.row().await;
            assert_eq!(row.privacy_tier, SessionClassification::Private);
            assert_eq!(
                row.privacy_reason.as_deref(),
                Some(format!("dictation:{first_private}").as_str()),
                "the reason names the first private dictation returned"
            );
        }

        /// The reader keeps its tier when no private dictation is returned: a private
        /// capability whose page holds only public ones, a public capability, and any
        /// capability sampled with the master switch off (AR-7), which sees every entry.
        #[tokio::test]
        async fn the_reader_keeps_its_tier_when_no_private_dictation_reaches_a_private_capability()
        {
            for (case, cap, query, sees_private) in [
                (
                    "private, public results only",
                    private_cap(),
                    "PUBLICSENTINEL",
                    false,
                ),
                ("public", public_cap(), "EPAS1", false),
                (
                    "public, switch off",
                    CallCapability::for_test(ProviderTier::Public, false),
                    "EPAS1",
                    true,
                ),
                (
                    "private, switch off",
                    CallCapability::for_test(ProviderTier::Private, false),
                    "EPAS1",
                    true,
                ),
            ] {
                let s = Searcher::new().await;
                s.seed(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public));
                s.seed(dictation(1, "EPAS1 PRIVATESENTINEL", EntryTier::Private));
                let answer = s.search(cap, json!({"query": query})).await.unwrap();
                assert!(answer.to_string().contains("PUBLICSENTINEL"), "{case}");
                assert_eq!(
                    answer.to_string().contains("PRIVATESENTINEL"),
                    sees_private,
                    "{case}: {answer}"
                );
                let row = s.row().await;
                assert_eq!(row.privacy_tier, SessionClassification::Public, "{case}");
                assert_eq!(row.privacy_reason, None, "{case}");
            }
        }

        /// A read this process cannot record is one it must not perform: when the ratchet's
        /// write fails, the answer is the refusal and no dictation text.
        #[tokio::test]
        async fn a_failed_ratchet_returns_nothing() {
            let s = Searcher::new().await;
            s.seed(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public));
            s.seed(dictation(1, "EPAS1 PRIVATESENTINEL", EntryTier::Private));
            let pool = s.sessions.storage().pool().await.unwrap();
            sqlx::query(&format!(
                "CREATE TRIGGER dictation_ratchet_fault BEFORE UPDATE ON sessions \
                 WHEN NEW.id = '{}' BEGIN SELECT RAISE(ABORT, 'injected fault'); END",
                s.chat
            ))
            .execute(pool)
            .await
            .unwrap();

            let refused = s
                .search(private_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap_err()
                .to_string();
            assert_eq!(refused, MSG_SEARCH_RATCHET_FAILED);
            assert!(!refused.contains("SENTINEL"));
            assert!(!s.row().await.privacy_tier.is_private());
            // The control: the same store answers a call that needs no ratchet, so the
            // refusal above came from the failed write.
            let public = s
                .search(public_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap()
                .to_string();
            assert!(public.contains("PUBLICSENTINEL") && !public.contains("PRIVATESENTINEL"));
        }

        /// The schema refuses `tier`, `include_private` and every unknown field, and the
        /// bounds are checked, before the session or the store is read: a fresh handle on
        /// a history full of entries has parsed nothing after every refusal.
        #[tokio::test]
        async fn the_schema_refuses_a_tier_or_any_unknown_field_before_the_store_is_read() {
            let s = Searcher::new().await;
            s.seed(dictation(0, "EPAS1 PRIVATESENTINEL", EntryTier::Private));
            let fresh = HistoryStore::open(s.history.dir());
            let ask = |args: Value, chat: &str| {
                let meta = McpMeta::new(chat, private_cap());
                let client = &s.client;
                let fresh = &fresh;
                async move { client.search_dictations_in(fresh, args, &meta).await }
            };
            for args in [
                json!({"tier": "private"}),
                json!({"include_private": true}),
                json!({"query": "EPAS1", "tier": "public"}),
                json!({"query": "EPAS1", "session_id": "anything"}),
            ] {
                for chat in [s.chat.as_str(), "no-such-chat"] {
                    let refused = ask(args.clone(), chat).await.unwrap_err().to_string();
                    assert!(refused.contains("unknown field"), "{args}: {refused}");
                }
            }
            for (args, sentence) in [
                (json!({"limit": 0}), MSG_SEARCH_LIMIT),
                (json!({"limit": 51}), MSG_SEARCH_LIMIT),
                (json!({"limit": -1}), MSG_SEARCH_LIMIT),
                (json!({"query": ""}), MSG_SEARCH_QUERY),
                (json!({"query": "x".repeat(201)}), MSG_SEARCH_QUERY),
                (json!({"after": "yesterday"}), MSG_SEARCH_TIMES),
                (json!({"before": "2026-11-26"}), MSG_SEARCH_TIMES),
                (json!({"cursor": "not a cursor"}), MSG_SEARCH_CURSOR),
            ] {
                let refused = ask(args.clone(), "no-such-chat")
                    .await
                    .unwrap_err()
                    .to_string();
                assert_eq!(refused, sentence, "{args}");
            }
            assert_eq!(fresh.parses(), 0, "a refused call read the history");
            // The instrument can fail: an accepted call parses the file once.
            ask(json!({"query": "EPAS1", "limit": 50}), &s.chat)
                .await
                .unwrap();
            assert_eq!(fresh.parses(), 1);
        }

        /// The advertised schema matches what the tool accepts.
        #[test]
        fn the_advertised_search_schema_is_closed_and_bounded() {
            let tool = DictionaryClient::tools()
                .into_iter()
                .find(|tool| tool.name.as_ref() == SEARCH_TOOL)
                .unwrap();
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            assert_eq!(schema["additionalProperties"], false);
            let mut properties = schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            properties.sort();
            assert_eq!(properties, ["after", "before", "cursor", "limit", "query"]);
            assert_eq!(schema["properties"]["limit"]["minimum"], 1);
            assert_eq!(schema["properties"]["limit"]["maximum"], 50);
            assert_eq!(schema["properties"]["query"]["maxLength"], 200);
            let annotations = tool.annotations.expect("the search is annotated");
            assert_eq!(annotations.read_only_hint, Some(true));
            assert_eq!(annotations.destructive_hint, Some(false));
            assert!(INSTRUCTIONS.contains(
                "Use dictionary__search_dictations only when the person asks about something they dictated."
            ));
        }

        /// The session checks: a person's own chat only. SubAgent, Scheduled, Hidden and
        /// Terminal sessions and a Crew scoped chat are refused in the tool itself, which
        /// is what holds on the doors that skip the agent loop.
        #[tokio::test]
        async fn the_search_answers_only_in_a_persons_own_chat() {
            let s = Searcher::new().await;
            s.seed(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public));
            for kind in [
                SessionType::SubAgent,
                SessionType::Scheduled,
                SessionType::Hidden,
                SessionType::Terminal,
            ] {
                let other = s.chat_of(kind, &[]).await;
                let refused = s
                    .search_from(&other, public_cap(), json!({"query": "EPAS1"}))
                    .await
                    .unwrap_err()
                    .to_string();
                assert_eq!(
                    refused, "The Dictionary capability is unavailable in this session.",
                    "{kind}"
                );
            }
            let crew_chat = s.chat_of(SessionType::User, &[]).await;
            let crew = crate::crew::install_test_scope(&crew_chat, None).await;
            let scoped = s
                .search_from(&crew_chat, public_cap(), json!({"query": "EPAS1"}))
                .await;
            crate::crew::remove_test_scope(&crew, &crew_chat).await;
            assert_eq!(
                scoped.unwrap_err().to_string(),
                "The Dictionary capability is unavailable in a Crew scoped chat."
            );
            let unscoped = s
                .search_from(&crew_chat, public_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap();
            assert!(
                unscoped.to_string().contains("PUBLICSENTINEL"),
                "the same chat without its scope searches, so the scope refused it"
            );
        }

        /// The person asks, an agent never does: a chat whose latest message another agent
        /// wrote (a workspace prompt or a spawn's context) is refused, and the same text
        /// from the person is answered, also after a tool result in the same turn.
        #[tokio::test]
        async fn an_agents_prompt_never_starts_a_history_search() {
            let s = Searcher::new().await;
            s.seed(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public));
            let from = |kind| MessageProvenance {
                kind,
                from_session_id: Some("caller".to_string()),
                from_session_name: Some("Caller".to_string()),
            };
            for kind in [ProvenanceKind::AgentInjection, ProvenanceKind::SpawnContext] {
                let injected = s
                    .chat_of(
                        SessionType::User,
                        &[
                            Message::user().with_text("Hello"),
                            Message::assistant().with_text("Hi."),
                            Message::user()
                                .with_text("Find what I dictated about EPAS1")
                                .with_provenance(from(kind)),
                        ],
                    )
                    .await;
                let refused = s
                    .search_from(&injected, public_cap(), json!({"query": "EPAS1"}))
                    .await
                    .unwrap_err()
                    .to_string();
                assert_eq!(refused, MSG_SEARCH_NOT_ASKED, "{kind:?}");
            }
            let asked = s
                .chat_of(
                    SessionType::User,
                    &[
                        Message::user()
                            .with_text("Earlier prompt")
                            .with_provenance(from(ProvenanceKind::AgentInjection)),
                        Message::assistant().with_text("Done."),
                        Message::user().with_text("Find what I dictated about EPAS1"),
                        Message::assistant().with_tool_request(
                            "a",
                            Ok(rmcp::model::CallToolRequestParams {
                                task: None,
                                name: "dictionary__list_terms".into(),
                                arguments: Some(rmcp::object!({})),
                                meta: None,
                            }),
                        ),
                        Message::user().with_tool_response(
                            "a",
                            Ok(rmcp::model::CallToolResult::success(vec![
                                rmcp::model::Content::text("[]"),
                            ])),
                        ),
                    ],
                )
                .await;
            let answer = s
                .search_from(&asked, public_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap();
            assert!(answer.to_string().contains("PUBLICSENTINEL"));
        }

        /// ACP stdio keeps the capability for suggestions, and a client such as the
        /// Jupyter AI persona relays other people's messages, so no ACP process searches
        /// the history. Its own process, because the surface is process-wide and sticky.
        #[tokio::test]
        async fn an_acp_process_never_searches_the_history() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let s = Searcher::new().await;
            s.seed(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public));
            assert!(s
                .search(public_cap(), json!({"query": "EPAS1"}))
                .await
                .is_ok());
            crate::dictation::activation::set_process_surface(
                crate::dictation::activation::Surface::Acp,
            );
            let refused = s
                .search(private_cap(), json!({"query": "EPAS1"}))
                .await
                .unwrap_err()
                .to_string();
            assert_eq!(refused, MSG_SEARCH_OVER_ACP);
        }

        /// `after` and `before` are inclusive instants.
        #[tokio::test]
        async fn the_time_window_is_inclusive_at_both_ends() {
            let s = Searcher::new().await;
            for seconds in [0, 10, 20] {
                s.seed(dictation(
                    seconds,
                    &format!("note at {seconds}"),
                    EntryTier::Public,
                ));
            }
            let stamp = |seconds: i64| at(seconds).to_rfc3339_opts(SecondsFormat::Secs, true);
            let window = |after: Option<String>, before: Option<String>| {
                let mut args = json!({});
                if let Some(after) = after {
                    args["after"] = json!(after);
                }
                if let Some(before) = before {
                    args["before"] = json!(before);
                }
                args
            };
            let only_ten = s
                .search(public_cap(), window(Some(stamp(10)), Some(stamp(10))))
                .await
                .unwrap();
            assert_eq!(texts(&only_ten), ["note at 10"]);
            let fraction = |seconds: i64| {
                (at(seconds) + chrono::Duration::milliseconds(500))
                    .to_rfc3339_opts(SecondsFormat::Millis, true)
            };
            let after_half = s
                .search(public_cap(), window(Some(fraction(10)), None))
                .await
                .unwrap();
            assert_eq!(texts(&after_half), ["note at 20"]);
            let before_half = s
                .search(public_cap(), window(None, Some(fraction(10))))
                .await
                .unwrap();
            assert_eq!(texts(&before_half), ["note at 10", "note at 0"]);
            assert_eq!(
                s.search(public_cap(), window(Some(stamp(20)), Some(stamp(0))))
                    .await
                    .unwrap()
                    .to_string(),
                EMPTY_ANSWER
            );
        }

        /// Each row is the id, time, surface word, tier and an excerpt: no chat id, no
        /// institutions, no model, no count.
        #[tokio::test]
        async fn a_row_names_where_and_when_and_nothing_else() {
            let s = Searcher::new().await;
            let mut entry = dictation(5, "EPAS1 typed elsewhere", EntryTier::Private);
            entry.surface = Surface::OtherApp;
            entry.session_id = Some("20261126_7".to_string());
            entry.session_incarnation = Some(3);
            entry.institutions = Some(vec!["ucsf".to_string()]);
            let id = s.seed(entry);
            let answer = s
                .search(affiliated("ucsf"), json!({"query": "EPAS1"}))
                .await
                .unwrap();
            let found = rows(&answer);
            assert_eq!(found.len(), 1);
            let mut keys = found[0]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            keys.sort();
            assert_eq!(keys, ["at", "id", "text", "tier", "where"]);
            assert_eq!(found[0]["id"], id.as_str());
            assert_eq!(found[0]["at"], stamp_of(5));
            assert_eq!(found[0]["where"], "other_app");
            assert_eq!(found[0]["tier"], "private");
            let text = answer.to_string();
            for absent in ["20261126_7", "ucsf", "whisper", "total"] {
                assert!(!text.contains(absent), "{absent}: {text}");
            }
        }

        fn stamp_of(seconds: i64) -> String {
            at(seconds).to_rfc3339_opts(SecondsFormat::Secs, true)
        }

        /// A long dictation is clipped to 1,200 characters around the first match, with a
        /// marker on each side that was cut; without a query it is clipped from the start.
        #[tokio::test]
        async fn a_long_dictation_is_clipped_around_the_first_match() {
            let s = Searcher::new().await;
            let text = format!("{} EPAS1 {}", "a".repeat(2_000), "b".repeat(2_000));
            s.seed(dictation(0, &text, EntryTier::Public));
            let matched = texts(
                &s.search(public_cap(), json!({"query": "epas1"}))
                    .await
                    .unwrap(),
            )
            .remove(0);
            assert!(matched.contains("EPAS1"), "the excerpt shows the match");
            assert!(matched.starts_with("…[earlier text not shown] "));
            assert!(matched.ends_with(&format!("… [truncated; {DICTATION_EXCERPT_HINT}]")));
            let shown = matched
                .trim_start_matches("…[earlier text not shown] ")
                .trim_end_matches(&format!("… [truncated; {DICTATION_EXCERPT_HINT}]"));
            assert_eq!(shown.chars().count(), MAX_DICTATION_EXCERPT_CHARS);
            let listed = texts(&s.search(public_cap(), json!({})).await.unwrap()).remove(0);
            assert!(listed.starts_with("aaaa") && !listed.contains("EPAS1"));
            assert!(listed.ends_with(&format!("… [truncated; {DICTATION_EXCERPT_HINT}]")));
        }

        /// A cursor naming a dictation the caller may not see reads as one naming none, so
        /// a made up cursor tells a public chat nothing; a private chat, which may see it,
        /// resumes right after it.
        #[tokio::test]
        async fn a_cursor_naming_a_withheld_dictation_reads_as_one_naming_none() {
            let s = Searcher::new().await;
            s.seed(dictation(9, "older public", EntryTier::Public));
            s.seed(dictation(10, "same second public", EntryTier::Public));
            let hidden = s.seed(dictation(10, "PRIVATESENTINEL", EntryTier::Private));
            let token = |id: &str| format!("{}:{id}", at(10).timestamp());
            let public_named = s
                .search(public_cap(), json!({"cursor": token(&hidden)}))
                .await
                .unwrap();
            let public_unknown = s
                .search(
                    public_cap(),
                    json!({"cursor": token(&history_store::new_entry_id())}),
                )
                .await
                .unwrap();
            assert_eq!(public_named, public_unknown);
            assert_eq!(texts(&public_named), ["older public"]);
            let private_named = s
                .search(private_cap(), json!({"cursor": token(&hidden)}))
                .await
                .unwrap();
            assert_eq!(
                texts(&private_named),
                ["same second public", "older public"]
            );
        }

        /// Managed policy denies the search in the agent loop under `enabled: false`,
        /// `dictionary: false` and `history: false`, and `history: false` leaves the other
        /// dictionary tools alone.
        #[tokio::test]
        async fn managed_policy_denies_the_search_in_the_loop() {
            use crate::managed::{ManagedPolicy, ManagedPolicyFile};
            use crate::permission::managed_inspector::{
                ManagedPolicyInspector, MANAGED_DENY_REASON,
            };
            use crate::tool_inspection::{InspectionAction, ToolInspector};
            let request = |name: &str| crate::conversation::message::ToolRequest {
                id: name.to_string(),
                tool_call: Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    name: name.to_string().into(),
                    arguments: Some(rmcp::object!({"query": "EPAS1"})),
                    meta: None,
                }),
                metadata: None,
                tool_meta: None,
            };
            let denied = |yaml: &str, name: &str| {
                let file: ManagedPolicyFile = serde_yaml::from_str(yaml).unwrap();
                let inspector =
                    ManagedPolicyInspector::new(Arc::new(ManagedPolicy::from_file(file)));
                let name = name.to_string();
                async move {
                    let results = inspector
                        .inspect(
                            &[request(&name)],
                            &[],
                            crate::config::BioRouterMode::Auto,
                            &crate::session::Session::default(),
                        )
                        .await
                        .unwrap();
                    results.iter().any(|result| {
                        result.action == InspectionAction::Deny
                            && result.reason == MANAGED_DENY_REASON
                    })
                }
            };
            let search = crate::dictation::history::SEARCH_DICTATIONS_TOOL;
            assert_eq!(search, format!("{EXTENSION_NAME}__{SEARCH_TOOL}"));
            for yaml in [
                "dictation:\n  enabled: false\n",
                "dictation:\n  dictionary: false\n",
                "dictation:\n  history: false\n",
            ] {
                assert!(denied(yaml, search).await, "{yaml:?}");
            }
            assert!(
                !denied("dictation:\n  history: false\n", "dictionary__list_terms").await,
                "history: false denies only the history search"
            );
            assert!(!denied("dictation:\n  capture: false\n", search).await);
        }

        /// The history's folder is refused to tools in the agent loop for both tiers: a
        /// shell `cat` of `history.jsonl` from a public chat and from a private one.
        #[tokio::test]
        async fn a_shell_read_of_the_history_file_is_refused_in_both_tiers() {
            use crate::agents::agent::{Agent, AgentConfig};
            use crate::tool_inspection::InspectionAction;
            let temp = tempfile::TempDir::new().unwrap();
            let sessions = Arc::new(SessionManager::new(temp.path().join("sessions")));
            let agent = Agent::with_config(AgentConfig::new(
                Arc::clone(&sessions),
                Arc::new(crate::config::permission::PermissionManager::new(
                    temp.path().to_path_buf(),
                )),
                None,
                crate::config::BioRouterMode::Auto,
            ));
            let file = crate::dictation::history::history_dir().join("history.jsonl");
            let request = crate::conversation::message::ToolRequest {
                id: "cat-history".to_string(),
                tool_call: Ok(rmcp::model::CallToolRequestParams {
                    task: None,
                    name: "developer__shell".into(),
                    arguments: Some(rmcp::object!({"command": format!("cat {}", file.display())})),
                    meta: None,
                }),
                metadata: None,
                tool_meta: None,
            };
            for (tier, cap) in [
                (SessionClassification::Public, public_cap()),
                (SessionClassification::Private, private_cap()),
            ] {
                let chat = sessions
                    .create_session(
                        temp.path().to_path_buf(),
                        "reader".into(),
                        SessionType::User,
                    )
                    .await
                    .unwrap();
                if tier == SessionClassification::Private {
                    sessions
                        .update(&chat.id)
                        .raise_privacy(SessionClassification::Private, "turn:test")
                        .apply()
                        .await
                        .unwrap();
                }
                let chat = sessions.get_session(&chat.id, false).await.unwrap();
                let results = agent
                    .tool_inspection_manager
                    .inspect_tools_with_capability(
                        std::slice::from_ref(&request),
                        &[],
                        crate::config::BioRouterMode::Auto,
                        &chat,
                        Some(cap),
                    )
                    .await
                    .unwrap();
                let refusal = results
                    .iter()
                    .find(|result| {
                        result.inspector_name
                            == crate::security::dictionary_store::DICTIONARY_STORE_INSPECTOR_NAME
                    })
                    .unwrap_or_else(|| panic!("{tier:?}: the history was read: {results:?}"));
                assert_eq!(refusal.action, InspectionAction::Deny, "{tier:?}");
                assert!(
                    refusal.reason.contains("dictation history"),
                    "{}",
                    refusal.reason
                );
            }
        }

        /// The tool reads only the capability it is handed: its production text never
        /// re-reads the master switch or samples the provider slot. The needles are
        /// assembled so this test's own text is not a match.
        #[test]
        fn the_search_reads_only_the_admitted_capability() {
            let source = include_str!("dictionary_extension.rs");
            let production = source
                .split_once(concat!("#[cfg(test)]\n", "mod tests {"))
                .map_or(source, |(before, _)| before);
            for needle in [
                concat!("privacy_tiers", "_enabled"),
                concat!("CallCapability::", "sample"),
                concat!("public_", "enforced("),
            ] {
                assert!(
                    !production.contains(needle),
                    "dictionary_extension.rs must not spell {needle}"
                );
            }
        }

        // ─────────── Through the real dispatch ───────────

        /// A provider whose only interesting property is its tier. No test here runs a
        /// turn.
        struct TierProvider(ProviderTier);

        #[async_trait]
        impl crate::providers::base::Provider for TierProvider {
            fn metadata() -> crate::providers::base::ProviderMetadata {
                crate::providers::base::ProviderMetadata::new(
                    "tier",
                    "Tier",
                    "tier test provider",
                    "m",
                    vec!["m"],
                    "",
                    vec![],
                )
            }
            fn get_name(&self) -> &str {
                "tier-test-provider"
            }
            fn tier(&self) -> ProviderTier {
                self.0
            }
            async fn complete_with_model(
                &self,
                _model_config: &crate::model::ModelConfig,
                _system: &str,
                _messages: &[Message],
                _tools: &[Tool],
            ) -> std::result::Result<
                (Message, crate::providers::base::ProviderUsage),
                crate::providers::errors::ProviderError,
            > {
                unreachable!("no test here runs a turn")
            }
            fn get_model_config(&self) -> crate::model::ModelConfig {
                crate::model::ModelConfig::new_or_fail("m")
            }
        }

        fn provider(tier: ProviderTier) -> Arc<dyn crate::providers::base::Provider> {
            Arc::new(TierProvider(tier))
        }

        /// A real agent with the Dictionary capability on a public model, a person's chat,
        /// and the process history (its own sandbox root) holding one dictation per tier.
        async fn dispatch_fixture(
            root: &std::path::Path,
        ) -> (Arc<crate::agents::agent::Agent>, crate::session::Session) {
            use crate::agents::agent::{Agent, AgentConfig};
            let sessions = Arc::new(SessionManager::new(root.join("sessions")));
            let agent = Arc::new(Agent::with_config(AgentConfig::new(
                Arc::clone(&sessions),
                Arc::new(crate::config::permission::PermissionManager::new(
                    root.to_path_buf(),
                )),
                None,
                crate::config::BioRouterMode::Auto,
            )));
            let target = crate::agents::extension_manager::resolve_bundled_extension("dictionary")
                .expect("Dictionary is a bundled capability");
            agent
                .add_extension(target.into_config("dictation search dispatch".into()))
                .await
                .unwrap();
            *agent.provider.lock().await = Some(provider(ProviderTier::Public));
            let chat = sessions
                .create_session(root.to_path_buf(), "person".into(), SessionType::User)
                .await
                .unwrap();
            let history = open_history();
            history
                .append(dictation(0, "EPAS1 PUBLICSENTINEL", EntryTier::Public))
                .unwrap();
            history
                .append(dictation(1, "EPAS1 PRIVATESENTINEL", EntryTier::Private))
                .unwrap();
            (agent, chat)
        }

        /// `Agent::dispatch_tool_call` samples the capability; the tool reads it.
        async fn dispatch_search(
            agent: &crate::agents::agent::Agent,
            chat: &crate::session::Session,
        ) -> std::result::Result<String, String> {
            let (_, dispatched) = agent
                .dispatch_tool_call(
                    rmcp::model::CallToolRequestParams {
                        meta: None,
                        name: crate::dictation::history::SEARCH_DICTATIONS_TOOL.into(),
                        arguments: Some(rmcp::object!({"query": "EPAS1"})),
                        task: None,
                    },
                    "req".to_string(),
                    None,
                    chat,
                )
                .await;
            let dispatched = dispatched.map_err(|error| error.message.to_string())?;
            let result = dispatched
                .result
                .await
                .map_err(|error| error.message.to_string())?;
            Ok(result
                .content
                .iter()
                .filter_map(|content| content.as_text().map(|text| text.text.clone()))
                .collect::<Vec<_>>()
                .join("\n"))
        }

        /// The capability is the one the call was admitted on: a call admitted Public,
        /// parked past admission while the chat moves to a private model, still finds only
        /// public dictations and ratchets nothing.
        #[tokio::test]
        async fn a_swap_after_admission_does_not_change_what_this_call_may_search() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let root = tempfile::TempDir::new().unwrap();
            let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
            let (agent, chat) = dispatch_fixture(root.path()).await;

            // Keyed on this chat and this tool, so only the dispatch below takes it.
            let held = crate::agents::agent::seams::hold_dispatch_queue(
                &chat.id,
                crate::dictation::history::SEARCH_DICTATIONS_TOOL,
            );
            let call = tokio::spawn({
                let agent = Arc::clone(&agent);
                let chat = chat.clone();
                async move { dispatch_search(&agent, &chat).await }
            });
            let release = tokio::time::timeout(std::time::Duration::from_secs(60), held)
                .await
                .expect("the search never reached the seam: the key or the hold point drifted")
                .unwrap();
            agent
                .update_provider(provider(ProviderTier::Private), &chat.id)
                .await
                .unwrap();
            release.send(()).unwrap();

            let text = call.await.unwrap().unwrap();
            assert!(text.contains("PUBLICSENTINEL"), "{text}");
            assert!(
                !text.contains("PRIVATESENTINEL"),
                "a call admitted as public found a private dictation: {text}"
            );
            let row = agent
                .config
                .session_manager
                .get_session(&chat.id, false)
                .await
                .unwrap();
            assert!(!row.privacy_tier.is_private(), "a public search ratcheted");
        }

        /// A turn that started Public keeps its calls Public after a switch to a private
        /// model (`no_wider_than`); once the turn's hold is gone, the same chat on the
        /// private model finds both tiers and is ratcheted.
        #[tokio::test]
        async fn a_turn_that_started_public_on_a_private_slot_searches_public_dictations_only() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let root = tempfile::TempDir::new().unwrap();
            let _root = crate::test_sandbox::relocate_path_root(root.path().to_str().unwrap());
            let (agent, chat) = dispatch_fixture(root.path()).await;

            let started_public = agent.call_capability(&chat.id).await;
            assert_eq!(started_public.tier(), ProviderTier::Public);
            let held = agent.turn_ceilings.open(&chat.id, started_public);
            agent
                .update_provider(provider(ProviderTier::Private), &chat.id)
                .await
                .unwrap();
            let text = dispatch_search(&agent, &chat).await.unwrap();
            assert!(text.contains("PUBLICSENTINEL"), "{text}");
            assert!(
                !text.contains("PRIVATESENTINEL"),
                "a turn that started public found a private dictation: {text}"
            );
            let row = agent
                .config
                .session_manager
                .get_session(&chat.id, false)
                .await
                .unwrap();
            assert!(!row.privacy_tier.is_private());

            drop(held);
            let text = dispatch_search(&agent, &chat).await.unwrap();
            assert!(
                text.contains("PUBLICSENTINEL") && text.contains("PRIVATESENTINEL"),
                "with no turn running, the private model finds both tiers: {text}"
            );
            let row = agent
                .config
                .session_manager
                .get_session(&chat.id, false)
                .await
                .unwrap();
            assert!(
                row.privacy_tier.is_private(),
                "the private read ratcheted the chat"
            );
        }
    }
}
