//! Agent-driven UI control — the `ui_*` tools an app's agent uses to **drive the
//! app**, not just answer in it.
//!
//! Every other tool an app agent has produces text that lands in the transcript.
//! These produce *commands* that travel down the app's own WebSocket and mutate
//! the page: mount a side panel, compose a dashboard, draw a chart, highlight a
//! section, restyle the app, or block on a form until the user answers.
//!
//! Shape (mirrors `datasql` / `files` / `compute`): an [`AppControlServer`] is
//! constructed **per app session** in `biorouter-server`'s `configure_agent`,
//! carrying a [`UiBridge`] whose sender is drained by that session's WebSocket
//! loop. It therefore can't be a `BUILTIN_EXTENSIONS` entry — it needs the live
//! socket. Scope is the app's own page: no filesystem, no network, no other tab.
//!
//! Round-trip: `ui_ask` emits a command carrying a `requestId`, parks on a
//! oneshot, and the socket loop resolves it when the browser sends back
//! `{"type":"ui_reply","requestId":…,"payload":{…}}`. The tool result *is* the
//! user's answer, so the agent can branch on it inside a single turn.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, ErrorCode, ErrorData, Implementation, ServerCapabilities,
        ServerInfo,
    },
    schemars::JsonSchema,
    tool, tool_handler, tool_router, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use super::manifest::{ActionDecl, SurfaceDecl, UiCapability, THEME_PACKS};

/// Catalog / protocol version stamped onto every `ui` frame (`"v"`), so an older
/// SDK can feature-detect and ignore frames it doesn't understand. Bump when the
/// frame vocabulary changes in a backward-incompatible way.
pub const CATALOG_VERSION: u64 = 1;

// ── Apps SDK v2 control-plane caps (Pillar 1, §3.5 / §3.7) ──────────────────
/// Max serialized bytes of a payload relayed across the agent↔app boundary: an
/// `app_call` result (capped with a truncation marker), an `emit_result` value
/// (rejected as a fixable error when larger), or an inbound signal payload
/// (rejected by [`UiBridge::validate_signal`]). Bounds the transcript / token
/// cost of the round-trips so an app cannot flood the agent.
pub const APP_PAYLOAD_MAX: usize = 65_536;
/// Seconds an `app_call` parks waiting for the app's registered handler before
/// it gives up and returns gracefully (so a missing/hung handler can't wedge the
/// turn). Tests shadow this via [`UiBridge::set_app_call_timeout_s`].
pub const APP_CALL_TIMEOUT_S: u64 = 60;
/// Default seconds a consulted worker profile gets to answer (design §3.8).
///
/// Lowered from 120 to 60. The old value was a compile-time constant with **no
/// configuration path at all** (`set_consult_timeout_s` was called only from a
/// test), and it was duplicated on both sides of the channel: the `consult` tool
/// started a 120 s timer *before* the request even reached the socket loop, and
/// `run_consult` then started a second one strictly later. The outer always won,
/// so the inner was effectively dead code — and when the outer fired, the loop was
/// still awaiting the worker, draining nothing, for another full deadline.
///
/// The loop side now owns the deadline (see `CONSULT_GRACE_S`), and an app may
/// override it per profile.
pub const CONSULT_TIMEOUT_S: u64 = 60;

/// Extra slack the parked `consult` TOOL allows beyond the loop's deadline.
///
/// The loop is the single owner of the consult deadline: it times the worker out,
/// cancels it, and resolves the tool with a structured result. This grace exists
/// only so a wedged loop cannot park the tool forever — it must never be the timer
/// that fires first, or we are back to two racing deadlines.
pub const CONSULT_GRACE_S: u64 = 20;

// ── Shared state document caps (Apps SDK v2, Pillar 2) ──────────────────────
// Enforced after every mutation so an unschema'd app can't become an unbounded
// injection / DoS path; a violating mutation is rejected and the doc is left
// unchanged.
/// Max serialized size of the whole state document.
pub const STATE_MAX_BYTES: usize = 262_144;
/// Max operations in a single RFC-6902 patch.
pub const STATE_MAX_PATCH_OPS: usize = 64;
/// Max nesting depth of the state document.
pub const STATE_MAX_DEPTH: usize = 8;
/// Max total object keys across the whole state document.
pub const STATE_MAX_KEYS: usize = 2000;

/// Widget node kinds the client SDK's `renderWidget` understands and that a
/// *generic* agent tree (`ui_panel` / `ui_render` / `ui_patch`) may emit. Kept
/// in sync with `templates/sdk.ts` (`WidgetNode`). Validated server-side so a
/// malformed tree comes back to the model as a fixable error instead of a blank
/// panel. The privileged kinds in [`PRIVILEGED_WIDGET_KINDS`] are **not** here.
pub const WIDGET_KINDS: &[&str] = &[
    // v1 built-ins
    "card",
    "row",
    "col",
    "text",
    "badge",
    "table",
    "chart",
    "graph",
    "stat",
    "divider",
    "input",
    "select",
    "checkbox",
    "button",
    "form",
    "progress",
    // Apps SDK v2 catalog additions (Pillar 3)
    "markdown",
    "image",
    "kpi",
    "log",
    "plot",
    "network",
    "component",
];

/// Widget kinds that only the dedicated server-side tools may construct, *after*
/// they have sanitized / rendered the payload. They are rejected by
/// [`validate_widget`] when they arrive inside a generic agent tree — an agent
/// cannot hand-write a `{t:"html"}` node and smuggle raw markup past the
/// sanitizer, nor forge a `{t:"figure"}` with arbitrary iframe content. `ui_html`
/// / `ui_figure` build these nodes themselves with `allow_privileged = true`.
pub const PRIVILEGED_WIDGET_KINDS: &[&str] = &["html", "figure"];

/// Placement slots a panel can occupy. `dock` is the SDK-provided right-hand
/// drawer that exists in every app even when the author declared no regions.
pub const PANEL_PLACES: &[&str] = &["dock", "left", "right", "bottom", "main", "modal"];

const MAX_WIDGET_DEPTH: usize = 12;
const MAX_WIDGET_NODES: usize = 600;
const MAX_TABLE_ROWS: usize = 500;
const MAX_CHART_POINTS: usize = 500;

// ── Apps SDK v2 catalog caps ────────────────────────────────────────────────
/// Max characters in a `markdown` node's `md`.
const MAX_MARKDOWN_CHARS: usize = 32_768;
/// Max bytes of an `image` `data:` URL (rendered inline).
const MAX_IMAGE_DATA_BYTES: usize = 512 * 1024;
/// Max lines in a `log` node.
const MAX_LOG_LINES: usize = 500;
/// Max total data points across a `plot` node's series/cells.
const MAX_PLOT_POINTS: usize = 2000;
/// Max nodes in a `network` node's spec.
const MAX_NETWORK_NODES: usize = 1500;
/// Max edges in a `network` node's spec.
const MAX_NETWORK_EDGES: usize = 4000;
/// Max operations in one `ui_patch` call.
const MAX_PATCH_OPS: usize = 32;
/// Max characters in an instance / patch id.
const MAX_INSTANCE_ID_LEN: usize = 64;
/// Max raw HTML bytes accepted by `ui_html` before sanitization.
const MAX_HTML_BYTES: usize = 64 * 1024;

fn invalid(msg: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, msg.into(), None)
}

/// Accept a JSON object/array that the model handed us as a *string*.
///
/// `spec` and `body` are typed `serde_json::Value`, so schemars emits an
/// unconstrained schema and many models (observed with Qwen via Ollama) send
/// `"{\"data\": [...]}"` instead of `{"data": [...]}`. They then retry forever,
/// because "must be an object" doesn't tell them what they actually did. Parse
/// the string once rather than bouncing the call back.
fn unstringify(value: &Value) -> Value {
    match value.as_str() {
        Some(s) => serde_json::from_str::<Value>(s).unwrap_or_else(|_| value.clone()),
        None => value.clone(),
    }
}

/// The error a still-stringified value earns: name the mistake precisely.
fn want_json(field: &str, kind: &str, example: &str) -> String {
    format!(
        "\"{field}\" must be {kind}, not a JSON string. Pass it as literal JSON, e.g. {example}"
    )
}

fn ok_text(msg: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![Content::text(msg.into())]))
}

// ── Layout grammar (Apps SDK v2, Pillar 6) ──────────────────────────────────
/// Max rows/columns in a `ui_layout` grid — bounds the template so the grammar
/// can never blow up into an arbitrary CSS grid.
const LAYOUT_MAX_ROWS: usize = 4;
const LAYOUT_MAX_COLS: usize = 4;

/// True when `s` is a safe grid area / track name (`^[a-z][a-z0-9_-]{0,23}$`),
/// so it can be spliced into `grid-template-areas` / a track list without any
/// chance of breaking out of the declaration.
fn is_layout_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 24
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

/// Validate a `grid-template-areas` grid: 1..=4 rectangular rows of ≤4 columns,
/// each cell a safe area name or `"."` (the CSS empty-cell token).
fn validate_layout_areas(areas: &[Vec<String>]) -> Result<(), String> {
    if areas.is_empty() {
        return Err("\"areas\" must have at least one row".to_string());
    }
    if areas.len() > LAYOUT_MAX_ROWS {
        return Err(format!("\"areas\" allows at most {LAYOUT_MAX_ROWS} rows"));
    }
    let cols = areas[0].len();
    if cols == 0 {
        return Err("\"areas\" rows must have at least one column".to_string());
    }
    if cols > LAYOUT_MAX_COLS {
        return Err(format!(
            "\"areas\" allows at most {LAYOUT_MAX_COLS} columns"
        ));
    }
    for (r, row) in areas.iter().enumerate() {
        if row.len() != cols {
            return Err(format!(
                "\"areas\" must be rectangular: row {r} has {} cells, expected {cols}",
                row.len()
            ));
        }
        for cell in row {
            if cell != "." && !is_layout_name(cell) {
                return Err(format!(
                    "\"areas\" cell {cell:?} must be a lowercase name \
                     (^[a-z][a-z0-9_-]{{0,23}}$) or \".\""
                ));
            }
        }
    }
    Ok(())
}

/// Validate a `sizes` value against the bounded vocabulary: `NNNpx` (80–800),
/// `NN%` (5–95), `Nfr` (1–6), or `auto`.
fn is_layout_size(v: &str) -> bool {
    if v == "auto" {
        return true;
    }
    if let Some(n) = v.strip_suffix("px") {
        return n.parse::<u32>().is_ok_and(|x| (80..=800).contains(&x));
    }
    if let Some(n) = v.strip_suffix('%') {
        return n.parse::<u32>().is_ok_and(|x| (5..=95).contains(&x));
    }
    if let Some(n) = v.strip_suffix("fr") {
        return n.parse::<u32>().is_ok_and(|x| (1..=6).contains(&x));
    }
    false
}

/// Validate a `sizes` map: safe track names → vocabulary values.
fn validate_layout_sizes(sizes: &HashMap<String, String>) -> Result<(), String> {
    for (k, v) in sizes {
        if !is_layout_name(k) {
            return Err(format!(
                "\"sizes\" key {k:?} must be a lowercase track/area name"
            ));
        }
        if !is_layout_size(v) {
            return Err(format!(
                "\"sizes\" value {v:?} must be one of: NNNpx (80-800), NN% (5-95), \
                 Nfr (1-6), auto"
            ));
        }
    }
    Ok(())
}

/// A fresh 16-hex-char id for an `app_call`, derived from a v4 UUID. Distinct
/// from [`UiBridge::next_id`]'s sequential ids so a call id can't collide with a
/// panel/ask id and is unguessable across sessions.
/// One worker's verdict about the inputs it was asked to reason over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceEntry {
    /// The worker profile key that reported it.
    pub profile: String,
    pub status: EvidenceStatus,
    /// Named inputs the worker says it did not have.
    pub missing: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceStatus {
    /// The worker had what it needed.
    Ok,
    /// The worker could not do the job with the inputs available.
    InsufficientData,
    /// The worker failed for another reason.
    Error,
}

impl EvidenceStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ok" => Some(Self::Ok),
            "insufficient_data" => Some(Self::InsufficientData),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::InsufficientData => "insufficient_data",
            Self::Error => "error",
        }
    }
}

/// How a value reaching the page was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvenanceSource {
    /// Computed by a tool, or reported by a worker that had its inputs.
    Grounded,
    /// Supplied by the user.
    User,
    /// MADE UP. Legal — a demo is legitimate — but it is labelled on the page.
    Synthetic,
}

impl ProvenanceSource {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if s == "synthetic" {
            return Some(Self::Synthetic);
        }
        if s == "user" {
            return Some(Self::User);
        }
        if s == "tool" || s.starts_with("consult:") {
            return Some(Self::Grounded);
        }
        None
    }
}

/// Resolve an RFC-6901 JSON Pointer against `doc`, or `Null` when it is absent.
/// Used by the mutate-action readback: an owned pointer that does not exist yet
/// and an owned pointer explicitly set to null are both "no value", and either
/// way what matters is whether the app's handler CHANGED it.
fn pointer_get(doc: &Value, pointer: &str) -> Value {
    if pointer.is_empty() {
        return doc.clone();
    }
    doc.pointer(pointer).cloned().unwrap_or(Value::Null)
}

fn fresh_call_id() -> String {
    // `simple()` is 32 ASCII hex chars; take the first 16 (byte == char here).
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(16)
        .collect()
}

/// Serialize `v` and cap it at [`APP_PAYLOAD_MAX`] bytes, appending a
/// `…[truncated]` marker when it overflows. Used to relay an `app_call` result
/// back to the model without letting a huge payload flood the transcript.
fn capped_json_text(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|_| "null".to_string());
    if s.len() <= APP_PAYLOAD_MAX {
        return s;
    }
    let mut end = APP_PAYLOAD_MAX;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    // `s.get(..end)` avoids a panicking slice index; `end` is a char boundary.
    format!("{}…[truncated]", s.get(..end).unwrap_or(s.as_str()))
}

