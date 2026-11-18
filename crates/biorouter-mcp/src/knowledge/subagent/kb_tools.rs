//! `KbToolDispatch` — maps tool-name strings to `KnowledgeService` operations.
//!
//! This is the concrete `ToolDispatch` implementation used by the sub-agent
//! macros.  It binds to a specific KB and an optional transaction branch so
//! every write is either committed immediately (no txn) or staged on the txn
//! branch (with txn).
//!
//! ## ⚠ This file is the SECOND tool surface, and no gate runs on it (DR-8)
//!
//! `KnowledgeServer::call_tool` is where a `kb_*` call meets the privacy
//! barrier: it resolves the named base (`gated_kb_id`), refuses one the caller
//! may not reach (`assert_kb_reachable`), and ratchets the base to the caller's
//! tier before a write. **Nothing below goes through it.** [`KbToolDispatch`]
//! dispatches `kb_write_page` / `kb_append_log` / `kb_add_raw_source` straight
//! to `store::*`, and [`tool_specs`] — not `KnowledgeServer::tool_router()` — is
//! the table the sub-agent's model actually sees. So
//! `every_tool_the_router_exposes_is_classified_by_the_probe_table` is
//! structurally blind to this file: it enumerates the router.
//!
//! This surface is safe **today**, and for exactly two reasons that hold
//! together and not separately:
//!
//! 1. The three macros (`ingest`, `query`, `lint`) call `assert_reachable` and
//!    `raise_tier_and_affiliation` at their own entry, before the sub-agent
//!    starts — so the base is cleared once, for the whole run.
//! 2. **[`KbToolDispatch`] never accepts a `kb_id`.** Every call it dispatches
//!    lands on `self.kb_id`, the base the macro already cleared. There is no
//!    argument by which the model can name a different one.
//!
//! Reason 2 is what makes reason 1 sufficient, which makes it load-bearing: a
//! new sub-agent tool taking a base id would read a private base from a public
//! session with **no gate anywhere on the path** — not the macro's (it cleared a
//! different base) and not `call_tool`'s (never reached). It is pinned by
//! `no_sub_agent_tool_takes_a_kb_id` and
//! `a_kb_id_in_the_arguments_does_not_move_the_dispatch`, and the fix if either
//! fails is to keep the surface kb_id-less, not to update the test.