/// Validate `value` against a JSON Schema, fail-closed. An empty/absent schema
/// (`{}` or `null`) is unconstrained. A schema that itself fails to compile is a
/// single error naming the manifest fix, never an accept-any fallback. Returns
/// the joined validation messages so the caller can prefix its own context.
/// Shared by `app_call` args, `emit_result` values, and inbound signal payloads.
fn json_schema_errors(value: &Value, schema: &Value) -> Result<(), String> {
    let unconstrained =
        schema.is_null() || schema.as_object().is_some_and(serde_json::Map::is_empty);
    if unconstrained {
        return Ok(());
    }
    let validator = jsonschema::validator_for(schema).map_err(|e| {
        format!("the declared JSON Schema failed to compile. Fix the app manifest: {e}")
    })?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|err| {
            let path = err.instance_path.to_string();
            if path.is_empty() {
                err.to_string()
            } else {
                format!("{path}: {err}")
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

// ---------------------------------------------------------------------------
// Shared state document (Apps SDK v2, Pillar 2)
// ---------------------------------------------------------------------------

/// The single shared JSON state document for one app session, plus its version
/// counter. Both the agent (`ui_state` / `ui_patch_state`) and the browser
/// (`state_write`) mutate it; the server (this bridge) is the ordering authority
/// that bumps `version` on every accepted change.
struct StateDoc {
    doc: Value,
    version: u64,
}

impl Default for StateDoc {
    fn default() -> Self {
        Self {
            doc: Value::Object(serde_json::Map::new()),
            version: 0,
        }
    }
}

/// Why a client-originated state write was refused.
#[derive(Debug)]
pub enum StateWriteError {
    /// `base_version` did not match the server's version. Carries the current
    /// authoritative `(doc, version)` so the server can push a fresh snapshot to
    /// the out-of-date client.
    Conflict(Value, u64),
    /// The write was malformed or would violate the state caps; the doc is
    /// unchanged. The message is safe to relay.
    Invalid(String),
}

/// Enforce the structural caps on a candidate state document. Called after every
/// mutation, against the post-mutation value, so a violation rejects the change.
fn validate_state_doc(doc: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(doc)
        .map(|b| b.len())
        .unwrap_or(usize::MAX);
    if bytes > STATE_MAX_BYTES {
        return Err(format!(
            "state document is {bytes} bytes; cap is {STATE_MAX_BYTES}"
        ));
    }
    let mut keys = 0usize;
    walk_state_doc(doc, 1, &mut keys)
}

/// Depth + total-key-count check. `depth` is 1 for the root value; each level of
/// object/array nesting adds one.
fn walk_state_doc(v: &Value, depth: usize, keys: &mut usize) -> Result<(), String> {
    if depth > STATE_MAX_DEPTH {
        return Err(format!(
            "state document is nested deeper than {STATE_MAX_DEPTH} levels"
        ));
    }
    match v {
        Value::Object(map) => {
            *keys += map.len();
            if *keys > STATE_MAX_KEYS {
                return Err(format!("state document exceeds {STATE_MAX_KEYS} keys"));
            }
            for child in map.values() {
                walk_state_doc(child, depth + 1, keys)?;
            }
        }
        Value::Array(arr) => {
            for child in arr {
                walk_state_doc(child, depth + 1, keys)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Set `value` at an RFC-6901 JSON Pointer within `root`, creating intermediate
/// objects as needed. An empty pointer replaces the whole document. Returns an
/// error only when the path runs *through* an existing non-object node.
fn pointer_set(root: &mut Value, pointer: &str, value: Value) -> Result<(), String> {
    if pointer.is_empty() {
        *root = value;
        return Ok(());
    }
    if !pointer.starts_with('/') {
        return Err(format!(
            "JSON pointer must be empty or start with '/', got {pointer:?}"
        ));
    }
    let tokens: Vec<String> = pointer
        .strip_prefix('/')
        .unwrap_or("")
        .split('/')
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
    let n = tokens.len();
    let mut cur = root;
    for (i, tok) in tokens.into_iter().enumerate() {
        let obj = object_for_set(cur)?;
        if i + 1 == n {
            obj.insert(tok, value);
            return Ok(());
        }
        cur = obj
            .entry(tok)
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    Ok(())
}

/// Coerce a node into a mutable object for pointer-set: `null` becomes a fresh
/// object (so absent intermediates are created), anything else must already be
/// an object.
fn object_for_set(v: &mut Value) -> Result<&mut serde_json::Map<String, Value>, String> {
    if v.is_null() {
        *v = Value::Object(serde_json::Map::new());
    }
    v.as_object_mut()
        .ok_or_else(|| "cannot set through a non-object node in the state document".to_string())
}

// ---------------------------------------------------------------------------
// Bridge: tools → WebSocket, and WebSocket → parked `ui_ask` tools
// ---------------------------------------------------------------------------

/// The state a single app session's UI shares between its agent's tools and its
/// WebSocket loop. Cloneable handle around shared interior state.
///
/// **Rebindable on purpose.** `AppState::get_agent` caches one agent per session
/// and `add_inprocess_server` is idempotent by name, so a reconnecting browser
/// reuses the *same* [`AppControlServer`] that the first connection injected. If
/// the outbound sender were owned by that server, every reload would leave the
/// `ui_*` tools writing into a dead channel. Instead the sender lives here behind
/// a lock: each connection calls [`UiBridge::attach`] to install a fresh channel
/// and [`UiBridge::detach`] on close.
#[derive(Clone)]
pub struct UiBridge {
    inner: Arc<BridgeInner>,
}

/// An armed structured-output request. The server calls
/// [`UiBridge::set_pending_output`] when the app opens a structured call; the
/// agent's `emit_result` tool consumes it, validates the value against `schema`
/// (when present), and pushes a top-level `output` frame carrying `call_id`.
struct PendingOutput {
    call_id: String,
    schema: Option<Value>,
}

/// A request from the `consult` tool for a named worker profile to answer a
/// self-contained sub-question (design §3.8 multi-agent profiles).
///
/// The tool cannot reach the worker agent (that lives in `biorouter-server`'s app
/// socket loop), so it hands the request through the bridge's consult channel and
/// parks on a oneshot keyed by `id`; the app socket loop runs a bounded worker
/// turn and unparks it via [`UiBridge::resolve_consult`].
#[derive(Debug, Clone)]
pub struct ConsultRequest {
    /// Correlates the reply the app socket loop posts back via `resolve_consult`.
    pub id: String,
    /// The worker profile name to run (must be a validated `orchestration.agents`
    /// entry; the loop rejects unknown ones).
    pub agent: String,
    /// The self-contained prompt for the worker to answer.
    pub prompt: String,
}

struct BridgeInner {
    /// Outbound half of the *current* connection. `None` between connections.
    tx: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    /// Serializes the complete attach/detach lifecycle. A generation check
    /// outside this lock could pass before a reload and then clear its new
    /// sender or cancel its freshly parked tools.
    generation: Mutex<u64>,
    /// `requestId` → responder for a parked `ui_ask`.
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    /// `callId` → responder for a parked `app_call`. Separate from [`Self::pending`]
    /// on purpose: asks carry ask-specific cancel/timeout semantics, so the two
    /// parking maps mirror each other's mechanics but never share a call id space.
    pending_calls: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    /// Sender for `consult` requests, installed by the app socket loop via
    /// [`UiBridge::set_consult_handler`]. `None` when no worker-profile handler is
    /// listening (an app with no profiles) — `consult` then reports so gracefully.
    consult_tx: Mutex<Option<mpsc::UnboundedSender<ConsultRequest>>>,
    /// `id` → responder for a parked `consult` (mirrors [`Self::pending_calls`]),
    /// so [`UiBridge::cancel_all`] can unpark a consult and the loop can reply.
    pending_consults: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    /// Per-turn EVIDENCE LEDGER: what the app's workers reported about the inputs
    /// they were asked to reason over.
    ///
    /// The platform had no representation of "the evidence is missing", so there
    /// was nothing to enforce against — a worker's "this is not defensible without
    /// sumstats" was, to the server, an ordinary paragraph. The main agent read it
    /// and published invented numbers anyway. The ledger is written by workers
    /// (`report_evidence`), never by the main agent, and it is what `app_call`
    /// checks before letting quantitative output reach the page.
    evidence: Mutex<Vec<EvidenceEntry>>,
    /// Seconds a `consult` waits before timing out. Initialized to
    /// [`CONSULT_TIMEOUT_S`]; only tests mutate it (via
    /// [`UiBridge::set_consult_timeout_s`]).
    consult_timeout_s: AtomicU64,
    /// A pending structured-output request the server armed for `emit_result`:
    /// the call id the app is waiting on plus an optional JSON Schema the result
    /// must satisfy. `None` unless a structured call is in flight.
    pending_output: Mutex<Option<PendingOutput>>,
    /// Signals (`surface.signals` names) the agent is currently subscribed to.
    /// Replaced wholesale by `ui_subscribe`; read by the server to decide which
    /// inbound signals to deliver.
    subscriptions: Mutex<HashSet<String>>,
    /// Signals the agent is subscribed to *by declaration* (`SignalDecl.eager`).
    ///
    /// Kept separate from `subscriptions` so a later `ui_subscribe` — which
    /// REPLACES the explicit set — can never drop below the declared floor. A
    /// single `ui_subscribe([])` would otherwise silently re-break the app.
    eager: Mutex<HashSet<String>>,
    /// The app's declared surface, mirrored onto the bridge (set by
    /// [`AppControlServer::new`]) so the server-facing signal accessors
    /// ([`UiBridge::signal_decl`] / [`UiBridge::validate_signal`]) can resolve a
    /// declaration without holding the `AppControlServer`.
    surface_decl: Mutex<SurfaceDecl>,
    /// Seconds an `app_call` waits before timing out. Initialized to
    /// [`APP_CALL_TIMEOUT_S`]; only tests mutate it (via
    /// [`UiBridge::set_app_call_timeout_s`]) to exercise the timeout path quickly.
    app_call_timeout_s: AtomicU64,
    /// The app session's shared state document + version. Both the agent
    /// (`ui_state` / `ui_patch_state`) and the browser (`state_write`) mutate it;
    /// this bridge is the ordering authority.
    state: Mutex<StateDoc>,
    /// Panel ids currently mounted, oldest first.
    panels: Mutex<Vec<String>>,
    /// Instance registry (Apps SDK v2, Pillar 3): widget node id → its last-known
    /// node, so `ui_patch` can add / replace / set_props / remove individual
    /// components by id. Reset on [`UiBridge::attach`] (v1 reconnect semantics:
    /// the UI resets, but the shared `state` doc persists).
    instances: Mutex<HashMap<String, Value>>,
    /// Last surface report the browser sent (regions, element ids, title).
    surface: Mutex<Option<Value>>,
    /// Fingerprint of the last `ui_describe` payload, so a repeat says "unchanged"
    /// (H3: nudges the model to stop re-polling within a turn).
    last_describe: Mutex<Option<String>>,
    seq: AtomicU64,
}

/// A handle to one attached connection. Passed back to [`UiBridge::detach`] so
/// only the connection that owns the current generation can tear it down.
#[derive(Debug, Clone, Copy)]
pub struct ConnToken(u64);

impl Default for UiBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl UiBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(BridgeInner {
                tx: Mutex::new(None),
                generation: Mutex::new(0),
                pending: Mutex::new(HashMap::new()),
                pending_calls: Mutex::new(HashMap::new()),
                consult_tx: Mutex::new(None),
                pending_consults: Mutex::new(HashMap::new()),
                evidence: Mutex::new(Vec::new()),
                consult_timeout_s: AtomicU64::new(CONSULT_TIMEOUT_S),
                pending_output: Mutex::new(None),
                subscriptions: Mutex::new(HashSet::new()),
                eager: Mutex::new(HashSet::new()),
                surface_decl: Mutex::new(SurfaceDecl::default()),
                app_call_timeout_s: AtomicU64::new(APP_CALL_TIMEOUT_S),
                state: Mutex::new(StateDoc::default()),
                panels: Mutex::new(Vec::new()),
                instances: Mutex::new(HashMap::new()),
                surface: Mutex::new(None),
                last_describe: Mutex::new(None),
                seq: AtomicU64::new(0),
            }),
        }
    }

    /// Bind this bridge to a new connection, returning the receiver its socket
    /// loop must drain plus a [`ConnToken`] to pass back to [`detach`]. Any
    /// previous connection's channel is dropped and its parked asks cancelled,
    /// and the per-connection view (mounted panels, reported surface) is reset —
    /// the reloaded page has none of it. The semantic `state` bag survives and is
    /// re-emitted so the fresh page rehydrates.
    pub fn attach(&self) -> (mpsc::UnboundedReceiver<Value>, ConnToken) {
        let mut generation = self
            .inner
            .generation
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *generation += 1;
        let gen = *generation;
        self.cancel_all();
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut slot) = self.inner.tx.lock() {
            *slot = Some(tx);
        }
        if let Ok(mut p) = self.inner.panels.lock() {
            p.clear();
        }
        // The reloaded page has rendered none of the prior instances, so the
        // registry that `ui_patch` targets resets with it. (The shared state
        // doc, below, deliberately survives.)
        if let Ok(mut i) = self.inner.instances.lock() {
            i.clear();
        }
        if let Ok(mut s) = self.inner.surface.lock() {
            *s = None;
        }
        let (doc, version) = self.state_snapshot();
        let has_state = version > 0 || doc.as_object().map(|m| !m.is_empty()).unwrap_or(false);
        if has_state {
            let _ = self.emit(json!({
                "cmd": "state",
                "mode": "snapshot",
                "doc": doc,
                "version": version,
            }));
        }
        (rx, ConnToken(gen))
    }

    /// Release a connection. A no-op unless `token` is the current generation, so
    /// a stale connection closing *after* a reload cannot tear down the fresh
    /// one. Parked `ui_ask` calls are unblocked so no tool outlives its socket.
    pub fn detach(&self, token: ConnToken) {
        let generation = self
            .inner
            .generation
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if *generation != token.0 {
            return; // a newer connection owns the bridge now
        }
        if let Ok(mut slot) = self.inner.tx.lock() {
            *slot = None;
        }
        self.cancel_all();
    }

    fn next_id(&self, prefix: &str) -> String {
        let n = self.inner.seq.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{n}")
    }

    /// Push a command frame at the browser. Fails when no socket is attached,
    /// which is the honest answer for the model: the app it was driving went away.
    fn emit(&self, mut cmd: Value) -> Result<(), ErrorData> {
        if let Some(obj) = cmd.as_object_mut() {
            obj.insert("type".into(), json!("ui"));
            // Stamp the catalog version on every ui frame so an older SDK can
            // feature-detect and ignore frames it doesn't understand.
            obj.insert("v".into(), json!(CATALOG_VERSION));
        }
        let gone = || {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "the app window is no longer connected; its UI cannot be updated".to_string(),
                None,
            )
        };
        let slot = self.inner.tx.lock().map_err(|_| gone())?;
        let tx = slot.as_ref().ok_or_else(gone)?;
        tx.send(cmd).map_err(|_| gone())
    }

    /// Record the surface report the browser sends after `ready`.
    pub fn set_surface(&self, surface: Value) {
        if let Ok(mut s) = self.inner.surface.lock() {
            *s = Some(surface);
        }
    }

    /// The current shared state document and its version. The server uses this to
    /// send a fresh `snapshot` on connect and to persist the doc.
    pub fn state_snapshot(&self) -> (Value, u64) {
        self.inner
            .state
            .lock()
            .map(|s| (s.doc.clone(), s.version))
            .unwrap_or_else(|_| (Value::Object(serde_json::Map::new()), 0))
    }

    /// Seed the state document from durable persistence. Applies **only** while
    /// the session is still fresh (`version == 0`), so a restore can never clobber
    /// live in-session state; call it before the first [`attach`](Self::attach).
    pub fn seed_state(&self, doc: Value, version: u64) {
        if let Ok(mut s) = self.inner.state.lock() {
            if s.version == 0 {
                s.doc = doc;
                s.version = version;
            }
        }
    }

    /// Apply a browser-originated state write (from a `state_write` frame). The
    /// caller passes exactly one of `set` (a JSON Pointer + value) or `patch` (an
    /// RFC-6902 op array). `base_version` must match the server's current version
    /// (last-writer-wins with an optimistic-concurrency check); a mismatch returns
    /// [`StateWriteError::Conflict`] carrying the authoritative `(doc, version)` so
    /// the server can resnapshot the client. On success returns the RFC-6902 ops
    /// actually applied (for rebroadcast) and the new version.
    pub fn apply_client_write(
        &self,
        set: Option<(String, Value)>,
        patch: Option<Value>,
        base_version: u64,
    ) -> Result<(Value, u64), StateWriteError> {
        let mut guard =
            self.inner.state.lock().map_err(|_| {
                StateWriteError::Invalid("the app state is unavailable".to_string())
            })?;
        if base_version != guard.version {
            return Err(StateWriteError::Conflict(guard.doc.clone(), guard.version));
        }

        // Mutate a clone; commit only once it validates, so a rejected write
        // leaves the live document untouched.
        let mut next = guard.doc.clone();
        let applied_ops: Value = match (set, patch) {
            (Some(_), Some(_)) => {
                return Err(StateWriteError::Invalid(
                    "provide either \"set\" or \"patch\", not both".to_string(),
                ))
            }
            (None, None) => {
                return Err(StateWriteError::Invalid(
                    "a state_write needs a \"set\" or a \"patch\"".to_string(),
                ))
            }
            (Some((pointer, value)), None) => {
                pointer_set(&mut next, &pointer, value.clone())
                    .map_err(StateWriteError::Invalid)?;
                // Rebroadcast the set as a single RFC-6902 add (JSON Pointer
                // semantics: add creates or replaces the member).
                json!([{ "op": "add", "path": pointer, "value": value }])
            }
            (None, Some(patch)) => {
                let ops = patch.as_array().ok_or_else(|| {
                    StateWriteError::Invalid(
                        "\"patch\" must be an array of RFC-6902 operations".to_string(),
                    )
                })?;
                if ops.len() > STATE_MAX_PATCH_OPS {
                    return Err(StateWriteError::Invalid(format!(
                        "\"patch\" has {} operations; cap is {STATE_MAX_PATCH_OPS}",
                        ops.len()
                    )));
                }
                let parsed: json_patch::Patch =
                    serde_json::from_value(patch.clone()).map_err(|e| {
                        StateWriteError::Invalid(format!(
                            "\"patch\" is not a valid RFC-6902 JSON Patch: {e}"
                        ))
                    })?;
                json_patch::patch(&mut next, &parsed).map_err(|e| {
                    StateWriteError::Invalid(format!(
                        "\"patch\" could not be applied to the current state: {e}"
                    ))
                })?;
                patch
            }
        };

        validate_state_doc(&next).map_err(StateWriteError::Invalid)?;
        let new_version = guard.version + 1;
        guard.doc = next;
        guard.version = new_version;
        Ok((applied_ops, new_version))
    }

    /// Resolve a parked `ui_ask`. Returns whether a tool was actually waiting.
    pub fn resolve(&self, request_id: &str, payload: Value) -> bool {
        let responder = self
            .inner
            .pending
            .lock()
            .ok()
            .and_then(|mut p| p.remove(request_id));
        match responder {
            Some(tx) => tx.send(payload).is_ok(),
            None => false,
        }
    }

    /// Fail every parked `ui_ask`, `app_call` **and** `consult` (socket closed /
    /// turn cancelled) so no tool hangs past the life of the connection. All three
    /// parking maps unblock with the same `{cancelled:true}` payload, which each
    /// tool recognizes and reports as a cancellation rather than a real answer.
    pub fn cancel_all(&self) {
        let asks: Vec<_> = self
            .inner
            .pending
            .lock()
            .map(|mut p| p.drain().map(|(_, tx)| tx).collect())
            .unwrap_or_default();
        let calls: Vec<_> = self
            .inner
            .pending_calls
            .lock()
            .map(|mut p| p.drain().map(|(_, tx)| tx).collect())
            .unwrap_or_default();
        let consults: Vec<_> = self
            .inner
            .pending_consults
            .lock()
            .map(|mut p| p.drain().map(|(_, tx)| tx).collect())
            .unwrap_or_default();
        for tx in asks.into_iter().chain(calls).chain(consults) {
            let _ = tx.send(json!({ "cancelled": true }));
        }
    }

    /// Push a **raw** frame at the browser, exactly as given — used for top-level
    /// frames that are *not* `ui` commands (e.g. the `{type:"output"}` frame that
    /// `emit_result` sends). Unlike [`emit`](Self::emit) it does not stamp
    /// `type:"ui"` / `v`, so the caller owns the whole envelope. Returns whether a
    /// socket was attached to receive it.
    pub fn emit_frame(&self, frame: Value) -> bool {
        let Ok(slot) = self.inner.tx.lock() else {
            return false;
        };
        match slot.as_ref() {
            Some(tx) => tx.send(frame).is_ok(),
            None => false,
        }
    }

    /// Resolve a parked `app_call`. Returns whether a call was actually waiting.
    /// The server calls this when the browser sends an `app_result` frame; the
    /// payload is `{result: …}` or `{error: "…"}`.
    pub fn resolve_app_call(&self, call_id: &str, payload: Value) -> bool {
        let responder = self
            .inner
            .pending_calls
            .lock()
            .ok()
            .and_then(|mut p| p.remove(call_id));
        match responder {
            Some(tx) => tx.send(payload).is_ok(),
            None => false,
        }
    }

    fn register_call(&self, call_id: String) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.inner.pending_calls.lock() {
            p.insert(call_id, tx);
        }
        rx
    }

    fn forget_call(&self, call_id: &str) {
        if let Ok(mut p) = self.inner.pending_calls.lock() {
            p.remove(call_id);
        }
    }

    // ── consult (multi-agent profiles, design §3.8) ─────────────────────────

    /// Install the consult handler channel and return the receiver the app socket
    /// loop drains. Called once per connection (like [`attach`](Self::attach)); a
    /// reconnect replaces the sender so a stale receiver is dropped. Only the MAIN
    /// agent's turn loop drains this — a worker turn deliberately does not, which
    /// is what bounds consult depth to 1.
    pub fn set_consult_handler(&self) -> mpsc::UnboundedReceiver<ConsultRequest> {
        let (tx, rx) = mpsc::unbounded_channel();
        if let Ok(mut slot) = self.inner.consult_tx.lock() {
            *slot = Some(tx);
        }
        rx
    }

    /// Hand a [`ConsultRequest`] to the installed handler. Returns whether a
    /// handler was listening (an app with no worker profiles has none).
    fn send_consult_request(&self, req: ConsultRequest) -> bool {
        let Ok(slot) = self.inner.consult_tx.lock() else {
            return false;
        };
        match slot.as_ref() {
            Some(tx) => tx.send(req).is_ok(),
            None => false,
        }
    }

    fn register_consult(&self, id: String) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.inner.pending_consults.lock() {
            p.insert(id, tx);
        }
        rx
    }

    fn forget_consult(&self, id: &str) {
        if let Ok(mut p) = self.inner.pending_consults.lock() {
            p.remove(id);
        }
    }

    /// Resolve a parked `consult`. Returns whether one was actually waiting. The
    /// app socket loop calls this once a worker turn produced an answer; the
    /// payload is `{text: "…"}`, `{error: "…"}`, or `{cancelled: true}`.
    pub fn resolve_consult(&self, id: &str, payload: Value) -> bool {
        let responder = self
            .inner
            .pending_consults
            .lock()
            .ok()
            .and_then(|mut p| p.remove(id));
        match responder {
            Some(tx) => tx.send(payload).is_ok(),
            None => false,
        }
    }

    /// Test-only knob to shorten the `consult` timeout so the timeout path can be
    /// exercised without a real 120-second wait.
    #[cfg(test)]
    pub fn set_consult_timeout_s(&self, secs: u64) {
        self.inner.consult_timeout_s.store(secs, Ordering::Relaxed);
    }

    /// Arm a structured-output request for `emit_result`: the app is now waiting
    /// on `call_id`, and the emitted result must satisfy `schema` when present.
    pub fn set_pending_output(&self, call_id: String, schema: Option<Value>) {
        if let Ok(mut o) = self.inner.pending_output.lock() {
            *o = Some(PendingOutput { call_id, schema });
        }
    }

    /// Take (and clear) the armed structured-output request, if any.
    pub fn take_pending_output(&self) -> Option<(String, Option<Value>)> {
        self.inner
            .pending_output
            .lock()
            .ok()
            .and_then(|mut o| o.take())
            .map(|p| (p.call_id, p.schema))
    }

    /// Replace the active signal subscription set (idempotent — the caller has
    /// already validated every name against `surface.signals`).
    fn replace_subscriptions(&self, names: Vec<String>) {
        if let Ok(mut s) = self.inner.subscriptions.lock() {
            *s = names.into_iter().collect();
        }
    }

    /// The signals the agent is currently subscribed to, sorted for a stable
    /// `ui_describe` view and deterministic tests.
    pub fn subscribed_signals(&self) -> Vec<String> {
        let mut set: HashSet<String> = self
            .inner
            .subscriptions
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        if let Ok(e) = self.inner.eager.lock() {
            set.extend(e.iter().cloned());
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    }

    /// Mirror the app's declared surface onto the bridge so the server-facing
    /// signal accessors can resolve declarations. Called by
    /// [`AppControlServer::new`]; no `apps.rs` change is required.
    pub fn set_surface_decl(&self, surface: SurfaceDecl) {
        // Declaration IS subscription. Seed the eager floor from the surface
        // before storing it, so a signal the user fires *before the agent's first
        // turn* — the normal case, since a click precedes any tool call — is
        // accepted rather than dropped.
        let eager: HashSet<String> = surface
            .signals
            .iter()
            .filter(|sig| sig.eager)
            .map(|sig| sig.name.clone())
            .collect();
        if let Ok(mut e) = self.inner.eager.lock() {
            *e = eager;
        }
        if let Ok(mut s) = self.inner.surface_decl.lock() {
            *s = surface;
        }
    }

    /// Narrow the *explicit* subscription set, as `ui_subscribe` does.
    ///
    /// Exposed so the eager-floor invariant is testable from an integration test:
    /// no `ui_subscribe` may drop a declared signal, or a single narrowing call
    /// would silently re-break app→agent signals for the rest of the session.
    pub fn replace_subscriptions_for_test(&self, names: Vec<String>) {
        self.replace_subscriptions(names);
    }

    /// Signals the agent is subscribed to by declaration (never emptied by a
    /// narrowing `ui_subscribe`).
    pub fn eager_signals(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .inner
            .eager
            .lock()
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Record a worker's verdict about its inputs. Called by `report_evidence`,
    /// which only WORKERS carry — the main agent cannot write its own alibi.
    pub fn record_evidence(&self, entry: EvidenceEntry) {
        if let Ok(mut e) = self.inner.evidence.lock() {
            e.push(entry);
        }
    }

    /// The evidence ledger for the current turn.
    pub fn evidence(&self) -> Vec<EvidenceEntry> {
        self.inner
            .evidence
            .lock()
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    /// Clear the ledger. The socket loop calls this at the start of each turn:
    /// "the data was missing last turn" must not block a turn where the user has
    /// since supplied it.
    pub fn clear_evidence(&self) {
        if let Ok(mut e) = self.inner.evidence.lock() {
            e.clear();
        }
    }

    /// Inputs some worker reported missing this turn, paired with who said so.
    pub fn missing_evidence(&self) -> Vec<(String, String)> {
        self.evidence()
            .into_iter()
            .filter(|e| e.status == EvidenceStatus::InsufficientData)
            .flat_map(|e| {
                let profile = e.profile.clone();
                e.missing.into_iter().map(move |m| (m, profile.clone()))
            })
            .collect()
    }

    /// The declared action with this name, if any.
    pub fn action_decl(&self, name: &str) -> Option<ActionDecl> {
        let s = self.inner.surface_decl.lock().ok()?;
        s.actions.iter().find(|a| a.name == name).cloned()
    }

    /// JSON Pointers owned by a `mutate` action's handler, paired with the action
    /// that owns each.
    ///
    /// A pointer listed here may only be changed by calling the app's real
    /// handler (`app_call`). Without this the agent can simply *write the number*
    /// with `ui_patch_state` and narrate the change as if the app had made it —
    /// which is exactly what specs 011/013/014 did. Prose telling the model to
    /// "call the action before you narrate it" is the instruction Agent Drafter
    /// already generated, and it was ignored.
    pub fn owned_pointers(&self) -> Vec<(String, String)> {
        let Ok(s) = self.inner.surface_decl.lock() else {
            return Vec::new();
        };
        s.actions
            .iter()
            .filter(|a| a.effect.is_mutate())
            .flat_map(|a| a.writes.iter().map(|p| (p.clone(), a.name.clone())))
            .collect()
    }

    /// The action that owns `path`, if any. A pointer is owned when it IS an
    /// owned pointer or sits UNDER one (`/params/lion_vision/min` is owned by the
    /// action that owns `/params/lion_vision`).
    pub fn owner_of_path(&self, path: &str) -> Option<String> {
        self.owned_pointers()
            .into_iter()
            .find_map(|(owned, action)| {
                if path == owned || path.starts_with(&format!("{owned}/")) {
                    Some(action)
                } else {
                    None
                }
            })
    }

    /// Refuse a direct state write that would forge an action's effect.
    pub fn check_write_allowed(&self, path: &str) -> Result<(), String> {
        match self.owner_of_path(path) {
            Some(action) => Err(format!(
                "\"{path}\" is owned by the action \"{action}\", so you cannot write it directly. \
                 Call app_call(name: \"{action}\", …) so the app's own handler makes the change. \
                 Writing the value yourself would put a number on the page that the app never \
                 computed."
            )),
            None => Ok(()),
        }
    }

    /// A declared signal's `(payload_schema, coalesce_ms)`, or `None` when the
    /// name is not declared. For the server's inbound-signal path.
    pub fn signal_decl(&self, name: &str) -> Option<(Option<Value>, u64)> {
        let s = self.inner.surface_decl.lock().ok()?;
        s.signals
            .iter()
            .find(|sig| sig.name == name)
            .map(|sig| (sig.payload.clone(), sig.coalesce_ms))
    }

    /// Validate an inbound signal the browser wants to deliver to the agent, in
    /// order: subscribed? → declared? → payload within [`APP_PAYLOAD_MAX`]? →
    /// schema-valid (when the declaration carries a payload schema)? The `Err`
    /// message is safe to relay/log.
    pub fn validate_signal(&self, name: &str, payload: &Value) -> Result<(), String> {
        let subscribed = self
            .inner
            .subscriptions
            .lock()
            .map(|s| s.contains(name))
            .unwrap_or(false)
            || self
                .inner
                .eager
                .lock()
                .map(|s| s.contains(name))
                .unwrap_or(false);
        if !subscribed {
            return Err(format!("signal \"{name}\" is not subscribed"));
        }
        let Some((schema, _coalesce)) = self.signal_decl(name) else {
            return Err(format!(
                "signal \"{name}\" is not declared in the app surface"
            ));
        };
        let bytes = serde_json::to_vec(payload)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        if bytes > APP_PAYLOAD_MAX {
            return Err(format!(
                "signal \"{name}\" payload is {bytes} bytes; cap is {APP_PAYLOAD_MAX}"
            ));
        }
        if let Some(sc) = schema.as_ref() {
            json_schema_errors(payload, sc).map_err(|e| {
                format!("signal \"{name}\" payload does not match its declared schema: {e}")
            })?;
        }
        Ok(())
    }

    /// Test-only knob to shorten the `app_call` timeout so the timeout path can be
    /// exercised without a real 60-second wait. Production keeps
    /// [`APP_CALL_TIMEOUT_S`].
    #[cfg(test)]
    pub fn set_app_call_timeout_s(&self, secs: u64) {
        self.inner.app_call_timeout_s.store(secs, Ordering::Relaxed);
    }

    fn register_ask(&self, request_id: String) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.inner.pending.lock() {
            p.insert(request_id, tx);
        }
        rx
    }

    fn forget_ask(&self, request_id: &str) {
        if let Ok(mut p) = self.inner.pending.lock() {
            p.remove(request_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Widget-tree validation
// ---------------------------------------------------------------------------

/// Context threaded through [`validate_widget`]: whether the privileged
/// `html`/`figure` kinds are allowed (they are, only when the *dedicated*
/// server-side tool builds the node after sanitizing/rendering), and the app's
/// declared surface (needed to validate a `component` instance against its
/// [`ComponentDecl`](super::manifest::ComponentDecl) props schema).
pub struct WidgetCtx<'a> {
    /// Permit `html` / `figure` nodes. FALSE for every generic agent tree
    /// (`ui_panel` / `ui_render` / `ui_patch`), TRUE only inside `ui_html` /
    /// `ui_figure`.
    pub allow_privileged: bool,
    /// The declared surface, so `component` instances can be schema-checked.
    pub surface: &'a SurfaceDecl,
}

fn validate_widget_kind(t: &str, ctx: &WidgetCtx) -> Result<(), String> {
    if WIDGET_KINDS.contains(&t) {
        return Ok(());
    }
    if PRIVILEGED_WIDGET_KINDS.contains(&t) {
        if ctx.allow_privileged {
            return Ok(());
        }
        return Err(format!(
            "\"{t}\" nodes cannot be placed in a widget tree directly: they are only \
             produced by the ui_{t} tool, which sanitizes/renders their contents \
             server-side. Call ui_{t} instead."
        ));
    }
    Err(format!(
        "unknown widget type \"{t}\"; use one of: {}",
        WIDGET_KINDS.join(", ")
    ))
}

fn validate_table(obj: &serde_json::Map<String, Value>) -> Result<(), String> {
    let cols = obj
        .get("columns")
        .and_then(Value::as_array)
        .ok_or_else(|| "\"table\" needs a \"columns\" array of strings".to_string())?;
    let rows = obj
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| "\"table\" needs a \"rows\" array of arrays".to_string())?;
    if rows.len() > MAX_TABLE_ROWS {
        return Err(format!(
            "\"table\" has {} rows; cap is {MAX_TABLE_ROWS} (aggregate or paginate)",
            rows.len()
        ));
    }
    for (i, row) in rows.iter().enumerate() {
        let cells = row
            .as_array()
            .ok_or_else(|| format!("\"table\" row {i} is not an array"))?;
        if cells.len() != cols.len() {
            return Err(format!(
                "\"table\" row {i} has {} cells but there are {} columns",
                cells.len(),
                cols.len()
            ));
        }
    }
    Ok(())
}

fn validate_widget_fields(
    obj: &serde_json::Map<String, Value>,
    t: &str,
    surface: &SurfaceDecl,
) -> Result<(), String> {
    match t {
        "markdown" => {
            let md = obj
                .get("md")
                .and_then(Value::as_str)
                .ok_or_else(|| "\"markdown\" needs a string \"md\"".to_string())?;
            let n = md.chars().count();
            if n > MAX_MARKDOWN_CHARS {
                return Err(format!(
                    "\"markdown\".md is {n} chars; cap is {MAX_MARKDOWN_CHARS}"
                ));
            }
        }
        "image" => {
            let src = obj
                .get("src")
                .and_then(Value::as_str)
                .ok_or_else(|| "\"image\" needs a string \"src\"".to_string())?;
            validate_image_src(src)?;
        }
        "kpi" => {
            if !obj.get("label").is_some_and(Value::is_string) {
                return Err("\"kpi\" needs a string \"label\"".to_string());
            }
            if !obj
                .get("value")
                .is_some_and(|v| v.is_string() || v.is_number())
            {
                return Err("\"kpi\" needs a string or number \"value\"".to_string());
            }
        }
        "log" => validate_log(obj)?,
        "plot" => validate_plot(obj.get("spec").unwrap_or(&Value::Null))?,
        "network" => validate_network(obj.get("spec").unwrap_or(&Value::Null))?,
        "component" => validate_component(obj, surface)?,
        "text" | "badge" => {
            if !obj.get("value").is_some_and(Value::is_string) {
                return Err(format!("\"{t}\" needs a string \"value\""));
            }
        }
        "stat" => {
            if !obj
                .get("value")
                .is_some_and(|v| v.is_string() || v.is_number())
            {
                return Err("\"stat\" needs a string or number \"value\"".to_string());
            }
        }
        "table" => validate_table(obj)?,
        "chart" => validate_chart(obj.get("spec").unwrap_or(&Value::Null))?,
        "graph" => validate_graph(obj.get("spec").unwrap_or(&Value::Null))?,
        "input" | "select" | "checkbox" => validate_form_control(obj, t)?,
        "button" => {
            if !obj.get("label").is_some_and(Value::is_string)
                || !obj.get("action").is_some_and(Value::is_string)
            {
                return Err("\"button\" needs string \"label\" and \"action\"".to_string());
            }
        }
        "progress" => {
            if !obj.get("value").is_some_and(Value::is_number) {
                return Err("\"progress\" needs a numeric \"value\" (0..1)".to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_form_control(obj: &serde_json::Map<String, Value>, t: &str) -> Result<(), String> {
    if !obj.get("name").is_some_and(Value::is_string) {
        return Err(format!("\"{t}\" needs a string \"name\" (the field key)"));
    }
    if t == "select" {
        let opts = obj
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| "\"select\" needs an \"options\" array".to_string())?;
        if opts.is_empty() {
            return Err("\"select\" needs at least one option".to_string());
        }
    }
    Ok(())
}

fn validate_widget_children(
    obj: &serde_json::Map<String, Value>,
    t: &str,
    depth: usize,
    budget: &mut usize,
    ctx: &WidgetCtx,
) -> Result<(), String> {
    if let Some(children) = obj.get("children") {
        let arr = children
            .as_array()
            .ok_or_else(|| format!("\"{t}\".children must be an array"))?;
        for child in arr {
            validate_widget(child, depth + 1, budget, ctx)?;
        }
    } else if matches!(t, "card" | "row" | "col" | "form") {
        return Err(format!("\"{t}\" needs a \"children\" array"));
    }
    Ok(())
}

/// Validate an agent-authored widget tree against what the SDK can render.
/// Returns a message aimed at the model (it will see it as a tool error and
/// retry), not at the user.
pub fn validate_widget(
    node: &Value,
    depth: usize,
    budget: &mut usize,
    ctx: &WidgetCtx,
) -> Result<(), String> {
    if depth > MAX_WIDGET_DEPTH {
        return Err(format!("widget tree nested deeper than {MAX_WIDGET_DEPTH}"));
    }
    *budget = budget
        .checked_sub(1)
        .ok_or_else(|| format!("widget tree exceeds {MAX_WIDGET_NODES} nodes"))?;

    let obj = node
        .as_object()
        .ok_or_else(|| "each widget node must be a JSON object".to_string())?;
    let t = obj
        .get("t")
        .and_then(Value::as_str)
        .ok_or_else(|| "each widget node needs a \"t\" (type) string".to_string())?;
    validate_widget_kind(t, ctx)?;

    validate_widget_fields(obj, t, ctx.surface)?;
    validate_widget_children(obj, t, depth, budget, ctx)
}

/// Coerce, validate, and return the widget array a tool was handed. `ctx` is
/// built by the calling tool: generic entry points (`ui_panel` / `ui_render`)
/// pass a non-privileged ctx, so `html`/`figure` nodes are rejected here.
fn checked_body(body: &Value, ctx: &WidgetCtx) -> Result<Value, ErrorData> {
    let body = unstringify(body);
    let nodes = body.as_array().ok_or_else(|| {
        invalid(want_json(
            "body",
            "an array of widget nodes",
            r#"[{"t":"text","value":"hello"}]"#,
        ))
    })?;
    if nodes.is_empty() {
        return Err(invalid("\"body\" must contain at least one widget node"));
    }
    let mut budget = MAX_WIDGET_NODES;
    for n in nodes {
        validate_widget(n, 0, &mut budget, ctx).map_err(invalid)?;
    }
    Ok(body)
}

/// The scheme of a URL, lowercased, or `None` for a relative URL (no scheme).
/// A scheme is `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`; anything else
/// before the first delimiter means the URL is relative.
fn url_scheme(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let mut scheme = match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => c.to_ascii_lowercase().to_string(),
        _ => return None,
    };
    for c in chars {
        if c == ':' {
            return Some(scheme);
        }
        if c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.' {
            scheme.push(c.to_ascii_lowercase());
        } else {
            return None; // hit '/', '?', '#', … before ':' → relative
        }
    }
    None
}

/// An `image.src` must be a `data:image/…` URL (≤ 512 KB), an `https:` URL, or a
/// relative path. `http:` and `javascript:` (and every other scheme) are refused
/// so a rendered image can neither leak referrers over cleartext nor execute.
fn validate_image_src(src: &str) -> Result<(), String> {
    let s = src.trim();
    if s.is_empty() {
        return Err("\"image\".src must not be empty".to_string());
    }
    match url_scheme(s) {
        None => Ok(()), // relative path
        Some(scheme) => match scheme.as_str() {
            "https" => Ok(()),
            "data" => {
                if !s.starts_with("data:image/") {
                    return Err(
                        "\"image\".src data URLs must be data:image/… (only images allowed)"
                            .to_string(),
                    );
                }
                if s.len() > MAX_IMAGE_DATA_BYTES {
                    return Err(format!(
                        "\"image\".src data URL is {} bytes; cap is {MAX_IMAGE_DATA_BYTES}",
                        s.len()
                    ));
                }
                Ok(())
            }
            other => Err(format!(
                "\"image\".src scheme \"{other}:\" is not allowed; use https:, a relative path, \
                 or a data:image/ URL"
            )),
        },
    }
}

/// A `log` node: `{lines: [{level?, text}] ≤ 500, max?}`.
fn validate_log(obj: &serde_json::Map<String, Value>) -> Result<(), String> {
    let lines = obj
        .get("lines")
        .and_then(Value::as_array)
        .ok_or_else(|| "\"log\" needs a \"lines\" array of {text, level?}".to_string())?;
    if lines.len() > MAX_LOG_LINES {
        return Err(format!(
            "\"log\" has {} lines; cap is {MAX_LOG_LINES}",
            lines.len()
        ));
    }
    for (i, ln) in lines.iter().enumerate() {
        let lo = ln
            .as_object()
            .ok_or_else(|| format!("\"log\".lines[{i}] must be an object"))?;
        if !lo.get("text").is_some_and(Value::is_string) {
            return Err(format!("\"log\".lines[{i}] needs a string \"text\""));
        }
        if lo.get("level").is_some_and(|v| !v.is_string()) {
            return Err(format!("\"log\".lines[{i}].level must be a string"));
        }
    }
    if obj.get("max").is_some_and(|v| !v.is_number()) {
        return Err("\"log\".max must be a number".to_string());
    }
    Ok(())
}

/// A `plot` node's spec: like `chart` but with more types and a 2000-point cap.
/// `box` carries `series:[{label, values:[…]}]`; `heatmap` carries
/// `{x:[], y:[], z:[[]]}`; the rest carry `data:[…]` or `series:[{name, data}]`.
fn validate_plot(spec: &Value) -> Result<(), String> {
    let obj = spec.as_object().ok_or_else(|| {
        want_json(
            "spec",
            "a plot object",
            r#"{"type":"scatter","title":"…","data":[{"label":"A","value":1}]}"#,
        )
    })?;
    let ty = obj.get("type").and_then(Value::as_str).unwrap_or("bar");
    const PLOT_TYPES: &[&str] = &["bar", "line", "pie", "scatter", "area", "box", "heatmap"];
    if !PLOT_TYPES.contains(&ty) {
        return Err(format!(
            "plot type \"{ty}\" must be one of: {}",
            PLOT_TYPES.join(", ")
        ));
    }
    match ty {
        "box" => {
            let series = obj.get("series").and_then(Value::as_array).ok_or_else(|| {
                "box plot needs a \"series\" array of {label, values:[…]}".to_string()
            })?;
            if series.is_empty() {
                return Err("box plot \"series\" is empty".to_string());
            }
            let mut total = 0usize;
            for (i, s) in series.iter().enumerate() {
                let so = s
                    .as_object()
                    .ok_or_else(|| format!("box plot series[{i}] must be an object"))?;
                if !so.get("label").is_some_and(Value::is_string) {
                    return Err(format!("box plot series[{i}] needs a string \"label\""));
                }
                let values = so.get("values").and_then(Value::as_array).ok_or_else(|| {
                    format!("box plot series[{i}] needs a \"values\" number array")
                })?;
                for (j, v) in values.iter().enumerate() {
                    if v.as_f64().map(f64::is_finite) != Some(true) {
                        return Err(format!(
                            "box plot series[{i}].values[{j}] must be a finite number"
                        ));
                    }
                }
                total += values.len();
            }
            if total > MAX_PLOT_POINTS {
                return Err(format!(
                    "box plot has {total} values; cap is {MAX_PLOT_POINTS}"
                ));
            }
        }
        "heatmap" => {
            let x = obj
                .get("x")
                .and_then(Value::as_array)
                .ok_or_else(|| "heatmap needs an \"x\" axis array".to_string())?;
            let y = obj
                .get("y")
                .and_then(Value::as_array)
                .ok_or_else(|| "heatmap needs a \"y\" axis array".to_string())?;
            let z = obj
                .get("z")
                .and_then(Value::as_array)
                .ok_or_else(|| "heatmap needs a \"z\" array of rows".to_string())?;
            if z.len() != y.len() {
                return Err(format!(
                    "heatmap \"z\" has {} rows but \"y\" has {}",
                    z.len(),
                    y.len()
                ));
            }
            let mut total = 0usize;
            for (i, row) in z.iter().enumerate() {
                let cells = row
                    .as_array()
                    .ok_or_else(|| format!("heatmap \"z\"[{i}] must be an array"))?;
                if cells.len() != x.len() {
                    return Err(format!(
                        "heatmap \"z\"[{i}] has {} cells but \"x\" has {}",
                        cells.len(),
                        x.len()
                    ));
                }
                for (j, c) in cells.iter().enumerate() {
                    if !c.is_number() && !c.is_null() {
                        return Err(format!("heatmap \"z\"[{i}][{j}] must be a number or null"));
                    }
                }
                total += cells.len();
            }
            if total > MAX_PLOT_POINTS {
                return Err(format!(
                    "heatmap has {total} cells; cap is {MAX_PLOT_POINTS}"
                ));
            }
        }
        _ => validate_plot_xy(obj)?,
    }
    Ok(())
}

/// Point-count + shape check shared by the `bar|line|pie|scatter|area` plot
/// types: either `series:[{name, data:[…]}]` or a single `data:[…]`, each point
/// an object; total points capped at [`MAX_PLOT_POINTS`].
fn validate_plot_xy(obj: &serde_json::Map<String, Value>) -> Result<(), String> {
    let count_points = |data: &[Value], where_: &str| -> Result<usize, String> {
        if data.is_empty() {
            return Err(format!("plot {where_} is empty"));
        }
        for (i, p) in data.iter().enumerate() {
            if !p.is_object() {
                return Err(format!("plot {where_}[{i}] must be an object"));
            }
        }
        Ok(data.len())
    };
    let total = if let Some(series) = obj.get("series").and_then(Value::as_array) {
        if series.is_empty() {
            return Err("plot \"series\" is empty".to_string());
        }
        if series.len() > 12 {
            return Err(format!("plot has {} series; cap is 12", series.len()));
        }
        let mut t = 0usize;
        for (si, s) in series.iter().enumerate() {
            let so = s
                .as_object()
                .ok_or_else(|| format!("plot series[{si}] must be an object with \"data\""))?;
            let data = so
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("plot series[{si}] needs a \"data\" array"))?;
            t += count_points(data, &format!("series[{si}].data"))?;
        }
        t
    } else {
        let data = obj.get("data").and_then(Value::as_array).ok_or_else(|| {
            "plot spec needs a \"data\" array (or \"series\":[{name, data}])".to_string()
        })?;
        count_points(data, "data")?
    };
    if total > MAX_PLOT_POINTS {
        return Err(format!("plot has {total} points; cap is {MAX_PLOT_POINTS}"));
    }
    Ok(())
}

/// A `network` node's spec: bounded node/edge counts, unique node ids, and every
/// edge endpoint resolving to a declared node.
fn validate_network(spec: &Value) -> Result<(), String> {
    let obj = spec.as_object().ok_or_else(|| {
        want_json(
            "spec",
            "a network object",
            r#"{"nodes":[{"id":"A"}],"edges":[{"source":"A","target":"B"}]}"#,
        )
    })?;
    let nodes = obj
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "network spec needs a \"nodes\" array".to_string())?;
    if nodes.len() > MAX_NETWORK_NODES {
        return Err(format!(
            "network has {} nodes; cap is {MAX_NETWORK_NODES}",
            nodes.len()
        ));
    }
    let mut ids: HashSet<&str> = HashSet::with_capacity(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        let no = n
            .as_object()
            .ok_or_else(|| format!("network nodes[{i}] must be an object"))?;
        let id = no
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("network nodes[{i}] needs a string \"id\""))?;
        if !ids.insert(id) {
            return Err(format!(
                "network nodes[{i}] repeats id \"{id}\"; node ids must be unique"
            ));
        }
    }
    let edges = obj
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| "network spec needs an \"edges\" array".to_string())?;
    if edges.len() > MAX_NETWORK_EDGES {
        return Err(format!(
            "network has {} edges; cap is {MAX_NETWORK_EDGES}",
            edges.len()
        ));
    }
    for (i, e) in edges.iter().enumerate() {
        let eo = e
            .as_object()
            .ok_or_else(|| format!("network edges[{i}] must be an object"))?;
        let src = eo
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("network edges[{i}] needs a string \"source\""))?;
        let tgt = eo
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("network edges[{i}] needs a string \"target\""))?;
        if !ids.contains(src) {
            return Err(format!(
                "network edges[{i}].source \"{src}\" is not a declared node id"
            ));
        }
        if !ids.contains(tgt) {
            return Err(format!(
                "network edges[{i}].target \"{tgt}\" is not a declared node id"
            ));
        }
    }
    // `encoding` / `physics` are optional shaping hints; only assert container type.
    if obj.get("encoding").is_some_and(|v| !v.is_object()) {
        return Err("network \"encoding\" must be an object".to_string());
    }
    if obj.get("physics").is_some_and(|v| !v.is_object()) {
        return Err("network \"physics\" must be an object".to_string());
    }
    Ok(())
}

/// A `component` node: `{name, props}` where `name` matches a declared
/// [`ComponentDecl`](super::manifest::ComponentDecl) and `props` validates
/// against that declaration's JSON Schema. **Fail-closed:** a declared schema
/// that itself fails to compile is treated as a validation failure that tells
/// the author to fix the manifest — never an accept-any fallback.
fn validate_component(
    obj: &serde_json::Map<String, Value>,
    surface: &SurfaceDecl,
) -> Result<(), String> {
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "\"component\" needs a string \"name\"".to_string())?;
    let decl = surface
        .components
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| {
            let known: Vec<&str> = surface.components.iter().map(|c| c.name.as_str()).collect();
            if known.is_empty() {
                format!(
                    "component \"{name}\" is not declared; this app registers no custom components"
                )
            } else {
                format!(
                    "component \"{name}\" is not declared; declared components: {}",
                    known.join(", ")
                )
            }
        })?;
    let props = obj.get("props").cloned().unwrap_or_else(|| json!({}));
    validate_against_schema(&props, &decl.props, name)
}

/// Validate `value` against a JSON Schema. An empty/absent schema (`{}` or
/// `null`) is unconstrained. A schema that fails to compile is a fail-closed
/// error naming the manifest fix, not a pass.
fn validate_against_schema(value: &Value, schema: &Value, comp: &str) -> Result<(), String> {
    let unconstrained =
        schema.is_null() || schema.as_object().is_some_and(serde_json::Map::is_empty);
    if unconstrained {
        return Ok(());
    }
    let validator = jsonschema::validator_for(schema).map_err(|e| {
        format!(
            "component \"{comp}\" has an invalid props schema in the app manifest. Fix the \
             declared schema (it failed to compile: {e})"
        )
    })?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|err| {
            let path = err.instance_path.to_string();
            if path.is_empty() {
                err.to_string()
            } else {
                format!("{path}: {err}")
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "component \"{comp}\" props do not match its declared schema: {}",
            errors.join("; ")
        ))
    }
}

fn validate_chart(spec: &Value) -> Result<(), String> {
    let obj = spec.as_object().ok_or_else(|| {
        want_json(
            "spec",
            "a chart object",
            r#"{"type":"bar","title":"Counts","data":[{"label":"A","value":1}]}"#,
        )
    })?;
    if let Some(t) = obj.get("type").and_then(Value::as_str) {
        if !["bar", "line", "pie"].contains(&t) {
            return Err(format!("chart type \"{t}\" must be bar, line, or pie"));
        }
    }
    // Validate one series' data array (the shared inner shape).
    let check_data = |data: &Vec<Value>, where_: &str| -> Result<(), String> {
        if data.is_empty() {
            return Err(format!("chart {where_} is empty"));
        }
        if data.len() > MAX_CHART_POINTS {
            return Err(format!(
                "chart {where_} has {} points; cap is {MAX_CHART_POINTS}",
                data.len()
            ));
        }
        for (i, p) in data.iter().enumerate() {
            let po = p
                .as_object()
                .ok_or_else(|| format!("chart {where_}[{i}] must be an object"))?;
            if !po.get("label").is_some_and(Value::is_string) {
                return Err(format!("chart {where_}[{i}] needs a string \"label\""));
            }
            let v = po
                .get("value")
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("chart {where_}[{i}] needs a numeric \"value\""))?;
            if !v.is_finite() {
                return Err(format!("chart {where_}[{i}].value must be finite"));
            }
        }
        Ok(())
    };
    // Multi-series (`series:[{name,data}]`) OR single (`data:[…]`).
    if let Some(series) = obj.get("series").and_then(Value::as_array) {
        if series.is_empty() {
            return Err("chart \"series\" is empty".to_string());
        }
        if series.len() > 12 {
            return Err(format!("chart has {} series; cap is 12", series.len()));
        }
        for (si, s) in series.iter().enumerate() {
            let so = s
                .as_object()
                .ok_or_else(|| format!("chart series[{si}] must be an object with \"data\""))?;
            let data = so.get("data").and_then(Value::as_array).ok_or_else(|| {
                format!("chart series[{si}] needs a \"data\" array of {{label, value}}")
            })?;
            check_data(data, &format!("series[{si}].data"))?;
        }
        return Ok(());
    }
    let data = obj.get("data").and_then(Value::as_array).ok_or_else(|| {
        "chart spec needs a \"data\" array of {label, value} (or \"series\":[{name, data}] for multiple lines)"
            .to_string()
    })?;
    check_data(data, "data")
}

fn validate_graph(spec: &Value) -> Result<(), String> {
    let obj = spec.as_object().ok_or_else(|| {
        want_json(
            "spec",
            "a graph object",
            r#"{"nodes":[{"id":"A"}],"edges":[{"source":"A","target":"B"}]}"#,
        )
    })?;
    let nodes = obj.get("nodes").and_then(Value::as_array);
    let edges = obj.get("edges").and_then(Value::as_array);
    if nodes.is_none() && edges.is_none() {
        return Err("graph spec needs \"nodes\" and/or \"edges\"".to_string());
    }
    if let Some(edges) = edges {
        for (i, e) in edges.iter().enumerate() {
            if e.is_string() {
                continue; // "A -> B : label" edge-list form
            }
            let eo = e
                .as_object()
                .ok_or_else(|| format!("graph edges[{i}] must be an object or an edge string"))?;
            if !eo.get("source").is_some_and(Value::is_string)
                || !eo.get("target").is_some_and(Value::is_string)
            {
                return Err(format!(
                    "graph edges[{i}] needs string \"source\" and \"target\""
                ));
            }
        }
    }
    Ok(())
}

/// A `target` selector accepted by `ui_render` / `ui_chart` / `ui_highlight`.
/// `@region:<name>` / `@panel:<id>` / `@chat` / `@main` are SDK-resolved aliases;
/// anything else is treated as a CSS selector by the client.
fn validate_target(target: &str) -> Result<(), ErrorData> {
    let t = target.trim();
    if t.is_empty() {
        return Err(invalid("\"target\" must not be empty"));
    }
    if t.len() > 200 {
        return Err(invalid("\"target\" is unreasonably long"));
    }
    Ok(())
}

/// The fixable error for a `ui_patch` op naming an id that isn't in the registry:
/// list up to 20 of the known ids so the model can pick a real one.
fn unknown_id_msg(i: usize, id: &str, reg: &HashMap<String, Value>) -> String {
    let mut known: Vec<&str> = reg.keys().map(String::as_str).collect();
    known.sort_unstable();
    if known.is_empty() {
        return format!("ops[{i}]: unknown id \"{id}\"; no instances exist yet, so add one first");
    }
    let shown: Vec<&str> = known.iter().take(20).copied().collect();
    let more = known.len().saturating_sub(shown.len());
    let suffix = if more > 0 {
        format!(" (+{more} more)")
    } else {
        String::new()
    };
    format!(
        "ops[{i}]: unknown id \"{id}\"; known ids: {}{suffix}",
        shown.join(", ")
    )
}

// ---------------------------------------------------------------------------
// HTML sanitization (ui_html — capability-gated, fail-closed; design §3.7)
// ---------------------------------------------------------------------------

/// Sanitize a block of author/agent-supplied HTML with a **pinned, fail-closed**
/// allowlist and return `(clean, removed_element_count)`.
///
/// Policy (an XSS barrier, not cosmetic):
/// - a common rich-text tag set + table + figure + basic inline SVG; everything
///   else (`script`, `style`, `form`, `input`, `button`, `iframe`, `object`,
///   `embed`, `link`, `meta`, `base`, …) is dropped, and the genuinely dangerous
///   ones also have their *content* removed (`clean_content_tags`);
/// - **no** event-handler attributes (ammonia never allows `on*`), no inline
///   `style`;
/// - absolute URLs restricted to `https:` / `mailto:` (`url_schemes`); relative
///   URLs pass through; **all** `data:` URLs are dropped — ammonia's scheme
///   filter is global, so rather than risk a `data:text/html` bypass we fail
///   closed and forbid `data:` here (an app that needs inline images uses the
///   `image` widget kind, which validates `data:image/` itself);
/// - `rel="noopener noreferrer"` forced on links (ammonia default).
fn sanitize_html(raw: &str) -> (String, usize) {
    let tags: HashSet<&str> = [
        "a",
        "abbr",
        "b",
        "blockquote",
        "br",
        "caption",
        "cite",
        "code",
        "col",
        "colgroup",
        "dd",
        "del",
        "details",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "ins",
        "kbd",
        "li",
        "mark",
        "ol",
        "p",
        "pre",
        "q",
        "s",
        "samp",
        "section",
        "small",
        "span",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "time",
        "tr",
        "u",
        "ul",
        "wbr",
        "img", // basic inline SVG
        "svg",
        "g",
        "path",
        "circle",
        "rect",
        "line",
        "polyline",
        "polygon",
        "ellipse",
        "text",
    ]
    .into_iter()
    .collect();
    // Tags whose entire *content* is discarded (not just the tag unwrapped),
    // because their text is executable or a data-exfiltration vector.
    let clean_content: HashSet<&str> = [
        "script", "style", "iframe", "object", "embed", "form", "noscript", "template",
    ]
    .into_iter()
    .collect();
    let schemes: HashSet<&str> = ["https", "mailto"].into_iter().collect();

    let clean = ammonia::Builder::default()
        .tags(tags)
        .clean_content_tags(clean_content)
        .url_schemes(schemes)
        .clean(raw)
        .to_string();

    let removed = count_start_tags(raw).saturating_sub(count_start_tags(&clean));
    (clean, removed)
}

/// Count HTML start tags (`<` immediately followed by an ASCII letter). A rough
/// proxy for "how many elements" so `ui_html` can report how much it stripped.
fn count_start_tags(s: &str) -> usize {
    let b = s.as_bytes();
    let mut n = 0usize;
    for i in 0..b.len().saturating_sub(1) {
        if b[i] == b'<' && b[i + 1].is_ascii_alphabetic() {
            n += 1;
        }
    }
    n
}

// ---------------------------------------------------------------------------
// Tool parameters
// ---------------------------------------------------------------------------

// Schema-only mirrors. The runtime types stay `serde_json::Value` (so we can
// also accept a stringified object — see `unstringify`), but a bare `Value`
// generates a permissive `true` schema that tells the model nothing. Attaching
// these via `#[schemars(with = ...)]` gives it the real shape up front.

/// Schema for `ui_chart.spec`.
#[derive(JsonSchema)]
#[schemars(inline)]
#[allow(dead_code)]
struct ChartSpecSchema {
    /// "bar" (default), "line", or "pie".
    r#type: Option<String>,
    title: Option<String>,
    data: Vec<ChartPointSchema>,
}

#[derive(JsonSchema)]
#[schemars(inline)]
#[allow(dead_code)]
struct ChartPointSchema {
    label: String,
    value: f64,
}

/// Schema for `ui_graph.spec`.
#[derive(JsonSchema)]
#[schemars(inline)]
#[allow(dead_code)]
struct GraphSpecSchema {
    title: Option<String>,
    nodes: Option<Vec<GraphNodeSchema>>,
    edges: Option<Vec<GraphEdgeSchema>>,
}

#[derive(JsonSchema)]
#[schemars(inline)]
#[allow(dead_code)]
struct GraphNodeSchema {
    id: String,
    label: Option<String>,
    group: Option<String>,
}

#[derive(JsonSchema)]
#[schemars(inline)]
#[allow(dead_code)]
struct GraphEdgeSchema {
    source: String,
    target: String,
    label: Option<String>,
}