use crate::knowledge::{
    biookf,
    convert::SourceInput,
    log as kb_log, manifest, okf, paths, raw,
    service::KnowledgeService,
    store,
    store::SearchScope,
    subagent::loop_::{ToolDispatch, VocabularyRejection},
    types::ChangeKind,
    types::KbFormat,
    validate,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rmcp::model::{JsonObject, Tool};
use serde_json::Value;
use std::{borrow::Cow, sync::Arc};

// ---------------------------------------------------------------------------
// KbToolDispatch
// ---------------------------------------------------------------------------

pub struct KbToolDispatch {
    pub svc: KnowledgeService,
    pub kb_id: String,
    /// The currently active transaction branch name, or an empty string when
    /// not inside a transaction (writes then commit directly to main).
    pub txn_branch: String,
    /// Whether this macro invocation may mutate the bound knowledge base.
    pub access: KbToolAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbToolAccess {
    ReadOnly,
    ReadWrite,
}

fn is_read_only_tool(name: &str) -> bool {
    matches!(
        name,
        "kb_list_pages"
            | "kb_read_page"
            | "kb_search"
            | "kb_classify_source"
            | VALIDATE_PAGE
            | "complete"
    )
}

#[async_trait]
impl ToolDispatch for KbToolDispatch {
    async fn call(&self, name: &str, args: Value) -> Result<String> {
        if self.access == KbToolAccess::ReadOnly && !is_read_only_tool(name) {
            anyhow::bail!("knowledge tool '{name}' is unavailable in read-only mode");
        }

        let kb_root = paths::kb_root(self.svc.root(), &self.kb_id);
        let txn_opt: Option<&str> = if self.txn_branch.is_empty() {
            None
        } else {
            Some(&self.txn_branch)
        };

        match name {
            // ------------------------------------------------------------------
            "kb_list_pages" => {
                let prefix = args
                    .get("path_prefix")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let pages = store::list_pages(&kb_root, prefix.as_deref())?;
                Ok(serde_json::to_string(&pages)?)
            }

            // ------------------------------------------------------------------
            "kb_read_page" => {
                let path = args["path"]
                    .as_str()
                    .ok_or_else(|| anyhow!("kb_read_page: missing 'path'"))?;
                let page = store::read_page(&kb_root, path)?;
                Ok(serde_json::to_string(&page)?)
            }

            // ------------------------------------------------------------------
            "kb_write_page" => {
                let path = args["path"]
                    .as_str()
                    .ok_or_else(|| anyhow!("kb_write_page: missing 'path'"))?;
                let content = args["content"]
                    .as_str()
                    .ok_or_else(|| anyhow!("kb_write_page: missing 'content'"))?;
                let msg = args
                    .get("commit_message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("subagent write");
                let sha = store::write_page(&kb_root, path, content, msg, txn_opt)?;
                Ok(serde_json::json!({ "commit_sha": sha }).to_string())
            }

            // ------------------------------------------------------------------
            "kb_search" => {
                let q = args["query"]
                    .as_str()
                    .ok_or_else(|| anyhow!("kb_search: missing 'query'"))?;
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
                let include_raw_sources = args
                    .get("include_raw_sources")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let scope = if include_raw_sources {
                    SearchScope::All
                } else {
                    SearchScope::Knowledge
                };
                let hits = store::search_with_scope(&kb_root, q, limit, scope)?;
                Ok(serde_json::to_string(&hits)?)
            }

            // ------------------------------------------------------------------
            "kb_append_log" => {
                let summary = args["summary"]
                    .as_str()
                    .ok_or_else(|| anyhow!("kb_append_log: missing 'summary'"))?;
                let kind_str = args
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("manual");
                let kind = parse_change_kind(kind_str)?;
                let delta = args.get("delta").and_then(|v| v.as_str());
                kb_log::append(&kb_root, kind, summary, delta, txn_opt)?;
                Ok(serde_json::json!({ "ok": true }).to_string())
            }

            // ------------------------------------------------------------------
            "kb_add_raw_source" => {
                let input = source_input_from_args(&args)?;
                let result = self.svc.add_raw_source(&self.kb_id, input, txn_opt).await?;
                Ok(serde_json::json!({
                    "source_id": result.source_id,
                    "source_md_path": result.source_md_path,
                })
                .to_string())
            }

            // ------------------------------------------------------------------
            "kb_classify_source" => {
                let source_id = args["source_id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("kb_classify_source: missing 'source_id'"))?;
                let meta = raw::read_meta(&kb_root, source_id)?;
                Ok(serde_json::to_string(&meta.credibility)?)
            }

            // ------------------------------------------------------------------
            // The typed writer. Declared only for a BioOKF base — but a tool
            // name arriving here is a string the model sent, and the base is the
            // one the macro bound, so the two facts are established in different
            // places and the dispatch checks rather than assumes. Imposing a
            // biomedical vocabulary on an OKF base, whose `type` is open and
            // whose type names are the user's own, would be a wrong answer
            // rather than a refused one.
            WRITE_CONCEPT => {
                let format = base_profile(&kb_root);
                if !format.is_some_and(KbFormat::is_biookf) {
                    anyhow::bail!(
                        "{WRITE_CONCEPT} is for a base in the BioOKF profile; this base is {}.                          Its `type` is open, so write it with kb_write_page",
                        format.map_or("from before the OKF format", |_| "plain OKF")
                    );
                }
                self.write_concept(&kb_root, &args, txn_opt)
            }

            // ------------------------------------------------------------------
            VALIDATE_PAGE => validate_draft(&kb_root, &args),

            // ------------------------------------------------------------------
            other => anyhow::bail!("KbToolDispatch: unknown tool '{other}'"),
        }
    }
}

/// The name of the typed BioOKF page writer, spelled once so the spec, the
/// dispatch and the procedures cannot drift apart.
pub const WRITE_CONCEPT: &str = "kb_write_concept";

/// The name of the sub-agent's pre-write validator; see [`WRITE_CONCEPT`].
pub const VALIDATE_PAGE: &str = "kb_validate_page";

/// Check a draft against the base's own profile and write nothing.
///
/// The sub-agent surface's copy of Stage 4's MCP tool, minus the `kb_id` — the
/// invariant this file exists to keep (DR-8). Only BioOKF pays to read the
/// bundle, because only BioOKF has cross-document rules; in OKF mode the page is
/// checked entirely against itself, and a legacy base is checked against nothing
/// and says so (DR-26).
fn validate_draft(kb_root: &std::path::Path, args: &Value) -> Result<String> {
    let content = args["content"]
        .as_str()
        .ok_or_else(|| anyhow!("{VALIDATE_PAGE}: missing 'content'"))?;
    let path = args.get("path").and_then(|v| v.as_str());
    let format = base_profile(kb_root);
    let pages = if format.is_some_and(KbFormat::is_biookf) {
        validate::load_bundle(kb_root)?
    } else {
        Vec::new()
    };
    let diagnostics = validate::validate_page(format, path, content, &pages);
    Ok(serde_json::json!({
        "path": path,
        "format": format.map(KbFormat::as_str),
        // DR-7: a verdict about *writing* it, which is the one action a
        // producer may be held to. Nothing here refuses a read of anything.
        "ok": diagnostics.errors() == 0,
        "errors": diagnostics.errors(),
        "warnings": diagnostics.count(validate::Severity::Warning),
        "diagnostics": diagnostics,
    })
    .to_string())
}

/// The base's profile, or `None` for a legacy base and for a manifest that will
/// not load.
///
/// Treating an unreadable manifest as legacy rather than guessing `Okf` is the
/// same call `macros::lint::format_diagnostics` makes, for the same reason: a
/// guess produces a flood of format findings about a base whose generation we
/// could not establish.
fn base_profile(kb_root: &std::path::Path) -> Option<KbFormat> {
    manifest::load(kb_root).ok().and_then(|m| m.profile())
}

impl KbToolDispatch {
    /// Compose and write one typed BioOKF page.
    ///
    /// Every controlled value is checked here, before a byte is written, and a
    /// failure comes back as a [`VocabularyRejection`] naming the closest legal
    /// value. That is DR-16's other half: the `enum` in the schema is what stops
    /// most invalid values being sampled at all, and this is what happens to the
    /// ones that are sampled anyway — the model gets a fix rather than a "no",
    /// and the loop can tell a run that died re-guessing from a run that ran out
    /// of work.
    fn write_concept(
        &self,
        kb_root: &std::path::Path,
        args: &Value,
        txn: Option<&str>,
    ) -> Result<String> {
        let type_str = args["type"]
            .as_str()
            .ok_or_else(|| anyhow!("{WRITE_CONCEPT}: missing 'type'"))?;
        let node_type = parse_node_type(type_str)?;
        let identifier = args["identifier"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "{WRITE_CONCEPT}: missing 'identifier'. It is this page's primary key — \
                     human-readable and unique in this bundle — and every edge that cites this \
                     page names it"
                )
            })?;

        // A sent `edges` is the page's whole edge list (rq2-027): an empty list
        // removes every edge, and only an omitted key keeps the ones already on
        // the page. Reading `[]` as "not supplied" kept an invalid edge that a
        // lint autofix then told the person it had removed.
        let edges = match args.get("edges") {
            None | Some(Value::Null) => None,
            Some(Value::Array(items)) => {
                Some(items.iter().map(edge_mapping).collect::<Result<Vec<_>>>()?)
            }
            Some(_) => anyhow::bail!(
                "{WRITE_CONCEPT}: 'edges' must be a list of edges. Send every edge the page \
                 should keep, an empty list to remove them all, or omit it to keep the edges \
                 already on the page"
            ),
        };

        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => default_concept_path(node_type, identifier),
        };
        let existing = existing(kb_root, &path);
        // Reported back so the model reads what the page now carries rather
        // than what it meant to send.
        let edge_count = match &edges {
            Some(edges) => edges.len(),
            None => existing
                .frontmatter
                .get("edges")
                .and_then(serde_yaml::Value::as_sequence)
                .map_or(0, Vec::len),
        };
        let content = compose_page(node_type, identifier, args, edges, &existing);
        let msg = args
            .get("commit_message")
            .and_then(|v| v.as_str())
            .unwrap_or("subagent write");
        let sha = store::write_page(kb_root, &path, &content, msg, txn)?;
        Ok(serde_json::json!({ "path": path, "commit_sha": sha, "edges": edge_count }).to_string())
    }
}

/// Does a successful call of this tool write a page under `knowledge/`?
///
/// A lint autofix counts these results as its `fixes_applied`. It used to count
/// `kb_write_page` alone, so a run that fixed pages with [`WRITE_CONCEPT`], the
/// writer the BioOKF procedures point at, reported that it had fixed nothing
/// (rq2-027). `kb_append_log` writes `log.md` and `kb_add_raw_source` writes
/// under `raw/`, so neither is a page; `every_writing_tool_is_classified` keeps
/// a new writer from being left out of this list.
pub fn writes_a_page(name: &str) -> bool {
    matches!(name, "kb_write_page" | WRITE_CONCEPT)
}

/// The frontmatter already at `path`, or an empty mapping.
///
/// ## The data loss this closes
///
/// [`compose_page`] builds a page out of the arguments it was given, and
/// `kb_write_page` **overwrites**. So a typed rewrite of an existing page — the
/// normal shape of an ingest that extends what a previous run wrote — would
/// otherwise delete every frontmatter key this tool does not declare a parameter
/// for: OKF §5.1's `sources`, §5.2's `generated` / `verified`, DR-5's
/// `br_credibility`, DR-3's `br_page_id`, and every unknown producer key that
/// OKF §11 requires a consumer to preserve. None of that would fail anything —
/// the page would stay conformant and simply be missing its provenance, which is
/// the worst shape a loss can take.
///
/// So the existing block is the base and this call's arguments are written over
/// it. An unparseable page contributes nothing rather than failing the write: a
/// page whose frontmatter cannot be read is one this write is repairing.
///
/// The **body** is carried the same way and for the same reason: a call that
/// adds one edge to a page and omits `body` is not asking for the page's prose
/// to be replaced by its own title.
fn existing(kb_root: &std::path::Path, path: &str) -> okf::Split {
    crate::knowledge::store::resolve_readable_path(kb_root, path)
        .ok()
        .and_then(|resolved| std::fs::read_to_string(resolved).ok())
        .and_then(|text| okf::frontmatter::split(&text).ok())
        .unwrap_or(okf::Split {
            frontmatter: serde_yaml::Mapping::new(),
            body: String::new(),
            had_block: false,
        })
}

/// One of the 28, or a rejection naming the nearest.
fn parse_node_type(value: &str) -> Result<biookf::NodeType> {
    biookf::NodeType::parse(value).ok_or_else(|| {
        VocabularyRejection {
            field: "type".to_string(),
            value: value.to_string(),
            closest: biookf::lint::closest(value, biookf::NodeType::ALL.iter().map(|t| t.as_str())),
            legal_count: biookf::NodeType::ALL.len(),
            detail: None,
        }
        .into()
    })
}

/// One edge, validated and rendered as the YAML mapping the page will carry.
///
/// Rendered rather than returned as an [`okf::Edge`] because the open bundles
/// (`quantitative`, `qualifiers`) are flattened into the edge mapping — BioOKF
/// §7.3's slots are edge attributes, and nesting them under a key of our own
/// would be a producer extension nothing else reads.
fn edge_mapping(edge: &Value) -> Result<serde_yaml::Value> {
    let predicate = edge
        .get("predicate")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("{WRITE_CONCEPT}: an edge is missing 'predicate'"))?;
    let predicate = parse_predicate(predicate)?;
    let object = edge
        .get("object")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "{WRITE_CONCEPT}: edge `{predicate}` is missing 'object'. It names the TARGET \
                 node's identifier"
            )
        })?;
    let knowledge_level = enum_field(edge, "knowledge_level", biookf::KNOWLEDGE_LEVELS)?;
    let agent_type = enum_field(edge, "agent_type", biookf::AGENT_TYPES)?;
    let primary_source = edge
        .get("primary_source")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "{WRITE_CONCEPT}: edge `{predicate} -> {object}` is missing 'primary_source'. It \
                 names a Publication/Study/Dataset/Agent page that exists in this bundle; a \
                 `reported_in` edge cites its own object"
            )
        })?;

    let mut map = serde_yaml::Mapping::new();
    map.insert("predicate".into(), predicate.to_string().into());
    map.insert("object".into(), object.into());
    map.insert("knowledge_level".into(), knowledge_level.into());
    map.insert("agent_type".into(), agent_type.into());
    map.insert("primary_source".into(), primary_source.into());
    // §7.3's slots are edge attributes, so the two open bundles are flattened
    // into the edge rather than nested under a key of our own — DR-27 keeps them
    // open precisely so a vocabulary addition needs no schema change, and a
    // producer-invented wrapper key is not something any BioOKF reader looks
    // under. A key that would land on one of the five structural fields is
    // dropped: `quantitative: {predicate: …}` is a mistake, and honouring it
    // would silently rewrite the claim the edge makes.
    for bundle in ["quantitative", "qualifiers"] {
        let Some(Value::Object(entries)) = edge.get(bundle) else {
            continue;
        };
        for (k, v) in entries {
            let key: serde_yaml::Value = k.as_str().into();
            if map.contains_key(&key) {
                continue;
            }
            map.insert(key, json_to_yaml(v));
        }
    }
    Ok(serde_yaml::Value::Mapping(map))
}

/// One of the 35, or a rejection. `NotNegatable` carries §6.F's explanation into
/// `detail`, because "not one of the 35" is true of `not_is_a` and is not what
/// is wrong with it.
fn parse_predicate(value: &str) -> Result<biookf::Predicate> {
    biookf::Predicate::parse(value).map_err(|err| {
        let detail =
            matches!(err, biookf::PredicateError::NotNegatable(_)).then(|| err.to_string());
        VocabularyRejection {
            field: "predicate".to_string(),
            value: value.to_string(),
            closest: biookf::lint::closest(
                value,
                biookf::Predicate::all().iter().map(ToString::to_string),
            ),
            legal_count: biookf::Predicate::all().len(),
            detail,
        }
        .into()
    })
}

/// A required enum-valued edge key: present, and a member.
fn enum_field(edge: &Value, field: &str, allowed: &[&str]) -> Result<String> {
    let Some(value) = edge.get(field).and_then(|v| v.as_str()) else {
        anyhow::bail!(
            "{WRITE_CONCEPT}: an edge is missing '{field}'. BioOKF §8 requires \
             knowledge_level, agent_type and primary_source on EVERY edge, `reported_in` and \
             `not_<X>` included"
        );
    };
    if allowed.contains(&value) {
        return Ok(value.to_string());
    }
    Err(VocabularyRejection {
        field: field.to_string(),
        value: value.to_string(),
        closest: biookf::lint::closest(value, allowed.iter().copied()),
        legal_count: allowed.len(),
        detail: None,
    }
    .into())
}

/// `knowledge/<lowercased type>/<slug>.md`, the convention both `schema.md`
/// templates teach and `scaffold_dirs` pre-creates the source half of.
fn default_concept_path(node_type: biookf::NodeType, identifier: &str) -> String {
    format!(
        "knowledge/{}/{}.md",
        node_type.as_str().to_lowercase(),
        slug(identifier)
    )
}

fn slug(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed: String = out
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(80)
        .collect();
    if trimmed.is_empty() {
        "page".to_string()
    } else {
        trimmed
    }
}

/// Frontmatter through `serde_yaml`, never `format!`.
///
/// This is not tidiness. `identifier: Chen 2020 (IL-6: severe COVID-19)` written
/// by hand is an unparseable YAML block, and an unparseable block does not fail
/// loudly — [`crate::knowledge::validate::load_bundle`] and `graph::load_pages`
/// both fall back to a default document, so the page exists, renders as prose
/// and is missing from the graph entirely. The serializer quotes it.
fn compose_page(
    node_type: biookf::NodeType,
    identifier: &str,
    args: &Value,
    edges: Option<Vec<serde_yaml::Value>>,
    existing: &okf::Split,
) -> String {
    // Start from what is already there (see `existing`), then write this call's
    // arguments over it. `type` and `identifier` always move, because they are
    // required parameters; everything else moves only when supplied, so omitting
    // `xref` extends a page rather than stripping it.
    let mut fm = existing.frontmatter.clone();
    fm.insert("type".into(), node_type.as_str().into());
    fm.insert("identifier".into(), identifier.into());
    for key in ["description", "subtype", "status"] {
        if let Some(v) = args
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            fm.insert(key.into(), v.into());
        }
    }
    for key in ["tags", "synonyms", "xref", "raw_source"] {
        let items = string_list(args, key);
        if !items.is_empty() {
            fm.insert(
                key.into(),
                serde_yaml::Value::Sequence(items.into_iter().map(Into::into).collect()),
            );
        }
    }
    // `edges` replaces rather than extends: a list is the page's whole edge
    // set, so `[]` removes the key and only `None` (not sent) keeps it.
    match edges {
        None => {}
        Some(edges) if edges.is_empty() => {
            fm.remove("edges");
        }
        Some(edges) => {
            fm.insert("edges".into(), serde_yaml::Value::Sequence(edges));
        }
    }
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(fm))
        .unwrap_or_else(|_| format!("type: {}\n", node_type.as_str()));

    let body = args
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or(&existing.body)
        .trim_end();
    // A heading only when the body does not open with one: the body is the
    // author's, and prefixing a second `#` to a page that already has one is how
    // a rewrite of an existing page grows a duplicate title on every pass.
    let needs_heading = !body.trim_start().starts_with('#');
    let mut out = format!("---\n{yaml}---\n\n");
    if needs_heading {
        out.push_str(&format!("# {identifier}\n\n"));
    }
    if !body.is_empty() {
        out.push_str(body);
        out.push('\n');
    }
    out
}

fn string_list(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        // A model that sends a bare string where a list is declared is
        // §11-tolerable and common; reading it as a one-element list costs
        // nothing and saves a retry.
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// JSON to YAML for the open `quantitative` / `qualifiers` bundles.
///
/// Numbers stay numbers and strings stay strings, so `p_value: 3.0e-6` is a
/// float the renderer can compare and `p_value: "<0.001"` is the string the
/// source actually printed — DR-27's open map is only worth having if it carries
/// both.
fn json_to_yaml(v: &Value) -> serde_yaml::Value {
    match v {
        Value::Null => serde_yaml::Value::Null,
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => n
            .as_i64()
            .map(serde_yaml::Value::from)
            .or_else(|| n.as_f64().map(serde_yaml::Value::from))
            .unwrap_or_else(|| n.to_string().into()),
        Value::String(s) => s.as_str().into(),
        Value::Array(items) => {
            serde_yaml::Value::Sequence(items.iter().map(json_to_yaml).collect())
        }
        Value::Object(entries) => serde_yaml::Value::Mapping(
            entries
                .iter()
                .map(|(k, v)| (k.as_str().into(), json_to_yaml(v)))
                .collect(),
        ),
    }
}

fn parse_change_kind(s: &str) -> Result<ChangeKind> {
    Ok(match s {
        "ingest" => ChangeKind::Ingest,
        "link" => ChangeKind::Link,
        "flag" => ChangeKind::Flag,
        "query" => ChangeKind::Query,
        "lint" => ChangeKind::Lint,
        "restore" => ChangeKind::Restore,
        "manual" => ChangeKind::Manual,
        other => anyhow::bail!("invalid ChangeKind: '{other}'"),
    })
}

/// Build a `SourceInput` from a JSON `Value` supplied by the sub-agent.
///
/// Accepted shapes:
/// ```json
/// { "type": "text",  "text": "…",  "title": "optional" }
/// { "type": "url",   "url":  "…" }
/// { "type": "file",  "bytes_b64": "…", "filename": "…", "mime": "optional" }
/// ```
fn source_input_from_args(args: &Value) -> Result<SourceInput> {
    let kind = args.get("type").and_then(|v| v.as_str()).unwrap_or("text");
    match kind {
        "text" => {
            let text = args["text"]
                .as_str()
                .ok_or_else(|| anyhow!("kb_add_raw_source: missing 'text'"))?
                .to_string();
            let title = args.get("title").and_then(|v| v.as_str()).map(String::from);
            Ok(SourceInput::Text { text, title })
        }
        "url" => {
            let url = args["url"]
                .as_str()
                .ok_or_else(|| anyhow!("kb_add_raw_source: missing 'url'"))?
                .to_string();
            Ok(SourceInput::Url(url))
        }
        "file" => {
            let b64 = args["bytes_b64"]
                .as_str()
                .ok_or_else(|| anyhow!("kb_add_raw_source: missing 'bytes_b64'"))?;
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| anyhow!("kb_add_raw_source: invalid base64: {e}"))?;
            let filename = args["filename"]
                .as_str()
                .ok_or_else(|| anyhow!("kb_add_raw_source: missing 'filename'"))?
                .to_string();
            let mime = args.get("mime").and_then(|v| v.as_str()).map(String::from);
            Ok(SourceInput::File {
                bytes,
                filename,
                mime,
            })
        }
        other => anyhow::bail!("kb_add_raw_source: unknown source type '{other}'"),
    }
}

// ---------------------------------------------------------------------------
// Tool specs for the sub-agent
// ---------------------------------------------------------------------------

/// One property of a tool's input schema.
///
/// ## Why this is not just `{"type": T}` any more (DR-16)
///
/// It was, and that made a closed vocabulary **unenforceable through the tool
/// interface**. `make_schema` could say a field is a string and nothing else:
/// no `description`, no `enum`, no arrays, no nesting. So the legal values for a
/// BioOKF `type` or `predicate` could only be taught in prose in the system
/// prompt — which the provider cannot use to constrain sampling, which means an
/// invalid value is caught at *dispatch*, as free text, in an error string. A
/// failed tool call is fed back as `error: …` and does not abort, so the model
/// retries, and the retries burn the step budget until the run dies for a reason
/// that has nothing to do with the actual mistake.
///
/// Declaring the vocabulary here instead kills the prompt bloat *and* makes it
/// enforceable — the same fix for two problems.
///
/// ## Nothing in this build uses the rich forms yet
///
/// Every existing spec below still goes through [`make_schema`] and emits
/// exactly the bytes it always did (pinned by
/// `todays_specs_are_byte_identical_to_the_minimal_shape`). Stage 5 is what
/// attaches the 28 node types and 35 predicates to the tools that take them.
/// This change only makes that expressible.
#[derive(Debug, Clone)]
pub struct Prop {
    ty: PropTy,
    description: Option<String>,
    /// The closed vocabulary for this property, empty when it is open.
    allowed: Vec<String>,
}

#[derive(Debug, Clone)]
enum PropTy {
    /// `"string"`, `"integer"`, `"boolean"`, `"number"`.
    Scalar(String),
    Array(Box<Prop>),
    Object {
        required: Vec<(String, Prop)>,
        optional: Vec<(String, Prop)>,
    },
    /// An object whose keys are not known in advance.
    Map,
}

impl Prop {
    /// A scalar of the given JSON Schema type name.
    pub fn scalar(ty: &str) -> Self {
        Self {
            ty: PropTy::Scalar(ty.to_string()),
            description: None,
            allowed: Vec::new(),
        }
    }

    pub fn string() -> Self {
        Self::scalar("string")
    }

    /// A homogeneous array. The item is a full [`Prop`], so an array of
    /// enum-constrained strings — a page's `tags`, a `verified` list — is one
    /// call and not a special case.
    pub fn array_of(item: Prop) -> Self {
        Self {
            ty: PropTy::Array(Box::new(item)),
            description: None,
            allowed: Vec::new(),
        }
    }