/// Schema for a `body` widget node. `children` stays untyped because the tree is
/// recursive; the tool description carries the full node grammar.
#[derive(JsonSchema)]
#[schemars(inline)]
#[allow(dead_code)]
struct WidgetNodeSchema {
    /// One of: card, row, col, text, badge, table, chart, graph, stat, divider,
    /// progress, input, select, checkbox, button, form.
    t: String,
    title: Option<String>,
    value: Option<Value>,
    label: Option<String>,
    children: Option<Vec<Value>>,
    spec: Option<Value>,
    columns: Option<Vec<String>>,
    rows: Option<Vec<Vec<Value>>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PanelParams {
    /// Stable panel id. Calling `ui_panel` again with the same id REPLACES that
    /// panel's contents (so you can refresh a dashboard in place).
    pub id: String,
    /// Heading shown on the panel.
    #[serde(default)]
    pub title: Option<String>,
    /// Where to mount it. Either an SDK dock slot — "dock" (default; a right-hand
    /// drawer the SDK always provides), "left", "right", "bottom", "main", or
    /// "modal" — OR a target that names part of the author's page:
    /// "@region:<name>" (an author-declared `data-br-region`), "@panel:<id>", or
    /// a CSS selector. Use a `@region:` target to drop a titled dashboard card
    /// straight into a region the app author laid out.
    #[serde(default)]
    pub place: Option<String>,
    /// The panel contents: an array of widget nodes. See the tool description
    /// for the node grammar.
    #[serde(default)]
    #[schemars(with = "Option<Vec<WidgetNodeSchema>>")]
    pub body: Option<Value>,
    /// Shorthand alternative to `body`: render this markdown as the panel body.
    #[serde(default)]
    pub markdown: Option<String>,
    /// Let the user collapse the panel. Default true.
    #[serde(default)]
    pub collapsible: Option<bool>,
    /// Remove the panel with this id instead of creating/replacing it.
    #[serde(default)]
    pub remove: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RenderParams {
    /// Where to render: `@region:<name>` (an author-declared
    /// `data-br-region="<name>"`), `@panel:<id>`, `@chat`, `@main`, or a CSS
    /// selector like `#results`. Call `ui_describe` to see what exists.
    pub target: String,
    /// Array of widget nodes to render.
    #[schemars(with = "Vec<WidgetNodeSchema>")]
    pub body: Value,
    /// "replace" (default) or "append".
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ChartParams {
    /// Where to draw it (see `ui_render.target`). Defaults to a dock panel.
    #[serde(default)]
    pub target: Option<String>,
    /// Panel id when `target` is omitted (so repeat calls replace, not stack).
    #[serde(default)]
    pub id: Option<String>,
    /// `{"type":"bar"|"line"|"pie", "title":"…", "data":[{"label":"A","value":1}]}`
    #[schemars(with = "ChartSpecSchema")]
    pub spec: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GraphParams {
    /// Where to draw it (see `ui_render.target`). Defaults to a dock panel.
    #[serde(default)]
    pub target: Option<String>,
    /// Panel id when `target` is omitted.
    #[serde(default)]
    pub id: Option<String>,
    /// `{"title":"…","nodes":[{"id":"A"}],"edges":[{"source":"A","target":"B","label":"binds"}]}`
    #[schemars(with = "GraphSpecSchema")]
    pub spec: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct HighlightParams {
    /// What to highlight (see `ui_render.target`).
    pub target: String,
    /// "outline" (default), "pulse", "focus" (dim everything else), or "clear".
    #[serde(default)]
    pub mode: Option<String>,
    /// A short callout rendered next to the highlighted element.
    #[serde(default)]
    pub note: Option<String>,
    /// Scroll the element into view. Default true.
    #[serde(default)]
    pub scroll: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ThemeParams {
    /// Switch the whole theme pack: "biorouter" (base), "clinical",
    /// "lab-notebook", "terminal", "journal", or "midnight".
    #[serde(default)]
    pub pack: Option<String>,
    /// Accent colour as a CSS colour (e.g. "#2f6f4e", "tomato").
    #[serde(default)]
    pub accent: Option<String>,
    /// "light", "dark", or "auto".
    #[serde(default)]
    pub mode: Option<String>,
    /// "comfortable" (default) or "compact".
    #[serde(default)]
    pub density: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LayoutParams {
    /// A named preset (an alias over the grammar): "single" (default),
    /// "sidebar-right", "sidebar-left", "split", or "dashboard" (a responsive
    /// grid). Supply this OR `areas`.
    #[serde(default)]
    pub preset: Option<String>,
    /// Sidebar width in CSS px (sidebar presets only).
    #[serde(default)]
    pub sidebar_width: Option<u32>,
    /// A `grid-template-areas` grid: rows of area names, e.g.
    /// `[["nav","nav"],["side","main"]]`. At most 4 rows × 4 columns; every row
    /// must have the same number of columns. Each cell is a lowercase area name
    /// (`^[a-z][a-z0-9_-]{0,23}$`) or `"."` for an empty cell. Author regions
    /// mount into an area via `@region:<area>`.
    #[serde(default)]
    pub areas: Option<Vec<Vec<String>>>,
    /// Track sizes keyed by area/track name, from a bounded vocabulary:
    /// `"<80-800>px"`, `"<5-95>%"`, `"<1-6>fr"`, or `"auto"`.
    #[serde(default)]
    pub sizes: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct NotifyParams {
    /// The message to show.
    pub message: String,
    /// "info" (default), "success", "warn", or "error".
    #[serde(default)]
    pub level: Option<String>,
    /// Auto-dismiss after this many ms (default 4000; 0 = sticky).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// One `ui_suggest` chip: a short label plus an optional prompt sent on click.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(inline)]
pub struct SuggestChip {
    /// Short button label (≤80 chars).
    pub label: String,
    /// Prompt sent when the chip is tapped (≤500 chars). Omit it to hand the
    /// click to the app's own `onCommand` listener as a synthetic `suggest`
    /// command carrying the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SuggestParams {
    /// Up to five suggestion chips to show as a dismissible row.
    pub chips: Vec<SuggestChip>,
    /// Where to mount the row (e.g. `@region:results`). Omit for the docked
    /// suggestion host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StateParams {
    /// Keys to set/overwrite in the app's shared state document (merged at the root).
    #[serde(default)]
    pub set: Option<Value>,
    /// Top-level keys to delete.
    #[serde(default)]
    pub remove: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PatchStateParams {
    /// An RFC-6902 JSON Patch: an array of operation objects, e.g.
    /// `[{"op":"add","path":"/cohort/count","value":42}]`. Capped at 64 ops.
    pub patch: Value,
}

/// Schema mirror for one `ui_patch` op (so the model sees the real shape).
#[derive(JsonSchema)]
#[schemars(inline)]
#[allow(dead_code)]
struct PatchOpSchema {
    /// "add", "replace", "set_props", or "remove".
    op: String,
    /// The instance id this op targets (non-empty, ≤ 64 chars).
    id: String,
    /// For op:"add": where to mount — "@region:x", "@panel:x", or a CSS
    /// selector. Defaults to the app's main results region.
    target: Option<String>,
    /// For op:"add": instance id of an existing node to append into.
    parent: Option<String>,
    /// For op:"add": insertion index within the parent/target.
    index: Option<i64>,
    /// For op:"add"/"replace": the widget node.
    node: Option<Value>,
    /// For op:"set_props": keys shallow-merged into the existing node.
    props: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PatchParams {
    /// The edit operations to apply (≤ 32), each one of:
    /// {"op":"add","id":"n1","target":"@region:results","node":{…}},
    /// {"op":"replace","id":"n1","node":{…}},
    /// {"op":"set_props","id":"n1","props":{…}},
    /// {"op":"remove","id":"n1"}.
    #[schemars(with = "Vec<PatchOpSchema>")]
    pub ops: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct HtmlParams {
    /// Where to render (see `ui_render.target`): "@region:x", "@panel:x", or a
    /// CSS selector.
    pub target: String,
    /// The HTML to sanitize and render (≤ 64 KB). Scripts, styles, forms,
    /// iframes, event handlers, and non-https/mailto URLs are stripped
    /// server-side before anything reaches the page.
    pub html: String,
    /// Optional heading — renders the HTML inside a titled card.
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FigureParams {
    /// The Auto Visualiser tool to call, e.g. "render_volcano",
    /// "render_kaplan_meier", "render_sankey", "render_dashboard".
    pub tool: String,
    /// That tool's exact arguments object.
    #[schemars(with = "Value")]
    pub args: Value,
    /// Where to place it (see `ui_render.target`). Omit for a dock panel.
    #[serde(default)]
    pub target: Option<String>,
    /// Optional panel title / caption.
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[schemars(inline)]
pub struct AskField {
    /// Key this field's answer appears under in the returned payload.
    pub name: String,
    /// Visible label.
    #[serde(default)]
    pub label: Option<String>,
    /// "text" (default), "number", "textarea", "select", or "checkbox".
    #[serde(default)]
    pub r#type: Option<String>,
    /// Options for `type: "select"`, as plain strings.
    #[serde(default)]
    pub options: Option<Vec<String>>,
    /// Pre-filled value.
    #[serde(default)]
    pub value: Option<String>,
    /// Placeholder text.
    #[serde(default)]
    pub placeholder: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AskParams {
    /// Question / instructions shown above the form.
    pub prompt: String,
    /// The fields the user fills in.
    pub fields: Vec<AskField>,
    /// Form heading.
    #[serde(default)]
    pub title: Option<String>,
    /// Submit button label (default "Submit").
    #[serde(default)]
    pub submit_label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DescribeParams {}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AppCallParams {
    /// The declared app action to invoke (must match a name in
    /// `surface.actions` — call `ui_describe` to list them).
    pub action: String,
    /// Arguments object for the action, validated against its declared params
    /// schema. Omit (or `{}`) for an action that takes none.
    #[serde(default)]
    #[schemars(with = "Value")]
    pub args: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EmitResultParams {
    /// The structured result object to deliver to the app's pending structured
    /// call. Validated against that call's output schema when one is in force.
    #[schemars(with = "Value")]
    pub result: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SubscribeParams {
    /// The declared signal names to subscribe to. REPLACES the current set.
    pub signals: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ConsultParams {
    /// The worker profile to consult (a name from the app's `ready.profiles`).
    pub agent: String,
    /// A self-contained question for that profile to answer independently.
    pub prompt: String,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// The `ui_*` tool server for one app session.
#[derive(Clone)]
pub struct AppControlServer {
    tool_router: ToolRouter<Self>,
    bridge: UiBridge,
    cap: UiCapability,
    /// The app's declared surface (state schema, actions, signals, components),
    /// reported to the model via `ui_describe`.
    surface: SurfaceDecl,
    /// Whether the `consult` tool may reach worker profiles (design §3.8). TRUE
    /// only for the MAIN agent's server when the app declares ≥1 valid profile;
    /// FALSE for a worker's server (a worker cannot consult — depth is 1) and for
    /// any app that declares no profiles.
    consult_enabled: bool,
}

impl AppControlServer {
    pub fn new(bridge: UiBridge, cap: UiCapability, surface: SurfaceDecl) -> Self {
        Self::new_with_consult(bridge, cap, surface, false)
    }

    /// Like [`new`](Self::new) but sets whether the `consult` tool is live. The
    /// app socket loop passes `true` for the MAIN agent when the manifest declares
    /// worker profiles; workers and profile-less apps get `false`.
    pub fn new_with_consult(
        bridge: UiBridge,
        cap: UiCapability,
        surface: SurfaceDecl,
        consult_enabled: bool,
    ) -> Self {
        // Mirror the declared surface onto the bridge so its server-facing signal
        // accessors (signal_decl / validate_signal) can resolve declarations
        // without holding this server — no apps.rs wiring required.
        bridge.set_surface_decl(surface.clone());
        Self {
            tool_router: Self::tool_router(),
            bridge,
            cap,
            surface,
            consult_enabled,
        }
    }

    fn denied(what: &str) -> ErrorData {
        invalid(format!(
            "this app does not grant {what}; ask the user to enable it in the app's manifest"
        ))
    }

    /// The app's declared surface, shaped for `ui_describe`: the actions the
    /// agent may call, the signals it may subscribe to, the custom components,
    /// and whether a state schema is declared.
    fn declared_surface(&self) -> Value {
        let s = &self.surface;
        json!({
            "actions": s.actions.iter().map(|a| json!({
                "name": a.name,
                "description": a.description,
                "params": a.params,
            })).collect::<Vec<_>>(),
            "signals": s.signals.iter().map(|sig| json!({
                "name": sig.name,
                "payload": sig.payload,
                "coalesceMs": sig.coalesce_ms,
            })).collect::<Vec<_>>(),
            "components": s.components.iter().map(|c| json!({
                "name": c.name,
                "props": c.props,
            })).collect::<Vec<_>>(),
            "hasStateSchema": s.state_schema.is_some(),
        })
    }

    /// Track a panel id, evicting the oldest when over the cap so a runaway loop
    /// can't paper the window with panels.
    fn note_panel(&self, id: &str) -> Option<String> {
        let mut panels = self.bridge.inner.panels.lock().ok()?;
        if panels.iter().any(|p| p == id) {
            return None;
        }
        panels.push(id.to_string());
        if panels.len() > self.cap.max_panels {
            return Some(panels.remove(0));
        }
        None
    }

    fn drop_panel(&self, id: &str) {
        if let Ok(mut panels) = self.bridge.inner.panels.lock() {
            panels.retain(|p| p != id);
        }
    }

    /// Build the validation context for a tool. `allow_privileged` is true only
    /// for `ui_html` / `ui_figure`, which construct `html`/`figure` nodes after
    /// sanitizing/rendering; every generic entry point passes false.
    fn widget_ctx(&self, allow_privileged: bool) -> WidgetCtx<'_> {
        WidgetCtx {
            allow_privileged,
            surface: &self.surface,
        }
    }

    /// Assign a stable id to every node in an (already-validated) body, register
    /// each in the instance registry, and return the assigned ids. Nodes with a
    /// non-empty string `id` keep it; the rest get `<base>#n<idx>` where `idx` is
    /// the node's pre-order position, so `ui_patch` can later target any of them.
    /// The body is mutated in place, so the emitted frame is fully ID-keyed.
    fn assign_and_register(&self, base: &str, body: &mut Value) -> Vec<String> {
        fn walk(base: &str, node: &mut Value, counter: &mut usize, out: &mut Vec<(String, Value)>) {
            let idx = *counter;
            *counter += 1;
            let id = {
                let Some(obj) = node.as_object_mut() else {
                    return;
                };
                match obj.get("id").and_then(Value::as_str) {
                    Some(existing) if !existing.trim().is_empty() => existing.to_string(),
                    _ => {
                        let auto = format!("{base}#n{idx}");
                        obj.insert("id".to_string(), json!(auto));
                        auto
                    }
                }
            };
            // Re-borrow to recurse (ends the `obj` borrow) so we can snapshot the
            // node afterwards.
            if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
                for child in children.iter_mut() {
                    walk(base, child, counter, out);
                }
            }
            out.push((id, node.clone()));
        }

        let mut counter = 0usize;
        let mut pairs: Vec<(String, Value)> = Vec::new();
        if let Some(arr) = body.as_array_mut() {
            for node in arr.iter_mut() {
                walk(base, node, &mut counter, &mut pairs);
            }
        }
        let ids: Vec<String> = pairs.iter().map(|(id, _)| id.clone()).collect();
        if let Ok(mut reg) = self.bridge.inner.instances.lock() {
            for (id, node) in pairs {
                reg.insert(id, node);
            }
        }
        ids
    }

    /// Append the "targetable ids" hint to a tool result. Capped so a 600-node
    /// render doesn't flood the transcript.
    fn ids_hint(ids: &[String]) -> String {
        if ids.is_empty() {
            return String::new();
        }
        const SHOWN: usize = 40;
        let head: Vec<&str> = ids.iter().take(SHOWN).map(String::as_str).collect();
        let mut s = format!(
            " Node ids you can target with ui_patch: {}",
            head.join(", ")
        );
        if ids.len() > SHOWN {
            s.push_str(&format!(" (+{} more)", ids.len() - SHOWN));
        }
        s.push('.');
        s
    }
}

#[tool_router(router = tool_router)]
impl AppControlServer {
    #[tool(
        name = "ui_describe",
        description = "Inspect the app's live UI: which regions the author declared \
                       (data-br-region), which element ids exist, which agent panels are \
                       currently mounted, and the shared state bag. Call this FIRST if you \
                       intend to render into an existing part of the page."
    )]
    pub async fn ui_describe(
        &self,
        Parameters(_): Parameters<DescribeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let surface = self
            .bridge
            .inner
            .surface
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or_else(|| json!({ "regions": [], "ids": [] }));
        let panels = self
            .bridge
            .inner
            .panels
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default();
        let (doc, version) = self.bridge.state_snapshot();
        let keys: Vec<String> = doc
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        // Live instance registry (id → kind), deterministic order, ≤ 100 shown, so
        // the model knows which node ids it can target with ui_patch.
        let instances: Vec<Value> = self
            .bridge
            .inner
            .instances
            .lock()
            .map(|m| {
                let mut v: Vec<Value> = m
                    .iter()
                    .map(|(id, node)| {
                        let kind = node.get("t").and_then(Value::as_str).unwrap_or("?");
                        json!({ "id": id, "kind": kind })
                    })
                    .collect();
                v.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
                v.truncate(100);
                v
            })
            .unwrap_or_default();
        let out = json!({
            "surface": surface,
            "panels": panels,
            "instances": instances,
            "state": {
                "version": version,
                "keys": keys,
            },
            "declared": self.declared_surface(),
            "subscribed": self.bridge.subscribed_signals(),
            "allowed": {
                "theme": self.cap.allow_theme,
                "layout": self.cap.allow_layout,
                "ask": self.cap.allow_ask,
                "html": self.cap.allow_html,
                "signals": self.cap.allow_signals,
                "autorun": self.cap.allow_autorun,
                "maxPanels": self.cap.max_panels,
            },
        });
        let body = serde_json::to_string_pretty(&out).unwrap_or_default();
        // If nothing about the page changed since the last describe, say so — the
        // model shouldn't burn turn budget re-polling a stable surface.
        let unchanged = self
            .bridge
            .inner
            .last_describe
            .lock()
            .map(|prev| prev.as_deref() == Some(body.as_str()))
            .unwrap_or(false);
        if let Ok(mut prev) = self.bridge.inner.last_describe.lock() {
            *prev = Some(body.clone());
        }
        if unchanged {
            ok_text(format!(
                "(Surface unchanged since your last ui_describe, so no need to call it \
                 again this turn.)\n{body}"
            ))
        } else {
            ok_text(body)
        }
    }

    #[tool(
        name = "ui_panel",
        description = "Mount (or replace, or remove) a panel in the app: a side panel, a \
                       dashboard, an inspector. This is the main way to give the user \
                       something richer than a paragraph.\n\n\
                       `body` is an array of widget nodes. Node shapes:\n\
                       {\"t\":\"card\",\"title\":\"…\",\"children\":[…]}\n\
                       {\"t\":\"row\"|\"col\",\"children\":[…]}\n\
                       {\"t\":\"text\",\"value\":\"…\",\"markdown\":true,\"muted\":false}\n\
                       {\"t\":\"stat\",\"label\":\"Median\",\"value\":42,\"unit\":\"ms\",\"delta\":\"+3%\"}\n\
                       {\"t\":\"badge\",\"value\":\"beta\"}\n\
                       {\"t\":\"divider\"}\n\
                       {\"t\":\"progress\",\"value\":0.62,\"label\":\"…\"}\n\
                       {\"t\":\"table\",\"columns\":[…],\"rows\":[[…],[…]]}\n\
                       {\"t\":\"chart\",\"spec\":{\"type\":\"bar\",\"title\":\"…\",\"data\":[{\"label\":\"A\",\"value\":1}]}}  (or \"series\":[{\"name\":\"train\",\"data\":[…]},…] for multiple lines)\n\
                       {\"t\":\"graph\",\"spec\":{\"nodes\":[{\"id\":\"A\"}],\"edges\":[{\"source\":\"A\",\"target\":\"B\"}]}}\n\
                       {\"t\":\"input\"|\"select\"|\"checkbox\",\"name\":\"…\",\"label\":\"…\"}\n\
                       {\"t\":\"button\",\"label\":\"Run\",\"action\":\"run\",\"submit\":true}\n\
                       A `button` sends its action (and, with submit:true, the form fields) \
                       back to you as your next turn.\n\n\
                       `place` picks where it mounts: a dock slot (\"dock\" default / \
                       \"left\" / \"right\" / \"bottom\" / \"main\" / \"modal\"), OR a target \
                       naming the author's own layout: \"@region:<name>\", \"@panel:<id>\", \
                       or a CSS selector. To fill a region the author declared (e.g. \
                       @region:dashboard), pass it as `place` directly."
    )]
    pub async fn ui_panel(
        &self,
        Parameters(p): Parameters<PanelParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = p.id.trim();
        if id.is_empty() {
            return Err(invalid("\"id\" must not be empty"));
        }

        if p.remove == Some(true) {
            self.drop_panel(id);
            self.bridge
                .emit(json!({ "cmd": "panel", "id": id, "remove": true }))?;
            return ok_text(format!("Removed panel \"{id}\"."));
        }

        // `place` is either an SDK-owned dock slot (dock/left/right/bottom/main/
        // modal) OR a target that names part of the author's page — `@region:x`,
        // `@panel:x`, `@main`, or a CSS selector — in which case the panel's card
        // mounts *into* that element. The latter is the overwhelmingly common
        // "put a titled dashboard into @region:dashboard" case; rejecting it made
        // the model burn 5–7 retries before falling back to `ui_render`.
        let place = p.place.as_deref().unwrap_or("dock");
        let is_target = place.starts_with('@') || !PANEL_PLACES.contains(&place);
        if is_target {
            validate_target(place)?;
        }

        let mut body = match (&p.body, &p.markdown) {
            (Some(b), _) => checked_body(b, &self.widget_ctx(false))?,
            (None, Some(md)) => json!([{ "t": "text", "value": md, "markdown": true }]),
            (None, None) => {
                return Err(invalid(
                    "provide either \"body\" (widget nodes) or \"markdown\"",
                ))
            }
        };
        let assigned = self.assign_and_register(id, &mut body);

        let evicted = self.note_panel(id);
        if let Some(old) = &evicted {
            self.bridge
                .emit(json!({ "cmd": "panel", "id": old, "remove": true }))?;
        }
        self.bridge.emit(json!({
            "cmd": "panel",
            "id": id,
            "title": p.title,
            "place": place,
            "collapsible": p.collapsible.unwrap_or(true),
            "body": body,
        }))?;

        let mut msg = format!("Panel \"{id}\" is now showing in the app ({place}).");
        if let Some(old) = evicted {
            msg.push_str(&format!(
                " Evicted the oldest panel \"{old}\" (limit {}).",
                self.cap.max_panels
            ));
        }
        msg.push_str(&Self::ids_hint(&assigned));
        ok_text(msg)
    }

    #[tool(
        name = "ui_render",
        description = "Render widget nodes into an existing part of the app: an \
                       author-declared region (`@region:results`), a panel you created \
                       (`@panel:summary`), or a CSS selector (`#out`). Use `ui_describe` to \
                       discover targets. Same `body` node schema as `ui_panel`."
    )]
    pub async fn ui_render(
        &self,
        Parameters(p): Parameters<RenderParams>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_target(&p.target)?;
        let mut body = checked_body(&p.body, &self.widget_ctx(false))?;
        let mode = p.mode.as_deref().unwrap_or("replace");
        if !["replace", "append"].contains(&mode) {
            return Err(invalid("\"mode\" must be \"replace\" or \"append\""));
        }
        let assigned = self.assign_and_register(p.target.trim(), &mut body);
        self.bridge.emit(json!({
            "cmd": "render",
            "target": p.target,
            "mode": mode,
            "body": body,
        }))?;
        ok_text(format!(
            "Rendered into {} ({mode}).{}",
            p.target,
            Self::ids_hint(&assigned)
        ))
    }

    #[tool(
        name = "ui_chart",
        description = "Draw a bar/line/pie chart in the app.\n\
                       Single series: {\"type\":\"bar\",\"title\":\"…\",\"data\":[{\"label\":\"A\",\"value\":12}]}.\n\
                       Multiple series on one axis (e.g. train vs validation loss, actual vs forecast, \
                       two arms): {\"type\":\"line\",\"title\":\"…\",\"series\":[{\"name\":\"train\",\
                       \"data\":[{\"label\":\"e1\",\"value\":0.9}, …]},{\"name\":\"val\",\"data\":[…]}]}, giving \
                       overlaid lines / grouped bars with a legend, series index-aligned by label. \
                       Omit `target` to put it in a dock panel."
    )]
    pub async fn ui_chart(
        &self,
        Parameters(p): Parameters<ChartParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let spec = unstringify(&p.spec);
        validate_chart(&spec).map_err(invalid)?;
        let node = json!({ "t": "chart", "spec": spec });
        self.emit_visual(p.target, p.id, "chart", node, &spec)
    }

    #[tool(
        name = "ui_graph",
        description = "Draw a node/edge graph (network, pathway, relationship map) in the app. \
                       spec: {\"title\":\"…\",\"nodes\":[{\"id\":\"TP53\"}],\"edges\":[{\"source\":\"TP53\",\"target\":\"MDM2\",\"label\":\"inhibits\"}]}. \
                       Omit `target` to put it in a dock panel."
    )]
    pub async fn ui_graph(
        &self,
        Parameters(p): Parameters<GraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let spec = unstringify(&p.spec);
        validate_graph(&spec).map_err(invalid)?;
        let node = json!({ "t": "graph", "spec": spec });
        self.emit_visual(p.target, p.id, "graph", node, &spec)
    }

    #[tool(
        name = "ui_highlight",
        description = "Draw the user's eye to part of the app: outline it, pulse it, or dim \
                       everything else (\"focus\"). Optionally attach a short note. Use \
                       mode:\"clear\" to remove all highlights. Target syntax matches ui_render."
    )]
    pub async fn ui_highlight(
        &self,
        Parameters(p): Parameters<HighlightParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mode = p.mode.as_deref().unwrap_or("outline");
        if !["outline", "pulse", "focus", "clear"].contains(&mode) {
            return Err(invalid("\"mode\" must be outline, pulse, focus, or clear"));
        }
        if mode != "clear" {
            validate_target(&p.target)?;
        }
        self.bridge.emit(json!({
            "cmd": "highlight",
            "target": p.target,
            "mode": mode,
            "note": p.note,
            "scroll": p.scroll.unwrap_or(true),
        }))?;
        ok_text(if mode == "clear" {
            "Cleared highlights.".to_string()
        } else {
            format!("Highlighted {} ({mode}).", p.target)
        })
    }

    #[tool(
        name = "ui_theme",
        description = "Restyle the app: theme pack (biorouter/clinical/lab-notebook/terminal/\
                       journal/midnight), accent colour, light/dark mode, density. Use this to \
                       make the app *feel* like what it is showing (e.g. a clinical pack for a \
                       cohort review, a terminal pack for a log viewer)."
    )]
    pub async fn ui_theme(
        &self,
        Parameters(p): Parameters<ThemeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.cap.allow_theme {
            return Err(Self::denied("theme control"));
        }
        if let Some(pack) = p.pack.as_deref() {
            if !THEME_PACKS.contains(&pack) {
                return Err(invalid(format!(
                    "\"pack\" must be one of: {}",
                    THEME_PACKS.join(", ")
                )));
            }
        }
        if let Some(m) = p.mode.as_deref() {
            if !["light", "dark", "auto"].contains(&m) {
                return Err(invalid("\"mode\" must be light, dark, or auto"));
            }
        }
        if let Some(d) = p.density.as_deref() {
            if !["comfortable", "compact"].contains(&d) {
                return Err(invalid("\"density\" must be comfortable or compact"));
            }
        }
        if let Some(a) = p.accent.as_deref() {
            // The client writes this into a CSS custom property; keep it to a
            // colour-ish token so it can't smuggle `};` and escape the rule.
            if a.len() > 32 || a.contains([';', '}', '{', '<', '>', '(', ')', '"', '\'', '\\', '/'])
            {
                return Err(invalid(
                    "\"accent\" must be a simple CSS colour like \"#2f6f4e\" or \"tomato\"",
                ));
            }
        }
        self.bridge.emit(json!({
            "cmd": "theme",
            "pack": p.pack,
            "accent": p.accent,
            "mode": p.mode,
            "density": p.density,
        }))?;
        ok_text("Theme updated.")
    }

    #[tool(
        name = "ui_layout",
        description = "Set the app's layout. Either a preset (\"single\", \"sidebar-right\", \
                       \"sidebar-left\", \"split\", or \"dashboard\", a responsive grid), or a \
                       grid grammar via `areas` (rows of area names, ≤4×4) with optional `sizes`. \
                       Pair with ui_panel/@region: to fill the regions."
    )]
    pub async fn ui_layout(
        &self,
        Parameters(p): Parameters<LayoutParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.cap.allow_layout {
            return Err(Self::denied("layout control"));
        }
        const PRESETS: &[&str] = &[
            "single",
            "sidebar-right",
            "sidebar-left",
            "split",
            "dashboard",
        ];
        if p.preset.is_none() && p.areas.is_none() {
            return Err(invalid("ui_layout needs a `preset` or an `areas` grid"));
        }
        if let Some(preset) = p.preset.as_deref() {
            if !PRESETS.contains(&preset) {
                return Err(invalid(format!(
                    "\"preset\" must be one of: {}",
                    PRESETS.join(", ")
                )));
            }
        }
        if let Some(areas) = &p.areas {
            validate_layout_areas(areas).map_err(invalid)?;
        }
        if let Some(sizes) = &p.sizes {
            validate_layout_sizes(sizes).map_err(invalid)?;
        }
        self.bridge.emit(json!({
            "cmd": "layout",
            "preset": p.preset,
            "areas": p.areas,
            "sizes": p.sizes,
            "sidebarWidth": p.sidebar_width,
        }))?;
        let what = match (&p.preset, &p.areas) {
            (Some(preset), _) => format!("preset {preset}"),
            (None, Some(areas)) => format!(
                "a {}×{} grid",
                areas.len(),
                areas.first().map_or(0, Vec::len)
            ),
            _ => "layout".to_string(),
        };
        ok_text(format!("Layout set to {what}."))
    }

    #[tool(
        name = "ui_notify",
        description = "Show a transient toast in the app (progress, warnings, completion)."
    )]
    pub async fn ui_notify(
        &self,
        Parameters(p): Parameters<NotifyParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let level = p.level.as_deref().unwrap_or("info");
        if !["info", "success", "warn", "error"].contains(&level) {
            return Err(invalid("\"level\" must be info, success, warn, or error"));
        }
        if p.message.trim().is_empty() {
            return Err(invalid("\"message\" must not be empty"));
        }
        self.bridge.emit(json!({
            "cmd": "notify",
            "message": p.message,
            "level": level,
            "timeoutMs": p.timeout_ms.unwrap_or(4000),
        }))?;
        ok_text("Notification shown.")
    }

    #[tool(
        name = "ui_suggest",
        description = "Offer up to five non-blocking suggestion chips: next steps the user can tap \
                       to send (or ignore). Unlike `ui_ask`, this never blocks the turn: it renders \
                       a lightweight 'you might want to…' rail the user can dismiss. Each chip is a \
                       short label plus an optional prompt sent on click (omit the prompt to hand \
                       the click to the app's own code). Easy to invoke, easy to ignore."
    )]
    pub async fn ui_suggest(
        &self,
        Parameters(p): Parameters<SuggestParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if p.chips.is_empty() {
            return Err(invalid("\"chips\" must contain at least one suggestion"));
        }
        if p.chips.len() > 5 {
            return Err(invalid("\"chips\" is capped at 5 suggestions"));
        }
        for c in &p.chips {
            if c.label.trim().is_empty() {
                return Err(invalid("each chip \"label\" must not be empty"));
            }
            if c.label.chars().count() > 80 {
                return Err(invalid(
                    "each chip \"label\" must be 80 characters or fewer",
                ));
            }
            if let Some(prompt) = &c.prompt {
                if prompt.chars().count() > 500 {
                    return Err(invalid(
                        "each chip \"prompt\" must be 500 characters or fewer",
                    ));
                }
            }
        }
        let mut frame = json!({
            "cmd": "suggest",
            "chips": serde_json::to_value(&p.chips).unwrap_or_else(|_| json!([])),
        });
        if let Some(target) = p.target.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            frame["target"] = json!(target);
        }
        self.bridge.emit(frame)?;
        ok_text(format!("Offered {} suggestion chip(s).", p.chips.len()))
    }

    #[tool(
        name = "ui_state",
        description = "Read/write the app's shared state document by top-level key. Keys you set \
                       are merged into the document and mirrored into the page (`br.state`), so \
                       the app's own code and bound elements react. Returns the full document \
                       after the update. For surgical edits to nested paths, prefer \
                       `ui_patch_state`."
    )]
    pub async fn ui_state(
        &self,
        Parameters(p): Parameters<StateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Refuse to forge an action's effect. A key owned by a `mutate` action may
        // only change by calling that action's real handler — otherwise the agent
        // can write the number itself and narrate a change the app never made.
        if let Some(set) = &p.set {
            if let Some(obj) = unstringify(set).as_object() {
                for key in obj.keys() {
                    self.bridge
                        .check_write_allowed(&format!("/{key}"))
                        .map_err(invalid)?;
                }
            }
        }

        // Track whether anything actually changed, so an identical repeat becomes
        // a cheap no-op that tells the model to stop re-sending it (H3: models
        // cascade ui_state with unchanged values and exhaust the turn budget).
        let mut guard = self
            .bridge
            .inner
            .state
            .lock()
            .map_err(|_| invalid("the app state is unavailable"))?;

        // Merge into a clone so a cap violation rejects the change and leaves the
        // live document untouched. The document is always an object at the root.
        let mut next = if guard.doc.is_object() {
            guard.doc.clone()
        } else {
            json!({})
        };
        let obj = next.as_object_mut().expect("normalized to an object");
        let mut changed = false;
        if let Some(set) = &p.set {
            let set = unstringify(set);
            let set_obj = set
                .as_object()
                .ok_or_else(|| invalid(want_json("set", "a JSON object", r#"{"gene":"TP53"}"#)))?;
            for (k, v) in set_obj {
                if obj.get(k) != Some(v) {
                    changed = true;
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(remove) = &p.remove {
            for k in remove {
                if obj.remove(k).is_some() {
                    changed = true;
                }
            }
        }

        let version = if changed {
            validate_state_doc(&next).map_err(invalid)?;
            guard.doc = next;
            guard.version += 1;
            guard.version
        } else {
            guard.version
        };
        let snapshot = guard.doc.clone();
        drop(guard);

        // Only touch the page when the document actually moved. An unchanged call
        // emits no frame and says so, so the model doesn't keep re-sending it.
        if changed {
            self.bridge.emit(json!({
                "cmd": "state",
                "mode": "snapshot",
                "doc": snapshot,
                "version": version,
            }))?;
            ok_text(serde_json::to_string(&snapshot).unwrap_or_default())
        } else if p.set.is_none() && p.remove.is_none() {
            // A read: return the current document.
            ok_text(serde_json::to_string(&snapshot).unwrap_or_default())
        } else {
            ok_text(format!(
                "No change: the state document already holds these values, so nothing was \
                 re-sent. Current state: {}",
                serde_json::to_string(&snapshot).unwrap_or_default()
            ))
        }
    }

    #[tool(
        name = "ui_patch_state",
        description = "Apply an RFC-6902 JSON Patch to the app's shared state document: an array \
                       of operations, e.g. [{\"op\":\"add\",\"path\":\"/cohort/count\",\"value\":42}]. \
                       Capped at 64 operations. Bumps the document version and pushes the delta to \
                       the page (bound elements re-render). Prefer this over `ui_state` for \
                       surgical updates to nested paths."
    )]
    pub async fn ui_patch_state(
        &self,
        Parameters(p): Parameters<PatchStateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let patch = unstringify(&p.patch);
        let ops = patch.as_array().ok_or_else(|| {
            invalid(want_json(
                "patch",
                "an array of RFC-6902 operations",
                r#"[{"op":"add","path":"/count","value":3}]"#,
            ))
        })?;
        if ops.is_empty() {
            return Err(invalid("\"patch\" must contain at least one operation"));
        }
        if ops.len() > STATE_MAX_PATCH_OPS {
            return Err(invalid(format!(
                "\"patch\" has {} operations; cap is {STATE_MAX_PATCH_OPS}",
                ops.len()
            )));
        }
        let parsed: json_patch::Patch = serde_json::from_value(patch.clone())
            .map_err(|e| invalid(format!("\"patch\" is not a valid RFC-6902 JSON Patch: {e}")))?;

        // Refuse to forge an action's effect. Every op's target path is checked
        // against the pointers a `mutate` action's handler owns — including paths
        // *under* an owned pointer, so `/params/lion_vision/min` is covered by the
        // action that owns `/params/lion_vision`. This is what makes the
        // narration-only path impossible: the number on the page can move only
        // through the app's real handler.
        for op in ops {
            if let Some(path) = op.get("path").and_then(|v| v.as_str()) {
                self.bridge.check_write_allowed(path).map_err(invalid)?;
            }
            // `move`/`copy` also write to their destination.
            if let Some(from) = op.get("from").and_then(|v| v.as_str()) {
                self.bridge.check_write_allowed(from).map_err(invalid)?;
            }
        }

        let mut guard = self
            .bridge
            .inner
            .state
            .lock()
            .map_err(|_| invalid("the app state is unavailable"))?;
        // Apply against a clone; commit only once it applies and validates, so a
        // rejected patch leaves the live document untouched.
        let mut next = guard.doc.clone();
        json_patch::patch(&mut next, &parsed).map_err(|e| {
            invalid(format!(
                "\"patch\" could not be applied to the current state: {e}"
            ))
        })?;
        validate_state_doc(&next).map_err(invalid)?;
        guard.doc = next;
        guard.version += 1;
        let version = guard.version;
        drop(guard);

        self.bridge.emit(json!({
            "cmd": "state",
            "mode": "patch",
            "patch": patch,
            "version": version,
        }))?;
        ok_text(
            serde_json::to_string(&json!({ "ok": true, "version": version })).unwrap_or_default(),
        )
    }

    #[tool(
        name = "ui_patch",
        description = "Incrementally edit the UI by node id, the preferred way to update an app \
                       once something is on the page (it preserves scroll, focus, and input \
                       state instead of re-rendering). `ops` is an array (≤ 32) of:\n\
                       {\"op\":\"add\",\"id\":\"kpi-cases\",\"target\":\"@region:results\",\"node\":{…}} \
                       mounts a new node (target defaults to the main results region; use \
                       \"parent\":<id> to nest inside an existing node);\n\
                       {\"op\":\"replace\",\"id\":\"kpi-cases\",\"node\":{…}} swaps a node's contents;\n\
                       {\"op\":\"set_props\",\"id\":\"kpi-cases\",\"props\":{…}} shallow-merges keys into a node;\n\
                       {\"op\":\"remove\",\"id\":\"kpi-cases\"} deletes a node.\n\
                       Nodes you create with ui_panel/ui_render get ids (returned in their tool \
                       result); use those ids here. Same node grammar as ui_panel."
    )]
    pub async fn ui_patch(
        &self,
        Parameters(p): Parameters<PatchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let ops_v = unstringify(&p.ops);
        let ops = ops_v.as_array().ok_or_else(|| {
            invalid(want_json(
                "ops",
                "an array of patch operations",
                r#"[{"op":"add","id":"n1","node":{"t":"text","value":"hi"}}]"#,
            ))
        })?;
        if ops.is_empty() {
            return Err(invalid("\"ops\" must contain at least one operation"));
        }
        if ops.len() > MAX_PATCH_OPS {
            return Err(invalid(format!(
                "\"ops\" has {} operations; cap is {MAX_PATCH_OPS}",
                ops.len()
            )));
        }

        let ctx = self.widget_ctx(false);
        let mut guard = self
            .bridge
            .inner
            .instances
            .lock()
            .map_err(|_| invalid("the app instance registry is unavailable"))?;
        // Validate & simulate against a clone; commit (and emit) only if EVERY op
        // is valid, so a rejected batch leaves the registry and page untouched.
        let mut next = guard.clone();

        for (i, op) in ops.iter().enumerate() {
            let oo = op
                .as_object()
                .ok_or_else(|| invalid(format!("ops[{i}] must be an object")))?;
            let kind = oo.get("op").and_then(Value::as_str).ok_or_else(|| {
                invalid(format!(
                    "ops[{i}] needs a string \"op\" (add|replace|set_props|remove)"
                ))
            })?;
            let id = oo
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .ok_or_else(|| invalid(format!("ops[{i}] needs a string \"id\"")))?;
            if id.is_empty() {
                return Err(invalid(format!("ops[{i}].id must not be empty")));
            }
            if id.len() > MAX_INSTANCE_ID_LEN {
                return Err(invalid(format!(
                    "ops[{i}].id is {} chars; cap is {MAX_INSTANCE_ID_LEN}",
                    id.len()
                )));
            }

            match kind {
                "add" => {
                    if next.contains_key(id) {
                        return Err(invalid(format!(
                            "ops[{i}]: id \"{id}\" already exists; use op:\"replace\" to change it"
                        )));
                    }
                    let node = oo
                        .get("node")
                        .ok_or_else(|| invalid(format!("ops[{i}] (add) needs a \"node\"")))?;
                    let mut budget = MAX_WIDGET_NODES;
                    validate_widget(node, 0, &mut budget, &ctx).map_err(invalid)?;
                    if let Some(t) = oo.get("target").and_then(Value::as_str) {
                        validate_target(t)?;
                    }
                    if let Some(parent) = oo.get("parent").and_then(Value::as_str) {
                        if !next.contains_key(parent) {
                            return Err(invalid(format!(
                                "ops[{i}].parent \"{parent}\" is not a known instance id"
                            )));
                        }
                    }
                    next.insert(id.to_string(), node.clone());
                }
                "replace" => {
                    if !next.contains_key(id) {
                        return Err(invalid(unknown_id_msg(i, id, &next)));
                    }
                    let node = oo
                        .get("node")
                        .ok_or_else(|| invalid(format!("ops[{i}] (replace) needs a \"node\"")))?;
                    let mut budget = MAX_WIDGET_NODES;
                    validate_widget(node, 0, &mut budget, &ctx).map_err(invalid)?;
                    next.insert(id.to_string(), node.clone());
                }
                "set_props" => {
                    if !next.contains_key(id) {
                        return Err(invalid(unknown_id_msg(i, id, &next)));
                    }
                    let props = oo.get("props").and_then(Value::as_object).ok_or_else(|| {
                        invalid(format!("ops[{i}] (set_props) needs a \"props\" object"))
                    })?;
                    let mut merged = next.get(id).cloned().unwrap_or_else(|| json!({}));
                    if let Some(m) = merged.as_object_mut() {
                        for (k, v) in props {
                            m.insert(k.clone(), v.clone());
                        }
                    }
                    let mut budget = MAX_WIDGET_NODES;
                    validate_widget(&merged, 0, &mut budget, &ctx).map_err(invalid)?;
                    next.insert(id.to_string(), merged);
                }
                "remove" => {
                    if !next.contains_key(id) {
                        return Err(invalid(unknown_id_msg(i, id, &next)));
                    }
                    next.remove(id);
                }
                other => {
                    return Err(invalid(format!(
                        "ops[{i}].op \"{other}\" must be add, replace, set_props, or remove"
                    )))
                }
            }
        }

        *guard = next;
        drop(guard);

        // Emit the validated ops verbatim (the client morphs by id).
        self.bridge.emit(json!({ "cmd": "patch", "ops": ops }))?;
        ok_text(format!("Applied {} patch op(s).", ops.len()))
    }

    #[tool(
        name = "ui_html",
        description = "Render a block of rich HTML into the app. The HTML is SANITIZED \
                       server-side before it is shown: scripts, styles, forms, iframes, \
                       event-handler attributes, and non-https/mailto/relative URLs are all \
                       stripped. Use it for prose-with-markup an app keeps hand-rolling; prefer \
                       ui_panel widgets or ui_figure for anything structured. Only available \
                       when the app grants `ui.allow_html`."
    )]
    pub async fn ui_html(
        &self,
        Parameters(p): Parameters<HtmlParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.cap.allow_html {
            return Err(Self::denied("raw HTML (ui.allow_html)"));
        }
        validate_target(&p.target)?;
        if p.html.trim().is_empty() {
            return Err(invalid("\"html\" must not be empty"));
        }
        if p.html.len() > MAX_HTML_BYTES {
            return Err(invalid(format!(
                "\"html\" is {} bytes; cap is {MAX_HTML_BYTES}",
                p.html.len()
            )));
        }
        let (clean, removed) = sanitize_html(&p.html);
        // Privileged node: constructed here, AFTER sanitization, so it can carry
        // the `html` kind that a generic tree may not.
        let html_node = json!({ "t": "html", "html": clean });
        let node = match &p.title {
            Some(t) => json!({ "t": "card", "title": t, "children": [html_node] }),
            None => html_node,
        };
        self.bridge.emit(json!({
            "cmd": "render",
            "target": p.target,
            "mode": "replace",
            "body": [node],
        }))?;
        let mut msg = format!("Rendered sanitized HTML into {}.", p.target);
        if removed > 0 {
            msg.push_str(&format!(
                " Sanitization removed {removed} disallowed element(s) \
                 (scripts, styles, forms, iframes, or unsafe URLs)."
            ));
        }
        ok_text(msg)
    }

    #[tool(
        name = "ui_figure",
        description = "Render a publication-grade Auto Visualiser figure into the app. Pass the \
                       tool name (e.g. \"render_volcano\", \"render_manhattan\", \
                       \"render_kaplan_meier\", \"render_forest\", \"render_sankey\", \
                       \"render_chord\", \"render_heatmap\", \"render_choropleth\", \
                       \"render_dashboard\") and that tool's exact `args`. The figure is rendered \
                       server-side and shown in a sandboxed frame, richer and more correct than \
                       hand-built ui_chart output. Omit `target` for a dock panel."
    )]
    pub async fn ui_figure(
        &self,
        Parameters(p): Parameters<FigureParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool = p.tool.trim();
        if tool.is_empty() {
            return Err(invalid(
                "\"tool\" must name an Auto Visualiser tool, e.g. \"render_volcano\"",
            ));
        }
        let args = unstringify(&p.args);
        // Trusted output: the autovisualiser renders/sanitizes the fragment, so
        // the resulting `figure` node is privileged and built here.
        let html = crate::autovisualiser::render_standalone_figure(tool, args)
            .await
            .map_err(|e| invalid(format!("could not render figure with \"{tool}\": {e}")))?;
        let node = json!({ "t": "figure", "html": html, "tool": tool });
        self.emit_figure(p.target, p.title, tool, node)
    }

    #[tool(
        name = "ui_ask",
        description = "Render a form in the app and WAIT for the user to submit it; the tool \
                       result is their answers, so you branch on them without ending your turn.\n\n\
                       ALWAYS use this instead of asking the user a question in prose. If you are \
                       about to write \"please provide…\", \"paste your data\", \"which would you \
                       like?\" or otherwise stop and wait for a reply, call `ui_ask` instead. \
                       Many apps have no chat box, so a prose question reaches nobody."
    )]
    pub async fn ui_ask(
        &self,
        Parameters(p): Parameters<AskParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.cap.allow_ask {
            return Err(Self::denied("interactive prompts"));
        }
        if p.fields.is_empty() {
            return Err(invalid("\"fields\" must contain at least one field"));
        }
        if p.fields.len() > 24 {
            return Err(invalid("\"fields\" is capped at 24"));
        }
        for f in &p.fields {
            if f.name.trim().is_empty() {
                return Err(invalid("every field needs a non-empty \"name\""));
            }
            let ty = f.r#type.as_deref().unwrap_or("text");
            if !["text", "number", "textarea", "select", "checkbox"].contains(&ty) {
                return Err(invalid(format!(
                    "field \"{}\": type must be text, number, textarea, select, or checkbox",
                    f.name
                )));
            }
            if ty == "select" && f.options.as_ref().is_none_or(Vec::is_empty) {
                return Err(invalid(format!(
                    "field \"{}\": a select needs a non-empty \"options\" list",
                    f.name
                )));
            }
        }

        let request_id = self.bridge.next_id("ask");
        let rx = self.bridge.register_ask(request_id.clone());
        let emit = self.bridge.emit(json!({
            "cmd": "ask",
            "requestId": request_id,
            "title": p.title,
            "prompt": p.prompt,
            "submitLabel": p.submit_label.unwrap_or_else(|| "Submit".to_string()),
            "fields": p.fields,
        }));
        if let Err(e) = emit {
            self.bridge.forget_ask(&request_id);
            return Err(e);
        }

        let timeout = Duration::from_secs(self.cap.ask_timeout_s.clamp(5, 3600));
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(payload)) => {
                if payload.get("cancelled").and_then(Value::as_bool) == Some(true) {
                    return ok_text(
                        "The user dismissed the form without answering. Continue without their input, \
                         or explain what you need.",
                    );
                }
                ok_text(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()))
            }
            // Sender dropped: the socket closed.
            Ok(Err(_)) => {
                self.bridge.forget_ask(&request_id);
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "the app window closed before the user answered".to_string(),
                    None,
                ))
            }
            Err(_) => {
                self.bridge.forget_ask(&request_id);
                self.bridge
                    .emit(json!({ "cmd": "ask_close", "requestId": request_id }))?;
                ok_text(format!(
                    "The user did not answer within {}s. Proceed with sensible defaults and say what you assumed.",
                    timeout.as_secs()
                ))
            }
        }
    }

    #[tool(
        name = "app_call",
        description = "Invoke an APP-DEFINED function on the page: one of the verbs the app author \
                       declared in `surface.actions` and registered a handler for. Unlike the \
                       other ui_* tools (which mutate the DOM), this calls into the app's own \
                       logic and returns its result to you as this tool's result. Call \
                       `ui_describe` FIRST to see the app's declared actions (name, description, \
                       and argument schema); `args` is validated against the named action's \
                       declared params schema. Blocks until the app responds or 60 seconds \
                       elapse, whichever comes first."
    )]
    pub async fn app_call(
        &self,
        Parameters(p): Parameters<AppCallParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let action = p.action.trim();
        if action.is_empty() {
            return Err(invalid(
                "\"action\" must name one of the app's declared actions (see ui_describe)",
            ));
        }
        // The action must be declared in the surface; an unknown one lists the
        // declared names so the model can correct itself.
        let decl = self
            .surface
            .actions
            .iter()
            .find(|a| a.name == action)
            .ok_or_else(|| {
                let known: Vec<&str> = self
                    .surface
                    .actions
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect();
                if known.is_empty() {
                    invalid(format!(
                        "action \"{action}\" is not declared; this app declares no actions"
                    ))
                } else {
                    invalid(format!(
                        "action \"{action}\" is not declared; declared actions: {}",
                        known.join(", ")
                    ))
                }
            })?;

        // `args` is an object (absent/null ⇒ empty), then schema-checked against
        // the declared params, fail-closed exactly like component props.
        let args = if p.args.is_null() {
            json!({})
        } else {
            unstringify(&p.args)
        };
        if !args.is_object() {
            return Err(invalid(want_json(
                "args",
                "a JSON object",
                r#"{"gene":"TP53"}"#,
            )));
        }
        json_schema_errors(&args, &decl.params).map_err(|e| {
            invalid(format!(
                "action \"{action}\" args do not match its declared schema: {e}"
            ))
        })?;

        // EVIDENCE GATE. This is the only place quantitative output can reach the
        // page, so it is the only place the check can be enforced.
        //
        // If a worker reported that an input this action REQUIRES is missing, a
        // non-synthetic call is refused. The model may still proceed — by saying
        // plainly that the numbers are made up (`_provenance: {"source":
        // "synthetic"}`), which stamps them and renders a DEMO badge — or it may
        // render the insufficient-data state. What it can no longer do is publish
        // invented statistics that look computed.
        let provenance = args
            .get("_provenance")
            .and_then(|p| p.get("source"))
            .and_then(Value::as_str)
            .and_then(ProvenanceSource::parse);

        if !decl.requires_evidence.is_empty() {
            let missing = self.bridge.missing_evidence();
            let blocking: Vec<String> = decl
                .requires_evidence
                .iter()
                .filter_map(|need| {
                    missing
                        .iter()
                        .find(|(m, _)| m == need)
                        .map(|(m, who)| format!("{m} (reported missing by \"{who}\")"))
                })
                .collect();

            if !blocking.is_empty() && provenance != Some(ProvenanceSource::Synthetic) {
                return Err(invalid(format!(
                    "\"{action}\" depends on evidence a worker reported it does NOT have: {}. \
                     Publishing numbers here would present invented values as computed ones. \
                     Either render the insufficient-data state and tell the user what is missing, \
                     or (if a demonstration is genuinely what you want) call this action again \
                     with `_provenance: {{\"source\": \"synthetic\"}}`, and the values will be \
                     labelled DEMO on the page.",
                    blocking.join(", ")
                )));
            }
        }

        if decl.provenance_required && provenance.is_none() {
            return Err(invalid(format!(
                "\"{action}\" publishes quantitative output and requires provenance. Add \
                 `_provenance: {{\"source\": \"tool\" | \"consult:<profile>\" | \"user\" | \
                 \"synthetic\"}}` to args, naming where these values actually came from."
            )));
        }

        // Snapshot the pointers this action declares it writes, so the tool result
        // can report what ACTUALLY moved. The agent otherwise has only its own
        // claim to go on — and specs 011/013/014 show it will confidently narrate
        // an intervention it never applied.
        let before: Vec<(String, Value)> = if decl.effect.is_mutate() {
            let (doc, _) = self.bridge.state_snapshot();
            decl.writes
                .iter()
                .map(|ptr| (ptr.clone(), pointer_get(&doc, ptr)))
                .collect()
        } else {
            Vec::new()
        };

        // Emit the app_call frame and park on a fresh oneshot until the app posts
        // an app_result (or the timeout fires / the turn is cancelled).
        let call_id = fresh_call_id();
        let rx = self.bridge.register_call(call_id.clone());
        let emit = self.bridge.emit(json!({
            "cmd": "app_call",
            "callId": call_id,
            "action": action,
            "args": args,
            // Carried to the page so the SDK can badge fabricated values. A demo is
            // legitimate; a demo that is indistinguishable from a real result is not.
            "synthetic": provenance == Some(ProvenanceSource::Synthetic),
        }));
        if let Err(e) = emit {
            self.bridge.forget_call(&call_id);
            return Err(e);
        }

        let secs = self.bridge.inner.app_call_timeout_s.load(Ordering::Relaxed);
        match tokio::time::timeout(Duration::from_secs(secs), rx).await {
            Ok(Ok(payload)) => {
                if payload.get("cancelled").and_then(Value::as_bool) == Some(true) {
                    return ok_text(format!(
                        "The app call \"{action}\" was cancelled before it returned."
                    ));
                }
                if let Some(err) = payload.get("error").and_then(Value::as_str) {
                    return ok_text(format!("the app reported an error: {err}"));
                }
                let result = payload.get("result").cloned().unwrap_or(Value::Null);
                let mut text = capped_json_text(&result);
                if let Some(readback) = self.readback(decl, &before) {
                    text.push_str("\n\n");
                    text.push_str(&readback);
                }
                ok_text(text)
            }
            // Sender dropped: the socket closed out from under the parked call.
            Ok(Err(_)) => {
                self.bridge.forget_call(&call_id);
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "the app window closed before it answered the call".to_string(),
                    None,
                ))
            }
            Err(_) => {
                self.bridge.forget_call(&call_id);
                ok_text(format!(
                    "The app did not handle \"{action}\" within {secs}s. It may not register a \
                     handler for this action; proceed without its result or tell the user."
                ))
            }
        }
    }

    /// After a `mutate` action returns, re-read the pointers it declares it writes
    /// and report the diff — or the absence of one.
    ///
    /// This is the ground truth that a narrated claim cannot survive. If the app's
    /// handler ran and changed nothing, the model is told so in the same turn,
    /// rather than being free to report success.
    fn readback(&self, decl: &ActionDecl, before: &[(String, Value)]) -> Option<String> {
        if !decl.effect.is_mutate() || before.is_empty() {
            return None;
        }
        let (doc, _) = self.bridge.state_snapshot();

        let mut moved = Vec::new();
        for (ptr, old) in before {
            let now = pointer_get(&doc, ptr);
            if &now != old {
                moved.push(format!(
                    "{ptr}: {} → {}",
                    capped_json_text(old),
                    capped_json_text(&now)
                ));
            }
        }

        Some(if moved.is_empty() {
            format!(
                "[readback] \"{}\" returned, but did NOT change any state pointer it declares it \
                 writes ({}). Do not tell the user the change was applied; it was not. Say what \
                 happened, or try a different argument.",
                decl.name,
                decl.writes.join(", ")
            )
        } else {
            format!("[readback] applied: {}", moved.join("; "))
        })
    }

    #[tool(
        name = "emit_result",
        description = "Deliver a STRUCTURED result to the app for the structured call it is \
                       currently awaiting. Use this instead of prose when the app opened a \
                       structured request (it will have told you the shape). If no structured \
                       call is pending, this is a no-op and you should just answer in prose. When \
                       a schema is in force, `result` is validated against it and a mismatch \
                       comes back as a fixable error to retry."
    )]
    pub async fn emit_result(
        &self,
        Parameters(p): Parameters<EmitResultParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some((call_id, schema)) = self.bridge.take_pending_output() else {
            return ok_text("no structured call is pending, so reply in prose instead.");
        };

        let result = unstringify(&p.result);

        // Size cap: reject (fixable) so the model returns something smaller. Put
        // the pending request back first so the retry can still satisfy it.
        let serialized = serde_json::to_string(&result).unwrap_or_default();
        if serialized.len() > APP_PAYLOAD_MAX {
            self.bridge.set_pending_output(call_id, schema);
            return Err(invalid(format!(
                "the structured result is {} bytes; cap is {APP_PAYLOAD_MAX}. Return a smaller \
                 result (summarize, drop rows, or reference data instead of inlining it)",
                serialized.len()
            )));
        }

        // Schema check (fixable): restore the pending request so a corrected
        // result can be re-emitted. Compute the error first so the borrow of
        // `schema` ends before it is moved back on failure.
        let schema_err = schema
            .as_ref()
            .and_then(|sc| json_schema_errors(&result, sc).err());
        if let Some(e) = schema_err {
            self.bridge.set_pending_output(call_id, schema);
            return Err(invalid(format!(
                "the structured result does not match the declared output schema: {e}"
            )));
        }

        // Top-level (non-ui) frame — built here and sent raw via emit_frame so the
        // `type:"output"` envelope is not overwritten with `type:"ui"`.
        let mut frame = json!({
            "type": "output",
            "callId": call_id,
            "value": result,
            "v": CATALOG_VERSION,
        });
        if let Some(sc) = schema {
            if let Some(obj) = frame.as_object_mut() {
                obj.insert("schema".to_string(), sc);
            }
        }
        if !self.bridge.emit_frame(frame) {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "the app window is no longer connected; the structured result was not delivered"
                    .to_string(),
                None,
            ));
        }
        ok_text("structured result delivered to the app.")
    }

    #[tool(
        name = "ui_subscribe",
        description = "Subscribe to app→agent signals the author declared in `surface.signals` \
                       (see ui_describe). Pass the signal names you want; this REPLACES your \
                       current subscription set (re-subscribing is idempotent). The app then \
                       delivers those signals to you, rate-limited by each one's coalesce window. \
                       Only available when the app grants `ui.allow_signals`."
    )]
    pub async fn ui_subscribe(
        &self,
        Parameters(p): Parameters<SubscribeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.cap.allow_signals {
            return Err(Self::denied("app→agent signals (ui.allow_signals)"));
        }
        // Every requested name must be declared; an undeclared one lists what is
        // available so the model can correct itself.
        for name in &p.signals {
            if !self.surface.signals.iter().any(|s| &s.name == name) {
                let known: Vec<&str> = self
                    .surface
                    .signals
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect();
                return Err(invalid(if known.is_empty() {
                    format!("signal \"{name}\" is not declared; this app declares no signals")
                } else {
                    format!(
                        "signal \"{name}\" is not declared; declared signals: {}",
                        known.join(", ")
                    )
                }));
            }
        }

        // Replace the set (dedup by collecting through the bridge's HashSet).
        self.bridge.replace_subscriptions(p.signals.clone());
        let active = self.bridge.subscribed_signals();
        let signals: Vec<Value> = active
            .iter()
            .map(|name| {
                let coalesce = self.bridge.signal_decl(name).map(|(_, ms)| ms).unwrap_or(0);
                json!({ "name": name, "coalesceMs": coalesce })
            })
            .collect();
        ok_text(
            serde_json::to_string(&json!({ "subscribed": signals }))
                .unwrap_or_else(|_| "{}".to_string()),
        )
    }

    #[tool(
        name = "consult",
        description = "Ask one of THIS app's declared worker agent profiles to independently \
                       handle a sub-question and return its answer to you. Use it for a second \
                       opinion (e.g. a 'critic' profile) or to delegate a specialized sub-task to \
                       a profile that runs on a different model, system prompt, or tool set. \
                       `agent` is the profile name (the app's ready frame lists the available \
                       profiles); `prompt` is the self-contained question. Blocks until the \
                       profile answers or times out. The consulted profile's work is visible to \
                       the user while it runs, and they may intervene in it. Only the app's MAIN \
                       agent may consult, and a consulted profile cannot itself consult (depth is \
                       1)."
    )]
    pub async fn consult(
        &self,
        Parameters(p): Parameters<ConsultParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Gated: only the main agent's server carries a live consult (workers and
        // profile-less apps get a friendly no-op so the model stops trying).
        if !self.consult_enabled {
            return ok_text(
                "consult is unavailable: this app declares no worker agent profiles to consult.",
            );
        }
        let agent = p.agent.trim();
        let prompt = p.prompt.trim();
        if agent.is_empty() {
            return Err(invalid(
                "\"agent\" must name one of the app's declared worker profiles (see ready.profiles)",
            ));
        }
        if prompt.is_empty() {
            return Err(invalid(
                "\"prompt\" must be a non-empty, self-contained question for the profile to answer",
            ));
        }

        // Park on a fresh oneshot and hand the request to the app socket loop,
        // which runs the bounded worker turn and resolves it (or the timeout fires
        // / the turn is cancelled).
        let id = fresh_call_id();
        let rx = self.bridge.register_consult(id.clone());
        if !self.bridge.send_consult_request(ConsultRequest {
            id: id.clone(),
            agent: agent.to_string(),
            prompt: prompt.to_string(),
        }) {
            self.bridge.forget_consult(&id);
            return ok_text(
                "consult is unavailable right now: no worker-profile handler is listening.",
            );
        }

        // Park for the loop's deadline PLUS a grace. The loop owns the timeout: it
        // cancels the worker and resolves this oneshot with a structured verdict.
        // This timer is a backstop against a wedged loop, and must never fire first.
        let secs = self.bridge.inner.consult_timeout_s.load(Ordering::Relaxed);
        match tokio::time::timeout(Duration::from_secs(secs + CONSULT_GRACE_S), rx).await {
            Ok(Ok(payload)) => {
                if payload.get("cancelled").and_then(Value::as_bool) == Some(true) {
                    return ok_text(format!(
                        "The consultation with \"{agent}\" was cancelled before it returned."
                    ));
                }
                // A timeout is an ERROR result, not prose. It used to come back as
                // ordinary text ("did not answer within 120s"), which the model was
                // free to read as an answer and move on from — and did, silently
                // completing the turn with no work done.
                if payload.get("status").and_then(Value::as_str) == Some("timeout") {
                    let elapsed = payload
                        .get("elapsed_s")
                        .and_then(Value::as_u64)
                        .unwrap_or(secs);
                    let partial = payload
                        .get("partial")
                        .and_then(Value::as_str)
                        .filter(|p| !p.trim().is_empty());
                    let mut msg = format!(
                        "The \"{agent}\" profile did NOT answer within {elapsed}s and was \
                         cancelled. You have no result from it. Do not present its conclusion as \
                         if you had one. Either work without it and say so, or tell the user."
                    );
                    if let Some(p) = partial {
                        msg.push_str(&format!("\n\nPartial output before cancellation:\n{p}"));
                    }
                    return Ok(CallToolResult::error(vec![Content::text(msg)]));
                }
                if let Some(err) = payload.get("error").and_then(Value::as_str) {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "The \"{agent}\" profile could not answer: {err}"
                    ))]));
                }
                let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
                ok_text(format!("[{agent}] {text}"))
            }
            // Sender dropped: the socket closed out from under the parked consult.
            Ok(Err(_)) => {
                self.bridge.forget_consult(&id);
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "the app window closed before the consulted profile answered".to_string(),
                    None,
                ))
            }
            Err(_) => {
                // The loop did not even resolve us within deadline + grace: it is
                // wedged. Still an ERROR, never prose.
                self.bridge.forget_consult(&id);
                Ok(CallToolResult::error(vec![Content::text(format!(
                    "The \"{agent}\" profile did not answer within {secs}s and the app's worker \
                     loop did not respond. You have no result from it, so do not invent one."
                ))]))
            }
        }
    }
}