    /// A nested object with its own required/optional split. Needed for the
    /// shapes the tools already accept in prose — `kb_add_raw_source`'s
    /// `{type, text, title}` union, BioOKF's per-edge provenance triplet —
    /// which today are documented in a doc-comment and validated by hand in
    /// [`source_input_from_args`].
    pub fn object(required: Vec<(&str, Prop)>, optional: Vec<(&str, Prop)>) -> Self {
        let own = |v: Vec<(&str, Prop)>| {
            v.into_iter()
                .map(|(n, p)| (n.to_string(), p))
                .collect::<Vec<_>>()
        };
        Self {
            ty: PropTy::Object {
                required: own(required),
                optional: own(optional),
            },
            description: None,
            allowed: Vec::new(),
        }
    }

    /// An object with open keys: BioOKF's `quantitative` and `qualifiers`
    /// bundles, which DR-27 makes open maps precisely so a vocabulary addition
    /// needs no schema change. Listing §7.3's twenty-odd slots as properties
    /// would put the thing DR-27 rejected back, one layer down.
    pub fn map() -> Self {
        Self {
            ty: PropTy::Map,
            description: None,
            allowed: Vec::new(),
        }
    }

    /// The one sentence the model reads about this field.
    #[must_use]
    pub fn describe(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }

    /// The closed vocabulary. This is the whole point of the rewrite: an `enum`
    /// here is a constraint the provider can apply while sampling, rather than a
    /// list the model may or may not have read.
    #[must_use]
    pub fn one_of(mut self, values: &[&str]) -> Self {
        self.allowed = values.iter().map(|v| (*v).to_string()).collect();
        self
    }

    fn to_json(&self) -> Value {
        let mut out = serde_json::Map::new();
        match &self.ty {
            PropTy::Scalar(ty) => {
                out.insert("type".into(), Value::String(ty.clone()));
            }
            PropTy::Array(item) => {
                out.insert("type".into(), Value::String("array".into()));
                out.insert("items".into(), item.to_json());
            }
            PropTy::Object { required, optional } => {
                out.insert("type".into(), Value::String("object".into()));
                out.insert(
                    "properties".into(),
                    Value::Object(properties(required, optional)),
                );
                out.insert("required".into(), required_names(required));
            }
            PropTy::Map => {
                out.insert("type".into(), Value::String("object".into()));
                out.insert("additionalProperties".into(), Value::Bool(true));
            }
        }
        // Only when set, so a bare property is still exactly `{"type": T}` and
        // every spec that predates this builder is byte-for-byte unchanged.
        if let Some(d) = &self.description {
            out.insert("description".into(), Value::String(d.clone()));
        }
        if !self.allowed.is_empty() {
            out.insert(
                "enum".into(),
                Value::Array(
                    self.allowed
                        .iter()
                        .map(|v| Value::String(v.clone()))
                        .collect(),
                ),
            );
        }
        Value::Object(out)
    }
}

fn properties(
    required: &[(String, Prop)],
    optional: &[(String, Prop)],
) -> serde_json::Map<String, Value> {
    required
        .iter()
        .chain(optional.iter())
        .map(|(name, prop)| (name.clone(), prop.to_json()))
        .collect()
}

fn required_names(required: &[(String, Prop)]) -> Value {
    Value::Array(
        required
            .iter()
            .map(|(name, _)| Value::String(name.clone()))
            .collect(),
    )
}

/// Build a tool's top-level input schema from named properties.
///
/// The rich form: every property is a [`Prop`], so it may carry a description,
/// an `enum`, an item type or a nested object.
fn schema_of(required: Vec<(&str, Prop)>, optional: Vec<(&str, Prop)>) -> Arc<JsonObject> {
    let Value::Object(map) = Prop::object(required, optional).to_json() else {
        unreachable!("Prop::object always renders an object");
    };
    Arc::new(map)
}

/// The minimal form, kept because most fields really are just "a string": name
/// and JSON Schema type name, nothing else.
fn make_schema(
    required: &[(&str, &str)], // (name, type)
    optional: &[(&str, &str)], // (name, type)
) -> Arc<JsonObject> {
    fn lift<'a>(pairs: &[(&'a str, &str)]) -> Vec<(&'a str, Prop)> {
        pairs
            .iter()
            .map(|(name, ty)| (*name, Prop::scalar(ty)))
            .collect()
    }
    schema_of(lift(required), lift(optional))
}

/// Returns the `Vec<Tool>` to pass to the sub-agent, for a base in `format`.
///
/// ## The vocabulary is here, not in the prompt (DR-16)
///
/// A BioOKF base gets two extra tools, and the reason is the whole of DR-16.
/// The 28 node types and 35 predicates are a *closed* vocabulary, and a closed
/// vocabulary stated in the system prompt is unenforceable: the provider cannot
/// constrain sampling with prose, so an invalid value is caught at dispatch, fed
/// back as free text, and retried until the step budget dies. Declared as `enum`
/// on [`Prop`] they are a constraint the provider applies while sampling, and
/// they cost the prompt nothing per step — which matters because a macro's
/// system prompt is re-sent on every one of up to 30 iterations.
///
/// `None` is a legacy base (DR-26) and gets the OKF set, because it has no
/// controlled vocabulary to declare.
///
/// ## The eight base tools are byte-identical to what they always were
///
/// Pinned by `todays_specs_are_byte_identical_to_the_minimal_shape`. A BioOKF
/// base *adds*; nothing about an OKF or legacy run changed.
pub fn tool_specs(format: Option<KbFormat>) -> Vec<Tool> {
    let mut tools = vec![
        Tool::new(
            Cow::Borrowed("kb_list_pages"),
            Cow::Borrowed("List knowledge pages. Optional path_prefix filter."),
            make_schema(&[], &[("path_prefix", "string")]),
        ),
        Tool::new(
            Cow::Borrowed("kb_read_page"),
            Cow::Borrowed("Read a knowledge page by logical path."),
            make_schema(&[("path", "string")], &[]),
        ),
        Tool::new(
            Cow::Borrowed("kb_write_page"),
            Cow::Borrowed("Create or overwrite a knowledge page (must be under knowledge/)."),
            make_schema(
                &[("path", "string"), ("content", "string")],
                &[("commit_message", "string")],
            ),
        ),
        Tool::new(
            Cow::Borrowed("kb_search"),
            Cow::Borrowed(
                "BM25 full-text search over curated knowledge pages. Set include_raw_sources true only when the user specifically asks for original/raw sources.",
            ),
            make_schema(
                &[("query", "string")],
                &[("limit", "integer"), ("include_raw_sources", "boolean")],
            ),
        ),
        Tool::new(
            Cow::Borrowed("kb_append_log"),
            Cow::Borrowed("Append an entry to the KB change log."),
            make_schema(
                &[("summary", "string")],
                &[("kind", "string"), ("delta", "string")],
            ),
        ),
        Tool::new(
            Cow::Borrowed("kb_add_raw_source"),
            Cow::Borrowed("Ingest a new raw source (text/url/file) into the KB."),
            make_schema(
                &[("type", "string")],
                &[
                    ("text", "string"),
                    ("title", "string"),
                    ("url", "string"),
                    ("bytes_b64", "string"),
                    ("filename", "string"),
                    ("mime", "string"),
                ],
            ),
        ),
        Tool::new(
            Cow::Borrowed("kb_classify_source"),
            Cow::Borrowed("Return the credibility metadata for a previously ingested source."),
            make_schema(&[("source_id", "string")], &[]),
        ),
        Tool::new(
            Cow::Borrowed("complete"),
            Cow::Borrowed("Signal that you have finished and the sub-agent loop should exit."),
            make_schema(&[], &[("message", "string")]),
        ),
    ];
    if format.is_some_and(KbFormat::is_biookf) {
        tools.push(write_concept_spec());
        tools.push(validate_page_spec());
    }
    tools
}

/// The query surface when `file_as_page` is false.
///
/// This is intentionally an allowlist of readers rather than a blocklist of
/// today's writers. A future tool is unavailable to a read-only query until it
/// is deliberately classified here and at the dispatch boundary above.
pub fn read_only_tool_specs(format: Option<KbFormat>) -> Vec<Tool> {
    tool_specs(format)
        .into_iter()
        .filter(|tool| is_read_only_tool(tool.name.as_ref()))
        .collect()
}

/// The legal values as `&[&str]`, borrowed from the vocabulary's own tables so
/// the schema can never list a 29th type or miss a new one.
fn node_type_values() -> Vec<&'static str> {
    biookf::NodeType::ALL.iter().map(|t| t.as_str()).collect()
}