impl AppControlServer {
    /// Shared tail of `ui_chart` / `ui_graph`: render into an explicit target, or
    /// wrap in a dock panel when none was given.
    fn emit_visual(
        &self,
        target: Option<String>,
        id: Option<String>,
        kind: &str,
        node: Value,
        spec: &Value,
    ) -> Result<CallToolResult, ErrorData> {
        let title = spec
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string);
        match target {
            Some(t) => {
                validate_target(&t)?;
                self.bridge.emit(json!({
                    "cmd": "render",
                    "target": t,
                    "mode": "replace",
                    "body": [node],
                }))?;
                ok_text(format!("Drew the {kind} into {t}."))
            }
            None => {
                let panel_id = id.unwrap_or_else(|| self.bridge.next_id(kind));
                let evicted = self.note_panel(&panel_id);
                if let Some(old) = &evicted {
                    self.bridge
                        .emit(json!({ "cmd": "panel", "id": old, "remove": true }))?;
                }
                self.bridge.emit(json!({
                    "cmd": "panel",
                    "id": panel_id,
                    "title": title,
                    "place": "dock",
                    "collapsible": true,
                    "body": [node],
                }))?;
                ok_text(format!("Drew the {kind} in panel \"{panel_id}\"."))
            }
        }
    }

    /// Placement tail for `ui_figure`, mirroring [`emit_visual`](Self::emit_visual):
    /// render into an explicit target, or wrap in a dock panel when none is given.
    fn emit_figure(
        &self,
        target: Option<String>,
        title: Option<String>,
        tool: &str,
        node: Value,
    ) -> Result<CallToolResult, ErrorData> {
        match target {
            Some(t) => {
                validate_target(&t)?;
                self.bridge.emit(json!({
                    "cmd": "render",
                    "target": t,
                    "mode": "replace",
                    "body": [node],
                }))?;
                ok_text(format!("Rendered the {tool} figure into {t}."))
            }
            None => {
                let panel_id = self.bridge.next_id("figure");
                let evicted = self.note_panel(&panel_id);
                if let Some(old) = &evicted {
                    self.bridge
                        .emit(json!({ "cmd": "panel", "id": old, "remove": true }))?;
                }
                self.bridge.emit(json!({
                    "cmd": "panel",
                    "id": panel_id,
                    "title": title,
                    "place": "dock",
                    "collapsible": true,
                    "body": [node],
                }))?;
                ok_text(format!(
                    "Rendered the {tool} figure in panel \"{panel_id}\"."
                ))
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AppControlServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "biorouter-appcontrol".to_string(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: None,
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "You can drive this app's interface, not just write text into it. Build panels \
                 and dashboards (ui_panel), render into the author's own regions (ui_render), \
                 draw charts and graphs (ui_chart / ui_graph), direct attention (ui_highlight), \
                 restyle (ui_theme), rearrange (ui_layout), notify (ui_notify), share state \
                 (ui_state), and ask the user structured questions mid-turn (ui_ask). Once \
                 something is on the page, prefer ui_patch to edit individual nodes by id \
                 (add / replace / set_props / remove) instead of re-rendering: it preserves \
                 scroll, focus, and input state. For publication-grade figures (volcano, \
                 Kaplan-Meier, Sankey, forest, heatmap, maps, Mermaid diagrams, multi-figure \
                 dashboards) call ui_figure with the Auto Visualiser tool name and its args. \
                 ui_html renders sanitized rich HTML, but only when the app grants it. Call \
                 app_call to invoke a function the app author declared (its verbs live in the \
                 declared surface, so call ui_describe to see the app's declared actions and their \
                 argument schemas), emit_result to hand the app a structured result when it is \
                 awaiting one, and ui_subscribe to listen for app→agent signals. Call \
                 ui_describe first to learn what the page already offers (regions, node ids, \
                 declared surface, declared actions/signals)."
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

/// The system-prompt section appended for apps that hold the `ui` capability.
/// Tells the model the tools exist and, crucially, *when* to reach for them.
pub fn ui_system_prompt(cap: &UiCapability) -> String {
    let mut s = String::from(
        "\n\n## Driving this app's interface\n\
         You are not limited to writing text. This app grants you `ui_*` tools that change the \
         page the user is looking at. Two rules override your usual habits:\n\n\
         **1. Never ask the user for information in prose.** If you are missing an input (data \
         to analyse, a threshold, a cohort, a file, a confirmation), call `ui_ask`. It renders a \
         form and returns their answers to you as the tool result, so you keep going in the same \
         turn. Writing \"please paste your data\" and stopping is a failure: many of these apps \
         have no chat box at all, so the user has nowhere to answer you. Check `hasChat` in \
         `ui_describe` if unsure.\n\n\
         **2. Put structured results on the page, not in a paragraph.** Before you finish a turn, \
         anything with structure belongs in the UI:\n\
         - Comparisons, distributions, time series → `ui_chart`.\n\
         - Entities and their relationships → `ui_graph`.\n\
         - Several metrics at once → a `ui_panel` of `stat` cards, or `ui_layout(\"dashboard\")`.\n\
         - Rows of evidence → a `table` node via `ui_panel` / `ui_render`.\n\
         - Pointing at something already on screen → `ui_highlight`.\n\
         - Long jobs → `ui_notify` for progress.\n\n\
         Call `ui_describe` once, up front, so you render into regions that actually exist. \
         Panels are addressed by a stable `id`: reuse the id to refresh a panel in place rather \
         than stacking near-duplicates. Then keep your prose short: say what changed and why it \
         matters, not what the panel already shows.\n\n\
         Be economical with tool calls: you have a bounded number of actions per turn. Call \
         `ui_describe` ONCE (the page rarely changes mid-turn). Do NOT re-send `ui_state` values \
         you already set. Set a key only when it actually changes; identical repeats do nothing \
         and waste the budget. Batch your UI updates: build each panel/chart once with its final \
         content rather than nudging it repeatedly. A turn is for acting on the user's input, not \
         for polling the page.\n\n\
         **3. Update in place, don't re-render.** Once a panel or region is on the page, prefer \
         `ui_patch` to change individual nodes by id: `add`, `replace`, `set_props`, `remove`. \
         It keeps scroll, focus, and input state alive where a full re-render would destroy them. \
         Nodes you build with `ui_panel`/`ui_render` come back with stable ids (listed in the \
         tool result and in `ui_describe`); target those.\n\n\
         **4. Reach for real figures.** For anything publication-grade (volcano, Manhattan, \
         Kaplan-Meier, forest, Sankey, chord, heatmap, maps, Mermaid diagrams, or a multi-figure \
         dashboard), call `ui_figure` with the Auto Visualiser tool name and its arguments \
         instead of hand-building a chart. It is richer and more correct than `ui_chart`.\n\n\
         **5. Call into the app's own logic.** Some apps declare *actions* (verbs the author \
         wired to real handlers) and *signals* (notifications the app can push you). Use \
         `app_call` to run a declared action, and call `ui_describe` first to see the app's declared \
         actions and their argument schemas. When the app is awaiting a structured answer, use \
         `emit_result` to hand it a typed object rather than writing prose.\n\n\
         You are ALREADY subscribed to every signal the app declares, so you do not need to call \
         `ui_subscribe` to start receiving them, and calling it to \"turn them on\" is wasted \
         budget. (The user clicks before your first tool call, so a subscription you had to \
         request would always arrive too late.) Signals you have received appear in your turn \
         context. `ui_subscribe` exists only to add a signal the app explicitly opted out of \
         eager delivery. `ui_describe` lists what you are listening to.",
    );
    if !cap.allow_signals {
        s.push_str("\n\nNote: `ui_subscribe` is disabled for this app.");
    }
    if !cap.allow_ask {
        s.push_str("\n\nNote: `ui_ask` is disabled for this app.");
    }
    if !cap.allow_theme {
        s.push_str("\n\nNote: `ui_theme` is disabled for this app.");
    }
    if !cap.allow_layout {
        s.push_str("\n\nNote: `ui_layout` is disabled for this app.");
    }
    if cap.allow_html {
        s.push_str(
            "\n\nThis app grants `ui_html`: you may render rich HTML (it is sanitized \
             server-side: scripts, styles, forms, iframes and unsafe URLs are stripped). \
             Prefer widgets and `ui_figure` for structured content; reach for `ui_html` only \
             when you genuinely need free-form markup.",
        );
    } else {
        s.push_str("\n\nNote: `ui_html` is disabled for this app.");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> (AppControlServer, mpsc::UnboundedReceiver<Value>) {
        server_with_surface(SurfaceDecl::default())
    }

    fn server_with_surface(
        surface: SurfaceDecl,
    ) -> (AppControlServer, mpsc::UnboundedReceiver<Value>) {
        let bridge = UiBridge::new();
        let (rx, _tok) = bridge.attach();
        (
            AppControlServer::new(bridge, UiCapability::default(), surface),
            rx,
        )
    }

    fn text_of(r: &CallToolResult) -> String {
        r.content
            .iter()
            .flat_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("")
    }

    /// A non-privileged validation context against an empty surface — the shape
    /// every generic agent tree is checked with.
    fn empty_surface() -> &'static SurfaceDecl {
        use std::sync::OnceLock;
        static S: OnceLock<SurfaceDecl> = OnceLock::new();
        S.get_or_init(SurfaceDecl::default)
    }
    fn gctx() -> WidgetCtx<'static> {
        WidgetCtx {
            allow_privileged: false,
            surface: empty_surface(),
        }
    }
    /// Convenience: validate a single node with a fresh budget.
    fn vw(node: &Value, ctx: &WidgetCtx) -> Result<(), String> {
        let mut b = MAX_WIDGET_NODES;
        validate_widget(node, 0, &mut b, ctx)
    }

    #[test]
    fn validates_widget_kinds_and_shapes() {
        assert!(vw(&json!({"t":"text","value":"hi"}), &gctx()).is_ok());

        let e = vw(&json!({"t":"blah"}), &gctx()).unwrap_err();
        assert!(e.contains("unknown widget type"), "{e}");

        let e = vw(&json!({"t":"text"}), &gctx()).unwrap_err();
        assert!(e.contains("string \"value\""), "{e}");

        // A card must declare children.
        assert!(vw(&json!({"t":"card","title":"x"}), &gctx()).is_err());
    }

    #[test]
    fn rejects_ragged_tables() {
        let node = json!({"t":"table","columns":["a","b"],"rows":[["1","2"],["3"]]});
        let e = vw(&node, &gctx()).unwrap_err();
        assert!(e.contains("row 1 has 1 cells"), "{e}");
    }

    #[test]
    fn rejects_nonfinite_and_empty_charts() {
        assert!(validate_chart(&json!({"data":[]})).is_err());
        assert!(validate_chart(&json!({"type":"donut","data":[{"label":"a","value":1}]})).is_err());
        assert!(validate_chart(&json!({"data":[{"label":"a"}]})).is_err());
        assert!(validate_chart(&json!({"data":[{"label":"a","value":1.5}]})).is_ok());
    }

    /// H2: multi-series charts (train vs val loss, comparisons, forecasts).
    #[test]
    fn accepts_multi_series_charts() {
        let ok = json!({"type":"line","title":"Loss","series":[
            {"name":"train","data":[{"label":"e1","value":0.9},{"label":"e2","value":0.5}]},
            {"name":"val","data":[{"label":"e1","value":1.0},{"label":"e2","value":0.7}]}
        ]});
        assert!(validate_chart(&ok).is_ok(), "two-series line must validate");
        // a series missing its data array is a precise error
        let bad = json!({"type":"line","series":[{"name":"x"}]});
        let e = bad_err(&bad);
        assert!(e.contains("series[0]") && e.contains("data"), "{e}");
        // a non-finite point inside a series is caught with a series-scoped path
        let nf = json!({"series":[{"name":"a","data":[{"label":"p","value":1}]},
                                  {"name":"b","data":[{"label":"p","value":"NaN"}]}]});
        assert!(bad_err(&nf).contains("series[1].data"));
        // empty series array rejected
        assert!(validate_chart(&json!({"series":[]})).is_err());
        // the single-series error now points the model at `series` too
        assert!(bad_err(&json!({"type":"bar"})).contains("series"));
    }

    fn bad_err(spec: &Value) -> String {
        validate_chart(spec).unwrap_err()
    }

    #[test]
    fn widget_depth_and_node_budget_are_bounded() {
        // Build a chain deeper than MAX_WIDGET_DEPTH.
        let mut node = json!({"t":"text","value":"leaf"});
        for _ in 0..(MAX_WIDGET_DEPTH + 2) {
            node = json!({"t":"col","children":[node]});
        }
        let e = vw(&node, &gctx()).unwrap_err();
        assert!(e.contains("nested deeper"), "{e}");

        // And a wide-but-shallow tree exhausts the node budget.
        let children: Vec<Value> = (0..MAX_WIDGET_NODES + 5)
            .map(|i| json!({"t":"text","value": i.to_string()}))
            .collect();
        let e = vw(&json!({"t":"col","children":children}), &gctx()).unwrap_err();
        assert!(e.contains("exceeds"), "{e}");
    }

    // ── ui_suggest (SDK v2 §3.5, mixed initiative) ──────────────────────────

    #[tokio::test]
    async fn suggest_emits_a_ui_frame_with_chips() {
        let (s, mut rx) = server();
        let r = s
            .ui_suggest(Parameters(SuggestParams {
                chips: vec![
                    SuggestChip {
                        label: "Show KM curve".into(),
                        prompt: Some("Plot the Kaplan-Meier survival curve".into()),
                    },
                    SuggestChip {
                        label: "Just this label".into(),
                        prompt: None,
                    },
                ],
                target: Some("@region:results".into()),
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("2"));
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["type"], "ui");
        assert_eq!(cmd["cmd"], "suggest");
        assert_eq!(cmd["target"], "@region:results");
        let chips = cmd["chips"].as_array().unwrap();
        assert_eq!(chips.len(), 2);
        assert_eq!(chips[0]["label"], "Show KM curve");
        assert_eq!(chips[0]["prompt"], "Plot the Kaplan-Meier survival curve");
        // A promptless chip omits the field (SDK hands the click to onCommand).
        assert_eq!(chips[1]["label"], "Just this label");
        assert!(chips[1].get("prompt").is_none());
    }

    #[tokio::test]
    async fn suggest_rejects_empty_chip_list() {
        let (s, mut rx) = server();
        let err = s
            .ui_suggest(Parameters(SuggestParams {
                chips: vec![],
                target: None,
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("at least one"));
        assert!(rx.try_recv().is_err(), "no frame on rejection");
    }

    #[tokio::test]
    async fn suggest_caps_at_five_chips() {
        let (s, mut rx) = server();
        let chips = (0..6)
            .map(|i| SuggestChip {
                label: format!("chip {i}"),
                prompt: None,
            })
            .collect();
        let err = s
            .ui_suggest(Parameters(SuggestParams {
                chips,
                target: None,
            }))
            .await
            .unwrap_err();
        assert!(err.message.contains("5"));
        assert!(rx.try_recv().is_err(), "no frame on rejection");
    }

    #[tokio::test]
    async fn suggest_rejects_empty_and_overlong_fields() {
        let (s, _rx) = server();
        // Empty label.
        assert!(s
            .ui_suggest(Parameters(SuggestParams {
                chips: vec![SuggestChip {
                    label: "   ".into(),
                    prompt: None
                }],
                target: None,
            }))
            .await
            .is_err());
        // Overlong label (>80).
        assert!(s
            .ui_suggest(Parameters(SuggestParams {
                chips: vec![SuggestChip {
                    label: "x".repeat(81),
                    prompt: None
                }],
                target: None,
            }))
            .await
            .is_err());
        // Overlong prompt (>500).
        assert!(s
            .ui_suggest(Parameters(SuggestParams {
                chips: vec![SuggestChip {
                    label: "ok".into(),
                    prompt: Some("x".repeat(501))
                }],
                target: None,
            }))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn panel_emits_a_ui_frame() {
        let (s, mut rx) = server();
        let r = s
            .ui_panel(Parameters(PanelParams {
                id: "summary".into(),
                title: Some("Summary".into()),
                place: None,
                body: Some(json!([{"t":"text","value":"hello"}])),
                markdown: None,
                collapsible: None,
                remove: None,
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("summary"));
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["type"], "ui");
        assert_eq!(cmd["cmd"], "panel");
        assert_eq!(cmd["id"], "summary");
        assert_eq!(cmd["place"], "dock");
    }

    /// H1: a real model (GPT-5.5) naturally mounts a dashboard into an author
    /// region with `place:"@region:dashboard"`. That used to be rejected
    /// ("must be one of: dock, left, …") and cost 5–7 retries. It must be
    /// accepted and passed through as the place.
    #[tokio::test]
    async fn panel_accepts_a_region_target_as_place() {
        let (s, mut rx) = server();
        let r = s
            .ui_panel(Parameters(PanelParams {
                id: "sentiment-dashboard".into(),
                title: Some("Sentiment".into()),
                place: Some("@region:dashboard".into()),
                body: Some(
                    json!([{"t":"row","children":[{"t":"stat","label":"Positive","value":2}]}]),
                ),
                markdown: None,
                collapsible: Some(false),
                remove: None,
            }))
            .await
            .expect("a @region: place must be accepted");
        assert!(text_of(&r).contains("dashboard"));
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["place"], "@region:dashboard");
    }

    #[tokio::test]
    async fn panel_still_rejects_an_empty_place_target() {
        let (s, _rx) = server();
        // An empty/whitespace place is neither a dock slot nor a valid target.
        let e = s
            .ui_panel(Parameters(PanelParams {
                id: "x".into(),
                title: None,
                place: Some("   ".into()),
                body: Some(json!([{"t":"text","value":"y"}])),
                markdown: None,
                collapsible: None,
                remove: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.to_lowercase().contains("empty") || e.message.contains("target"));
    }

    #[tokio::test]
    async fn panel_markdown_shorthand_becomes_a_text_node() {
        let (s, mut rx) = server();
        s.ui_panel(Parameters(PanelParams {
            id: "notes".into(),
            title: None,
            place: Some("right".into()),
            body: None,
            markdown: Some("# Hi".into()),
            collapsible: None,
            remove: None,
        }))
        .await
        .unwrap();
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["body"][0]["t"], "text");
        assert_eq!(cmd["body"][0]["markdown"], true);
    }

    #[tokio::test]
    async fn panel_cap_evicts_the_oldest() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let cap = UiCapability {
            max_panels: 2,
            ..Default::default()
        };
        let s = AppControlServer::new(bridge, cap, SurfaceDecl::default());
        for id in ["a", "b", "c"] {
            s.ui_panel(Parameters(PanelParams {
                id: id.into(),
                title: None,
                place: None,
                body: Some(json!([{"t":"text","value":"x"}])),
                markdown: None,
                collapsible: None,
                remove: None,
            }))
            .await
            .unwrap();
        }
        let cmds: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        // a, b, then (remove a), c
        let removes: Vec<&Value> = cmds.iter().filter(|c| c["remove"] == true).collect();
        assert_eq!(removes.len(), 1);
        assert_eq!(removes[0]["id"], "a");
    }

    #[tokio::test]
    async fn theme_rejects_css_injection_in_accent() {
        let (s, _rx) = server();
        let e = s
            .ui_theme(Parameters(ThemeParams {
                pack: None,
                accent: Some("red; background:url(http://x)".into()),
                mode: None,
                density: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("simple CSS colour"), "{}", e.message);
    }

    #[tokio::test]
    async fn theme_and_layout_respect_capability_gates() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let cap = UiCapability {
            allow_theme: false,
            allow_layout: false,
            ..Default::default()
        };
        let s = AppControlServer::new(bridge, cap, SurfaceDecl::default());
        assert!(s
            .ui_theme(Parameters(ThemeParams {
                pack: None,
                accent: None,
                mode: Some("dark".into()),
                density: None
            }))
            .await
            .is_err());
        assert!(s
            .ui_layout(Parameters(LayoutParams {
                preset: Some("dashboard".into()),
                sidebar_width: None,
                areas: None,
                sizes: None,
            }))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn theme_pack_switch_validates_and_emits() {
        let (s, mut rx) = server();
        let r = s
            .ui_theme(Parameters(ThemeParams {
                pack: Some("terminal".into()),
                accent: Some("#3ddc84".into()),
                mode: None,
                density: Some("compact".into()),
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("Theme updated"));
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "theme");
        assert_eq!(cmd["pack"], "terminal");
        assert_eq!(cmd["accent"], "#3ddc84");
        assert_eq!(cmd["density"], "compact");

        // Every curated pack name is accepted.
        for pack in THEME_PACKS {
            assert!(s
                .ui_theme(Parameters(ThemeParams {
                    pack: Some((*pack).to_string()),
                    accent: None,
                    mode: None,
                    density: None,
                }))
                .await
                .is_ok());
        }

        // An unknown pack is rejected and the message names the field.
        let e = s
            .ui_theme(Parameters(ThemeParams {
                pack: Some("neon".into()),
                accent: None,
                mode: None,
                density: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("pack"), "{}", e.message);
    }

    #[tokio::test]
    async fn layout_preset_still_works_and_emits() {
        let (s, mut rx) = server();
        let r = s
            .ui_layout(Parameters(LayoutParams {
                preset: Some("sidebar-right".into()),
                sidebar_width: Some(320),
                areas: None,
                sizes: None,
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("sidebar-right"));
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "layout");
        assert_eq!(cmd["preset"], "sidebar-right");
        assert_eq!(cmd["sidebarWidth"], 320);
        assert!(cmd["areas"].is_null());

        // A bad preset is still rejected.
        assert!(s
            .ui_layout(Parameters(LayoutParams {
                preset: Some("holodeck".into()),
                sidebar_width: None,
                areas: None,
                sizes: None,
            }))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn layout_grammar_validates_areas_and_sizes() {
        let (s, mut rx) = server();
        let areas = vec![
            vec!["nav".to_string(), "nav".to_string()],
            vec!["side".to_string(), "main".to_string()],
        ];
        let mut sizes = HashMap::new();
        sizes.insert("side".to_string(), "240px".to_string());
        sizes.insert("main".to_string(), "1fr".to_string());
        let r = s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: Some(areas.clone()),
                sizes: Some(sizes),
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("2×2 grid"), "{}", text_of(&r));
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "layout");
        assert_eq!(cmd["areas"][0][0], "nav");
        assert_eq!(cmd["areas"][1][1], "main");
        assert_eq!(cmd["sizes"]["side"], "240px");
        assert!(cmd["preset"].is_null());

        // Neither preset nor areas → error.
        assert!(s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: None,
                sizes: None,
            }))
            .await
            .is_err());

        // A ragged grid is rejected.
        let ragged = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["c".to_string()],
        ];
        let e = s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: Some(ragged),
                sizes: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("rectangular"), "{}", e.message);

        // Too many rows.
        let tall: Vec<Vec<String>> = (0..5).map(|_| vec!["a".to_string()]).collect();
        let e = s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: Some(tall),
                sizes: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("at most 4 rows"), "{}", e.message);

        // Too many columns.
        let wide = vec![vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
            "e".to_string(),
        ]];
        let e = s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: Some(wide),
                sizes: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("at most 4 columns"), "{}", e.message);

        // An illegal area name (spaces/caps).
        let bad_name = vec![vec!["Main Panel".to_string()]];
        let e = s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: Some(bad_name),
                sizes: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("lowercase name"), "{}", e.message);
        // The CSS empty-cell token is allowed.
        assert!(s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: Some(vec![vec!["main".to_string(), ".".to_string()]]),
                sizes: None,
            }))
            .await
            .is_ok());

        // A size outside the bounded vocabulary.
        let mut bad_sizes = HashMap::new();
        bad_sizes.insert("main".to_string(), "1000px".to_string()); // > 800
        let e = s
            .ui_layout(Parameters(LayoutParams {
                preset: None,
                sidebar_width: None,
                areas: Some(areas.clone()),
                sizes: Some(bad_sizes),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("must be one of"), "{}", e.message);
    }

    #[tokio::test]
    async fn state_round_trips_and_emits() {
        let (s, mut rx) = server();
        let r = s
            .ui_state(Parameters(StateParams {
                set: Some(json!({"gene":"TP53","n":3})),
                remove: None,
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("TP53"));
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "state");
        assert_eq!(cmd["mode"], "snapshot");
        assert_eq!(cmd["doc"]["gene"], "TP53");
        assert_eq!(cmd["version"], 1);

        let r = s
            .ui_state(Parameters(StateParams {
                set: None,
                remove: Some(vec!["gene".into()]),
            }))
            .await
            .unwrap();
        assert!(!text_of(&r).contains("TP53"));
    }

    /// H3: re-sending the SAME state is a cheap no-op that emits no frame and
    /// tells the model so — this is what breaks the ui_state cascade that
    /// exhausts the turn budget.
    #[tokio::test]
    async fn state_noop_when_unchanged_emits_nothing() {
        let (s, mut rx) = server();
        s.ui_state(Parameters(StateParams {
            set: Some(json!({"dose": 1000})),
            remove: None,
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["cmd"], "state"); // first change emits

        // identical re-send: no frame, "no change" message
        let r = s
            .ui_state(Parameters(StateParams {
                set: Some(json!({"dose": 1000})),
                remove: None,
            }))
            .await
            .unwrap();
        assert!(
            text_of(&r).to_lowercase().contains("no change"),
            "{}",
            text_of(&r)
        );
        assert!(
            rx.try_recv().is_err(),
            "an unchanged ui_state must emit no frame"
        );

        // a real change emits again
        s.ui_state(Parameters(StateParams {
            set: Some(json!({"dose": 1250})),
            remove: None,
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["doc"]["dose"], 1250);
    }

    /// H3: a repeated ui_describe over an unchanged page is flagged so the model
    /// stops re-polling.
    #[tokio::test]
    async fn describe_flags_an_unchanged_repeat() {
        let (s, _rx) = server();
        let first = text_of(&s.ui_describe(Parameters(DescribeParams {})).await.unwrap());
        assert!(!first.to_lowercase().contains("unchanged"));
        let second = text_of(&s.ui_describe(Parameters(DescribeParams {})).await.unwrap());
        assert!(second.to_lowercase().contains("unchanged"), "{second}");
    }

    #[tokio::test]
    async fn ask_parks_then_resolves_with_the_users_payload() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );

        let task = tokio::spawn(async move {
            s.ui_ask(Parameters(AskParams {
                prompt: "Pick a threshold".into(),
                fields: vec![AskField {
                    name: "p".into(),
                    label: None,
                    r#type: Some("number".into()),
                    options: None,
                    value: None,
                    placeholder: None,
                }],
                title: None,
                submit_label: None,
            }))
            .await
        });

        let cmd = loop {
            match rx.recv().await {
                Some(c) if c["cmd"] == "ask" => break c,
                Some(_) => continue,
                None => panic!("channel closed"),
            }
        };
        let rid = cmd["requestId"].as_str().unwrap().to_string();
        assert!(bridge.resolve(&rid, json!({"p": "0.05"})));

        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("0.05"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn ask_returns_a_usable_result_when_the_user_dismisses() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        let task = tokio::spawn(async move {
            s.ui_ask(Parameters(AskParams {
                prompt: "?".into(),
                fields: vec![AskField {
                    name: "x".into(),
                    label: None,
                    r#type: None,
                    options: None,
                    value: None,
                    placeholder: None,
                }],
                title: None,
                submit_label: None,
            }))
            .await
        });
        // Drain the emitted ask frame, then simulate the socket closing.
        let _ = rx.recv().await;
        bridge.cancel_all();
        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("dismissed"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn ask_rejects_a_select_without_options() {
        let (s, _rx) = server();
        let e = s
            .ui_ask(Parameters(AskParams {
                prompt: "?".into(),
                fields: vec![AskField {
                    name: "k".into(),
                    label: None,
                    r#type: Some("select".into()),
                    options: None,
                    value: None,
                    placeholder: None,
                }],
                title: None,
                submit_label: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("non-empty \"options\""), "{}", e.message);
    }

    /// A reload reuses the cached agent (and therefore the already-injected
    /// `AppControlServer`). Re-attaching must point its tools at the NEW socket,
    /// otherwise every `ui_*` call after the first reload fails forever.
    #[tokio::test]
    async fn rebinding_the_bridge_keeps_a_reused_server_working() {
        let bridge = UiBridge::new();
        let (first, _tok1) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );

        // The browser reloads: old channel dropped, new one attached.
        drop(first);
        let (mut second, _tok2) = bridge.attach();

        s.ui_notify(Parameters(NotifyParams {
            message: "still here".into(),
            level: None,
            timeout_ms: None,
        }))
        .await
        .expect("ui_* must work after a reconnect");
        let cmd = second.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "notify");
        assert_eq!(cmd["message"], "still here");
    }

    /// A reload race: the new connection attaches, then the OLD connection's
    /// socket-close handler finally runs `detach`. Without the generation guard,
    /// that stale detach would null the fresh connection's channel and cancel its
    /// parked `ui_ask`. The token makes the stale detach a no-op.
    #[tokio::test]
    async fn a_stale_detach_after_reattach_is_a_noop() {
        let bridge = UiBridge::new();
        let (_first_rx, first_tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );

        // Browser reloads: fresh connection attaches…
        let (mut second_rx, _second_tok) = bridge.attach();
        // …then the OLD connection's close handler fires, late.
        bridge.detach(first_tok);

        // The fresh connection's tools must still work.
        s.ui_notify(Parameters(NotifyParams {
            message: "alive".into(),
            level: None,
            timeout_ms: None,
        }))
        .await
        .expect("a stale detach must not tear down the fresh connection");
        assert_eq!(second_rx.try_recv().unwrap()["message"], "alive");

        // And a parked ask on the fresh connection is NOT cancelled by it.
        let s2 = s.clone();
        let ask = tokio::spawn(async move {
            s2.ui_ask(Parameters(AskParams {
                prompt: "?".into(),
                fields: vec![AskField {
                    name: "x".into(),
                    label: None,
                    r#type: None,
                    options: None,
                    value: None,
                    placeholder: None,
                }],
                title: None,
                submit_label: None,
            }))
            .await
        });
        // Drain the ask frame, fire the stale detach again — still a no-op.
        loop {
            match second_rx.recv().await {
                Some(c) if c["cmd"] == "ask" => break,
                Some(_) => continue,
                None => panic!("channel closed"),
            }
        }
        bridge.detach(first_tok);
        // The ask is still parked (not cancelled): a short wait must time out.
        assert!(
            tokio::time::timeout(Duration::from_millis(300), ask)
                .await
                .is_err(),
            "a stale detach must not cancel the fresh connection's parked ask"
        );
    }

    #[test]
    fn a_detach_paused_at_its_sender_cannot_clear_a_concurrent_reattach() {
        let bridge = UiBridge::new();
        let (_old_frames, old_token) = bridge.attach();

        // Force detach to pause after claiming the connection lifecycle, while
        // a reload tries to install a new sender and park a new UI request.
        let sender = bridge.inner.tx.lock().unwrap();
        let closing = bridge.clone();
        let detached = std::thread::spawn(move || closing.detach(old_token));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if matches!(
                bridge.inner.generation.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detach did not claim the lifecycle lock"
            );
            std::thread::yield_now();
        }

        let reloading = bridge.clone();
        let attached = std::thread::spawn(move || {
            let (frames, token) = reloading.attach();
            let answer = reloading.register_ask("concurrent-reconnect-ask".to_string());
            assert!(reloading.emit_frame(json!({"message":"fresh connection"})));
            (frames, token, answer)
        });
        drop(sender);
        detached.join().unwrap();
        let (mut frames, token, mut answer) = attached.join().unwrap();
        assert_eq!(frames.try_recv().unwrap()["message"], "fresh connection");
        assert!(bridge.resolve("concurrent-reconnect-ask", json!({"answer":"kept"})));
        assert_eq!(answer.try_recv().unwrap(), json!({"answer":"kept"}));
        bridge.detach(token);
    }

    /// Panels are per-connection (the reloaded DOM has none), but the semantic
    /// state bag survives and is replayed so the fresh page rehydrates.
    #[tokio::test]
    async fn attach_resets_panels_and_replays_state() {
        let bridge = UiBridge::new();
        let (mut first, _tok1) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        s.ui_state(Parameters(StateParams {
            set: Some(json!({"gene":"BRCA1"})),
            remove: None,
        }))
        .await
        .unwrap();
        s.ui_panel(Parameters(PanelParams {
            id: "p".into(),
            title: None,
            place: None,
            body: Some(json!([{"t":"text","value":"x"}])),
            markdown: None,
            collapsible: None,
            remove: None,
        }))
        .await
        .unwrap();
        while first.try_recv().is_ok() {}

        let (mut second, _tok2) = bridge.attach();
        // State is replayed to the fresh page as one snapshot frame…
        let replay = second.try_recv().unwrap();
        assert_eq!(replay["cmd"], "state");
        assert_eq!(replay["mode"], "snapshot");
        assert_eq!(replay["doc"]["gene"], "BRCA1");
        assert_eq!(replay["v"], 1);
        // …and the panel registry starts empty again.
        let r = s.ui_describe(Parameters(DescribeParams {})).await.unwrap();
        let described: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(described["panels"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn detach_unblocks_a_parked_ask() {
        let bridge = UiBridge::new();
        let (mut rx, tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        let task = tokio::spawn(async move {
            s.ui_ask(Parameters(AskParams {
                prompt: "?".into(),
                fields: vec![AskField {
                    name: "x".into(),
                    label: None,
                    r#type: None,
                    options: None,
                    value: None,
                    placeholder: None,
                }],
                title: None,
                submit_label: None,
            }))
            .await
        });
        let _ = rx.recv().await;
        bridge.detach(tok);
        // Must not hang: detach() cancels every parked ask.
        let r = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("ui_ask should not outlive its socket")
            .unwrap()
            .unwrap();
        assert!(text_of(&r).contains("dismissed"));
    }

    /// Live runs against a small local model showed the failure this text exists
    /// to prevent: asked for missing data, the agent wrote "please paste your
    /// abstracts" and ended the turn — into an app with no chat box. The prompt
    /// must state the `ui_ask` rule imperatively, not as a preference.
    #[test]
    fn system_prompt_forbids_asking_in_prose() {
        let s = ui_system_prompt(&UiCapability::default());
        assert!(
            s.contains("Never ask the user for information in prose"),
            "the ui_ask rule must be imperative"
        );
        assert!(s.contains("`ui_ask`"));
        assert!(
            s.contains("hasChat"),
            "tell the agent how to check for a chat box"
        );
        // And every tool the capability grants is named, so the model knows they exist.
        for tool in [
            "ui_chart",
            "ui_graph",
            "ui_panel",
            "ui_highlight",
            "ui_notify",
            "ui_describe",
        ] {
            assert!(s.contains(tool), "system prompt never mentions {tool}");
        }
    }

    #[test]
    fn system_prompt_announces_revoked_switches() {
        let cap = UiCapability {
            allow_ask: false,
            allow_theme: false,
            allow_layout: false,
            ..Default::default()
        };
        let s = ui_system_prompt(&cap);
        assert!(s.contains("`ui_ask` is disabled"));
        assert!(s.contains("`ui_theme` is disabled"));
        assert!(s.contains("`ui_layout` is disabled"));
    }

    #[tokio::test]
    async fn tools_fail_cleanly_once_the_socket_is_gone() {
        let (s, rx) = server();
        drop(rx);
        let e = s
            .ui_notify(Parameters(NotifyParams {
                message: "hi".into(),
                level: None,
                timeout_ms: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("no longer connected"), "{}", e.message);
    }

    #[tokio::test]
    async fn describe_reports_surface_panels_and_state() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        bridge.set_surface(json!({"regions":["results"],"ids":["out"]}));
        s.ui_panel(Parameters(PanelParams {
            id: "p1".into(),
            title: None,
            place: None,
            body: Some(json!([{"t":"text","value":"x"}])),
            markdown: None,
            collapsible: None,
            remove: None,
        }))
        .await
        .unwrap();
        let r = s.ui_describe(Parameters(DescribeParams {})).await.unwrap();
        let t = text_of(&r);
        assert!(t.contains("results"), "{t}");
        assert!(t.contains("p1"), "{t}");
    }

    /// Observed live with qwen3.6 via Ollama: because `spec` is a bare `Value`,
    /// the model JSON-*encodes* it into a string. It then retried three times and
    /// never recovered. Accept it.
    #[tokio::test]
    async fn chart_accepts_a_spec_the_model_stringified() {
        let (s, mut rx) = server();
        s.ui_chart(Parameters(ChartParams {
            target: None,
            id: Some("gene-counts".into()),
            spec: json!(
                r#"{"type": "bar", "title": "Gene Counts", "data": [{"label": "TP53", "value": 120}]}"#
            ),
        }))
        .await
        .expect("a stringified spec must be accepted, not bounced back");
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["title"], "Gene Counts");
        assert_eq!(cmd["body"][0]["spec"]["data"][0]["label"], "TP53");
        assert_eq!(cmd["body"][0]["spec"]["type"], "bar");
    }

    #[tokio::test]
    async fn render_and_panel_and_state_accept_stringified_json() {
        let (s, mut rx) = server();
        s.ui_render(Parameters(RenderParams {
            target: "@region:results".into(),
            body: json!(r#"[{"t": "text", "value": "hi"}]"#),
            mode: None,
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["body"][0]["value"], "hi");

        s.ui_panel(Parameters(PanelParams {
            id: "p".into(),
            title: None,
            place: None,
            body: Some(json!(r#"[{"t": "badge", "value": "b"}]"#)),
            markdown: None,
            collapsible: None,
            remove: None,
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["body"][0]["t"], "badge");

        s.ui_state(Parameters(StateParams {
            set: Some(json!(r#"{"gene": "TP53"}"#)),
            remove: None,
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["doc"]["gene"], "TP53");
    }

    /// A string that isn't JSON at all still fails — but with a message that
    /// names the mistake instead of a bare "must be an object".
    #[tokio::test]
    async fn a_non_json_string_fails_with_an_actionable_message() {
        let (s, _rx) = server();
        let e = s
            .ui_chart(Parameters(ChartParams {
                target: None,
                id: None,
                spec: json!("just some prose"),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("not a JSON string"), "{}", e.message);
        assert!(e.message.contains(r#"{"type":"bar""#), "{}", e.message);
    }

    /// A bare `Value` generates a permissive `true` schema, which is what made
    /// the model guess. The tool must advertise the real shape.
    #[test]
    fn chart_and_graph_advertise_a_real_spec_schema() {
        let router = AppControlServer::tool_router();
        let tools: Vec<_> = router.list_all();
        let chart = tools
            .iter()
            .find(|t| t.name == "ui_chart")
            .expect("ui_chart");
        let spec = &chart.input_schema["properties"]["spec"];
        assert_eq!(spec["type"], "object", "spec schema: {spec}");
        assert!(
            spec["properties"]["data"].is_object(),
            "spec schema: {spec}"
        );

        let graph = tools
            .iter()
            .find(|t| t.name == "ui_graph")
            .expect("ui_graph");
        let gspec = &graph.input_schema["properties"]["spec"];
        assert_eq!(gspec["type"], "object", "graph spec schema: {gspec}");

        let render = tools
            .iter()
            .find(|t| t.name == "ui_render")
            .expect("ui_render");
        let body = &render.input_schema["properties"]["body"];
        assert_eq!(body["type"], "array", "body schema: {body}");
    }

    /// Subschemas must be inlined. `$ref`/`$defs` are legal JSON Schema, but
    /// several providers (notably Ollama-hosted models) mishandle them, and a
    /// tool whose schema the model can't read is a tool it guesses at.
    #[test]
    fn tool_schemas_contain_no_refs() {
        for tool in AppControlServer::tool_router().list_all() {
            let schema = serde_json::to_string(&tool.input_schema).unwrap();
            assert!(
                !schema.contains("$ref") && !schema.contains("$defs"),
                "{} schema must be self-contained: {schema}",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn chart_without_target_wraps_itself_in_a_dock_panel() {
        let (s, mut rx) = server();
        s.ui_chart(Parameters(ChartParams {
            target: None,
            id: None,
            spec: json!({"type":"bar","title":"Counts","data":[{"label":"a","value":1}]}),
        }))
        .await
        .unwrap();
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "panel");
        assert_eq!(cmd["place"], "dock");
        assert_eq!(cmd["title"], "Counts");
        assert_eq!(cmd["body"][0]["t"], "chart");
    }

    // ── Apps SDK v2: shared state document ──────────────────────────────────

    /// Every emitted ui frame is stamped with the catalog version.
    #[tokio::test]
    async fn ui_frames_carry_the_catalog_version() {
        let (s, mut rx) = server();
        s.ui_notify(Parameters(NotifyParams {
            message: "hi".into(),
            level: None,
            timeout_ms: None,
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["v"], CATALOG_VERSION);
    }

    #[tokio::test]
    async fn patch_state_applies_and_bumps_version() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        let r = s
            .ui_patch_state(Parameters(PatchStateParams {
                patch: json!([{"op":"add","path":"/count","value":3}]),
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("\"version\":1"), "{}", text_of(&r));
        assert!(text_of(&r).contains("\"ok\":true"), "{}", text_of(&r));

        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "state");
        assert_eq!(cmd["mode"], "patch");
        assert_eq!(cmd["version"], 1);
        assert_eq!(cmd["patch"][0]["op"], "add");
        assert_eq!(cmd["v"], CATALOG_VERSION);

        let (doc, version) = bridge.state_snapshot();
        assert_eq!(doc["count"], 3);
        assert_eq!(version, 1);
    }

    #[tokio::test]
    async fn patch_state_rejects_too_many_ops() {
        let (s, _rx) = server();
        let ops: Vec<Value> = (0..(STATE_MAX_PATCH_OPS + 1))
            .map(|i| json!({"op":"add","path":format!("/k{i}"),"value":i}))
            .collect();
        let e = s
            .ui_patch_state(Parameters(PatchStateParams { patch: json!(ops) }))
            .await
            .unwrap_err();
        assert!(e.message.contains("cap is 64"), "{}", e.message);
    }

    #[test]
    fn validate_state_doc_enforces_the_size_cap() {
        let big = "x".repeat(STATE_MAX_BYTES + 1_000);
        let e = validate_state_doc(&json!({ "blob": big })).unwrap_err();
        assert!(e.contains("bytes"), "{e}");
        assert!(validate_state_doc(&json!({ "a": 1 })).is_ok());
    }

    #[test]
    fn validate_state_doc_enforces_the_depth_cap() {
        // Nest well past the depth cap.
        let mut v = json!(1);
        for _ in 0..(STATE_MAX_DEPTH + 4) {
            v = json!({ "n": v });
        }
        let e = validate_state_doc(&v).unwrap_err();
        assert!(e.contains("nested deeper"), "{e}");
        // A shallow document is fine.
        assert!(validate_state_doc(&json!({ "a": { "b": 1 } })).is_ok());
    }

    /// A patch that would blow a cap is rejected and the live document is left
    /// untouched (applied against a clone, committed only when valid).
    #[tokio::test]
    async fn patch_state_over_cap_leaves_the_doc_unchanged() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        let big = "x".repeat(STATE_MAX_BYTES + 1_000);
        let e = s
            .ui_patch_state(Parameters(PatchStateParams {
                patch: json!([{"op":"add","path":"/blob","value":big}]),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("bytes"), "{}", e.message);
        let (doc, version) = bridge.state_snapshot();
        assert_eq!(version, 0, "a rejected patch must not bump the version");
        assert!(doc.as_object().unwrap().is_empty());
    }

    #[test]
    fn client_write_conflict_returns_the_current_doc() {
        let bridge = UiBridge::new();
        // First write advances the version to 1.
        let (_ops, v1) = bridge
            .apply_client_write(Some(("/x".into(), json!(1))), None, 0)
            .expect("first write applies");
        assert_eq!(v1, 1);
        // A stale write against base_version 0 conflicts and hands back the truth.
        match bridge.apply_client_write(Some(("/y".into(), json!(2))), None, 0) {
            Err(StateWriteError::Conflict(doc, ver)) => {
                assert_eq!(ver, 1);
                assert_eq!(doc["x"], 1);
                assert!(doc.get("y").is_none(), "the losing write must not apply");
            }
            other => panic!("expected a Conflict, got {other:?}"),
        }
    }

    #[test]
    fn client_write_pointer_set_creates_intermediate_objects() {
        let bridge = UiBridge::new();
        let (ops, ver) = bridge
            .apply_client_write(Some(("/a/b/c".into(), json!(5))), None, 0)
            .expect("pointer set applies");
        assert_eq!(ver, 1);
        assert_eq!(ops[0]["op"], "add");
        assert_eq!(ops[0]["path"], "/a/b/c");
        let (doc, _) = bridge.state_snapshot();
        assert_eq!(doc["a"]["b"]["c"], 5);
    }

    #[test]
    fn client_write_patch_round_trips() {
        let bridge = UiBridge::new();
        let (ops, ver) = bridge
            .apply_client_write(
                None,
                Some(json!([{"op":"add","path":"/hits","value":42}])),
                0,
            )
            .expect("patch applies");
        assert_eq!(ver, 1);
        assert_eq!(ops[0]["value"], 42);
        assert_eq!(bridge.state_snapshot().0["hits"], 42);
    }

    #[test]
    fn seed_state_only_applies_while_fresh() {
        let bridge = UiBridge::new();
        bridge.seed_state(json!({ "a": 1 }), 7);
        let (doc, ver) = bridge.state_snapshot();
        assert_eq!(ver, 7);
        assert_eq!(doc["a"], 1);
        // A second seed on a non-fresh doc is ignored.
        bridge.seed_state(json!({ "b": 2 }), 99);
        let (doc2, ver2) = bridge.state_snapshot();
        assert_eq!(ver2, 7);
        assert!(doc2.get("b").is_none());
    }

    /// A reconnect replays a single snapshot frame (not a `ui_state` bag), and
    /// only when there is state worth sending.
    #[tokio::test]
    async fn attach_replays_a_state_snapshot() {
        let bridge = UiBridge::new();
        // A fresh, empty session replays nothing.
        let (mut empty, _t0) = bridge.attach();
        assert!(empty.try_recv().is_err());

        bridge.seed_state(json!({ "gene": "BRCA1" }), 3);
        let (mut rx, _t1) = bridge.attach();
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame["cmd"], "state");
        assert_eq!(frame["mode"], "snapshot");
        assert_eq!(frame["doc"]["gene"], "BRCA1");
        assert_eq!(frame["version"], 3);
        assert_eq!(frame["v"], CATALOG_VERSION);
    }

    #[tokio::test]
    async fn describe_reports_the_declared_surface() {
        use crate::agent_drafter::manifest::{ActionDecl, ComponentDecl, SignalDecl};
        let surface = SurfaceDecl {
            state_schema: Some(json!({ "type": "object" })),
            actions: vec![ActionDecl {
                name: "move_avatar".into(),
                description: "Move the avatar".into(),
                params: json!({ "type": "object" }),
                ..Default::default()
            }],
            signals: vec![SignalDecl {
                name: "node_selected".into(),
                payload: None,
                coalesce_ms: 250,
                ..Default::default()
            }],
            components: vec![ComponentDecl {
                name: "pathway_map".into(),
                props: json!({}),
            }],
            ..Default::default()
        };
        let (s, _rx) = server_with_surface(surface);
        let r = s.ui_describe(Parameters(DescribeParams {})).await.unwrap();
        let d: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(d["declared"]["hasStateSchema"], true);
        assert_eq!(d["declared"]["actions"][0]["name"], "move_avatar");
        assert_eq!(
            d["declared"]["actions"][0]["description"],
            "Move the avatar"
        );
        assert_eq!(d["declared"]["signals"][0]["name"], "node_selected");
        assert_eq!(d["declared"]["signals"][0]["coalesceMs"], 250);
        assert_eq!(d["declared"]["components"][0]["name"], "pathway_map");
        assert_eq!(d["state"]["version"], 0);
        assert!(d["state"]["keys"].as_array().unwrap().is_empty());
    }

    // ── Apps SDK v2: catalog kinds ──────────────────────────────────────────

    #[test]
    fn markdown_kind_validates_and_caps() {
        assert!(vw(&json!({"t":"markdown","md":"# Hello"}), &gctx()).is_ok());
        // missing md
        let e = vw(&json!({"t":"markdown"}), &gctx()).unwrap_err();
        assert!(e.contains("string \"md\""), "{e}");
        // over the char cap
        let big = "x".repeat(MAX_MARKDOWN_CHARS + 1);
        let e = vw(&json!({"t":"markdown","md":big}), &gctx()).unwrap_err();
        assert!(e.contains("cap is"), "{e}");
    }

    #[test]
    fn image_kind_restricts_url_schemes() {
        assert!(vw(&json!({"t":"image","src":"https://x/y.png"}), &gctx()).is_ok());
        assert!(vw(&json!({"t":"image","src":"figures/a.png"}), &gctx()).is_ok());
        assert!(vw(
            &json!({"t":"image","src":"data:image/png;base64,AAAA"}),
            &gctx()
        )
        .is_ok());
        // http: refused
        let e = vw(&json!({"t":"image","src":"http://x/y.png"}), &gctx()).unwrap_err();
        assert!(e.contains("not allowed"), "{e}");
        // javascript: refused
        assert!(vw(&json!({"t":"image","src":"javascript:alert(1)"}), &gctx()).is_err());
        // data:text/html refused (not an image)
        let e = vw(&json!({"t":"image","src":"data:text/html,<b>"}), &gctx()).unwrap_err();
        assert!(e.contains("data:image/"), "{e}");
        // oversized data URL refused
        let huge = format!("data:image/png;base64,{}", "A".repeat(MAX_IMAGE_DATA_BYTES));
        assert!(vw(&json!({"t":"image","src":huge}), &gctx()).is_err());
    }

    #[test]
    fn kpi_kind_validates() {
        assert!(vw(
            &json!({"t":"kpi","label":"Cases","value":42,"delta":"+3%","unit":"n"}),
            &gctx()
        )
        .is_ok());
        let e = vw(&json!({"t":"kpi","value":1}), &gctx()).unwrap_err();
        assert!(e.contains("string \"label\""), "{e}");
    }

    #[test]
    fn log_kind_validates_and_caps() {
        assert!(vw(
            &json!({"t":"log","lines":[{"text":"a"},{"text":"b","level":"warn"}]}),
            &gctx()
        )
        .is_ok());
        // a line missing text
        let e = vw(&json!({"t":"log","lines":[{"level":"info"}]}), &gctx()).unwrap_err();
        assert!(e.contains("string \"text\""), "{e}");
        // too many lines
        let lines: Vec<Value> = (0..(MAX_LOG_LINES + 1))
            .map(|_| json!({"text":"x"}))
            .collect();
        let e = vw(&json!({"t":"log","lines":lines}), &gctx()).unwrap_err();
        assert!(e.contains("cap is"), "{e}");
    }

    #[test]
    fn plot_kind_validates_types_and_shapes() {
        assert!(vw(
            &json!({"t":"plot","spec":{"type":"scatter","data":[{"x":1,"y":2}]}}),
            &gctx()
        )
        .is_ok());
        assert!(vw(
            &json!({"t":"plot","spec":{"type":"box","series":[{"label":"A","values":[1,2,3]}]}}),
            &gctx()
        )
        .is_ok());
        assert!(vw(
            &json!({"t":"plot","spec":{"type":"heatmap","x":["a","b"],"y":["r1"],"z":[[1,2]]}}),
            &gctx()
        )
        .is_ok());
        // bad type
        let e = vw(
            &json!({"t":"plot","spec":{"type":"banana","data":[{"x":1}]}}),
            &gctx(),
        )
        .unwrap_err();
        assert!(e.contains("must be one of"), "{e}");
        // heatmap row/col mismatch
        let e = vw(
            &json!({"t":"plot","spec":{"type":"heatmap","x":["a","b"],"y":["r1"],"z":[[1]]}}),
            &gctx(),
        )
        .unwrap_err();
        assert!(e.contains("cells but"), "{e}");
        // over the point cap
        let data: Vec<Value> = (0..(MAX_PLOT_POINTS + 1)).map(|i| json!({"x":i})).collect();
        let e = vw(
            &json!({"t":"plot","spec":{"type":"scatter","data":data}}),
            &gctx(),
        )
        .unwrap_err();
        assert!(e.contains("cap is"), "{e}");
    }

    #[test]
    fn network_kind_checks_ids_and_endpoints() {
        assert!(vw(
            &json!({"t":"network","spec":{"nodes":[{"id":"A"},{"id":"B"}],
                "edges":[{"source":"A","target":"B","kind":"binds"}],
                "encoding":{"negated_kinds":["inhibits"]},"physics":{"charge":-30}}}),
            &gctx()
        )
        .is_ok());
        // duplicate node id
        let e = vw(
            &json!({"t":"network","spec":{"nodes":[{"id":"A"},{"id":"A"}],"edges":[]}}),
            &gctx(),
        )
        .unwrap_err();
        assert!(e.contains("repeats id"), "{e}");
        // dangling edge endpoint
        let e = vw(
            &json!({"t":"network","spec":{"nodes":[{"id":"A"}],"edges":[{"source":"A","target":"Z"}]}}),
            &gctx(),
        )
        .unwrap_err();
        assert!(e.contains("not a declared node id"), "{e}");
    }

    fn surface_with_component(schema: Value) -> SurfaceDecl {
        use crate::agent_drafter::manifest::ComponentDecl;
        SurfaceDecl {
            components: vec![ComponentDecl {
                name: "pathway_map".into(),
                props: schema,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn component_kind_validates_against_declared_schema() {
        let sfc = surface_with_component(json!({
            "type":"object",
            "properties":{"title":{"type":"string"}},
            "required":["title"]
        }));
        let ctx = WidgetCtx {
            allow_privileged: false,
            surface: &sfc,
        };
        // passes when props match
        assert!(vw(
            &json!({"t":"component","name":"pathway_map","props":{"title":"X"}}),
            &ctx
        )
        .is_ok());
        // fails when props violate the schema
        let e = vw(
            &json!({"t":"component","name":"pathway_map","props":{}}),
            &ctx,
        )
        .unwrap_err();
        assert!(e.contains("do not match its declared schema"), "{e}");
        // fails when the component is not declared
        let e = vw(&json!({"t":"component","name":"nope","props":{}}), &ctx).unwrap_err();
        assert!(e.contains("not declared"), "{e}");
    }

    #[test]
    fn component_schema_that_fails_to_compile_fails_closed() {
        // A declared props schema that itself is invalid must be a validation
        // FAILURE (fix-the-manifest), never an accept-any fallback.
        let bad = surface_with_component(json!({"type": 5}));
        let ctx = WidgetCtx {
            allow_privileged: false,
            surface: &bad,
        };
        let e = vw(
            &json!({"t":"component","name":"pathway_map","props":{"any":"thing"}}),
            &ctx,
        )
        .unwrap_err();
        assert!(
            e.contains("invalid props schema") && e.contains("manifest"),
            "{e}"
        );
    }

    #[test]
    fn html_and_figure_are_rejected_from_generic_trees() {
        // Non-privileged (the generic ui_panel/ui_render/ui_patch path) refuses them…
        let e = vw(&json!({"t":"html","html":"<b>x</b>"}), &gctx()).unwrap_err();
        assert!(e.contains("ui_html"), "{e}");
        let e = vw(
            &json!({"t":"figure","html":"<div/>","tool":"render_volcano"}),
            &gctx(),
        )
        .unwrap_err();
        assert!(e.contains("ui_figure"), "{e}");
        // …but the dedicated tools may build them (allow_privileged).
        let pctx = WidgetCtx {
            allow_privileged: true,
            surface: empty_surface(),
        };
        assert!(vw(&json!({"t":"html","html":"<b>x</b>"}), &pctx).is_ok());
        assert!(vw(
            &json!({"t":"figure","html":"<div/>","tool":"render_volcano"}),
            &pctx
        )
        .is_ok());
    }

    #[tokio::test]
    async fn ui_render_rejects_a_privileged_html_node() {
        let (s, _rx) = server();
        let e = s
            .ui_render(Parameters(RenderParams {
                target: "@main".into(),
                body: json!([{"t":"html","html":"<b>x</b>"}]),
                mode: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("ui_html"), "{}", e.message);
    }

    // ── Apps SDK v2: ui_patch + instance registry ───────────────────────────

    async fn describe_json(s: &AppControlServer) -> Value {
        // The result may carry a leading "(Surface unchanged …)\n" note; parse
        // from the first '{'.
        let t = text_of(&s.ui_describe(Parameters(DescribeParams {})).await.unwrap());
        let start = t.find('{').expect("ui_describe returns a JSON body");
        let body = t.get(start..).expect("valid slice at a char boundary");
        serde_json::from_str(body).expect("ui_describe body is valid JSON")
    }

    #[tokio::test]
    async fn ui_patch_round_trips_registry_and_emits_one_frame() {
        let (s, mut rx) = server();
        // add
        let r = s
            .ui_patch(Parameters(PatchParams {
                ops: json!([{"op":"add","id":"n1","target":"@region:results",
                    "node":{"t":"text","value":"hi"}}]),
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("Applied 1"), "{}", text_of(&r));
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame["cmd"], "patch");
        assert_eq!(frame["type"], "ui");
        assert_eq!(frame["v"], CATALOG_VERSION);
        assert_eq!(frame["ops"][0]["op"], "add");
        assert_eq!(frame["ops"][0]["id"], "n1");
        // registry reflects it (via ui_describe)
        let d = describe_json(&s).await;
        let inst = d["instances"].as_array().unwrap();
        assert_eq!(inst.len(), 1);
        assert_eq!(inst[0]["id"], "n1");
        assert_eq!(inst[0]["kind"], "text");

        // replace
        s.ui_patch(Parameters(PatchParams {
            ops: json!([{"op":"replace","id":"n1","node":{"t":"badge","value":"b"}}]),
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["ops"][0]["op"], "replace");
        assert_eq!(describe_json(&s).await["instances"][0]["kind"], "badge");

        // set_props (shallow merge)
        s.ui_patch(Parameters(PatchParams {
            ops: json!([{"op":"set_props","id":"n1","props":{"value":"c"}}]),
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["ops"][0]["op"], "set_props");

        // remove
        s.ui_patch(Parameters(PatchParams {
            ops: json!([{"op":"remove","id":"n1"}]),
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["ops"][0]["op"], "remove");
        assert!(describe_json(&s).await["instances"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn ui_patch_unknown_id_is_a_fixable_error() {
        let (s, _rx) = server();
        let e = s
            .ui_patch(Parameters(PatchParams {
                ops: json!([{"op":"replace","id":"ghost","node":{"t":"text","value":"x"}}]),
            }))
            .await
            .unwrap_err();
        assert!(
            e.message.contains("unknown id") && e.message.contains("ghost"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn ui_patch_add_existing_id_suggests_replace() {
        let (s, _rx) = server();
        s.ui_patch(Parameters(PatchParams {
            ops: json!([{"op":"add","id":"n1","node":{"t":"text","value":"x"}}]),
        }))
        .await
        .unwrap();
        let e = s
            .ui_patch(Parameters(PatchParams {
                ops: json!([{"op":"add","id":"n1","node":{"t":"text","value":"y"}}]),
            }))
            .await
            .unwrap_err();
        assert!(
            e.message.contains("already exists") && e.message.contains("replace"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn ui_patch_caps_the_op_count() {
        let (s, _rx) = server();
        let ops: Vec<Value> = (0..(MAX_PATCH_OPS + 1))
            .map(|i| json!({"op":"add","id":format!("n{i}"),"node":{"t":"text","value":"x"}}))
            .collect();
        let e = s
            .ui_patch(Parameters(PatchParams { ops: json!(ops) }))
            .await
            .unwrap_err();
        assert!(e.message.contains("cap is 32"), "{}", e.message);
    }

    #[tokio::test]
    async fn ui_patch_rejected_batch_leaves_registry_untouched() {
        let (s, mut rx) = server();
        // A batch with a bad op (unknown-id remove) must apply NONE of its ops.
        let e = s
            .ui_patch(Parameters(PatchParams {
                ops: json!([
                    {"op":"add","id":"ok1","node":{"t":"text","value":"a"}},
                    {"op":"remove","id":"ghost"}
                ]),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("unknown id"), "{}", e.message);
        assert!(
            rx.try_recv().is_err(),
            "a rejected batch must emit no frame"
        );
        assert!(describe_json(&s).await["instances"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn ui_render_assigns_and_returns_node_ids() {
        let (s, mut rx) = server();
        let r = s
            .ui_render(Parameters(RenderParams {
                target: "@region:results".into(),
                body: json!([{"t":"card","children":[{"t":"text","value":"a"}]}]),
                mode: None,
            }))
            .await
            .unwrap();
        let txt = text_of(&r);
        assert!(txt.contains("ui_patch"), "{txt}");
        let frame = rx.try_recv().unwrap();
        let top_id = frame["body"][0]["id"].as_str().unwrap();
        assert!(top_id.starts_with("@region:results#n"), "{top_id}");
        assert!(frame["body"][0]["children"][0]["id"].is_string());
        assert!(
            txt.contains(top_id),
            "the assigned ids must be returned: {txt}"
        );
        // and they are targetable via ui_patch
        s.ui_patch(Parameters(PatchParams {
            ops: json!([{"op":"remove","id":top_id}]),
        }))
        .await
        .expect("an assigned id must be a valid ui_patch target");
    }

    #[tokio::test]
    async fn ui_render_keeps_explicit_node_ids() {
        let (s, mut rx) = server();
        s.ui_render(Parameters(RenderParams {
            target: "@main".into(),
            body: json!([{"t":"text","id":"my-node","value":"a"}]),
            mode: None,
        }))
        .await
        .unwrap();
        assert_eq!(rx.try_recv().unwrap()["body"][0]["id"], "my-node");
        // targetable by that explicit id
        s.ui_patch(Parameters(PatchParams {
            ops: json!([{"op":"set_props","id":"my-node","props":{"value":"b"}}]),
        }))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn attach_clears_instances_but_keeps_state() {
        let bridge = UiBridge::new();
        let (mut first, _t1) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        s.ui_state(Parameters(StateParams {
            set: Some(json!({"gene":"BRCA1"})),
            remove: None,
        }))
        .await
        .unwrap();
        s.ui_patch(Parameters(PatchParams {
            ops: json!([{"op":"add","id":"n1","node":{"t":"text","value":"x"}}]),
        }))
        .await
        .unwrap();
        while first.try_recv().is_ok() {}

        // Reconnect.
        let (mut second, _t2) = bridge.attach();
        // State survives (replayed as one snapshot)…
        let replay = second.try_recv().unwrap();
        assert_eq!(replay["cmd"], "state");
        assert_eq!(replay["doc"]["gene"], "BRCA1");
        // …but the instance registry reset with the page.
        let d = describe_json(&s).await;
        assert!(d["instances"].as_array().unwrap().is_empty());
    }

    // ── Apps SDK v2: ui_html (capability-gated, server-side sanitization) ────

    fn html_server() -> (AppControlServer, mpsc::UnboundedReceiver<Value>) {
        let bridge = UiBridge::new();
        let (rx, _tok) = bridge.attach();
        let cap = UiCapability {
            allow_html: true,
            ..Default::default()
        };
        (
            AppControlServer::new(bridge, cap, SurfaceDecl::default()),
            rx,
        )
    }

    #[tokio::test]
    async fn ui_html_denied_by_default() {
        let (s, _rx) = server(); // default cap: allow_html = false
        let e = s
            .ui_html(Parameters(HtmlParams {
                target: "@main".into(),
                html: "<p>hi</p>".into(),
                title: None,
            }))
            .await
            .unwrap_err();
        assert!(
            e.message.contains("does not grant") && e.message.contains("HTML"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn ui_html_sanitizes_a_bypass_corpus() {
        let (s, mut rx) = html_server();

        // Each case: (raw html, a token that must NOT survive sanitization).
        let corpus: &[(&str, &str)] = &[
            ("<script>alert(1)</script><p>ok</p>", "alert(1)"),
            ("<img src=x onerror=alert(1)>", "onerror"),
            ("<svg onload=alert(1)></svg>", "onload"),
            ("<a href=\"javascript:alert(1)\">x</a>", "javascript:"),
            (
                "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>",
                "data:text/html",
            ),
            ("<base href=\"http://evil/\">", "<base"),
            ("<style>@import url(https://evil)</style>", "@import"),
            (
                "<form action=\"https://evil\"><input name=p></form>",
                "<form",
            ),
            (
                "<iframe srcdoc=\"<script>alert(1)</script>\"></iframe>",
                "srcdoc",
            ),
            (
                "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></noscript>",
                "onerror",
            ),
            ("<object data=\"evil.swf\"></object>", "<object"),
            ("<embed src=\"evil\">", "<embed"),
        ];

        for (raw, forbidden) in corpus {
            let _ = s
                .ui_html(Parameters(HtmlParams {
                    target: "@main".into(),
                    html: (*raw).into(),
                    title: None,
                }))
                .await
                .unwrap();
            let frame = rx.try_recv().unwrap();
            let clean = frame["body"][0]["html"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase();
            assert!(
                !clean.contains(&forbidden.to_ascii_lowercase()),
                "sanitizer let {forbidden:?} survive for input {raw:?}: {clean}"
            );
        }
    }

    #[tokio::test]
    async fn ui_html_reports_stripped_elements_and_keeps_safe_markup() {
        let (s, mut rx) = html_server();
        let r = s
            .ui_html(Parameters(HtmlParams {
                target: "@region:notes".into(),
                html: "<p>Kept <b>bold</b></p><script>evil()</script>".into(),
                title: Some("Notes".into()),
            }))
            .await
            .unwrap();
        assert!(text_of(&r).contains("removed"), "{}", text_of(&r));
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame["cmd"], "render");
        // Title ⇒ wrapped in a card whose child is the privileged html node.
        assert_eq!(frame["body"][0]["t"], "card");
        let html = frame["body"][0]["children"][0]["html"].as_str().unwrap();
        assert!(
            html.contains("<b>bold</b>"),
            "safe markup must survive: {html}"
        );
        assert!(!html.contains("script"), "{html}");
    }

    // ── Apps SDK v2: ui_figure (trusted Auto Visualiser output) ─────────────

    #[tokio::test]
    async fn ui_figure_renders_autovis_into_a_dock_panel() {
        let (s, mut rx) = server();
        s.ui_figure(Parameters(FigureParams {
            tool: "show_chart".into(),
            args: json!({"data": {
                "type": "bar",
                "labels": ["A", "B"],
                "datasets": [{"label": "S", "data": [1.0, 2.0]}]
            }}),
            target: None,
            title: Some("Counts".into()),
        }))
        .await
        .expect("show_chart should render");
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "panel");
        assert_eq!(cmd["place"], "dock");
        assert_eq!(cmd["body"][0]["t"], "figure");
        assert_eq!(cmd["body"][0]["tool"], "show_chart");
        assert!(
            cmd["body"][0]["html"].as_str().unwrap().contains("<html")
                || cmd["body"][0]["html"]
                    .as_str()
                    .unwrap()
                    .contains("<!DOCTYPE")
        );
    }

    #[tokio::test]
    async fn ui_figure_renders_into_an_explicit_target() {
        let (s, mut rx) = server();
        s.ui_figure(Parameters(FigureParams {
            tool: "volcano".into(),
            args: json!({"data": {"points": [{"label":"MYC","log2fc":2.4,"negLog10P":4.0}]}}),
            target: Some("@region:results".into()),
            title: None,
        }))
        .await
        .unwrap();
        let cmd = rx.try_recv().unwrap();
        assert_eq!(cmd["cmd"], "render");
        assert_eq!(cmd["target"], "@region:results");
        assert_eq!(cmd["body"][0]["t"], "figure");
    }

    #[tokio::test]
    async fn ui_figure_reports_a_bad_tool_as_a_fixable_error() {
        let (s, _rx) = server();
        let e = s
            .ui_figure(Parameters(FigureParams {
                tool: "totally_made_up".into(),
                args: json!({"data": {}}),
                target: None,
                title: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("totally_made_up"), "{}", e.message);
    }

    // ── Apps SDK v2: app_call / emit_result / signals (Phase 3) ─────────────

    /// A surface declaring two actions (one schema-constrained, one free) and two
    /// signals (one schema-constrained, one free) — the fixture the control-plane
    /// tests exercise.
    fn call_surface() -> SurfaceDecl {
        use crate::agent_drafter::manifest::{ActionDecl, SignalDecl};
        let num_obj = json!({
            "type": "object",
            "properties": { "n": { "type": "number" } },
            "required": ["n"],
        });
        SurfaceDecl {
            actions: vec![
                ActionDecl {
                    name: "echo".into(),
                    description: "Echo a number".into(),
                    params: num_obj.clone(),
                    ..Default::default()
                },
                ActionDecl {
                    name: "ping".into(),
                    description: "Takes no args".into(),
                    params: json!({}),
                    ..Default::default()
                },
            ],
            signals: vec![
                SignalDecl {
                    name: "tick".into(),
                    payload: Some(num_obj),
                    coalesce_ms: 100,
                    ..Default::default()
                },
                SignalDecl {
                    name: "raw".into(),
                    payload: None,
                    coalesce_ms: 500,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// Drain frames until the first `app_call` command; panics if the channel
    /// closes first.
    async fn next_app_call(rx: &mut mpsc::UnboundedReceiver<Value>) -> Value {
        loop {
            match rx.recv().await {
                Some(c) if c["cmd"] == "app_call" => return c,
                Some(_) => continue,
                None => panic!("channel closed before an app_call frame"),
            }
        }
    }

    #[tokio::test]
    async fn app_call_unknown_action_lists_declared() {
        let (s, _rx) = server_with_surface(call_surface());
        let e = s
            .app_call(Parameters(AppCallParams {
                action: "nope".into(),
                args: json!({}),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("not declared"), "{}", e.message);
        assert!(
            e.message.contains("echo") && e.message.contains("ping"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn app_call_rejects_args_that_break_the_schema() {
        let (s, _rx) = server_with_surface(call_surface());
        let e = s
            .app_call(Parameters(AppCallParams {
                action: "echo".into(),
                args: json!({ "n": "not-a-number" }),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("declared schema"), "{}", e.message);
    }

    #[tokio::test]
    async fn app_call_round_trips_a_result() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(bridge.clone(), UiCapability::default(), call_surface());
        let task = tokio::spawn(async move {
            s.app_call(Parameters(AppCallParams {
                action: "echo".into(),
                args: json!({ "n": 7 }),
            }))
            .await
        });
        let frame = next_app_call(&mut rx).await;
        assert_eq!(frame["type"], "ui");
        assert_eq!(frame["action"], "echo");
        assert_eq!(frame["args"]["n"], 7);
        let call_id = frame["callId"].as_str().unwrap().to_string();
        assert_eq!(call_id.len(), 16, "call id is 16 hex chars: {call_id}");
        assert!(bridge.resolve_app_call(&call_id, json!({ "result": { "echoed": 7 } })));
        let r = task.await.unwrap().unwrap();
        let out: Value = serde_json::from_str(&text_of(&r)).unwrap();
        assert_eq!(out["echoed"], 7);
    }

    #[tokio::test]
    async fn app_call_reports_an_app_error() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(bridge.clone(), UiCapability::default(), call_surface());
        let task = tokio::spawn(async move {
            s.app_call(Parameters(AppCallParams {
                action: "ping".into(),
                args: json!({}),
            }))
            .await
        });
        let call_id = next_app_call(&mut rx).await["callId"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(bridge.resolve_app_call(&call_id, json!({ "error": "boom" })));
        let r = task.await.unwrap().unwrap();
        assert!(
            text_of(&r).contains("the app reported an error: boom"),
            "{}",
            text_of(&r)
        );
    }

    #[tokio::test]
    async fn app_call_caps_a_huge_result() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(bridge.clone(), UiCapability::default(), call_surface());
        let task = tokio::spawn(async move {
            s.app_call(Parameters(AppCallParams {
                action: "ping".into(),
                args: json!({}),
            }))
            .await
        });
        let call_id = next_app_call(&mut rx).await["callId"]
            .as_str()
            .unwrap()
            .to_string();
        let big = "x".repeat(APP_PAYLOAD_MAX + 100);
        assert!(bridge.resolve_app_call(&call_id, json!({ "result": big })));
        let r = task.await.unwrap().unwrap();
        let t = text_of(&r);
        assert!(t.ends_with("…[truncated]"), "should be truncated");
        assert!(t.len() <= APP_PAYLOAD_MAX + "…[truncated]".len());
    }

    #[tokio::test]
    async fn app_call_times_out_gracefully() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        bridge.set_app_call_timeout_s(1);
        let s = AppControlServer::new(bridge.clone(), UiCapability::default(), call_surface());
        let task = tokio::spawn(async move {
            s.app_call(Parameters(AppCallParams {
                action: "ping".into(),
                args: json!({}),
            }))
            .await
        });
        // Drain the frame, but never resolve — let the 1s timeout fire.
        let _ = next_app_call(&mut rx).await;
        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("within 1s"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn app_call_unparks_on_cancel_all() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(bridge.clone(), UiCapability::default(), call_surface());
        let task = tokio::spawn(async move {
            s.app_call(Parameters(AppCallParams {
                action: "ping".into(),
                args: json!({}),
            }))
            .await
        });
        let _ = next_app_call(&mut rx).await;
        bridge.cancel_all();
        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("cancelled"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn app_call_unparks_on_reattach() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(bridge.clone(), UiCapability::default(), call_surface());
        let task = tokio::spawn(async move {
            s.app_call(Parameters(AppCallParams {
                action: "ping".into(),
                args: json!({}),
            }))
            .await
        });
        let _ = next_app_call(&mut rx).await;
        // A reload re-attaches, which cancels the parked call (via cancel_all).
        let (_rx2, _tok2) = bridge.attach();
        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("cancelled"), "{}", text_of(&r));
    }

    // ── consult (multi-agent profiles, design §3.8) ─────────────────────────

    #[tokio::test]
    async fn consult_disabled_when_no_profiles() {
        // A default server has consult_enabled == false → a friendly no-op.
        let (s, _rx) = server();
        let r = s
            .consult(Parameters(ConsultParams {
                agent: "critic".into(),
                prompt: "check this".into(),
            }))
            .await
            .unwrap();
        assert!(
            text_of(&r).contains("no worker agent profiles"),
            "{}",
            text_of(&r)
        );
    }

    #[tokio::test]
    async fn consult_without_handler_reports_unavailable() {
        // consult_enabled but no handler installed → graceful message, no hang.
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let s = AppControlServer::new_with_consult(
            bridge,
            UiCapability::default(),
            SurfaceDecl::default(),
            true,
        );
        let r = s
            .consult(Parameters(ConsultParams {
                agent: "critic".into(),
                prompt: "check this".into(),
            }))
            .await
            .unwrap();
        assert!(
            text_of(&r).contains("no worker-profile handler"),
            "{}",
            text_of(&r)
        );
    }

    #[tokio::test]
    async fn consult_round_trips_a_worker_answer() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let mut consult_rx = bridge.set_consult_handler();
        let s = AppControlServer::new_with_consult(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
            true,
        );
        let task = tokio::spawn(async move {
            s.consult(Parameters(ConsultParams {
                agent: "critic".into(),
                prompt: "is this sound?".into(),
            }))
            .await
        });
        let req = consult_rx.recv().await.expect("a consult request arrives");
        assert_eq!(req.agent, "critic");
        assert_eq!(req.prompt, "is this sound?");
        assert_eq!(req.id.len(), 16, "consult id is 16 hex chars: {}", req.id);
        assert!(bridge.resolve_consult(&req.id, json!({ "text": "looks good to me" })));
        let r = task.await.unwrap().unwrap();
        assert!(
            text_of(&r).contains("[critic] looks good to me"),
            "{}",
            text_of(&r)
        );
    }

    #[tokio::test]
    async fn consult_reports_a_worker_error() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let mut consult_rx = bridge.set_consult_handler();
        let s = AppControlServer::new_with_consult(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
            true,
        );
        let task = tokio::spawn(async move {
            s.consult(Parameters(ConsultParams {
                agent: "ghost".into(),
                prompt: "hi".into(),
            }))
            .await
        });
        let req = consult_rx.recv().await.unwrap();
        assert!(bridge.resolve_consult(&req.id, json!({ "error": "no such profile \"ghost\"" })));
        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("could not answer"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn consult_times_out_gracefully() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        bridge.set_consult_timeout_s(1);
        let mut consult_rx = bridge.set_consult_handler();
        let s = AppControlServer::new_with_consult(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
            true,
        );
        let task = tokio::spawn(async move {
            s.consult(Parameters(ConsultParams {
                agent: "critic".into(),
                prompt: "slow one".into(),
            }))
            .await
        });
        // Receive but never resolve → let the 1s timeout fire.
        let _ = consult_rx.recv().await.unwrap();
        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("within 1s"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn consult_unparks_on_cancel_all() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let mut consult_rx = bridge.set_consult_handler();
        let s = AppControlServer::new_with_consult(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
            true,
        );
        let task = tokio::spawn(async move {
            s.consult(Parameters(ConsultParams {
                agent: "critic".into(),
                prompt: "cancel me".into(),
            }))
            .await
        });
        let _ = consult_rx.recv().await.unwrap();
        bridge.cancel_all();
        let r = task.await.unwrap().unwrap();
        assert!(text_of(&r).contains("cancelled"), "{}", text_of(&r));
    }

    #[tokio::test]
    async fn consult_rejects_blank_args() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let _consult_rx = bridge.set_consult_handler();
        let s = AppControlServer::new_with_consult(
            bridge,
            UiCapability::default(),
            SurfaceDecl::default(),
            true,
        );
        assert!(s
            .consult(Parameters(ConsultParams {
                agent: "  ".into(),
                prompt: "hi".into(),
            }))
            .await
            .is_err());
        assert!(s
            .consult(Parameters(ConsultParams {
                agent: "critic".into(),
                prompt: "   ".into(),
            }))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn emit_result_with_no_pending_says_so() {
        let (s, _rx) = server();
        let r = s
            .emit_result(Parameters(EmitResultParams {
                result: json!({ "x": 1 }),
            }))
            .await
            .unwrap();
        assert!(
            text_of(&r).contains("no structured call is pending"),
            "{}",
            text_of(&r)
        );
    }

    #[tokio::test]
    async fn emit_result_valid_emits_output_frame_and_clears_pending() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        bridge.set_pending_output(
            "call-1".into(),
            Some(json!({ "type": "object", "required": ["ok"] })),
        );
        let r = s
            .emit_result(Parameters(EmitResultParams {
                result: json!({ "ok": true }),
            }))
            .await
            .unwrap();
        assert!(
            text_of(&r).contains("structured result delivered"),
            "{}",
            text_of(&r)
        );
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame["type"], "output");
        assert_eq!(frame["callId"], "call-1");
        assert_eq!(frame["value"]["ok"], true);
        assert_eq!(frame["v"], CATALOG_VERSION);
        assert!(frame.get("schema").is_some(), "schema echoed on the frame");
        // The pending request was consumed.
        assert!(bridge.take_pending_output().is_none());
    }

    #[tokio::test]
    async fn emit_result_schema_mismatch_is_fixable_and_keeps_pending() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        bridge.set_pending_output(
            "c".into(),
            Some(json!({ "type": "object", "required": ["ok"] })),
        );
        let e = s
            .emit_result(Parameters(EmitResultParams {
                result: json!({ "nope": 1 }),
            }))
            .await
            .unwrap_err();
        assert!(
            e.message.contains("declared output schema"),
            "{}",
            e.message
        );
        // The request is still armed so a corrected retry can satisfy it.
        assert!(bridge.take_pending_output().is_some());
    }

    #[tokio::test]
    async fn emit_result_oversized_is_rejected_and_keeps_pending() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let s = AppControlServer::new(
            bridge.clone(),
            UiCapability::default(),
            SurfaceDecl::default(),
        );
        bridge.set_pending_output("c".into(), None);
        let big = "x".repeat(APP_PAYLOAD_MAX + 10);
        let e = s
            .emit_result(Parameters(EmitResultParams {
                result: json!({ "blob": big }),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("cap is"), "{}", e.message);
        assert!(bridge.take_pending_output().is_some());
    }

    #[tokio::test]
    async fn ui_subscribe_replaces_the_set_and_reports_coalesce() {
        let (s, _rx) = server_with_surface(call_surface());
        let r = s
            .ui_subscribe(Parameters(SubscribeParams {
                signals: vec!["tick".into(), "raw".into()],
            }))
            .await
            .unwrap();
        let out: Value = serde_json::from_str(&text_of(&r)).unwrap();
        let arr = out["subscribed"].as_array().unwrap();
        let names: Vec<&str> = arr.iter().map(|v| v["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["raw", "tick"]); // sorted
        let tick = arr.iter().find(|v| v["name"] == "tick").unwrap();
        assert_eq!(tick["coalesceMs"], 100);
        // Re-subscribing narrows the EXPLICIT set, but cannot drop a signal the app
        // declared eagerly. Before eager subscription this asserted the set shrank
        // to one — which is exactly the hole that let a single `ui_subscribe([…])`
        // silently unsubscribe the app's own declared signals for the rest of the
        // session. Both `tick` and `raw` are declared, so both stay live.
        let r2 = s
            .ui_subscribe(Parameters(SubscribeParams {
                signals: vec!["tick".into()],
            }))
            .await
            .unwrap();
        let out2: Value = serde_json::from_str(&text_of(&r2)).unwrap();
        // The tool reports what the agent is EFFECTIVELY subscribed to — the
        // explicit set unioned with the declared floor — not merely what this call
        // asked for. Reporting the narrowed set would tell the model it had
        // unsubscribed from signals it is in fact still receiving.
        let names2: Vec<&str> = out2["subscribed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert_eq!(names2, vec!["raw", "tick"]);
        assert_eq!(
            s.bridge.subscribed_signals(),
            vec!["raw".to_string(), "tick".to_string()],
            "a declared signal stays subscribed no matter how ui_subscribe narrows"
        );
        assert!(s.bridge.validate_signal("raw", &json!({})).is_ok());
    }

    #[tokio::test]
    async fn ui_subscribe_rejects_an_undeclared_signal() {
        let (s, _rx) = server_with_surface(call_surface());
        let e = s
            .ui_subscribe(Parameters(SubscribeParams {
                signals: vec!["ghost".into()],
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("not declared"), "{}", e.message);
        assert!(e.message.contains("tick"), "{}", e.message);
    }

    #[tokio::test]
    async fn ui_subscribe_denied_when_capability_off() {
        let bridge = UiBridge::new();
        let (_rx, _tok) = bridge.attach();
        let cap = UiCapability {
            allow_signals: false,
            ..Default::default()
        };
        let s = AppControlServer::new(bridge, cap, call_surface());
        let e = s
            .ui_subscribe(Parameters(SubscribeParams {
                signals: vec!["tick".into()],
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("allow_signals"), "{}", e.message);
    }

    #[test]
    fn validate_signal_checks_subscription_declaration_size_and_schema() {
        let bridge = UiBridge::new();
        // AppControlServer::new does this in production; here we mirror it directly.
        bridge.set_surface_decl(call_surface());

        // DECLARED ⇒ subscribed, with no tool call. This assertion used to be the
        // opposite ("unsubscribed → refused"), and that was the bug: the user's
        // first click lands before the agent's first tool call, so the gesture was
        // always validated against an empty set and dropped.
        assert!(
            bridge.validate_signal("tick", &json!({ "n": 1 })).is_ok(),
            "declaring a signal subscribes to it"
        );

        // An explicit subscribe is still honoured (and still can't drop the floor).
        bridge.replace_subscriptions(vec!["tick".into()]);
        assert!(bridge.validate_signal("tick", &json!({ "n": 1 })).is_ok());

        // Schema failure.
        let e = bridge
            .validate_signal("tick", &json!({ "n": "x" }))
            .unwrap_err();
        assert!(e.contains("declared schema"), "{e}");

        // Oversized payload (raw has no schema, so the size cap is what trips).
        bridge.replace_subscriptions(vec!["raw".into()]);
        let big = "x".repeat(APP_PAYLOAD_MAX + 10);
        let e = bridge
            .validate_signal("raw", &json!({ "blob": big }))
            .unwrap_err();
        assert!(e.contains("cap is"), "{e}");

        // A name the surface never declared is still refused, even when something
        // explicitly subscribed to it (a stale server push). Eager subscription
        // widens the floor to the CONTRACT — it does not make the bridge accept
        // anything.
        bridge.replace_subscriptions(vec!["mystery".into()]);
        let e = bridge.validate_signal("mystery", &json!({})).unwrap_err();
        assert!(e.contains("not declared"), "{e}");
    }

    #[test]
    fn emit_frame_sends_raw_frames_unstamped() {
        let bridge = UiBridge::new();
        let (mut rx, _tok) = bridge.attach();
        assert!(bridge.emit_frame(json!({ "type": "custom", "hi": 1 })));
        let f = rx.try_recv().unwrap();
        assert_eq!(f["type"], "custom");
        assert_eq!(f["hi"], 1);
        // Unlike emit(), emit_frame does not stamp `type:ui` / `v`.
        assert!(f.get("v").is_none());
        // With no receiver attached, it reports failure instead of panicking.
        drop(rx);
        assert!(!bridge.emit_frame(json!({ "x": 1 })));
    }
}