/// All 35 predicates — the 24 positives and the 11 derived negatives — rendered
/// through [`biookf::Predicate`]'s own `Display`, which is where `not_` is
/// spelled. A hand-written `format!("not_{p}")` here would be a second speller.
fn predicate_values() -> Vec<String> {
    biookf::Predicate::all()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn borrow(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

/// The `edges` item schema: the five §8 keys the profile requires on every
/// edge, plus DR-27's two open bundles.
fn edge_schema() -> Prop {
    let predicates = predicate_values();
    Prop::object(
        vec![
            (
                "predicate",
                Prop::string()
                    .describe(
                        "The relation this edge asserts, directed from THIS page to `object`. \
                         There are no inverse predicates: author `encodes` on the gene, never \
                         `encoded_by` on the protein.",
                    )
                    .one_of(&borrow(&predicates)),
            ),
            (
                "object",
                Prop::string().describe(
                    "The TARGET node's `identifier` — never a path, never a CURIE. The target \
                     page may not exist yet; that is legal and is recorded.",
                ),
            ),
            (
                "knowledge_level",
                Prop::string()
                    .describe(
                        "What the source did: an assertion its authors make is \
                         `knowledge_assertion`, a correlation they measured is \
                         `statistical_association`, a model output is `prediction`. Never \
                         silently elevate one to another.",
                    )
                    .one_of(biookf::KNOWLEDGE_LEVELS),
            ),
            (
                "agent_type",
                Prop::string()
                    .describe(
                        "Who produced the claim. Reading it out of a document is \
                         `text_mining_agent`.",
                    )
                    .one_of(biookf::AGENT_TYPES),
            ),
            (
                "primary_source",
                Prop::string().describe(
                    "The `identifier` of a Publication/Study/Dataset/Agent page that EXISTS in \
                     this bundle. Not a CURIE, not a URL, not a file path. A `reported_in` edge \
                     cites its own object here — a source attests its own contents.",
                ),
            ),
        ],
        vec![
            (
                "quantitative",
                Prop::map().describe(
                    "Every number the source reports for this claim: effect_metric, \
                     effect_size, ci_lower, ci_upper, p_value, adjusted_p_value, \
                     standard_error, sample_size, sensitivity, specificity, auc, frequency, \
                     clinical_phase, response_direction, unit. Write the value as given, \
                     `<0.001` included.",
                ),
            ),
            (
                "qualifiers",
                Prop::map().describe(
                    "Context that qualifies the claim rather than measuring it: \
                     species_context, sex, age_group, timepoint. A p-value filed here is a \
                     category error a renderer cannot undo.",
                ),
            ),
        ],
    )
}

/// Everything on a concept page that is optional. Split out of
/// [`write_concept_spec`] only for length; the order is the order the model
/// reads them in.
fn concept_optional_props() -> Vec<(&'static str, Prop)> {
    vec![
        (
            "path",
            Prop::string().describe(
                "Where to write it. Defaults to knowledge/<lowercased type>/<slug of \
                 identifier>.md, which is this base's convention — pass a path only to update a \
                 page that already lives somewhere else.",
            ),
        ),
        (
            "description",
            Prop::string().describe("One sentence, used as the page's summary."),
        ),
        (
            "subtype",
            Prop::string().describe(
                "Free text, never validated. Where the specificity goes when the type is \
                 deliberately coarse: a Molecule with subtype `monoclonal-antibody`.",
            ),
        ),
        ("tags", Prop::array_of(Prop::string())),
        (
            "synonyms",
            Prop::array_of(Prop::string())
                .describe("Other names for this same node, so a later search finds it."),
        ),
        (
            "xref",
            Prop::array_of(Prop::string()).describe(
                "External identifiers as `prefix:local` — PMID:32504360, UniProtKB:P05231, \
                 DRUGBANK:DB06273.",
            ),
        ),
        (
            "status",
            Prop::string().one_of(&["draft", "stable", "deprecated"]),
        ),
        (
            "raw_source",
            Prop::array_of(Prop::string()).describe(
                "For a source node only: the raw/ paths this node's bytes live at. What anchors \
                 the provenance chain to something immutable.",
            ),
        ),
        (
            "edges",
            Prop::array_of(edge_schema()).describe(
                "The typed relationships this page asserts. ONLY these are part of the graph — \
                 prose may restate a relationship and markdown links are fine for navigation, \
                 but neither is an edge. This list replaces the page's edges: send every edge \
                 the page should keep, an empty list to remove them all, or omit it to keep the \
                 edges already there.",
            ),
        ),
        (
            "body",
            Prop::string().describe(
                "The page's markdown body, below the frontmatter. A `# <identifier>` heading is \
                 added when you do not write one. Omit it to keep the body already on the page.",
            ),
        ),
        ("commit_message", Prop::string()),
    ]
}

/// The typed page writer. This is the tool the BioOKF procedures point at, and
/// the reason it exists rather than a sentence telling the model to be careful
/// with `kb_write_page`'s `content` string.
///
/// Three things it buys that a markdown blob cannot:
///
/// 1. **The vocabulary is a schema constraint**, not a hope (DR-16).
/// 2. **The frontmatter is composed by `serde_yaml`**, so an `identifier`
///    containing `: ` — `Chen 2020 (IL-6: severe COVID-19)` — is quoted rather
///    than turning the whole block into an unparseable page the graph then
///    silently omits.
/// 3. **The rejection is diagnosable.** An invalid value comes back naming the
///    closest legal one, and a run that dies retrying one says so.
fn write_concept_spec() -> Tool {
    Tool::new(
        Cow::Borrowed(WRITE_CONCEPT),
        Cow::Borrowed(
            "Write a typed BioOKF concept page: this tool takes the controlled vocabulary as \
             enums and composes the YAML frontmatter for you, so an unquoted identifier or an \
             invented predicate cannot produce a page the graph silently drops. Prefer it over \
             kb_write_page for every page whose frontmatter you are creating or changing; use \
             kb_write_page for index.md and for prose-only edits. Fields you omit keep whatever \
             the page already had. An `edges` list you send replaces the page's edges: to add \
             one, send the page's edges plus the new one, and send an empty list to remove \
             them all. The result says how many edges the page now carries.",
        ),
        schema_of(
            vec![
                (
                    "type",
                    Prop::string()
                        .describe(
                            "The node type. Type the entity by what it IS, not by what it is \
                             doing in this sentence. Nothing fits: use `Other` and say what it \
                             is in the body — never invent a type outside this list.",
                        )
                        .one_of(&node_type_values()),
                ),
                (
                    "identifier",
                    Prop::string().describe(
                        "The page's primary key: human-readable AND unique in this bundle. \
                         Other pages cite this exact string in `object` and `primary_source`. \
                         CURIEs belong in `xref`, never here.",
                    ),
                ),
            ],
            concept_optional_props(),
        ),
    )
}

/// Validate before writing, which is what BioOKF's own toolchain does. The
/// sub-agent surface's copy of the MCP tool Stage 4 added — deliberately
/// **without** a `kb_id`, which is the invariant this file exists to keep (DR-8).
fn validate_page_spec() -> Tool {
    Tool::new(
        Cow::Borrowed(VALIDATE_PAGE),
        Cow::Borrowed(
            "Check a draft page against this base's format WITHOUT writing it: an invalid type \
             or predicate, a missing provenance key, an edge naming a page that does not exist, \
             a duplicate identifier and a domain/range violation are all reported here, one page \
             at a time, while you can still fix them. Writes nothing and commits nothing.",
        ),
        make_schema(&[("content", "string")], &[("path", "string")]),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{page_fixtures::valid_page, service::KnowledgeService};

    fn fresh_svc() -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base("k", "K", None).unwrap();
        (dir, svc)
    }

    /// The minimal shape, spelled out independently of the builder: an object
    /// whose every property is exactly `{"type": T}`, in `type`/`properties`/
    /// `required` order.
    fn minimal_shape(required: &[(&str, &str)], optional: &[(&str, &str)]) -> Value {
        let mut props = serde_json::Map::new();
        for (n, t) in required.iter().chain(optional.iter()) {
            props.insert((*n).to_string(), serde_json::json!({ "type": t }));
        }
        serde_json::json!({
            "type": "object",
            "properties": props,
            "required": required.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        })
    }

    /// DR-16 replaced `make_schema` with a builder that *can* express
    /// descriptions, enums, arrays and nesting — and must not have changed a
    /// single byte of what today's eight tools declare while doing it. Stage 5
    /// is where the richer forms get used; a change here now would be a change
    /// to what the model is told, smuggled in under a refactor.
    #[test]
    fn todays_specs_are_byte_identical_to_the_minimal_shape() {
        let expected: &[(&str, Value)] = &[
            (
                "kb_list_pages",
                minimal_shape(&[], &[("path_prefix", "string")]),
            ),
            ("kb_read_page", minimal_shape(&[("path", "string")], &[])),
            (
                "kb_write_page",
                minimal_shape(
                    &[("path", "string"), ("content", "string")],
                    &[("commit_message", "string")],
                ),
            ),
            (
                "kb_search",
                minimal_shape(
                    &[("query", "string")],
                    &[("limit", "integer"), ("include_raw_sources", "boolean")],
                ),
            ),
            (
                "kb_append_log",
                minimal_shape(
                    &[("summary", "string")],
                    &[("kind", "string"), ("delta", "string")],
                ),
            ),
            (
                "kb_add_raw_source",
                minimal_shape(
                    &[("type", "string")],
                    &[
                        ("text", "string"),
                        ("title", "string"),
                        ("url", "string"),
                        ("bytes_b64", "string"),
                        ("filename", "string"),
                        ("mime", "string"),
                    ],
                ),
            ),
            (
                "kb_classify_source",
                minimal_shape(&[("source_id", "string")], &[]),
            ),
            ("complete", minimal_shape(&[], &[("message", "string")])),
        ];

        // Every profile: DR-16 lets a BioOKF base *add* tools and changes
        // nothing about the eight an OKF or legacy run has always been given.
        for format in [None, Some(KbFormat::Okf), Some(KbFormat::Biookf)] {
            let specs = tool_specs(format);
            for (name, want) in expected {
                let tool = specs
                    .iter()
                    .find(|t| t.name == *name)
                    .unwrap_or_else(|| panic!("{name} is no longer declared for {format:?}"));
                let got = Value::Object((*tool.input_schema).clone());
                assert_eq!(&got, want, "{name}'s schema changed under {format:?}");
            }
        }
        assert_eq!(
            tool_specs(None).len(),
            expected.len(),
            "a tool was added to or removed from the legacy set"
        );
        assert_eq!(
            tool_specs(Some(KbFormat::Okf)).len(),
            expected.len(),
            "a tool was added to or removed from the OKF set"
        );
    }

    /// The whole of DR-16 in one assertion: the 28 types and the 35 predicates
    /// reach the provider as `enum`s it can constrain sampling with, rather than
    /// as a paragraph in a system prompt it cannot read.
    ///
    /// Derived from the vocabulary rather than typed out — a hand-written list
    /// here would pass while the schema shipped 27 of them.
    #[test]
    fn biookf_declares_the_vocabulary_as_enums_on_the_write_tool() {
        let specs = tool_specs(Some(KbFormat::Biookf));
        let tool = specs
            .iter()
            .find(|t| t.name == WRITE_CONCEPT)
            .expect("a BioOKF base gets the typed writer");
        let schema = Value::Object((*tool.input_schema).clone());

        let types = &schema["properties"]["type"]["enum"];
        assert_eq!(
            types,
            &serde_json::json!(biookf::NodeType::ALL
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()),
        );
        assert_eq!(types.as_array().map(Vec::len), Some(28));

        let edge = &schema["properties"]["edges"]["items"];
        let predicates = &edge["properties"]["predicate"]["enum"];
        assert_eq!(
            predicates,
            &serde_json::json!(biookf::Predicate::all()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()),
        );
        assert_eq!(predicates.as_array().map(Vec::len), Some(35));

        // The provenance triplet is required ON THE EDGE, and two thirds of it
        // is closed too — §8's `knowledge_level` and `agent_type`.
        assert_eq!(
            edge["required"],
            serde_json::json!([
                "predicate",
                "object",
                "knowledge_level",
                "agent_type",
                "primary_source"
            ])
        );
        assert_eq!(
            edge["properties"]["knowledge_level"]["enum"],
            serde_json::json!(biookf::KNOWLEDGE_LEVELS)
        );
        assert_eq!(
            edge["properties"]["agent_type"]["enum"],
            serde_json::json!(biookf::AGENT_TYPES)
        );
        // DR-27: the quantitative bundle is an OPEN map. Listing §7.3's slots
        // as properties would put back the fixed field list DR-27 removed.
        assert_eq!(
            edge["properties"]["quantitative"]["type"],
            serde_json::json!("object")
        );
        assert!(edge["properties"]["quantitative"]
            .get("properties")
            .is_none());
    }

    /// An OKF or legacy base has no closed vocabulary, so it is handed no tool
    /// that declares one. The negative half of the test above: a BioOKF-only
    /// tool leaking into the OKF set would teach a base with an open `type` that
    /// its type must be one of 28.
    #[test]
    fn only_a_biookf_base_gets_the_typed_tools() {
        for format in [None, Some(KbFormat::Okf)] {
            let names: Vec<String> = tool_specs(format)
                .iter()
                .map(|t| t.name.to_string())
                .collect();
            assert!(
                !names.contains(&WRITE_CONCEPT.to_string()),
                "{format:?} was handed the BioOKF writer: {names:?}"
            );
            assert!(
                !names.contains(&VALIDATE_PAGE.to_string()),
                "{format:?} was handed the BioOKF validator: {names:?}"
            );
        }
        let biookf: Vec<String> = tool_specs(Some(KbFormat::Biookf))
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(biookf.contains(&WRITE_CONCEPT.to_string()));
        assert!(biookf.contains(&VALIDATE_PAGE.to_string()));
    }

    #[test]
    fn a_closed_vocabulary_reaches_the_provider_as_an_enum() {
        // The half that was impossible before: the legal values as data, where
        // the provider can constrain sampling with them, instead of as a
        // sentence in the system prompt that the model may not honour and the
        // provider cannot read at all.
        let schema = schema_of(
            vec![(
                "predicate",
                Prop::string()
                    .describe("The BioOKF relation this edge asserts.")
                    .one_of(&["treats", "associated_with"]),
            )],
            vec![],
        );
        let got = Value::Object((*schema).clone());
        assert_eq!(
            got["properties"]["predicate"],
            serde_json::json!({
                "type": "string",
                "description": "The BioOKF relation this edge asserts.",
                "enum": ["treats", "associated_with"],
            })
        );
        assert_eq!(got["required"], serde_json::json!(["predicate"]));
    }

    #[test]
    fn arrays_and_nested_objects_are_expressible() {
        // The shape `kb_add_raw_source` already accepts and documents only in a
        // doc-comment, plus the list form an `edges:` argument needs.
        let schema = schema_of(
            vec![(
                "source",
                Prop::object(
                    vec![("type", Prop::string().one_of(&["text", "url", "file"]))],
                    vec![("title", Prop::string())],
                ),
            )],
            vec![("tags", Prop::array_of(Prop::string()))],
        );
        let got = Value::Object((*schema).clone());
        assert_eq!(got["properties"]["source"]["type"], "object");
        assert_eq!(
            got["properties"]["source"]["properties"]["type"]["enum"],
            serde_json::json!(["text", "url", "file"])
        );
        assert_eq!(
            got["properties"]["source"]["required"],
            serde_json::json!(["type"])
        );
        assert_eq!(
            got["properties"]["tags"],
            serde_json::json!({ "type": "array", "items": { "type": "string" } })
        );
    }

    // ── DR-8: the invariant that makes the macro-entry barrier sufficient ────

    /// **The load-bearing one.** The macros clear ONE base at their entry; this
    /// surface is safe only because it cannot be pointed at another. A tool here
    /// that took a base id would read a private base from a public session with
    /// no gate anywhere on the path — not the macro's, which cleared a different
    /// base, and not `KnowledgeServer::call_tool`, which is never reached.
    ///
    /// Asserted three ways, because each alone has a hole: the schema (what the
    /// model is *told* it may send), the source (what the dispatch *reads*), and
    /// — in the test below — the behaviour (what it does when sent one anyway).
    #[test]
    fn no_sub_agent_tool_takes_a_kb_id() {
        // Every profile's set, not just the default one: the BioOKF tools are
        // exactly the kind of addition this invariant exists to catch, and a
        // loop over one format would not have seen them.
        let all: Vec<Tool> = [None, Some(KbFormat::Okf), Some(KbFormat::Biookf)]
            .into_iter()
            .flat_map(tool_specs)
            .collect();
        for tool in all {
            let schema = Value::Object((*tool.input_schema).clone());
            let properties = schema["properties"].as_object().expect("an object schema");
            assert!(
                !properties.contains_key("kb_id"),
                "{} declares a kb_id; see this module's header — the macro-entry \
                 barrier stops being sufficient the moment a tool here can name a \
                 second base",
                tool.name
            );
        }

        // …and the dispatch never reads one, however it were spelled. Assembled
        // at runtime so this test's own text cannot satisfy the grep, and taken
        // over the production half so the assertions above do not either.
        let production = include_str!("kb_tools.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("kb_tools.rs has a production half above its tests");
        let key = concat!("\"kb_", "id\"");
        assert!(
            !production.contains(key),
            "the dispatch reads a {key} argument; that is the DR-8 hole"
        );
    }

    /// The behavioural half, and the one that would still fail if a future
    /// dispatch read the id under another name. A `kb_id` in the arguments is
    /// ignored: the write lands in the bound base and the read does not reach
    /// the other one.
    #[tokio::test]
    async fn a_kb_id_in_the_arguments_does_not_move_the_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base("bound", "Bound", None).unwrap();
        svc.create_base("other", "Other", None).unwrap();
        crate::knowledge::store::write_page(
            &svc.root().join("other"),
            "knowledge/secret.md",
            &valid_page("note", "Secret", "n=412 T2D patients"),
            "seed",
            None,
        )
        .unwrap();

        let dispatch = KbToolDispatch {
            svc: svc.clone(),
            kb_id: "bound".to_string(),
            txn_branch: String::new(),
            access: KbToolAccess::ReadWrite,
        };

        // A read aimed at the other base does not reach it.
        let read = dispatch
            .call(
                "kb_read_page",
                serde_json::json!({ "path": "knowledge/secret.md", "kb_id": "other" }),
            )
            .await;
        assert!(
            read.is_err(),
            "a kb_id argument reached another base: {read:?}"
        );

        // A write aimed at the other base lands in the bound one.
        dispatch
            .call(
                "kb_write_page",
                serde_json::json!({
                    "path": "knowledge/mine.md",
                    "content": valid_page("note", "Mine", "body"),
                    "kb_id": "other",
                }),
            )
            .await
            .unwrap();
        let bound = crate::knowledge::store::list_pages(&svc.root().join("bound"), None).unwrap();
        assert!(
            bound.iter().any(|p| p.path == "knowledge/mine.md"),
            "the write did not land in the bound base: {bound:?}"
        );
        let other = crate::knowledge::store::list_pages(&svc.root().join("other"), None).unwrap();
        assert!(
            !other.iter().any(|p| p.path == "knowledge/mine.md"),
            "the write landed in the base the arguments named: {other:?}"
        );
    }

    // ── DR-16: the typed writer and its rejections ──────────────────────────

    fn biookf_svc() -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base_as(
            "bio",
            "Bio",
            None,
            KbFormat::Biookf,
            false,
            &Default::default(),
        )
        .unwrap();
        (dir, svc)
    }

    fn biookf_dispatch(svc: &KnowledgeService) -> KbToolDispatch {
        KbToolDispatch {
            svc: svc.clone(),
            kb_id: "bio".to_string(),
            txn_branch: String::new(),
            access: KbToolAccess::ReadWrite,
        }
    }

    /// The provenance triplet, spelled once so a test about types is not also a
    /// test about remembering three keys.
    fn cited(predicate: &str, object: &str, source: &str) -> Value {
        serde_json::json!({
            "predicate": predicate,
            "object": object,
            "knowledge_level": "knowledge_assertion",
            "agent_type": "text_mining_agent",
            "primary_source": source,
        })
    }

    /// DR-16's second half. The `enum` stops most invalid values being sampled;
    /// this is what happens to the ones that are sampled anyway, and it is the
    /// difference between a model that fixes the call and a model that guesses
    /// again until the budget dies.
    #[tokio::test]
    async fn a_rejected_type_names_the_closest_legal_one() {
        let (_dir, svc) = biookf_svc();
        let err = biookf_dispatch(&svc)
            .call(
                WRITE_CONCEPT,
                serde_json::json!({ "type": "Diseases", "identifier": "Type 2 diabetes" }),
            )
            .await
            .expect_err("an invalid type is refused");

        assert!(
            VocabularyRejection::is_one(&err),
            "the refusal must be recoverable by type, not by message text: {err}"
        );
        let rejection = err.downcast_ref::<VocabularyRejection>().unwrap();
        assert_eq!(rejection.field, "type");
        assert_eq!(rejection.closest.as_deref(), Some("Disease"));
        assert_eq!(rejection.legal_count, 28);
        // …and the message the model actually reads carries it.
        assert!(
            err.to_string().contains("`Disease`"),
            "the message must name the closest value: {err}"
        );
    }

    #[tokio::test]
    async fn a_rejected_predicate_names_the_closest_legal_one() {
        let (_dir, svc) = biookf_svc();
        let err = biookf_dispatch(&svc)
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Molecule",
                    "identifier": "Tocilizumab",
                    "edges": [cited("treat", "COVID-19", "RECOVERY trial")],
                }),
            )
            .await
            .expect_err("an invalid predicate is refused");
        let rejection = err.downcast_ref::<VocabularyRejection>().unwrap();
        assert_eq!(rejection.field, "predicate");
        assert_eq!(rejection.closest.as_deref(), Some("treats"));
        assert_eq!(rejection.legal_count, 35);
    }

    /// `not_is_a` is one of the 24 with a prefix, so "not one of the 35" is true
    /// of it and is not what is wrong with it. §6.F's explanation rides along in
    /// `detail`, because the fix is to stop negating a structural predicate
    /// rather than to try a different spelling.
    #[tokio::test]
    async fn negating_a_structural_predicate_is_refused_with_the_reason() {
        let (_dir, svc) = biookf_svc();
        let err = biookf_dispatch(&svc)
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Molecule",
                    "identifier": "Aspirin",
                    "edges": [cited("not_is_a", "NSAID", "DrugBank")],
                }),
            )
            .await
            .expect_err("§6.F refuses this");
        let rejection = err.downcast_ref::<VocabularyRejection>().unwrap();
        assert!(
            rejection
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("§6.F")),
            "the reason must travel with the refusal: {rejection:?}"
        );
    }

    /// The other two closed edge keys. They are the ones a model most often
    /// invents a plausible-sounding value for, because they read like free text.
    #[tokio::test]
    async fn a_rejected_provenance_value_is_a_vocabulary_rejection_too() {
        let (_dir, svc) = biookf_svc();
        for (field, bad, want) in [
            (
                "knowledge_level",
                "statistical_associations",
                Some("statistical_association"),
            ),
            (
                "agent_type",
                "text_mining_agents",
                Some("text_mining_agent"),
            ),
            // A value nothing in the vocabulary resembles gets NO suggestion,
            // and that is the right answer: naming an arbitrary member of a
            // six-word list because it happened to be nearest is a
            // misdirection, and the message says to re-read the enum instead.
            ("agent_type", "llm", None),
        ] {
            let mut edge = cited("treats", "COVID-19", "RECOVERY trial");
            edge[field] = Value::String(bad.to_string());
            let err = biookf_dispatch(&svc)
                .call(
                    WRITE_CONCEPT,
                    serde_json::json!({
                        "type": "Molecule",
                        "identifier": "Tocilizumab",
                        "edges": [edge],
                    }),
                )
                .await
                .expect_err("an invalid {field} is refused");
            let rejection = err.downcast_ref::<VocabularyRejection>().unwrap();
            assert_eq!(rejection.field, field);
            assert_eq!(rejection.closest.as_deref(), want, "for `{bad}`");
            if want.is_none() {
                assert!(
                    err.to_string().contains("re-read the enum"),
                    "a suggestionless refusal must still say where to look: {err}"
                );
            }
        }
    }

    /// The point of composing the page in Rust rather than asking for a markdown
    /// blob: what comes out is conformant, and it is conformant by construction.
    #[tokio::test]
    async fn a_typed_write_produces_a_page_the_profile_accepts() {
        let (_dir, svc) = biookf_svc();
        let kb_root = svc.root().join("bio");
        let dispatch = biookf_dispatch(&svc);

        // A source node first, because §8.1 makes every edge cite one.
        dispatch
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Study",
                    "identifier": "RECOVERY trial",
                    "xref": ["ISRCTN:50189673"],
                    "raw_source": ["raw/recovery/source.md"],
                    "edges": [cited("reported_in", "RECOVERY trial", "RECOVERY trial")],
                }),
            )
            .await
            .unwrap();
        dispatch
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Disease",
                    "identifier": "COVID-19",
                    "edges": [cited("reported_in", "RECOVERY trial", "RECOVERY trial")],
                }),
            )
            .await
            .unwrap();
        let written = dispatch
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Molecule",
                    "identifier": "Tocilizumab",
                    "subtype": "monoclonal-antibody",
                    "description": "An IL-6 receptor antagonist.",
                    "xref": ["DRUGBANK:DB06273"],
                    "body": "Trialled in severe COVID-19.",
                    "edges": [
                        {
                            "predicate": "treats",
                            "object": "COVID-19",
                            "knowledge_level": "statistical_association",
                            "agent_type": "data_analysis_pipeline",
                            "primary_source": "RECOVERY trial",
                            "quantitative": { "effect_metric": "relative_risk", "effect_size": 0.85, "p_value": "<0.001" },
                            "qualifiers": { "species_context": "human" }
                        },
                        cited("reported_in", "RECOVERY trial", "RECOVERY trial"),
                    ],
                }),
            )
            .await
            .unwrap();

        // The path was derived, not supplied: `knowledge/<lowercased type>/<slug>.md`.
        let written: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(written["path"], "knowledge/molecule/tocilizumab.md");

        let text = std::fs::read_to_string(kb_root.join("knowledge/molecule/tocilizumab.md"))
            .expect("the page is on disk");
        let pages = validate::load_bundle(&kb_root).unwrap();
        let diagnostics = validate::validate_page(
            Some(KbFormat::Biookf),
            Some("knowledge/molecule/tocilizumab.md"),
            &text,
            &pages,
        );
        assert_eq!(
            diagnostics.errors(),
            0,
            "the composed page must be conformant: {:#?}",
            diagnostics.items
        );

        // DR-27: the quantitative bundle is flattened onto the edge as §7.3
        // attributes, and a number stays a number while `<0.001` stays a string.
        assert!(text.contains("effect_size: 0.85"), "{text}");
        assert!(
            text.contains("p_value: <0.001") || text.contains("p_value: \"<0.001\""),
            "{text}"
        );
        assert!(text.contains("species_context: human"), "{text}");
    }

    /// The failure this composition exists to make unreachable. A hand-written
    /// `identifier: Chen 2020 (IL-6: severe COVID-19)` is an unparseable YAML
    /// block — and unparseable does not fail loudly: `load_bundle` and
    /// `graph::load_pages` both fall back to a default document, so the page
    /// renders as prose and is simply absent from the graph.
    #[tokio::test]
    async fn an_identifier_containing_a_colon_is_quoted_rather_than_breaking_the_block() {
        let (_dir, svc) = biookf_svc();
        let identifier = "Chen 2020: IL-6 and severe COVID-19";
        biookf_dispatch(&svc)
            .call(
                WRITE_CONCEPT,
                serde_json::json!({ "type": "Publication", "identifier": identifier }),
            )
            .await
            .unwrap();
        let text = std::fs::read_to_string(
            svc.root()
                .join("bio/knowledge/publication/chen-2020-il-6-and-severe-covid-19.md"),
        )
        .unwrap();
        let page = crate::knowledge::okf::Page::parse(&text).expect("frontmatter still parses");
        assert_eq!(page.doc.identifier.as_deref(), Some(identifier));
    }

    /// ⚠ **The loss a typed writer creates if nobody thinks about it.**
    /// `kb_write_page` overwrites, and `compose_page` builds a page out of the
    /// arguments it was given — so a second call that adds one edge would delete
    /// every key this tool has no parameter for. None of it would fail anything:
    /// the page stays conformant and quietly loses its provenance, which is the
    /// worst shape a loss can take.
    ///
    /// So an existing page is the base and the call is written over it. What
    /// this pins is the whole set at risk: OKF §5.1's `sources`, §5.2's
    /// `generated`, DR-5's `br_credibility`, DR-3's `br_page_id`, an arbitrary
    /// producer key §11 requires a consumer to preserve, and the body's prose.
    #[tokio::test]
    async fn rewriting_a_page_keeps_the_keys_the_call_did_not_name() {
        let (_dir, svc) = biookf_svc();
        let kb_root = svc.root().join("bio");
        let dispatch = biookf_dispatch(&svc);
        let path = "knowledge/molecule/tocilizumab.md";
        crate::knowledge::store::write_page(
            &kb_root,
            path,
            "---\ntype: Molecule\nidentifier: Tocilizumab\nxref: [DRUGBANK:DB06273]\n\
             sources:\n  - id: recovery\n    resource: raw/recovery/source.md\n\
             generated: { by: biorouter, at: 2026-08-19T12:00:00Z }\n\
             br_page_id: 01J8XABCDEF\nbr_credibility: { tier: peer_reviewed }\n\
             some_other_producers_key: keep me\n---\n\n\
             # Tocilizumab\n\nAn IL-6 receptor antagonist.\n",
            "seed",
            None,
        )
        .unwrap();

        // A second pass that only adds an edge.
        dispatch
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Molecule",
                    "identifier": "Tocilizumab",
                    "path": path,
                    "edges": [cited("treats", "COVID-19", "RECOVERY trial")],
                }),
            )
            .await
            .unwrap();

        let text = std::fs::read_to_string(kb_root.join(path)).unwrap();
        let split = crate::knowledge::okf::frontmatter::split(&text).unwrap();
        for key in [
            "sources",
            "generated",
            "br_page_id",
            "br_credibility",
            "some_other_producers_key",
            "xref",
        ] {
            assert!(
                split
                    .frontmatter
                    .contains_key(serde_yaml::Value::String(key.to_string())),
                "the rewrite dropped `{key}`:\n{text}"
            );
        }
        assert!(
            split.body.contains("An IL-6 receptor antagonist."),
            "the rewrite dropped the body:\n{text}"
        );
        assert!(text.contains("predicate: treats"), "{text}");
    }

    /// rq2-027. The lint autofix sent `edges: []` to drop an invalid `loves`
    /// edge, the writer read the empty list as "not supplied", kept the edge and
    /// answered ok, and the model then told the person the edge was gone. An
    /// explicit list is the page's whole edge list, so an empty one removes them;
    /// only an omitted key keeps what the page already had.
    #[tokio::test]
    async fn write_concept_with_an_empty_edge_list_removes_the_edges() {
        let (_dir, svc) = biookf_svc();
        let kb_root = svc.root().join("bio");
        let dispatch = biookf_dispatch(&svc);
        let path = "knowledge/gene/badgene.md";
        crate::knowledge::store::write_page(
            &kb_root,
            path,
            "---\ntype: Spaceship\nidentifier: BADGENE\nedges:\n  - predicate: loves\n    \
             object: BRCA note\n    knowledge_level: knowledge_assertion\n    \
             agent_type: manual_agent\n    primary_source: BRCA note\n---\n\n# BADGENE\n\nBody.\n",
            "seed",
            None,
        )
        .unwrap();
        let edges_on_disk = || {
            let text = std::fs::read_to_string(kb_root.join(path)).unwrap();
            crate::knowledge::okf::frontmatter::split(&text)
                .unwrap()
                .frontmatter
                .get(serde_yaml::Value::String("edges".into()))
                .cloned()
        };

        // Omitted: the page keeps its edges, which is the extend rule.
        dispatch
            .call(
                WRITE_CONCEPT,
                serde_json::json!({ "type": "Other", "identifier": "BADGENE", "path": path }),
            )
            .await
            .unwrap();
        assert!(
            edges_on_disk().is_some(),
            "omitting `edges` must keep the page's edges"
        );

        // Empty: the edges are gone, and the result says how many remain.
        let out = dispatch
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Other",
                    "identifier": "BADGENE",
                    "path": path,
                    "edges": [],
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            edges_on_disk(),
            None,
            "`edges: []` must remove the page's edges"
        );
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["edges"], 0, "{out}");
        let text = std::fs::read_to_string(kb_root.join(path)).unwrap();
        assert!(!text.contains("loves"), "{text}");
        assert!(text.contains("Body."), "the body must survive: {text}");
    }

    /// Every tool a sub-agent may call that is not a reader is either a page
    /// writer, which a lint autofix counts as a fix, or one of the two tools
    /// known to write something else. A new writer has to be put on one side.
    #[test]
    fn every_writing_tool_is_classified() {
        const WRITES_ELSEWHERE: [&str; 2] = ["kb_append_log", "kb_add_raw_source"];
        for tool in tool_specs(Some(KbFormat::Biookf)) {
            let name = tool.name.as_ref();
            if is_read_only_tool(name) {
                assert!(!writes_a_page(name), "{name} is a reader and a page writer");
                continue;
            }
            assert!(
                writes_a_page(name) || WRITES_ELSEWHERE.contains(&name),
                "{name} writes but is not classified; add it to `writes_a_page` if it writes \
                 a page under knowledge/"
            );
        }
        assert!(writes_a_page("kb_write_page"));
        assert!(writes_a_page(WRITE_CONCEPT));
    }

    /// A non-list `edges` is refused rather than read as "keep", which was the
    /// silent shape of the bug above.
    #[tokio::test]
    async fn write_concept_refuses_edges_that_are_not_a_list() {
        let (_dir, svc) = biookf_svc();
        let err = biookf_dispatch(&svc)
            .call(
                WRITE_CONCEPT,
                serde_json::json!({
                    "type": "Molecule",
                    "identifier": "Aspirin",
                    "edges": "none",
                }),
            )
            .await
            .expect_err("a string is not an edge list");
        assert!(err.to_string().contains("'edges' must be a list"), "{err}");
    }

    /// A base that is not in the BioOKF profile refuses the typed writer rather
    /// than accepting it. The tool is not declared to an OKF run, so this can
    /// only be reached by a model sending the name anyway — and the two facts
    /// (which tools were declared, which base was bound) are established in
    /// different places, so the dispatch checks. Imposing a closed biomedical
    /// vocabulary on a base whose `type` names are the user's own would be a
    /// wrong answer rather than a refused one.
    #[tokio::test]
    async fn the_typed_writer_refuses_a_base_that_is_not_biookf() {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base_as(
            "okf",
            "Okf",
            None,
            KbFormat::Okf,
            false,
            &Default::default(),
        )
        .unwrap();
        let dispatch = KbToolDispatch {
            svc: svc.clone(),
            kb_id: "okf".to_string(),
            txn_branch: String::new(),
            access: KbToolAccess::ReadWrite,
        };
        let err = dispatch
            .call(
                WRITE_CONCEPT,
                serde_json::json!({ "type": "Molecule", "identifier": "Aspirin" }),
            )
            .await
            .expect_err("an OKF base refuses the typed writer");
        assert!(err.to_string().contains("plain OKF"), "{err}");
        assert!(
            !svc.root()
                .join("okf/knowledge/molecule/aspirin.md")
                .exists(),
            "the refusal must not have written"
        );
    }

    /// The sub-agent's pre-write check. Without it the BioOKF procedure would
    /// name a tool the sub-agent does not have, which is the retry storm the
    /// stage exists to remove rather than a fix for it.
    #[tokio::test]
    async fn the_sub_agent_can_validate_a_draft_without_writing_it() {
        let (_dir, svc) = biookf_svc();
        let dispatch = biookf_dispatch(&svc);
        let draft = "---\ntype: Molekule\nidentifier: Tocilizumab\n---\n\n# Tocilizumab\n";
        let out = dispatch
            .call(
                VALIDATE_PAGE,
                serde_json::json!({
                    "content": draft,
                    "path": "knowledge/molecule/tocilizumab.md",
                }),
            )
            .await
            .unwrap();
        let out: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(out["format"], "biookf");
        assert_eq!(out["ok"], false);
        assert!(out["errors"].as_u64().unwrap() >= 1);
        assert!(
            !svc.root()
                .join("bio/knowledge/molecule/tocilizumab.md")
                .exists(),
            "validation must not write"
        );
    }

    #[tokio::test]
    async fn kb_write_then_read_roundtrip() {
        let (_dir, svc) = fresh_svc();
        let dispatch = KbToolDispatch {
            svc,
            kb_id: "k".to_string(),
            txn_branch: String::new(), // no txn — commit directly
            access: KbToolAccess::ReadWrite,
        };

        // Write a page
        let write_args = serde_json::json!({
            "path": "knowledge/entities/hrv.md",
            "content": valid_page("entity", "HRV", "Heart rate variability."),
            "commit_message": "test write"
        });
        let write_result = dispatch.call("kb_write_page", write_args).await.unwrap();
        let parsed: Value = serde_json::from_str(&write_result).unwrap();
        // Should contain a commit_sha
        assert!(parsed.get("commit_sha").is_some());

        // Read it back
        let read_args = serde_json::json!({ "path": "knowledge/entities/hrv.md" });
        let read_result = dispatch.call("kb_read_page", read_args).await.unwrap();
        let page: Value = serde_json::from_str(&read_result).unwrap();
        let body = page["content"].as_str().unwrap_or("");
        assert!(body.contains("Heart rate variability"), "body was: {body}");
    }
}
