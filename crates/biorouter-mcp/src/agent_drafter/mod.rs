//! Agent Drafter — author interactive **Biorouter apps**: TypeScript front-ends
//! wired to a *real* Biorouter agent backend.
//!
//! An Agent-Drafter app is a self-contained project the assistant builds for the
//! user. The UI is authored in TypeScript (bundled with esbuild) and the app
//! talks to Biorouter over a per-app WebSocket: when the user sends a message,
//! the Biorouter backend runs the **full agent loop** — the app's own model,
//! extensions, skills and knowledge base — and streams the answer (text /
//! markdown / tool activity) straight back into the app. Apps are *launched in
//! the browser* (GUI) or via a printed URL (CLI), not embedded in a chat iframe.
//!
//! Apps live under `~/.config/biorouter/agent_drafter/<id>/` (a project dir with
//! `manifest.json`, `index.html`, `src/*.ts`, `dist/app.js`). `biorouterd` serves
//! them at `/apps/<id>/` and exposes the agent socket at `/apps/<id>/agent`.
//! `export_app` produces a standalone runnable TypeScript project.

pub mod bundle;
pub mod catalog;
pub mod control;
pub mod declare;
pub mod evidence;
pub mod manifest;
pub mod render;
pub mod resolved;
pub mod store;
pub mod validate;
pub mod vault;

use crate::developer::shell::strip_daemon_private_env_std;
use crate::knowledge::caller::KbCaller;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use indoc::formatdoc;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, ErrorCode, ErrorData, Implementation, RawResource,
        ResourceContents, Role, ServerCapabilities, ServerInfo,
    },
    schemars::JsonSchema,
    service::RequestContext,
    tool, tool_router, RoleServer, ServerHandler,
};
use serde::{de::DeserializeOwned, Deserialize};
use std::path::{Path, PathBuf};

/// Meta key the agent loop uses to pass the current chat session id into MCP
/// tool calls. Must match `biorouter::session_context::SESSION_ID_HEADER`
/// (duplicated here to avoid a circular dependency on the `biorouter` crate,
/// the same way `knowledge::server` does).
const SESSION_ID_META_KEY: &str = "biorouter-session-id";

use manifest::{
    ActionDecl, Capabilities, ComponentDecl, GuardrailsConfig, ModelSettings, Orchestration,
    ReliabilityConfig, SignalDecl, SurfaceDecl,
};
use store::{AgentConfig, ArtifactKind, ArtifactStore, Manifest, ModelSelection};

/// Optional suggestions only. Apps are **provider-agnostic**: by default an app
/// pins no model and inherits whatever provider/model the user has configured in
/// Biorouter (any supported provider). A specific provider+model is stored only
/// when the caller explicitly chooses one. These constants are not auto-applied.
pub const DEFAULT_APP_PROVIDER: &str = "xiaomi_mimo";
// mimo-v2.5 shuts down 2026-10-21 with no replacement routing; V2.6 Flash is
// its same-price successor and the xiaomi_mimo provider's default.
pub const DEFAULT_APP_MODEL: &str = "mimo-v2.6-flash";

// ---------------------------------------------------------------------------
// Tool parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileSpec {
    /// Path relative to the app root (e.g. "src/main.ts", "assets/logo.svg").
    pub path: String,
    /// File contents.
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct ModelSettingsParam {
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<i32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub verbosity: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct ModelParam {
    /// Provider name (e.g. "xiaomi_mimo", "anthropic", "openai").
    #[serde(default)]
    pub provider: Option<String>,
    /// Model name (e.g. "mimo-v2.6-flash", "claude-opus-4-8").
    #[serde(default)]
    pub model: Option<String>,
    /// Optional provider-agnostic generation settings.
    #[serde(default)]
    pub settings: Option<ModelSettingsParam>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateAppParams {
    /// Human-readable title; also used to derive the app id when `id` is omitted.
    pub title: String,
    /// Optional explicit app id (slugified). If given and an app already exists
    /// at that id, it is REPLACED — so re-creating the same app is idempotent
    /// and the app stays addressable. Omit to auto-derive a unique id from the
    /// title.
    #[serde(default)]
    pub id: Option<String>,
    /// Short description of what the app does.
    #[serde(default)]
    pub description: String,
    /// The app's declared contract: state schema, the actions the AGENT may call
    /// on the app, the signals the APP sends the agent, and any custom components.
    ///
    /// Declare it HERE, at creation. An agentic app whose `src/main.ts` registers
    /// actions but declares no surface has an agent with no verbs it can call —
    /// and lint fails it. When you supply your own `html`/`src/main.ts` (the normal
    /// case for a real spec), nothing else seeds a surface for you.
    #[serde(default)]
    pub surface: Option<declare::SurfaceParam>,
    /// The app's theme pack (+ optional accent / token overrides).
    #[serde(default)]
    pub theme: Option<declare::ThemeParam>,
    /// Starter archetype (Apps SDK v2): "explorer", "dashboard", "workbench",
    /// "wizard", "canvas", or "chat". Omit to infer one from the title +
    /// description (a non-chat archetype unless the brief asks for a chat /
    /// assistant / Q&A). When the caller supplies no `html`/`src/main.ts`, the
    /// chosen archetype seeds a working, lint-clean index.html + src/main.ts and
    /// the matching declared `surface` (actions / signals / components /
    /// state_schema). Only shapes agentic apps; static apps ignore it.
    #[serde(default)]
    pub archetype: Option<String>,
    /// "agentic" (default — wired to a Biorouter agent) or "static".
    #[serde(default)]
    pub kind: Option<String>,
    /// Entry HTML (index.html). If omitted, a Biorouter-styled starter is used.
    #[serde(default)]
    pub html: Option<String>,
    /// Additional files (TypeScript under `src/`, assets, etc.). Provide your own
    /// `src/main.ts` to drive a custom UI; otherwise a starter is written.
    #[serde(default)]
    pub files: Vec<FileSpec>,
    /// System prompt defining the app agent's behavior/persona.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Greeting shown when the chat panel mounts.
    #[serde(default)]
    pub greeting: Option<String>,
    /// Provider+model the app's agent runs on (any Biorouter-supported provider).
    /// Omit to inherit the user's configured provider/model (provider-agnostic).
    #[serde(default)]
    pub model: Option<ModelParam>,
    /// Builtin/platform extension names the agent may use (e.g. "developer",
    /// "autovisualiser", "computercontroller", "knowledge").
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Skill ids the agent should have available.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Knowledge base id to scope the agent to.
    #[serde(default)]
    pub knowledge_base: Option<String>,
    /// Deny-by-default capability grants: files, data/knowledge sources,
    /// compute, vault, memory, tracing, and lifecycle event subscriptions.
    #[serde(default)]
    pub capabilities: Option<serde_json::Value>,
    /// Declarative goal, content guardrails, and human approval requirements.
    #[serde(default)]
    pub guardrails: Option<serde_json::Value>,
    /// Tool-loop reliability settings such as timeouts and parallel tool use.
    #[serde(default)]
    pub reliability: Option<serde_json::Value>,
    /// Multi-agent sub-agents, handoff targets, and named workflows.
    #[serde(default)]
    pub orchestration: Option<serde_json::Value>,
    /// JSON Schema for the agent's final structured output.
    #[serde(default)]
    pub output_type: Option<serde_json::Value>,
    /// Durable/resumable per-app sessions. Omit to keep the default enabled.
    #[serde(default)]
    pub durable_session: Option<bool>,
}

// ---------------------------------------------------------------------------
// Archetype starters (Apps SDK v2, Pillar 6 — design §3.6)
// ---------------------------------------------------------------------------

/// HTML-escape a title/description before substituting it into a starter
/// template. (`render::html_escape` is private to `render`, so mirror it here
/// for the archetype path.)
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The starter archetype a fresh agentic app is seeded from. Each non-`Chat`
/// archetype ships a distinct `index.html` + `src/main.ts` (under
/// `templates/starters/<archetype>/`) plus a declared manifest `surface`, so a
/// new app is a *working, lint-clean example of that shape* rather than a chat
/// box — the structural answer to "every generated app is a chatbot" (design
/// §2.3 item 1, §3.6). `Chat` is today's default template, kept as one option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archetype {
    /// Network/graph the agent renders + inspector + search.
    Explorer,
    /// KPI grid bound to shared state + a refresh action.
    Dashboard,
    /// Data table + row-select signal + a bound detail panel.
    Workbench,
    /// Staged form that writes shared state, then submits.
    Wizard,
    /// Author-registered draw surface + agent-called actions (the avatar shape).
    Canvas,
    /// The pre-v2 default: a chat card wired to the agent.
    Chat,
}

impl Archetype {
    /// Parse an explicit `archetype` argument (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "explorer" => Some(Self::Explorer),
            "dashboard" => Some(Self::Dashboard),
            "workbench" => Some(Self::Workbench),
            "wizard" => Some(Self::Wizard),
            "canvas" => Some(Self::Canvas),
            "chat" => Some(Self::Chat),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Dashboard => "dashboard",
            Self::Workbench => "workbench",
            Self::Wizard => "wizard",
            Self::Canvas => "canvas",
            Self::Chat => "chat",
        }
    }

    /// Infer an archetype from the title + description when the caller didn't
    /// pick one. Keywords match against whole words by prefix (so "metrics" →
    /// "metric", "simulation" → "simulat"), which avoids false hits like
    /// "platform" matching "form". The fallback is `Dashboard`; `Chat` is chosen
    /// only when the brief actually asks for a chat / assistant / Q&A.
    pub fn infer(title: &str, description: &str) -> Self {
        let hay = format!("{title} {description}").to_lowercase();
        let words: Vec<&str> = hay
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();
        let has = |kws: &[&str]| words.iter().any(|w| kws.iter().any(|k| w.starts_with(k)));
        if has(&["graph", "network", "explore"]) {
            Self::Explorer
        } else if has(&["dashboard", "metric", "kpi"]) {
            Self::Dashboard
        } else if has(&["table", "cohort", "browse", "workbench"]) {
            Self::Workbench
        } else if has(&["wizard", "form", "survey", "questionnaire", "intake"]) {
            Self::Wizard
        } else if has(&["canvas", "scene", "avatar", "game", "simulat", "animat"]) {
            Self::Canvas
        } else if has(&["chat", "assistant", "chatbot", "conversation", "qa"])
            || hay.contains("q&a")
        {
            Self::Chat
        } else {
            Self::Dashboard
        }
    }

    /// The starter `index.html` (with `{{TITLE}}` / `{{DESCRIPTION}}`
    /// placeholders) for a non-chat archetype. `Chat` returns `None` — it reuses
    /// the default `render::starter`.
    fn index_template(self) -> Option<&'static str> {
        Some(match self {
            Self::Explorer => include_str!("templates/starters/explorer/index.html"),
            Self::Dashboard => include_str!("templates/starters/dashboard/index.html"),
            Self::Workbench => include_str!("templates/starters/workbench/index.html"),
            Self::Wizard => include_str!("templates/starters/wizard/index.html"),
            Self::Canvas => include_str!("templates/starters/canvas/index.html"),
            Self::Chat => return None,
        })
    }

    /// The starter `src/main.ts` for a non-chat archetype. `Chat` returns `None`
    /// — it reuses `bundle::default_sources`.
    fn main_ts(self) -> Option<&'static str> {
        Some(match self {
            Self::Explorer => include_str!("templates/starters/explorer/main.ts"),
            Self::Dashboard => include_str!("templates/starters/dashboard/main.ts"),
            Self::Workbench => include_str!("templates/starters/workbench/main.ts"),
            Self::Wizard => include_str!("templates/starters/wizard/main.ts"),
            Self::Canvas => include_str!("templates/starters/canvas/main.ts"),
            Self::Chat => return None,
        })
    }

    /// Render the starter index HTML with the title/description substituted
    /// (`None` for `Chat`).
    fn index_html(self, title: &str, description: &str) -> Option<String> {
        self.index_template().map(|t| {
            t.replace("{{TITLE}}", &escape_html(title))
                .replace("{{DESCRIPTION}}", &escape_html(description))
        })
    }

    /// The manifest `surface` seeded for this archetype: the actions / signals /
    /// components / state_schema its starter `main.ts` registers, so the agent's
    /// `app_call`s, subscriptions, and component instances validate server-side
    /// and the seeded project lints clean. This is a small in-code table (the
    /// authoritative source), NOT parsed from the template header comments.
    /// `Chat` declares nothing (identical to a v1 app).
    fn surface(self) -> SurfaceDecl {
        match self {
            Self::Explorer => explorer_surface(),
            Self::Dashboard => dashboard_surface(),
            Self::Workbench => workbench_surface(),
            Self::Wizard => wizard_surface(),
            Self::Canvas => canvas_surface(),
            Self::Chat => SurfaceDecl::default(),
        }
    }
}

fn surface_action(name: &str, description: &str, params: serde_json::Value) -> ActionDecl {
    ActionDecl {
        name: name.into(),
        description: description.into(),
        params,
        ..Default::default()
    }
}

fn surface_signal(name: &str, payload: serde_json::Value) -> SignalDecl {
    SignalDecl {
        name: name.into(),
        payload: Some(payload),
        ..Default::default()
    }
}

fn explorer_surface() -> SurfaceDecl {
    SurfaceDecl {
        state_initial: Some(serde_json::json!({ "query": "", "selection": {} })),
        state_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "selection": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "label": { "type": "string" },
                        "type": { "type": "string" }
                    }
                }
            }
        })),
        actions: vec![surface_action(
            "focus_node",
            "Center and select a graph node.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "label": { "type": "string" },
                    "type": { "type": "string" }
                },
                "required": ["id"]
            }),
        )],
        signals: vec![
            surface_signal(
                "node_selected",
                serde_json::json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
            ),
            surface_signal(
                "search_submitted",
                serde_json::json!({ "type": "object", "properties": { "query": { "type": "string" } } }),
            ),
        ],
        components: vec![],
    }
}

fn dashboard_surface() -> SurfaceDecl {
    SurfaceDecl {
        state_initial: Some(serde_json::json!({
            "metrics": {
                "cohorts": { "value": "—", "delta": "Not loaded" },
                "samples": { "value": "—", "delta": "Not loaded" },
                "alerts": { "value": "—", "delta": "Not loaded" }
            }
        })),
        state_schema: Some(serde_json::json!({
            "type": "object",
            "properties": { "metrics": { "type": "object" } }
        })),
        actions: vec![surface_action(
            "set_metric",
            "Write one KPI tile into shared state.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "value": {},
                    "delta": {}
                },
                "required": ["key"]
            }),
        )],
        signals: vec![surface_signal(
            "refresh_requested",
            serde_json::json!({ "type": "object", "properties": { "at": { "type": "number" } } }),
        )],
        components: vec![],
    }
}

fn workbench_surface() -> SurfaceDecl {
    SurfaceDecl {
        state_initial: Some(serde_json::json!({ "filter": "", "detail": {} })),
        state_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "filter": { "type": "string" },
                "detail": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" }
                    }
                }
            }
        })),
        actions: vec![surface_action(
            "open_row",
            "Open one table row into the detail panel.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "body": { "type": "string" }
                },
                "required": ["id"]
            }),
        )],
        signals: vec![
            surface_signal(
                "row_selected",
                serde_json::json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
            ),
            surface_signal(
                "filter_changed",
                serde_json::json!({ "type": "object", "properties": { "filter": { "type": "string" } } }),
            ),
        ],
        components: vec![],
    }
}

fn wizard_surface() -> SurfaceDecl {
    SurfaceDecl {
        state_initial: Some(serde_json::json!({
            "step": 1,
            "form": { "name": "", "goal": "" }
        })),
        state_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "step": { "type": "integer" },
                "form": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "goal": { "type": "string" }
                    }
                }
            }
        })),
        actions: vec![surface_action(
            "go_to_step",
            "Move the wizard to a stage.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "step": { "type": "integer", "minimum": 1, "maximum": 2 }
                },
                "required": ["step"]
            }),
        )],
        signals: vec![
            surface_signal(
                "step_changed",
                serde_json::json!({ "type": "object", "properties": { "step": { "type": "integer" } } }),
            ),
            surface_signal(
                "submitted",
                serde_json::json!({ "type": "object", "properties": { "name": { "type": "string" } } }),
            ),
        ],
        components: vec![],
    }
}

fn canvas_surface() -> SurfaceDecl {
    SurfaceDecl {
        state_initial: Some(serde_json::json!({ "scene": { "x": 0, "y": 0 } })),
        state_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "scene": {
                    "type": "object",
                    "properties": {
                        "x": { "type": "number" },
                        "y": { "type": "number" }
                    }
                }
            }
        })),
        actions: vec![
            surface_action(
                "move_avatar",
                "Move the avatar on the grid.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "direction": { "enum": ["up", "down", "left", "right"] },
                        "steps": { "type": "integer", "minimum": 1, "maximum": 20 }
                    },
                    "required": ["direction"]
                }),
            ),
            surface_action(
                "reset_scene",
                "Return the avatar to center.",
                serde_json::json!({}),
            ),
        ],
        signals: vec![surface_signal(
            "avatar_moved",
            serde_json::json!({
                "type": "object",
                "properties": { "x": { "type": "number" }, "y": { "type": "number" } }
            }),
        )],
        components: vec![ComponentDecl {
            name: "scene".into(),
            props: serde_json::json!({
                "type": "object",
                "properties": {
                    "x": { "type": "number" },
                    "y": { "type": "number" }
                }
            }),
        }],
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConfigureAppParams {
    /// App id.
    pub id: String,
    /// New system prompt / persona.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// New greeting.
    #[serde(default)]
    pub greeting: Option<String>,
    /// Provider+model override.
    #[serde(default)]
    pub model: Option<ModelParam>,
    /// Replace the extension list.
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
    /// Replace the skills list.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// Set (or clear, with empty string) the knowledge base id. Must name a
    /// knowledge base that is actually installed — call `list_platform_catalog`
    /// first. An id that does not exist here is rejected; use `requires` to record
    /// the need instead.
    #[serde(default)]
    pub knowledge_base: Option<String>,
    /// Platform capabilities this app needs that may not exist on this install.
    ///
    /// Use this instead of inventing an id. `[{"kind":"knowledge_base",
    /// "id":"clinvar","reason":"variant annotations"}]` says "this app wants a
    /// ClinVar KB" honestly; configuring `knowledge_base: "clinvar"` when no such
    /// KB exists arms tools scoped to nothing and fails the app's first turn.
    #[serde(default)]
    pub requires: Option<Vec<crate::agent_drafter::store::Requirement>>,
    /// Bound the agent's tool-calling loop per message. Raise this for
    /// workflow-style apps that chain many tool calls; lower it to keep apps
    /// snappy. Unset → a safe server default.
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// Replace deny-by-default capability grants: files, data/knowledge sources,
    /// compute, vault, memory, tracing, and lifecycle event subscriptions.
    #[serde(default)]
    pub capabilities: Option<serde_json::Value>,
    /// Replace declarative goal, content guardrails, and approval requirements.
    #[serde(default)]
    pub guardrails: Option<serde_json::Value>,
    /// Replace tool-loop reliability settings.
    #[serde(default)]
    pub reliability: Option<serde_json::Value>,
    /// Replace multi-agent sub-agents, handoff targets, and named workflows.
    #[serde(default)]
    pub orchestration: Option<serde_json::Value>,
    /// Replace the final-answer JSON Schema contract.
    #[serde(default)]
    pub output_type: Option<serde_json::Value>,
    /// Set durable/resumable per-app sessions.
    #[serde(default)]
    pub durable_session: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetAppSizeParams {
    /// App id.
    pub id: String,
    /// Preferred width in CSS px (omit to fill).
    #[serde(default)]
    pub width: Option<u32>,
    /// Preferred height in CSS px (omit for auto).
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateAppParams {
    /// App id.
    pub id: String,
    /// File to modify (defaults to the entry HTML).
    #[serde(default)]
    pub path: Option<String>,
    /// Full new contents (write mode).
    #[serde(default)]
    pub content: Option<String>,
    /// Exact substring to replace (str-replace mode; requires `new_str`).
    #[serde(default)]
    pub old_str: Option<String>,
    /// Replacement text for `old_str`.
    #[serde(default)]
    pub new_str: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAppsParams {
    /// Optional filter: "static" or "agentic".
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadAppParams {
    /// App id.
    pub id: String,
    /// File to read. If omitted, returns the manifest.
    #[serde(default)]
    pub path: Option<String>,
    /// Manifest view (ignored when `path` is set):
    /// - `"resolved"` (default) — a canonical, fully-populated skeleton: every
    ///   optional block present, the theme pack resolved, and `_server_managed`
    ///   naming the keys you must not write. Edit this.
    /// - `"raw"` — the bytes exactly as stored on disk (a diff against defaults,
    ///   so a field holding its default value is *absent*, not visible).
    #[serde(default)]
    pub view: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeclareSurfaceParams {
    /// App id.
    pub id: String,
    /// The contract to declare.
    pub surface: declare::SurfaceParam,
    /// `true` upserts actions/signals/components by name (leaving others alone);
    /// `false` (default) replaces the whole surface.
    #[serde(default)]
    pub merge: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetThemeParams {
    /// App id.
    pub id: String,
    /// Theme pack (+ optional accent / token overrides).
    pub theme: declare::ThemeParam,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProfileParam {
    /// The profile KEY. This is what `consult(agent: "<key>")` targets, so it must
    /// be a stable identifier: lowercase letters, digits and underscores only
    /// (`prosecutor`, `fine_mapper`). A capitalised or spaced key is rejected —
    /// the lookup is an exact match, and a display-name key silently fails to
    /// resolve at runtime. Put the display name in `description`.
    pub key: String,
    /// Human-readable name/role, shown in the UI. Free-form.
    #[serde(default)]
    pub description: Option<String>,
    /// The worker's system prompt.
    pub system_prompt: String,
    /// Provider + model for this worker. Omit to inherit the app's.
    #[serde(default)]
    pub model: Option<ModelParam>,
    /// Extensions this worker may use. Must exist on this install.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Skills this worker may use. Must be installed.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Bound on this worker's tool-calling loop.
    #[serde(default)]
    pub max_turns: Option<u32>,
}

/// Arguments for `build_app`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BuildAppParams {
    /// App id.
    pub id: String,
    /// `true` runs the guardrail harness alone and skips the bundler. Use it to
    /// re-check findings without paying for a rebuild; the report is the same
    /// one a successful build prints.
    #[serde(default)]
    pub check_only: Option<bool>,
}

/// Preferred preview-card size. `SetAppSizeParams` minus the id, so the nested
/// shape and the retired flat one cannot drift apart.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AppSizeParam {
    /// Preferred width in CSS px (omit to fill).
    #[serde(default)]
    pub width: Option<u32>,
    /// Preferred height in CSS px (omit for auto).
    #[serde(default)]
    pub height: Option<u32>,
}

/// Every typed manifest region, in ONE call.
///
/// `declare_surface`, `set_theme`, `declare_profiles`, `set_routes` and
/// `set_app_size` were five spellings of one job — load the manifest, assign ONE
/// region, save, touch — and two of them (`set_routes`, `declare_profiles`) wrote
/// two fields of the SAME struct. A model had to know five names to fill in five
/// slots of one document, and declaring a surface plus a theme plus profiles
/// cost three round-trips and three save cycles.
///
/// ⚠ **A FLAT bag of independent optionals, never a tagged union.**
/// `providers/formats/google.rs` accepts only a small set of schema keys and
/// silently strips `oneOf`/`const`, so a discriminated merge would reach Gemini
/// with no usable parameters at all; Code Execution cannot render one into a
/// module signature either. Here the discriminator is simply which field is
/// present, and each field keeps the exact typed schema its own tool had.
///
/// An omitted region is LEFT ALONE. That is the semantic the five separate tools
/// got for free — each had no other field to clear — and the one a single tool
/// has to mean deliberately.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DeclareAppParams {
    /// App id.
    pub id: String,
    /// The app's CONTRACT: state schema, the actions the agent may call, the
    /// signals the app sends, and any custom components.
    #[serde(default)]
    pub surface: Option<declare::SurfaceParam>,
    /// Theme pack, plus optional accent and token overrides. Always replaces.
    #[serde(default)]
    pub theme: Option<declare::ThemeParam>,
    /// Worker agent profiles for multi-agent work, reached with
    /// `consult(agent: "<key>")`.
    #[serde(default)]
    pub agents: Option<Vec<ProfileParam>>,
    /// Named model routes a `call` may select per invocation. Always replaces.
    #[serde(default)]
    pub routes: Option<Vec<declare::RouteParam>>,
    /// Preferred preview-card size in CSS px. Always replaces.
    #[serde(default)]
    pub size: Option<AppSizeParam>,
    /// `true` upserts `surface` and `agents` by name/key, leaving the rest of
    /// each alone; `false` (default) replaces them. `theme`, `routes` and `size`
    /// always replace — they have no per-item identity to merge on.
    #[serde(default)]
    pub merge: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeclareProfilesParams {
    /// App id.
    pub id: String,
    /// The worker profiles. Replaces any existing profiles unless merge=true.
    pub agents: Vec<ProfileParam>,
    /// `true` upserts by key; `false` (default) replaces the whole set.
    #[serde(default)]
    pub merge: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetRoutesParams {
    /// App id.
    pub id: String,
    /// The named model routes. Replaces any existing routes.
    pub routes: Vec<declare::RouteParam>,
}

/// Replace entries with a matching name, append the rest. Used by
/// `declare_surface(merge: true)` so an author can add one action without
/// re-sending the whole contract.
fn upsert_by_name<T>(existing: &mut Vec<T>, incoming: Vec<T>, key: impl Fn(&T) -> String) {
    for item in incoming {
        let k = key(&item);
        match existing.iter().position(|e| key(e) == k) {
            Some(i) => existing[i] = item,
            None => existing.push(item),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppIdParams {
    /// App id.
    pub id: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ExportAppParams {
    /// App id.
    pub id: String,
    /// Destination directory (created if missing).
    pub target_dir: String,
    /// Override the agent WebSocket endpoint the exported app connects to.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Export mode: `"launcher"` (default) ships only the app + launch scripts,
    /// running against whatever knowledge bases / skills / extensions already
    /// exist on the target machine. `"full"` additionally stages the app's
    /// server-side payload under `payload/` and writes an audit manifest
    /// (`export.json`) — see Task 2/3 of the standalone-export design (§3.9).
    #[serde(default)]
    pub mode: Option<String>,
    /// Full-mode payload selection: `{"knowledge_bases": [ids], "skills":
    /// [names], "extensions": [names]}`. Any omitted key falls back to what the
    /// app's agent config references (KB → `agent.knowledge_base`; skills →
    /// `agent.skills`; extensions → `agent.extensions` minus built-ins). Ignored
    /// in launcher mode.
    #[serde(default)]
    pub include: Option<serde_json::Value>,
    /// Bundle the daemon binary for a self-contained ("fat") export:
    /// `"none"` (default) or `"current"` (the current platform's `biorouterd`).
    /// `"all"` (universal, every platform) is out of scope in this build and is
    /// treated as `"current"` with a note.
    #[serde(default)]
    pub bundle_daemon: Option<String>,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Agent Drafter MCP server.
#[derive(Clone)]
pub struct AgentDrafterServer {
    tool_router: ToolRouter<Self>,
    instructions: String,
    root: PathBuf,
}

impl Default for AgentDrafterServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared default store root (`<config>/agent_drafter`). Public so the server's
/// `/apps` routes resolve the same location. Honours `BIOROUTER_PATH_ROOT` via
/// [`crate::paths`], so a sandboxed run never writes into the user's global store.
pub fn default_root() -> PathBuf {
    crate::paths::in_config_dir("agent_drafter")
}

/// Run `scripts/agent-drafter/app-smoke.mjs` against a built app.
///
/// The executing check lives in Node because it needs a real browser: only a real
/// browser dispatches native range key handling, real pointer drags, and computed
/// styles — the three things the remaining findings depend on. jsdom cannot see any
/// of them, which is exactly why the audit could not either.
///
/// **Environment (issue #62).** This is the one Agent Drafter path that
/// *executes* agent-authored application code — the siblings only parse it
/// (`bundle.rs` runs esbuild, `render.rs` runs `node --check`). The Node harness
/// serves the app to a chromium launched with `--no-sandbox`, so the daemon's
/// auth secret must not be in the environment either process inherits: holding
/// `BIOROUTER_SERVER__SECRET_KEY` makes the holder a fully authenticated client
/// of `biorouterd`'s REST API, which is a cross-session read of everything
/// (issue #57).
///
/// The rule is the *shared* one, applied through
/// [`prepare_agent_drafter_child`] — where the reasons for using it rather than
/// a stricter allow-list are recorded.
fn run_smoke(dir: &Path) -> Result<String, String> {
    if std::env::var("BIOROUTER_APP_SMOKE")
        .unwrap_or_default()
        .eq_ignore_ascii_case("off")
    {
        return Ok("smoke check skipped (BIOROUTER_APP_SMOKE=off)".to_string());
    }

    let script = smoke_script_path().ok_or_else(|| "app-smoke.mjs not found".to_string())?;
    let mut command = std::process::Command::new("node");
    command.arg(&script).arg(dir);
    // Last, so nothing set above can leave a daemon credential in the child.
    prepare_agent_drafter_child(&mut command);
    let out = command
        .output()
        .map_err(|e| format!("could not run node: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    match out.status.code() {
        Some(0) => Ok(format!("smoke check PASSED.\n{stdout}")),
        Some(1) => Ok(format!(
            "smoke check found real defects; a user would hit these:\n{stdout}"
        )),
        _ => Err(format!("{stderr}{stdout}").trim().to_string()),
    }
}

/// The environment every Agent Drafter child is spawned with (issue #62).
///
/// One named seam for both spawns — [`run_smoke`]'s node harness and
/// `bundle::run_esbuild` — so neither can drift and a third one has an obvious
/// thing to call. The rule itself is the *shared* one,
/// [`strip_daemon_private_env_std`], the same function the developer shell,
/// and stdio extensions call.
///
/// A second, stricter mechanism here (an allow-list of, say, `PATH` + `HOME`)
/// was considered and rejected: it would be a parallel copy of one security
/// rule, which is how the next hole appears, and it would break the harness in
/// ordinary configurations. Playwright resolves its browser cache from `HOME`
/// *or* `PLAYWRIGHT_BROWSERS_PATH`, chromium wants `TMPDIR`/`XDG_*`, corporate
/// installs need the proxy variables, and on Windows a child holding only
/// `PATH` cannot even open the loopback socket the harness's mock daemon binds
/// (no `SystemRoot`). Issue #24 was that regression in miniature, for `PATH`
/// alone. Nothing here needs a credential; if it ever does, pass it explicitly.
pub(super) fn prepare_agent_drafter_child(command: &mut std::process::Command) {
    strip_daemon_private_env_std(command);
    crate::developer::shell::no_console_window_std(command);
}

/// Absolute path to `printenv`, for the environment-probe shims in this
/// module's and `bundle`'s tests.
///
/// The shims must not resolve it through `PATH`: sibling tests in this binary
/// swap `PATH` process-wide (`env_lock`), so a `PATH`-resolved `printenv`
/// execs fine alone and fails only in a full parallel run.
#[cfg(all(test, unix))]
pub(crate) fn printenv_bin() -> &'static str {
    ["/usr/bin/printenv", "/bin/printenv"]
        .into_iter()
        .find(|c| Path::new(c).exists())
        .unwrap_or("printenv")
}

/// Locate the smoke script relative to the running binary or the source tree.
fn smoke_script_path() -> Option<PathBuf> {
    let rel = "scripts/agent-drafter/app-smoke.mjs";
    // Dev tree: walk up from CARGO_MANIFEST_DIR.
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join(rel);
    if src.exists() {
        return Some(src);
    }
    // Installed: next to the executable.
    let exe = std::env::current_exe().ok()?;
    let near = exe.parent()?.join(rel);
    near.exists().then_some(near)
}

fn err(code: ErrorCode, msg: impl Into<String>) -> ErrorData {
    ErrorData::new(code, msg.into(), None)
}

fn internal(e: impl std::fmt::Display) -> ErrorData {
    err(ErrorCode::INTERNAL_ERROR, e.to_string())
}

impl From<ModelParam> for ModelSelection {
    fn from(p: ModelParam) -> Self {
        ModelSelection {
            provider: p.provider.filter(|s| !s.trim().is_empty()),
            model: p.model.filter(|s| !s.trim().is_empty()),
            settings: p.settings.map(Into::into),
        }
    }
}

impl From<ModelSettingsParam> for ModelSettings {
    fn from(p: ModelSettingsParam) -> Self {
        Self {
            temperature: p.temperature,
            max_tokens: p.max_tokens,
            top_p: p.top_p,
            reasoning_effort: p.reasoning_effort.filter(|s| !s.trim().is_empty()),
            verbosity: p.verbosity.filter(|s| !s.trim().is_empty()),
        }
    }
}

fn decode_agent_field<T: DeserializeOwned>(
    value: serde_json::Value,
    field: &str,
) -> Result<T, ErrorData> {
    serde_json::from_value(value).map_err(|e| {
        err(
            ErrorCode::INVALID_PARAMS,
            format!("{field} must match the Agent Drafter manifest schema: {e}"),
        )
    })
}

fn create_app_kind(p: &CreateAppParams) -> Result<ArtifactKind, ErrorData> {
    match p.kind.as_deref() {
        Some(kind) => ArtifactKind::parse(kind).ok_or_else(|| {
            err(
                ErrorCode::INVALID_PARAMS,
                "kind must be 'static' or 'agentic'",
            )
        }),
        None => Ok(ArtifactKind::Agentic),
    }
}

fn create_app_archetype(p: &CreateAppParams) -> Result<Archetype, ErrorData> {
    match p
        .archetype
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(archetype) => Archetype::parse(archetype).ok_or_else(|| {
            err(
                ErrorCode::INVALID_PARAMS,
                "archetype must be one of: explorer, dashboard, workbench, wizard, canvas, chat",
            )
        }),
        None => Ok(Archetype::infer(&p.title, &p.description)),
    }
}

fn create_app_files(
    p: &CreateAppParams,
    kind: ArtifactKind,
    archetype: Archetype,
) -> (bool, Vec<(String, String)>) {
    let entry = "index.html";
    let provided: std::collections::HashSet<&str> =
        p.files.iter().map(|file| file.path.as_str()).collect();
    let use_starter = kind == ArtifactKind::Agentic
        && archetype != Archetype::Chat
        && p.html.is_none()
        && !provided.contains("src/main.ts");
    let entry_html = match &p.html {
        Some(html) => html.clone(),
        None if use_starter => archetype
            .index_html(&p.title, &p.description)
            .unwrap_or_else(|| render::starter(&p.title, &p.description)),
        None => render::starter(&p.title, &p.description),
    };
    let mut files = vec![(entry.to_string(), entry_html)];
    if kind == ArtifactKind::Agentic {
        for (path, content) in bundle::default_sources() {
            let path = path.to_string_lossy().to_string();
            if path == entry || provided.contains(path.as_str()) {
                continue;
            }
            let content = if path == "src/main.ts" && use_starter {
                archetype.main_ts().map(str::to_string).unwrap_or(content)
            } else {
                content
            };
            files.push((path, content));
        }
    }
    files.extend(
        p.files
            .iter()
            .filter(|file| file.path != entry)
            .map(|file| (file.path.clone(), file.content.clone())),
    );
    (use_starter, files)
}

fn created_agent_config(p: &mut CreateAppParams) -> Result<AgentConfig, ErrorData> {
    let model = p
        .model
        .take()
        .map(ModelSelection::from)
        .filter(|m| m.is_set());
    let mut agent = AgentConfig {
        system_prompt: p.system_prompt.take().unwrap_or_default(),
        greeting: p.greeting.take(),
        tools: Vec::new(),
        model,
        extensions: std::mem::take(&mut p.extensions),
        skills: std::mem::take(&mut p.skills),
        knowledge_base: p
            .knowledge_base
            .take()
            .filter(|value| !value.trim().is_empty()),
        max_turns: None,
        ..Default::default()
    };
    if let Some(value) = p.capabilities.take() {
        agent.capabilities = decode_agent_field::<Capabilities>(value, "capabilities")?;
    }
    if let Some(value) = p.guardrails.take() {
        agent.guardrails = Some(decode_agent_field::<GuardrailsConfig>(value, "guardrails")?);
    }
    if let Some(value) = p.reliability.take() {
        agent.reliability = Some(decode_agent_field::<ReliabilityConfig>(
            value,
            "reliability",
        )?);
    }
    if let Some(value) = p.orchestration.take() {
        agent.orchestration = decode_agent_field::<Orchestration>(value, "orchestration")?;
    }
    agent.output_type = p.output_type.take();
    agent.durable_session = p.durable_session.take();
    Ok(agent)
}

fn persist_created_app(
    store: &ArtifactStore,
    manifest: &mut Manifest,
    mut p: CreateAppParams,
    kind: ArtifactKind,
    archetype: Archetype,
    use_starter: bool,
    // Issue #56 (CP5). The catalog this write boundary validates against is the
    // one the CALLER may see, so a public session cannot save an app scoped to a
    // private base and cannot learn the base exists from the rejection.
    //
    // ⚠ Finding 17: BOTH axes, as one value. This was a bare `caller_is_private`
    // and the catalogue below asked the tier axis alone — see
    // `Catalog::discover`.
    caller: &KbCaller,
) -> Result<(), ErrorData> {
    if let Some(theme) = p.theme.take() {
        manifest.theme = theme.into_config();
    }

    if kind != ArtifactKind::Agentic {
        store.save_manifest(manifest).map_err(internal)?;
        return Ok(());
    }

    let agent = created_agent_config(&mut p)?;
    let catalog = catalog::Catalog::discover(caller);
    validate::check_all(
        agent.knowledge_base.as_deref(),
        &agent.skills,
        &agent.extensions,
        &catalog,
    )
    .map_err(|e| err(ErrorCode::INVALID_PARAMS, e))?;
    manifest.agent = Some(agent);

    if let Some(surface) = p.surface.take() {
        if !surface.is_empty() {
            manifest.surface = surface.into_decl();
        }
    } else if use_starter {
        manifest.surface = archetype.surface();
    }
    store.save_manifest(manifest).map_err(internal)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Directories inside an app that must never leave the author's machine. The
/// vault holds AES-sealed secrets whose key lives in the author's OS keyring —
/// worthless to a recipient, and not ours to copy around.
const EXPORT_EXCLUDED_DIRS: &[&str] = &[".vault/", ".git/"];

/// Recursively collect an app's files (relative path → contents).
///
/// `manifest.json` is skipped here and re-emitted by `scaffold_standalone` from
/// the parsed [`Manifest`], so the export always carries a canonical one.
fn collect_files(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_files(base, &path, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel == "manifest.json" || EXPORT_EXCLUDED_DIRS.iter().any(|d| rel.starts_with(d)) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                out.push((rel, content));
            }
        }
    }
}

/// Whether `id`'s bundle needs rebuilding before it can be served: it has never
/// been built, or it was built against an older App SDK than this binary ships.
///
/// The second case matters because each app vendors its own `src/sdk.ts`. Without
/// this, an app authored before a protocol addition keeps running the old runtime
/// forever and silently drops frames the server now sends (e.g. the `ui` commands
/// that let the agent drive the page).
pub fn bundle_is_stale(store: &ArtifactStore, id: &str, manifest: &Manifest) -> bool {
    if manifest.kind != ArtifactKind::Agentic {
        return false;
    }
    if !store.file_exists(id, "dist/app.js") {
        return true;
    }
    manifest.sdk_hash.as_deref() != Some(&bundle::sdk_fingerprint())
}

/// Rebuild `id` and stamp the manifest with the build time + SDK fingerprint.
/// Blocking (runs esbuild); callers on an async runtime must `spawn_blocking`.
///
pub fn rebuild_and_stamp(store: &ArtifactStore, id: &str) -> std::io::Result<bundle::BuildReport> {
    // `build_app` refreshes the vendored `src/sdk.ts` before bundling, so the
    // fingerprint we stamp below always describes what actually went into
    // `dist/app.js` — never a current hash over a stale runtime.
    let report = bundle::build_app(&store.artifact_dir(id)?)?;
    if report.ok {
        if let Ok(mut m) = store.load_manifest(id) {
            m.built_at = Some(now_secs());
            m.sdk_hash = Some(bundle::sdk_fingerprint());
            let _ = store.save_manifest(&m);
        }
    }
    Ok(report)
}

/// Give a written export file the exec bit (owner+group+other rx, owner w).
/// No-op on Windows, where executability isn't a file mode.
fn make_executable(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Gather an app's files and produce the standalone export scaffold (a file map
/// of relative-path → contents). Shared by the `export_app` tool and the
/// server's `GET /apps/{id}/export` route. `endpoint` overrides the agent
/// WebSocket the exported app connects to (None → derived from the page origin,
/// with a loopback fallback).
pub fn export_scaffold(
    root: &std::path::Path,
    id: &str,
    endpoint: Option<&str>,
) -> std::io::Result<Vec<(String, String)>> {
    let store = ArtifactStore::new(root.to_path_buf());
    let manifest = store.load_manifest(id)?;
    let dir = store.artifact_dir(id)?;
    let mut all_files = Vec::new();
    collect_files(&dir, &dir, &mut all_files);
    let entry_html = all_files
        .iter()
        .find(|(p, _)| p == &manifest.entry)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "entry file missing"))?;
    let src_files: Vec<(String, String)> = all_files
        .iter()
        .filter(|(p, _)| p.starts_with("src/"))
        .cloned()
        .collect();
    let extra_files: Vec<(String, String)> = all_files
        .iter()
        .filter(|(p, _)| p != &manifest.entry && !p.starts_with("src/") && !p.starts_with("dist/"))
        .cloned()
        .collect();
    let mut scaffold =
        render::scaffold_standalone(&manifest, &entry_html, &src_files, &extra_files, endpoint);
    // Ship a prebuilt bundle so the export is directly runnable with no build
    // step (the launcher / a static server can serve it as-is). Build on demand.
    if manifest.kind == ArtifactKind::Agentic {
        // Ship a bundle built from the CURRENT SDK, not a stale one the app
        // happens to have on disk — an export is the copy that leaves the machine.
        if bundle_is_stale(&store, id, &manifest) {
            let _ = rebuild_and_stamp(&store, id);
        }
        if let Ok(js) = store.read_file(id, "dist/app.js") {
            scaffold.push(("dist/app.js".to_string(), js));
        }
    }
    Ok(scaffold)
}

// ---------------------------------------------------------------------------
// Full-export payload staging (design §3.9 — Task 2/3)
// ---------------------------------------------------------------------------

/// The biorouter-mcp built-in MCP server names (see `BUILTIN_EXTENSIONS` in
/// `lib.rs`). Built-in extensions travel with the daemon, so a full export never
/// stages them — only *external* extensions are recorded as registry references
/// in `export.json`. Hardcoded (with this comment) because the app manifest only
/// stores extension *names*, not whether each is built in.
pub(crate) const BUILTIN_EXTENSION_NAMES: &[&str] = &[
    "developer",
    "computercontroller",
    "webdocuments",
    "autovisualiser",
    "memory",
    "agent_drafter",
    "knowledge",
];

fn is_builtin_extension(name: &str) -> bool {
    BUILTIN_EXTENSION_NAMES.contains(&name)
}

/// Resolve a `KnowledgeService` for reading the author's knowledge bases while
/// staging a full export. Honours `BIOROUTER_KNOWLEDGE_DIR` (an override used by
/// tests and power users); otherwise the canonical store the `/knowledge` routes
/// use. `None` when the store can't be opened → the caller notes the KBs were
/// skipped rather than failing the export.
fn knowledge_service_for_export() -> Option<crate::knowledge::service::KnowledgeService> {
    use crate::knowledge::service::KnowledgeService;
    if let Ok(dir) = std::env::var("BIOROUTER_KNOWLEDGE_DIR") {
        if !dir.trim().is_empty() {
            return Some(KnowledgeService::new(PathBuf::from(dir)));
        }
    }
    KnowledgeService::new_default().ok()
}

/// The installed-skills directory (`<config>/skills`). Honours
/// `BIOROUTER_SKILLS_DIR` as a test/override hook, mirroring [`default_root`]'s
/// config-dir resolution.
fn skills_root_for_export() -> PathBuf {
    if let Ok(dir) = std::env::var("BIOROUTER_SKILLS_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    crate::paths::in_config_dir("skills")
}

/// Locate a `biorouterd` binary to bundle for a fat export. This code runs
/// inside `biorouterd` (GUI) **or** `biorouter` (CLI), so `current_exe` is not
/// necessarily the daemon — we look next to it (the GUI ships them side by
/// side), then on `PATH`. `BIOROUTERD_BIN` overrides (and is the test hook).
/// `None` → the caller falls back to a thin export.
fn find_biorouterd_binary() -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        "biorouterd.exe"
    } else {
        "biorouterd"
    };
    if let Ok(p) = std::env::var("BIOROUTERD_BIN") {
        let pb = PathBuf::from(p);
        return pb.is_file().then_some(pb);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join(exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Recursively copy `src` into `dst` (used to stage a skill directory tree).
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Extract a `Vec<String>` from `include[key]` when it is an array of strings.
/// `None` (key absent) means the caller falls back to the agent-config default;
/// `Some(vec![])` (key present but empty) means "select none".
fn selected_list(include: Option<&serde_json::Value>, key: &str) -> Option<Vec<String>> {
    let arr = include?.get(key)?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

/// The pieces staged for a full export, recorded in `export.json` and surfaced
/// as notes in the tool result.
struct StagedPayload {
    /// `{id, file, bytes}` per staged `.brkb`.
    knowledge_bases: Vec<serde_json::Value>,
    /// `{name, path}` per staged skill directory.
    skills: Vec<serde_json::Value>,
    /// `{name, source:"registry", note}` per external extension.
    extensions: Vec<serde_json::Value>,
    /// Human-readable notes (skips, failures) — never fatal.
    notes: Vec<String>,
}

impl StagedPayload {
    fn empty() -> Self {
        Self {
            knowledge_bases: Vec::new(),
            skills: Vec::new(),
            extensions: Vec::new(),
            notes: Vec::new(),
        }
    }
}

/// Stage the app's server-side payload under `<target>/payload/` (full mode).
///
/// Selection is per-item: an explicit `include` list wins; an omitted key falls
/// back to what the app's agent config references (KB → `agent.knowledge_base`,
/// skills → `agent.skills`, extensions → `agent.extensions` minus built-ins).
/// A missing KB / skill is skipped with a note — it never fails the export.
fn stage_full_payload(
    manifest: &Manifest,
    target: &std::path::Path,
    include: Option<&serde_json::Value>,
    // Issue #56 (CP4). The capability of the session that asked for the export —
    // BOTH axes as one value (issue #56 DR-26 / Task 50, and finding 17's
    // lesson): a signature that took them separately let one call site pass one
    // caller's tier and another's institution, and the wrong one compiles.
    caller: &KbCaller,
) -> StagedPayload {
    let agent = manifest.agent.clone().unwrap_or_default();

    let mut kb_ids = selected_list(include, "knowledge_bases")
        .unwrap_or_else(|| agent.knowledge_base.clone().into_iter().collect());
    let mut skill_names = selected_list(include, "skills").unwrap_or_else(|| agent.skills.clone());
    let mut ext_names = selected_list(include, "extensions").unwrap_or_else(|| {
        agent
            .extensions
            .iter()
            .filter(|e| !is_builtin_extension(e))
            .cloned()
            .collect()
    });
    // Deterministic, deduped ordering so the export is auditable + reproducible.
    for v in [&mut kb_ids, &mut skill_names, &mut ext_names] {
        v.sort();
        v.dedup();
    }

    let payload_dir = target.join("payload");
    let mut out = StagedPayload::empty();

    // ── Knowledge bases → payload/knowledge/<id>.brkb ──────────────────────
    if !kb_ids.is_empty() {
        match knowledge_service_for_export() {
            Some(svc) => {
                let kdir = payload_dir.join("knowledge");
                for kb in &kb_ids {
                    // Issue #56 (CP4), Task 10C. `export_brkb` writes the WHOLE
                    // base into the payload, and `kb_ids` comes from the
                    // model-supplied `include.knowledge_bases` — a strictly
                    // wider `kb_export` that never touches `KnowledgeServer`, so
                    // CP1 cannot see it. Skip-and-note rather than fail the
                    // export, matching `search_visible_bases`: the rest of the
                    // payload is still useful and the user is told what was left
                    // out.
                    if let Err(e) = caller.assert_reachable(svc.root(), kb) {
                        out.notes
                            .push(format!("skipped knowledge base '{kb}': {e}"));
                        continue;
                    }
                    match svc.export_brkb(kb) {
                        Ok(bytes) => {
                            let fname = format!("{kb}.brkb");
                            let ok = std::fs::create_dir_all(&kdir).is_ok()
                                && std::fs::write(kdir.join(&fname), &bytes).is_ok();
                            if ok {
                                out.knowledge_bases.push(serde_json::json!({
                                    "id": kb,
                                    "file": format!("payload/knowledge/{fname}"),
                                    "bytes": bytes.len(),
                                }));
                            } else {
                                out.notes.push(format!(
                                    "could not write knowledge base '{kb}' into the payload; skipped"
                                ));
                            }
                        }
                        Err(e) => out
                            .notes
                            .push(format!("skipped knowledge base '{kb}': {e}")),
                    }
                }
            }
            None => out.notes.push(format!(
                "knowledge store unavailable; skipped {} knowledge base(s)",
                kb_ids.len()
            )),
        }
    }

    // ── Skills → payload/skills/<name>/ (plain directory copy) ─────────────
    // A plain recursive copy is used rather than a marketplace-format zip: it is
    // simpler, needs no extra crate, and the launcher installs it with a plain
    // dir copy into the skills dir. (The design allows either.)
    if !skill_names.is_empty() {
        let skills_root = skills_root_for_export();
        let sdir = payload_dir.join("skills");
        for name in &skill_names {
            let src = skills_root.join(name);
            if src.is_dir() {
                let dst = sdir.join(name);
                if copy_dir_recursive(&src, &dst).is_ok() {
                    out.skills.push(serde_json::json!({
                        "name": name,
                        "path": format!("payload/skills/{name}"),
                    }));
                } else {
                    out.notes.push(format!(
                        "could not copy skill '{name}' into the payload; skipped"
                    ));
                }
            } else {
                out.notes
                    .push(format!("skill '{name}' not installed; skipped"));
            }
        }
    }

    // ── External extensions → recorded as registry references only ─────────
    // Staging installed `.brxt` bundles (the installed-bundle layout) is out of
    // scope; a full export records external extensions as pinned registry
    // references the first-run installer resolves from BAAM.
    for name in &ext_names {
        out.extensions.push(serde_json::json!({
            "name": name,
            "source": "registry",
            "note": "install from the BAAM registry on the target; .brxt bundle staging is out of scope",
        }));
    }

    out
}

/// Assemble the deterministic `export.json` payload manifest (design §3.9 —
/// Task 3). `required_credentials` and `runtime_requirements` are empty: the app
/// manifest does not carry the extensions' declared credential env keys or
/// runtime prerequisites (those live in each extension's own bundle metadata /
/// the BAAM registry, not in the app manifest), so they cannot be enumerated
/// here without the registry. The first-run credential dialog (§3.9 item 3)
/// resolves what a specific extension needs.
fn build_export_json(
    id: &str,
    mode: &str,
    staged: &StagedPayload,
    bundled_daemon: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "app": id,
        "mode": mode,
        "knowledge_bases": staged.knowledge_bases,
        "skills": staged.skills,
        "extensions": staged.extensions,
        "bundled_daemon": bundled_daemon,
        "required_credentials": [],
        "runtime_requirements": [],
    })
}

/// The assistant-facing summary `export_app` returns, split out of
/// `export_app_inner` so that function stays under `clippy::too_many_lines`
/// (issue #56, review round 5). No behaviour change.
fn export_summary(
    id: &str,
    mode: &str,
    target: &Path,
    scaffold_files: usize,
    staged: &StagedPayload,
    wrote_manifest: bool,
    notes: &[String],
) -> String {
    let mut msg = format!(
        "Exported '{}' ({mode} mode) to {} ({} scaffold files). \
         To run it: double-click run.command (macOS), `bash run.sh` (Linux), or run.bat (Windows). \
         That installs the app, starts a biorouterd if one isn't already up, and opens it in the browser, with \
         no npm install and no build step.",
        id,
        target.display(),
        scaffold_files
    );
    if mode == "full" {
        msg.push_str(&format!(
            "\nPayload: {} knowledge base(s), {} skill(s), {} external extension reference(s).",
            staged.knowledge_bases.len(),
            staged.skills.len(),
            staged.extensions.len()
        ));
    }
    if wrote_manifest {
        msg.push_str(" Wrote export.json (audit manifest).");
    }
    for n in notes {
        msg.push_str(&format!("\n- {n}"));
    }
    msg
}

/// Stage the current-platform `biorouterd` under `payload/bin/` for a fat
/// export. Returns the `export.json` record on success (or `None` → thin), plus
/// a note either way.
fn stage_current_daemon(target: &std::path::Path) -> (Option<serde_json::Value>, String) {
    let bin_name = if cfg!(windows) {
        "biorouterd.exe"
    } else {
        "biorouterd"
    };
    match find_biorouterd_binary() {
        Some(bin) => {
            let dst = target.join("payload").join("bin").join(bin_name);
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::copy(&bin, &dst) {
                Ok(bytes) => {
                    make_executable(&dst);
                    let rec = serde_json::json!({
                        "platform": std::env::consts::OS,
                        "arch": std::env::consts::ARCH,
                        "file": format!("payload/bin/{bin_name}"),
                        "bytes": bytes,
                    });
                    (
                        Some(rec),
                        format!(
                            "bundled the {}-{} daemon at payload/bin/{bin_name} (fat export)",
                            std::env::consts::OS,
                            std::env::consts::ARCH
                        ),
                    )
                }
                Err(e) => (
                    None,
                    format!("could not copy biorouterd into the payload ({e}); thin export instead"),
                ),
            }
        }
        None => (
            None,
            "biorouterd not found for a fat export; thin export instead (the launcher locates or installs a daemon at run time)".to_string(),
        ),
    }
}

/// Issue #56 (CP4/CP5). `export_app`, `configure_app` and `update_app` gained a
/// `RequestContext` so they can read the caller's capability; the unit tests
/// below drive their bodies without fabricating one, and are all public-caller
/// cases. This is the same split `create_app` / `create_app_inner` already uses,
/// named so the caller's tier is legible at each call site rather than hidden in
/// a default.
///
/// The tests that need a *private* caller — or that need to prove the TOOL reads
/// the right value rather than that the body honours it — go through the router
/// instead (`privacy_catalog::call_drafter_tool_as`).
#[cfg(test)]
impl AgentDrafterServer {
    async fn export_app_public(
        &self,
        params: Parameters<ExportAppParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.export_app_inner(params.0, &KbCaller::restricted())
            .await
    }

    async fn configure_app_public(
        &self,
        params: Parameters<ConfigureAppParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.configure_app_inner(params.0, &KbCaller::restricted())
            .await
    }

    async fn update_app_public(
        &self,
        params: Parameters<UpdateAppParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.update_app_inner(params.0, &KbCaller::restricted())
            .await
    }
}

#[tool_router(router = tool_router)]
impl AgentDrafterServer {
    pub fn new() -> Self {
        Self::with_root(default_root())
    }

    /// The caller's capability, from the request meta (issue #56, CP5).
    ///
    /// Delegates to [`crate::knowledge::tier::caller_is_private`] rather than
    /// re-reading the key, so CP4 and CP5 cannot drift from CP1 — the same reason
    /// `KnowledgeServer::caller_is_private` delegates. A second spelling of
    /// `biorouter-capability-tier` compiles, passes every drafter test, and
    /// silently stops matching the day the key changes.
    fn caller_is_private(context: &RequestContext<RoleServer>) -> bool {
        crate::knowledge::tier::caller_is_private(&context.meta)
    }

    /// The caller's **affiliation**, from the same request meta (issue #56,
    /// DR-26 / Task 50). Delegates for the reason above: CP4 and CP1 must read
    /// one key with one reader.
    fn caller_affiliation(
        context: &RequestContext<RoleServer>,
    ) -> crate::knowledge::affiliation::CallerAffiliation {
        crate::knowledge::affiliation::caller_affiliation(&context.meta)
    }

    /// Both axes of the caller's identity, off ONE request meta at one instant
    /// (issue #56 DR-26; audit finding 17).
    ///
    /// ⚠ **The only value CP4 and CP5 may take.** They used to take the tier
    /// alone (CP5) or the two axes as separate arguments (CP4), which is how a
    /// listing and a barrier over the same capability came to ask different
    /// questions. `KbCaller` cannot express half a caller, so a future filter
    /// cannot silently drop an axis — there is no narrower thing to pass. The
    /// twin of `KnowledgeServer::CallerIdentity::from_context`, and it fails
    /// closed on both fields for the same reason.
    fn caller(context: &RequestContext<RoleServer>) -> KbCaller {
        KbCaller::new(
            Self::caller_is_private(context),
            Self::caller_affiliation(context),
        )
    }

    /// The chat session id carried in a tool call's request meta, if present.
    fn session_id_from_context(context: &RequestContext<RoleServer>) -> Option<String> {
        context
            .meta
            .0
            .get(SESSION_ID_META_KEY)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    #[allow(clippy::too_many_lines)]
    pub fn with_root(root: PathBuf) -> Self {
        let instructions = formatdoc! {r#"
            Agent Drafter builds interactive **Biorouter apps** for the user:
            TypeScript front-ends wired to a real Biorouter agent. Think "Claude
            artifacts", but each app embeds a genuine Biorouter backend: when the
            user sends a message, Biorouter runs the full agent loop (the app's own
            model, enabled capabilities, installed extensions, skills, and knowledge base) and streams the answer back
            into the app. The GUI presents a clickable preview and the CLI prints
            a browser URL; apps are NOT shown in a chat iframe.

            Two kinds:
            - "agentic" (default): a UI plus a live Biorouter agent + chat. Use this
              for assistants, dashboards that reason over results, search tools, etc.
            - "static": a plain interactive page with no agent.

            CAPABILITY TERMINOLOGY. Biorouter-shipped tool surfaces are capabilities;
            third-party or user-installed connectors are extensions. The app manifest's
            legacy `agent.extensions` field can contain either kind for compatibility,
            but that field name does not reclassify a built-in capability as an
            extension. Call `list_platform_catalog` before configuration and preserve
            the catalog's classification in user-facing explanations.

            Project layout (kept consistent):
            - `index.html`: the UI shell you author.
            - `src/main.ts`: your app logic (TypeScript), `import`ing `./sdk`.
            - `src/sdk.ts`: the Biorouter App SDK (provided), which opens the agent
              WebSocket, streams markdown, handles multimodal (image) input, and can
              auto-mount a chat panel into any element with `data-br-chat`.
            - `dist/app.js`: the esbuild bundle (produced by `build_app`).

            ARCHETYPES FIRST, and the #1 rule: do NOT make every app a chat box.
            `create_app` seeds a starter **archetype** (pass `archetype`, or one
            is inferred from the title/description). Each non-chat archetype ships
            a working, lint-clean `index.html` + `src/main.ts` + a declared
            `surface` you then extend, teaching by example. Pick the shape that
            fits the task:
            - `explorer`:   a graph/network the agent renders + inspector + search.
            - `dashboard`:  a KPI grid bound to shared state + a refresh action.
            - `workbench`:  a data table + row-select signal + a bound detail panel.
            - `wizard`:     a staged form that writes state, then submits.
            - `canvas`:     an author-registered draw surface + agent-called
              actions (the avatar / scene / simulation shape).
            - `chat`:       today's chat card; pick it ONLY for a pure
              assistant/Q&A. `chat` is one option among six, never the default.
            One compact exemplar per archetype (the starter files are the full
            version, so read/extend them rather than rewriting from scratch):
              // explorer: agent centers a node; the inspector is bound to it.
              br.actions.register("focus_node", (a) => br.state.set("/selection", a));
              // dashboard: agent writes a KPI tile; the bound grid re-renders it.
              br.actions.register("set_metric", (m) => br.state.set("/metrics/"+m.key, m));
              // workbench: agent opens a row into the bound detail panel.
              br.actions.register("open_row", (r) => br.state.set("/detail", r));
              // wizard: submit sends a typed turn carrying the collected form.
              submit.onclick = () => br.call("submit", {{ name, goal }});
              // canvas: agent moves the avatar; the scene redraws from /scene.
              br.actions.register("move_avatar", (a) => world.move(a.direction, a.steps));
              // chat: the default, where createApp() auto-mounts a [data-br-chat] panel.

            DECLARE THE SURFACE. An app's contract is `manifest.surface` (seed it
            in `create_app`, or edit the seeded one). Registrations in `main.ts`
            must match the declarations exactly, because typed actions/components fail
            closed:
            - `actions`:      verbs the AGENT may call (`app_call`). Register each
              with `br.actions.register("name", fn)`; the handler's return value
              resolves the agent's tool call.
            - `signals`:      app→agent notifications the agent may subscribe to.
              Emit with `br.signals.emit("name", payload)`; every emitted name
              must be declared.
            - `components`:   custom catalog kinds you draw. Register with
              `br.components.register("name", {{ mount, update? }})`; props are
              agent-controlled (untrusted), so render via textContent, not innerHTML.
            - `state_schema`: JSON Schema for the shared state doc. Declare one
              whenever you use `data-br-bind`, so the agent's writes are validated.

            WIRE TYPED CALLS, NOT PROMPT STRINGS. When the app declares actions,
            drive the agent with structured data, not hand-assembled English:
              const r = await br.call("rank_genes", {{ cohort, top: 10 }}); // typed
            Keep `br.run(prompt, '#out')` for genuine natural-language asks
            ("explain this selection") and to stream markdown into a result
            surface, but do NOT concatenate control state into a prompt string
            when a typed action/call fits. The agent invokes your actions via
            `app_call`; you never parse prose to discover intent.

            SHARED REACTIVE STATE + BINDINGS: one JSON state document both sides
            write:
              <span data-br-bind="/cohort/count"></span>   // re-renders on write
              br.state.set("/cohort/count", 42);            // author write (agent too)
            Bindings are a non-executing sink (textContent / safe attributes only).
            Bind the parts of the UI the agent should keep live; the runtime
            re-renders only the bound nodes, so focus/scroll/input survive.

            THE DYNAVIS RULE. After fulfilling a natural-language request that
            CHANGED a parameter, emit a *persistent bound control* for it, so the
            user refines by direct manipulation instead of re-prompting. E.g.
            after "make the KM curve use a 90-day window", `ui_patch` in a slider
            bound to `/plot/km_window`. NL bootstraps; a synthesized control
            refines, which is the single most user-validated GenUI pattern. Reach for it
            every time a prompt tuned a knob.

            PRESENCE & NARRATION. Make agent UI changes legible, not startling:
            - The SDK renders an ambient activity chip for every `ui_*` frame;
              give `ui_highlight` a `narrate` note ("scoring the top variants…").
            - Observe, don't hijack: agent updates MARK rather than steal focus
              (no auto-scroll unless you pass `scroll:true`).
            - Prefer `ui_ask` (blocking) for a required answer; `ui_suggest`
              (dismissible chips) for optional nudges.

            PUBLICATION FIGURES. For a real scientific figure (volcano, Manhattan,
            Kaplan-Meier, Sankey, chord, map, Mermaid…) have the agent emit a
            `ui_figure` (an Auto Visualiser fragment) into your results region,
            rather than a hand-rolled chart. Reserve the lighter `ui_chart` /
            `ui_graph` for quick inline glances.

            THEME PACK. Choose a `theme` pack that fits the domain instead of
            shipping the default light theme everywhere: `biorouter` (default),
            `clinical`, `lab-notebook`, `terminal`, `journal`, `midnight` (each
            with its own native light or dark palette). Compose within the pack's
            tokens; for distinctive, non-generic layouts within the token system,
            consult the **frontend-design** skill.

            DESIGN CONTRACT. The user's requested product design is the source of
            truth. Do not force a pre-designed app pattern, dashboard structure, or
            visual style when the user specified something else. Biorouter injects
            a design system only as a dependable primitive set and fallback. Use
            its classes (`br-container`, `br-card`, `br-btn`, `br-input`,
            `br-textarea`, `br-label`, `br-field`, `br-row`, `br-badge`,
            `br-panel`, `br-slab`, `br-swatch`, `br-chat`, `br-output`,
            `br-run-status`) and CSS variables for portability, but choose layout,
            information architecture, controls, visualization grammar, and
            interaction flow from the user's brief. Favor filled surfaces, shaded
            blocks, quiet token-backed color fields, and useful visual hierarchy.
            Avoid making the app look like a wireframe: do not outline every UI
            element, do not rely on line-drawn boxes as the main design language,
            and keep borders faint unless the user explicitly asks for a technical
            schematic look. If the user has not specified a visual direction, ask
            only when it would materially change the result; otherwise use a calm,
            readable Biorouter-native style and keep it easy to revise.

            THEME TOKENS. Colour every piece of text with the design tokens, never
            a hardcoded hex/rgb. Use `var(--br-text)` for primary text,
            `var(--br-text-muted)` for secondary/placeholder text, `var(--br-surface)`
            / `var(--br-bg)` for backgrounds, `var(--br-border)` for lines. A
            hardcoded `color:#333` looks fine while you author in light mode but goes
            invisible when the app is themed dark. An unthemed app renders light by
            default; a selected pack supplies its native palette, and the agent may
            switch it with `ui_theme`.

            AGENT DESIGN CONTRACT. Before or while authoring an agentic app,
            make the agent's operational choices explicit:
            - preferred provider/model and generation settings (`model.settings`);
              omit the model only when the user wants to inherit Biorouter's global
              provider/model.
            - knowledge bases and data sources (`knowledge_base` and
              `capabilities.data.sources`, including `kind: "knowledge"`).
            - extensions, skills, and which of them are user-changeable in the app.
            - the ordered workflow, where multi-agent collaboration is needed
              (`orchestration.sub_agents` / `orchestration.workflows`), where
              compaction or memory should happen, and which guardrails/approvals
              apply.
            - what freedom the final app user should receive: include UI controls
              for model switching via `br.model.list()` / `br.model.select(...)`
              when allowed, and expose behavior/skill/extension choices as normal
              controls that are folded into the prompt or workflow.

            Driving the agent from `src/main.ts`:
              import {{ createApp }} from "./sdk";
              const br = createApp({{ autoChat: false }});   // false → build your OWN UI
              const r = await br.call("act", {{ arg: 1 }});   // typed turn + structured result
              await br.run("...prompt...", '#out');            // stream markdown+visuals into a result element
              const text = await br.ask("...");               // collect full reply as a string
              await br.prompt("...", {{ images: [{{ mimeType, data }}] }}); // multimodal
              br.actions.register("verb", (args) => {{ /* agent-called */ }}); // app_call handler
              br.signals.emit("event", payload);              // notify a subscribed agent
              br.on("message", (e) => {{ if (e.type === "message") {{}} }}); // low-level stream

            CONTROL PALETTE. The starter archetypes already wire a custom UI; when
            you add or replace controls, use the themed, Biorouter-native ones and
            wire them to `br.call(...)` / `br.run(...)`, in a mix that fits the task:
            - buttons / button grids: `br-btn`, `br-grid`
            - dropdowns: `<select class="br-select">`
            - sliders: `<input type="range" class="br-slider">` (+ `br-slider-val`)
            - toggles: `<label class="br-switch"><input type="checkbox"><span class="br-switch__track"></span></label>`
            - checkboxes/radios: `br-check`; selectable chips/tags: `br-chips`/`br-chip`
            - tabs: `br-tabs`/`br-tab`; cards: `br-card`; layout: `br-grid`/`br-row`
            - drag & drop: `br-dropzone` (drop files/text) and `br-draglist`/`br-dragitem` (reorder)
            - region/map pick: `br-mapgrid`/`br-region` (clickable cells; no external map lib)
            - results: a `<div class="br-output" data-placeholder="…">` target for `br.run`
            On `change`/`click`/`drop`: prefer a typed `br.call(action, args)` or
            an emitted signal built from the control state; fall back to
            `br.run(...)` only for a genuine natural-language ask. Each app should
            look and interact differently from the others, and the archetypes make
            that the default, not an afterthought.

            Typical workflow:
            1. `create_app` (title, description, `archetype`, optional html/files,
              system_prompt,
              greeting, model, extensions, skills, knowledge_base, capabilities,
              guardrails, reliability, orchestration, output_type). A preview card
              is shown to the user.
            2. Author the UI: `update_app` the entry HTML and `src/main.ts`.
            3. `configure_app` to change the model/extensions/skills/knowledge/persona.
            4. `build_app` to bundle the TypeScript.
            5. `launch_app` to open it in the browser (returns the URL).
            6. `export_app` for a standalone runnable project.

            Use `list_apps` and `read_app` to inspect existing apps — `read_app` with
            `view: "card"` renders the preview the user sees. You can query and modify
            any previously-built app.

            KNOW YOUR ENVIRONMENT BEFORE YOU CONFIGURE IT. Call
            `list_platform_catalog` before naming any knowledge base, skill, or
            extension. It returns exactly what this install has. Ids that are not
            in it are REJECTED. Configuring a knowledge base or skill that does
            not exist arms tools scoped to nothing and makes the app fail on its
            first turn. If the app needs something this machine does not have,
            that is fine and normal: leave the id unset and record the need in
            `requires` (e.g.
            `[{{"kind":"knowledge_base","id":"clinvar","reason":"variant annotations"}}]`).
            The user is shown the unmet requirement honestly. NEVER invent an id
            to express a need; `requires` is what that is for. (In particular
            `br.kb` is the CLIENT API your app calls at runtime, never an id.)

            WORKFLOW-STYLE APPS (multi-step agentic loops, not just chat): every
            user message runs Biorouter's full agent loop, so the agent can call
            many tools in sequence and reason over the results before replying, so
            an app can encode a real pipeline. Design one by: (a) giving it the
            extensions/skills/knowledge it needs, (b) writing a system_prompt that
            spells out the ordered procedure ("1. search … 2. extract … 3.
            summarize as a table … 4. emit a ```chart or ```graph block"), and (c) raising
            `max_turns` (via `configure_app`) so it can chain enough tool calls.
            `max_turns` also bounds the loop (a guardrail against runaway/cost).
            The app must surface each step to the user. `br.run(...)` and the
            default chat panel automatically show a detailed `br-run-status`
            timeline for tool calls, guardrails, handoffs, compaction, context,
            model switches, completion, and errors. If you drive `br.prompt(...)`
            or `br.ask(...)` manually, mount the same timeline with
            `mountTimeline(br, '#progress')` or build an equivalent visible
            progress/debug panel. Never leave long-running agent work as only a
            spinner.

            VISUALIZATION CONTRACT. When an app claims to create visualizations,
            the final answer must contain at least one rendered visualization
            block in the visible result surface, not only a table, code snippet,
            or tool-call transcript. The SDK renders these fenced blocks:
            - ```chart with JSON `{{ "type": "bar" | "line" | "pie", "title": "...",
              "data": [{{ "label": "...", "value": 3 }}] }}`
            - ```graph / ```diagram / ```network with JSON `{{ "title": "...",
              "nodes": [{{ "id": "A", "label": "..." }}], "edges": [
              {{ "source": "A", "target": "B", "label": "..." }}] }}`
            - ```graph / ```diagram / ```mermaid with simple edge lines like
              `A -> B : relationship`
            For graph, map, chart, figure, dashboard, timeline, or analysis tools,
            put the required fence format directly in the agent system prompt and
            in the UI-built run prompt. Tables are useful evidence, but they do
            not satisfy the visualization requirement by themselves.

            AGENT-DRIVEN UI. The app's agent does not only *answer inside* the
            app, it can *change* the app. Every agentic app is granted `ui_*`
            tools (`capabilities.ui`, on by default) that push commands down its
            own WebSocket:
            - `ui_panel`: mount/replace/remove a panel or dashboard (widget
              nodes: card/row/col/text/badge/stat/divider/progress/table/chart/
              graph/input/select/checkbox/button/form).
            - `ui_render`: render into a region the AUTHOR declared, a panel, or
              a CSS selector.
            - `ui_chart` / `ui_graph`: draw a figure straight into the page.
            - `ui_highlight`: outline/pulse/focus part of the app, with a note.
            - `ui_theme` / `ui_layout`: restyle, or switch to a sidebar/dashboard.
            - `ui_notify`: progress toasts. `ui_state`: a shared state bag the
              app's own code can subscribe to via `br.ui.onState(...)`.
            - `ui_ask`: render a form and BLOCK until the user submits; the tool
              result is their answers, so the agent branches on them mid-turn.
            - `ui_describe`: list the regions/ids/panels the page actually has.

            To let the agent fill parts of YOUR markup, mark them:
              <section data-br-region="results"></section>
            and it can target `@region:results`. Panels need no region: the SDK
            always provides a dock. From the app side, `br.ui.onCommand(fn)` and
            `br.ui.onState(fn)` observe what the agent does.

            Design apps around this. A good agentic app's system_prompt says WHEN
            to reach for a `ui_*` tool ("after ranking the genes, call ui_chart
            with the top 10; highlight @region:cohort while you explain it"),
            rather than describing results in prose. Prefer `ui_ask` over asking
            a question in text and waiting for the next message. Set
            `capabilities.ui.enabled = false` only for deliberately text-only apps.

            RUN GUARD. Do NOT fire an agent turn (`br.run`/`br.prompt`) until the
            user has supplied the minimum input the task needs. Never call the
            agent on page boot with an empty form, and guard every control handler
            (`if (selected.length < 2) return;`, `if (!query.trim()) return;`).
            An agent turn on an empty/underspecified state wastes a round trip,
            renders a confusing "nothing selected" result, and (because turns in
            one app session run one at a time) a stuck empty turn blocks the real
            turn queued behind it. Handle empty/partial states locally in the page
            (a "pick 2 models to compare" placeholder); only prompt the agent once
            there is real work. Also pass the user's current selection/state
            explicitly in the run prompt, and do not make the agent call `ui_describe`
            to discover what the user chose (`ui_describe` is for verifying your
            own render, not for reading user input).

            ITERATIVE LOOPS. For an app that works through a list one item at a
            time (triage a ranked issue list, quiz through sub-skills, resolve
            findings), the agent's system_prompt MUST track progress in `ui_state`
            and never re-offer a finished item. Each turn: (1) pick the next
            UNRESOLVED item (persist a `resolved`/`done` id set in `ui_state` and
            skip anything already in it); (2) act on it; (3) append a numbered
            step to the visible log (`ui_render` a "Step N: item, choice, score
            before→after" line) so the user sees the state advance; (4) define and
            check a clear termination condition (e.g. "stop when no unresolved item
            is above threshold") and render a final summary when done. Without an
            explicit resolved-set the model re-asks the same top item forever and
            the loop never advances, so spell this out in the prompt.

            BUILD HARNESS / guardrails: `build_app` runs a validation harness on
            whatever you generate and reports findings; `build_app` with
            `check_only: true` runs the harness alone, without rebundling. It
            enforces five guardrails; fix any ERRORs before `launch_app`/`export_app`:
            1. SDK wiring: agentic apps import from "./sdk" in `src/main.ts`.
               Call `br.run`/`br.prompt`/`br.ask` or enable autoChat when the task
               needs agent work. Intentional local-only controls with
               `autoChat: false` may omit agent calls.
            2. Self-contained: no external `<script>`/`<link>`/CDN in index.html
               and no non-local imports in `src/main.ts` (so exports run offline).
            3. On-theme and user-directed: uses `br-*` classes/CSS variables for
               portability, while following the user's specified design.
            4. Observable: actual long-running agent work must expose a visible
               progress surface (`br.run`, `[data-br-chat]`, `br-run-status`, or
               `mountTimeline`). Wire the surface to real run events so users can
               debug step-by-step execution. Never add empty or dummy progress
               elements just to satisfy lint. Local-only controls need no
               agent-progress surface.
            5. Surface integrity (SDK v2, fail-closed): every `actions.register` /
               `components.register` name must be declared in `manifest.surface`
               and vice-versa; emitted signal names must be declared;
               `data-br-bind*` bindings want a `state_schema`; component props may
               not flow into innerHTML. The seeded starters already satisfy this, so
               keep declarations and registrations in lockstep when you extend them.
            Always `build_app` after editing `src/`, address the harness findings,
            and verify via `launch_app` before `export_app`.
        "#};
        Self {
            tool_router: Self::tool_router(),
            instructions,
            root,
        }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    fn store(&self) -> ArtifactStore {
        ArtifactStore::new(self.root.clone())
    }

    /// The app's live preview card as a user-audience `ui://` resource.
    ///
    /// Every tool that leaves an app in a new visible state returns this, so the
    /// GUI's artifact panel shows the app itself rather than a line of text
    /// describing it.
    fn card_content(&self, manifest: &Manifest) -> Result<Content, ErrorData> {
        let store = self.store();
        let entry_html = store
            .read_file(&manifest.id, &manifest.entry)
            .map_err(|e| err(ErrorCode::INTERNAL_ERROR, format!("read entry: {e}")))?;
        let html = render::assemble_card(manifest, &entry_html);
        let blob = STANDARD.encode(html.as_bytes());

        let width_css = manifest
            .width
            .map(|w| format!("{w}px"))
            .unwrap_or_else(|| "100%".to_string());
        let height_css = manifest
            .height
            .map(|h| format!("{h}px"))
            .unwrap_or_else(|| "420px".to_string());
        let mut meta_obj = serde_json::Map::new();
        meta_obj.insert(
            "mcpui.dev/ui-preferred-frame-size".to_string(),
            serde_json::json!([width_css, height_css]),
        );

        let resource = ResourceContents::BlobResourceContents {
            uri: format!("ui://agent-drafter/{}", manifest.id),
            mime_type: Some("text/html".to_string()),
            blob,
            meta: Some(rmcp::model::Meta(meta_obj)),
        };
        Ok(Content::resource(resource).with_audience(vec![Role::User]))
    }

    /// Build a card-preview result (ui:// blob for the user + assistant note).
    fn card_result(&self, manifest: &Manifest, note: &str) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::success(vec![
            self.card_content(manifest)?,
            Content::text(note.to_string()).with_audience(vec![Role::Assistant]),
        ]))
    }

    #[tool(
        name = "create_app",
        description = "Create a new Biorouter app: a TypeScript UI wired to a live Biorouter agent (kind 'agentic', default) or a static page. Returns a preview card."
    )]
    pub async fn create_app(
        &self,
        params: Parameters<CreateAppParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // Record the chat session this app was built in, so the GUI can reopen
        // that conversation to keep iterating. (Absent in headless/CLI calls
        // that don't carry session meta.)
        self.create_app_inner(
            params.0,
            Self::session_id_from_context(&context),
            &Self::caller(&context),
        )
        .await
    }

    async fn create_app_inner(
        &self,
        p: CreateAppParams,
        session_id: Option<String>,
        // Issue #56 (CP5). Threaded beside `session_id` rather than defaulted, so
        // the caller's identity is legible at each call site.
        caller: &KbCaller,
    ) -> Result<CallToolResult, ErrorData> {
        if p.title.trim().is_empty() {
            return Err(err(ErrorCode::INVALID_PARAMS, "title must not be empty"));
        }
        let kind = create_app_kind(&p)?;
        let archetype = create_app_archetype(&p)?;
        let (use_starter, files) = create_app_files(&p, kind, archetype);
        let store = self.store();
        let mut manifest = match p.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(explicit) => store.create_with_id(
                explicit,
                &p.title,
                &p.description,
                kind,
                "index.html",
                &files,
            ),
            None => store.create(&p.title, &p.description, kind, "index.html", &files),
        }
        .map_err(internal)?;
        manifest.session_id = session_id.clone();

        persist_created_app(
            &store,
            &mut manifest,
            p,
            kind,
            archetype,
            use_starter,
            caller,
        )?;

        let arch_note = if kind == ArtifactKind::Agentic {
            format!(" [{} archetype]", archetype.as_str())
        } else {
            String::new()
        };
        self.card_result(
            &manifest,
            &format!(
                "Created {kind:?} app '{}' (id: {}){arch_note}. Author src/main.ts and index.html, then build_app + launch_app.",
                manifest.title, manifest.id
            ),
        )
    }

    #[tool(
        name = "configure_app",
        description = "Set an app's agent config: system prompt/persona, greeting, model (provider+model), extensions, skills, knowledge base, and max_turns (bound/raise the tool-calling loop for workflow-style apps). Makes the app agentic if it wasn't."
    )]
    pub async fn configure_app(
        &self,
        params: Parameters<ConfigureAppParams>,
        // Issue #56 (CP5). `validate::check_all` below renders the catalog's kb
        // ids into the rejection this tool hands back to the model, so a
        // deliberately-invalid call was an enumeration oracle.
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.configure_app_inner(params.0, &Self::caller(&context))
            .await
    }

    async fn configure_app_inner(
        &self,
        p: ConfigureAppParams,
        caller: &KbCaller,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;
        manifest.kind = ArtifactKind::Agentic;
        let mut agent = manifest.agent.take().unwrap_or_default();
        if let Some(sp) = p.system_prompt {
            agent.system_prompt = sp;
        }
        if let Some(g) = p.greeting {
            agent.greeting = Some(g).filter(|s| !s.is_empty());
        }
        if let Some(m) = p.model {
            // Empty provider+model clears the pin → inherit the user's configured
            // provider/model. Any Biorouter-supported provider may be pinned.
            agent.model = Some(ModelSelection::from(m)).filter(|s| s.is_set());
        }
        if let Some(ext) = p.extensions {
            agent.extensions = ext;
        }
        if let Some(sk) = p.skills {
            agent.skills = sk;
        }
        if let Some(kb) = p.knowledge_base {
            agent.knowledge_base = Some(kb).filter(|s| !s.trim().is_empty());
        }
        if let Some(req) = p.requires {
            agent.requires = req;
        }

        // An id that does not exist here cannot be saved. The catalog is what
        // makes this checkable; the error names what IS installed so the retry is
        // grounded rather than another guess — and, since issue #56, what is
        // installed *and reachable by this caller*.
        let catalog = catalog::Catalog::discover(caller);
        validate::check_all(
            agent.knowledge_base.as_deref(),
            &agent.skills,
            &agent.extensions,
            &catalog,
        )
        .map_err(|e| err(ErrorCode::INVALID_PARAMS, e))?;
        if let Some(mt) = p.max_turns {
            agent.max_turns = Some(mt).filter(|&n| n > 0);
        }
        if let Some(v) = p.capabilities {
            agent.capabilities = decode_agent_field::<Capabilities>(v, "capabilities")?;
        }
        if let Some(v) = p.guardrails {
            agent.guardrails = Some(decode_agent_field::<GuardrailsConfig>(v, "guardrails")?);
        }
        if let Some(v) = p.reliability {
            agent.reliability = Some(decode_agent_field::<ReliabilityConfig>(v, "reliability")?);
        }
        if let Some(v) = p.orchestration {
            agent.orchestration = decode_agent_field::<Orchestration>(v, "orchestration")?;
        }
        if let Some(v) = p.output_type {
            agent.output_type = Some(v);
        }
        if let Some(durable) = p.durable_session {
            agent.durable_session = Some(durable);
        }
        manifest.agent = Some(agent);
        store.save_manifest(&manifest).map_err(internal)?;
        store.touch(&p.id).map_err(internal)?;
        self.card_result(&manifest, &format!("Configured app '{}'.", p.id))
    }

    #[tool(
        name = "set_app_size",
        description = "Set an app's preferred preview-card size in CSS px (width/height). Omit a value to fill/auto."
    )]
    pub async fn set_app_size(
        &self,
        params: Parameters<SetAppSizeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;
        manifest.width = p.width;
        manifest.height = p.height;
        store.save_manifest(&manifest).map_err(internal)?;
        store.touch(&p.id).map_err(internal)?;
        self.card_result(&manifest, &format!("Set preview size for '{}'.", p.id))
    }

    #[tool(
        name = "update_app",
        description = "Edit a file in an app: provide full `content` to overwrite, or `old_str`+`new_str` to replace a snippet. Defaults to index.html. Editing src/ marks the bundle stale (re-run build_app)."
    )]
    pub async fn update_app(
        &self,
        params: Parameters<UpdateAppParams>,
        // Issue #56 (CP5). The `manifest.json` path re-runs the same write-boundary
        // check as `configure_app`, and renders the same list.
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // ⚠ **The largest duplicate surface in this capability, closed at the
        // tool boundary.** A whole-manifest write through here can set
        // `surface`, `theme`, `agent.orchestration.agents`, `agent.orchestration
        // .routes` and `width`/`height` in one call — structurally duplicating
        // `declare_app` and `configure_app`, the typed writers that were added
        // to REPLACE it. `declare_surface`'s own description already said "do
        // NOT rewrite manifest.json"; nothing enforced it.
        //
        // Refused HERE rather than in `update_app_inner` on purpose. The inner
        // branch does real work — it restores the server-owned fields and
        // re-runs `validate::check_all` — and a test drives it directly to hold
        // that logic. Refusing at the tool keeps that coverage while removing
        // the door the MODEL can walk through.
        if params.0.path.as_deref().map(str::trim) == Some("manifest.json") {
            return Err(err(
                ErrorCode::INVALID_PARAMS,
                "Do not rewrite manifest.json. Use `declare_app` for surface, theme, agents, \
                 routes and size, and `configure_app` for the agent's model, prompt, extensions, \
                 skills and capabilities. Both validate what they write; a raw manifest rewrite \
                 can silently drop a field."
                    .to_string(),
            ));
        }
        self.update_app_inner(params.0, &Self::caller(&context))
            .await
    }

    async fn update_app_inner(
        &self,
        p: UpdateAppParams,
        caller: &KbCaller,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;
        let path = p.path.clone().unwrap_or_else(|| manifest.entry.clone());

        let updated_content = if let Some(content) = p.content {
            content
        } else if let (Some(old), Some(new)) = (p.old_str.as_ref(), p.new_str.as_ref()) {
            let current = store
                .read_file(&p.id, &path)
                .map_err(|e| err(ErrorCode::INVALID_PARAMS, format!("read {path}: {e}")))?;
            if !current.contains(old.as_str()) {
                return Err(err(
                    ErrorCode::INVALID_PARAMS,
                    format!("old_str not found in {path}"),
                ));
            }
            current.replacen(old.as_str(), new.as_str(), 1)
        } else {
            return Err(err(
                ErrorCode::INVALID_PARAMS,
                "provide either `content` or both `old_str` and `new_str`",
            ));
        };

        if path == "manifest.json" {
            let mut parsed: Manifest = serde_json::from_str(&updated_content).map_err(|e| {
                err(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "manifest.json must be valid Agent Drafter manifest JSON: {e}. \
                         Read it back with `read_app` (the default resolved view shows every \
                         field, including ones holding their default) and edit that."
                    ),
                )
            })?;
            if parsed.id != p.id {
                return Err(err(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "manifest.json id '{}' must match the app id '{}'",
                        parsed.id, p.id
                    ),
                ));
            }

            // MERGE, don't replace. The raw bytes used to be written verbatim, so
            // any server-owned field the caller omitted was silently destroyed:
            // `built_at` and `sdk_hash` (the app then looked unbuilt and its
            // vendored SDK unfingerprinted), `session_id` (the GUI lost the
            // originating conversation), and a truthful `created_at`. A model
            // composing a manifest has no way to know these values and no business
            // inventing them — so we restore them from disk regardless of what it
            // wrote.
            let on_disk = store
                .load_manifest(&p.id)
                .map_err(|e| err(ErrorCode::INVALID_PARAMS, format!("no app '{}': {e}", p.id)))?;
            parsed.id = on_disk.id.clone();
            parsed.created_at = on_disk.created_at;
            parsed.built_at = on_disk.built_at;
            parsed.sdk_hash = on_disk.sdk_hash.clone();
            parsed.session_id = on_disk.session_id.clone();

            // Same write-boundary rule as create/configure: a manifest cannot name
            // a knowledge base, skill, or extension that does not exist here.
            if let Some(agent) = parsed.agent.as_ref() {
                let catalog = catalog::Catalog::discover(caller);
                validate::check_all(
                    agent.knowledge_base.as_deref(),
                    &agent.skills,
                    &agent.extensions,
                    &catalog,
                )
                .map_err(|e| err(ErrorCode::INVALID_PARAMS, e))?;
            }

            store.save_manifest(&parsed).map_err(internal)?;
            store.touch(&p.id).map_err(internal)?;
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "updated {}/manifest.json",
                p.id
            ))]));
        }

        store
            .write_file(&p.id, &path, &updated_content)
            .map_err(internal)?;
        // Editing sources invalidates the build.
        if path.starts_with("src/") {
            manifest.built_at = None;
            store.save_manifest(&manifest).map_err(internal)?;
        }
        store.touch(&p.id).map_err(internal)?;

        // ⚠ There used to be an `if path == "manifest.json"` arm here. It was
        // dead: the dedicated whole-manifest branch above returns
        // unconditionally, so control never reached this point with that path.
        // Dead code sitting under a live duplicate is how a duplicate survives
        // a reading — it looks like the branch that handles the case.
        if path == manifest.entry {
            self.card_result(&manifest, &format!("Updated {path} in '{}'.", p.id))
        } else {
            let hint = if path.starts_with("src/") {
                " (run build_app to rebundle)"
            } else {
                ""
            };
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Updated {path} in '{}'.{hint}",
                p.id
            ))]))
        }
    }

    #[tool(
        name = "build_app",
        description = "Bundle the app's TypeScript (src/main.ts → dist/app.js) with esbuild. Run after editing src/. Returns the build log."
    )]
    pub async fn build_app(
        &self,
        params: Parameters<BuildAppParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        if !store.exists(&p.id) {
            return Err(err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)));
        }
        // ⚠ BEFORE `rebuild_and_stamp`, not after. `build_app` only lints once
        // `report.ok`, and an app that fails to compile is exactly the app
        // anyone wants to lint — so folding `lint_app` in behind the bundler
        // would have deleted the only case it was reached for.
        if p.check_only.unwrap_or(false) {
            return self.lint_app(Parameters(AppIdParams { id: p.id })).await;
        }
        let store2 = store.clone();
        let id2 = p.id.clone();
        let report = tokio::task::spawn_blocking(move || rebuild_and_stamp(&store2, &id2))
            .await
            .map_err(internal)?
            .map_err(internal)?;
        if report.ok {
            // `rebuild_and_stamp` already persisted built_at + sdk_hash; reload so
            // the preview card reflects them.
            let manifest = store.load_manifest(&p.id).map_err(internal)?;
            // Run the guardrail harness and surface findings so the agent can
            // self-correct (SDK-wired, self-contained, on-theme).
            let app_dir = store.artifact_dir(&p.id).map_err(internal)?;
            let lint = bundle::lint_app(&app_dir);
            // A fresh bundle is a new visible state: show the rebuilt app, don't
            // just tell the user it compiled.
            Ok(CallToolResult::success(vec![
                self.card_content(&manifest)?,
                Content::text(format!(
                    "Built '{}' with {} → dist/app.js.\n{}\n\n{}",
                    p.id,
                    report.used,
                    bundle::format_lint(&lint),
                    report.log
                ))
                .with_audience(vec![Role::Assistant]),
            ]))
        } else {
            Err(err(
                ErrorCode::INTERNAL_ERROR,
                format!("build failed for '{}':\n{}", p.id, report.log),
            ))
        }
    }

    #[tool(
        name = "lint_app",
        description = "Run the build harness guardrails on an app and report findings: does it reach the backend via the App SDK, is it self-contained (no CDN/external assets), on-theme (Biorouter classes/tokens), and (for SDK v2 apps) do its custom components match the manifest's declared surface (registered ⇔ declared, string-literal names, no prop-fed HTML sinks) and its state bindings stay safe (declared state_schema, no on*/style bind-attr)? Fix ERRORs before launch/export."
    )]
    pub async fn lint_app(
        &self,
        params: Parameters<AppIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        if !store.exists(&p.id) {
            return Err(err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)));
        }
        let app_dir = store.artifact_dir(&p.id).map_err(internal)?;
        let findings = bundle::lint_app(&app_dir);
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Harness check for '{}':\n{}",
            p.id,
            bundle::format_lint(&findings)
        ))]))
    }

    #[tool(
        name = "launch_app",
        description = "Build (if needed) and launch a Biorouter app. Returns a browser URL. The GUI shows a clickable preview and the CLI prints the URL."
    )]
    pub async fn launch_app(
        &self,
        params: Parameters<AppIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;

        // Ensure a fresh bundle exists.
        if manifest.kind == ArtifactKind::Agentic
            && (manifest.built_at.is_none() || !store.file_exists(&p.id, "dist/app.js"))
        {
            let dir = store.artifact_dir(&p.id).map_err(internal)?;
            let report = tokio::task::spawn_blocking(move || bundle::build_app(&dir))
                .await
                .map_err(internal)?
                .map_err(internal)?;
            if !report.ok {
                return Err(err(
                    ErrorCode::INTERNAL_ERROR,
                    format!("build failed before launch:\n{}", report.log),
                ));
            }
            manifest.built_at = Some(now_secs());
            store.save_manifest(&manifest).map_err(internal)?;
        }

        let app_dir = store.artifact_dir(&p.id).map_err(internal)?;
        let lint = bundle::lint_app(&app_dir);
        if lint.iter().any(|f| f.level == bundle::LintLevel::Error) {
            return Err(err(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "harness ERRORs block launch for '{}':\n{}",
                    p.id,
                    bundle::format_lint(&lint)
                ),
            ));
        }

        let path = format!("/apps/{}/", manifest.id);
        let browser_url = std::env::var("BIOROUTER_APP_BASE_URL")
            .ok()
            .and_then(|base| url::Url::parse(base.trim()).ok())
            .filter(|base| {
                matches!(base.scheme(), "http" | "https")
                    && base.host_str().is_some()
                    && base.username().is_empty()
                    && base.password().is_none()
                    && base.query().is_none()
                    && base.fragment().is_none()
            })
            .map(|base| format!("{}{}", base.as_str().trim_end_matches('/'), path))
            .unwrap_or_else(|| {
                let port = std::env::var("BIOROUTER_PORT")
                    .ok()
                    .and_then(|value| value.parse::<u16>().ok())
                    .unwrap_or(3000);
                format!("http://127.0.0.1:{port}{path}")
            });

        // Surface a launch marker clients can turn into a user-initiated preview,
        // plus a human URL line that also works in plain terminals.
        let mut meta = serde_json::Map::new();
        meta.insert(
            "biorouter/launch-app".to_string(),
            serde_json::json!(manifest.id),
        );
        meta.insert("biorouter/app-path".to_string(), serde_json::json!(path));
        let mut link = RawResource::new(&browser_url, format!("{} app", manifest.id));
        link.title = Some(format!("Open {}", manifest.title));
        link.mime_type = Some("text/html".to_string());
        let mut content = vec![Content::resource_link(link).with_audience(vec![Role::User])];
        content.push(
            Content::text(format!(
                "App '{}' is ready. Open it in your browser: {}\n(The desktop GUI shows a click-only preview link; in the CLI open the URL above with a running biorouterd.)",
                manifest.id, browser_url
            ))
            .with_audience(vec![Role::Assistant]),
        );
        let mut result = CallToolResult::success(content);
        result.meta = Some(rmcp::model::Meta(meta));
        Ok(result)
    }

    /// One call for every typed manifest region.
    ///
    /// Composes the five inherent writers rather than reimplementing them, so
    /// each region's validation, its result text and its side effects are
    /// byte-for-byte what the retired tool did — including
    /// `declare_profiles`'s catalog check (issue #56 CP5, which is why this
    /// takes a `RequestContext`) and `set_app_size`'s preview card.
    ///
    /// ⚠ **Applied region by region, not atomically** — each writer loads,
    /// assigns, saves and touches on its own. That is exactly what five
    /// separate calls did, so a mid-sequence failure leaves the same state it
    /// always would; what changes is that the caller now learns about it in one
    /// result instead of noticing the fourth call never happened. The order is
    /// fixed and the first failure stops the rest, so a partial result is a
    /// prefix, never a scatter.
    #[tool(
        name = "declare_app",
        description = "Declare an app's typed manifest regions in one call: `surface` (its contract — state schema, actions, signals, components), `theme`, `agents` (worker profiles), `routes` (named model routes) and `size`. Pass only the regions you are setting; an omitted region is left alone. `merge: true` upserts `surface` and `agents` by name/key instead of replacing them. Do NOT rewrite manifest.json to do any of this."
    )]
    pub async fn declare_app(
        &self,
        params: Parameters<DeclareAppParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let mut applied: Vec<String> = Vec::new();
        // `size` is applied LAST so its preview card reflects every other region
        // set in the same call.
        if let Some(surface) = p.surface {
            self.declare_surface(Parameters(DeclareSurfaceParams {
                id: p.id.clone(),
                surface,
                merge: p.merge,
            }))
            .await?;
            applied.push("surface".to_string());
        }
        if let Some(theme) = p.theme {
            self.set_theme(Parameters(SetThemeParams {
                id: p.id.clone(),
                theme,
            }))
            .await?;
            applied.push("theme".to_string());
        }
        if let Some(agents) = p.agents {
            self.declare_profiles(
                Parameters(DeclareProfilesParams {
                    id: p.id.clone(),
                    agents,
                    merge: p.merge,
                }),
                context,
            )
            .await?;
            applied.push("agents".to_string());
        }
        if let Some(routes) = p.routes {
            self.set_routes(Parameters(SetRoutesParams {
                id: p.id.clone(),
                routes,
            }))
            .await?;
            applied.push("routes".to_string());
        }
        if let Some(size) = p.size {
            applied.push("size".to_string());
            // Returns the preview card, which is the most useful thing to hand
            // back when a size was set.
            return self
                .set_app_size(Parameters(SetAppSizeParams {
                    id: p.id,
                    width: size.width,
                    height: size.height,
                }))
                .await;
        }
        if applied.is_empty() {
            return Err(err(
                ErrorCode::INVALID_PARAMS,
                "declare_app needs at least one region: surface, theme, agents, routes or size."
                    .to_string(),
            ));
        }
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} now declares: {}",
            p.id,
            applied.join(", ")
        ))]))
    }

    #[tool(
        name = "declare_surface",
        description = "Declare (or update) an app's CONTRACT: its state schema, the actions the \
                       AGENT may call on the app, the signals the APP sends the agent, and any \
                       custom components. This is the typed way to do it, so do NOT rewrite \
                       manifest.json. Every action you declare must be registered in src/main.ts \
                       with `br.actions.register(...)`, and vice versa; lint enforces both \
                       directions. Pass merge=true to upsert by name, false (default) to replace."
    )]
    pub async fn declare_surface(
        &self,
        params: Parameters<DeclareSurfaceParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;

        let incoming = p.surface.into_decl();
        if p.merge.unwrap_or(false) {
            upsert_by_name(&mut manifest.surface.actions, incoming.actions, |a| {
                a.name.clone()
            });
            upsert_by_name(&mut manifest.surface.signals, incoming.signals, |s| {
                s.name.clone()
            });
            upsert_by_name(&mut manifest.surface.components, incoming.components, |c| {
                c.name.clone()
            });
            if incoming.state_schema.is_some() {
                manifest.surface.state_schema = incoming.state_schema;
            }
            // `state_initial` was missing from this merge, so `declare_surface(merge:
            // true)` SILENTLY DROPPED it — the caller declared an initial document,
            // got a success result, and the manifest was unchanged. Found by pointing
            // the fixed platform's own agent at a broken app: it cost 8 extra
            // round-trips of "declare it again, it still isn't there".
            //
            // Exactly the class of bug this campaign is about — a tool that reports
            // success while doing nothing.
            if incoming.state_initial.is_some() {
                manifest.surface.state_initial = incoming.state_initial;
            }
        } else {
            manifest.surface = incoming;
        }

        store.save_manifest(&manifest).map_err(internal)?;
        store.touch(&p.id).map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "declared surface for {}: {} action(s), {} signal(s), {} component(s)",
            p.id,
            manifest.surface.actions.len(),
            manifest.surface.signals.len(),
            manifest.surface.components.len(),
        ))]))
    }

    #[tool(
        name = "set_theme",
        description = "Set an app's theme pack (biorouter | clinical | lab-notebook | terminal | \
                       journal | midnight), with an optional accent colour and `--br-*` token \
                       overrides. The pack is an enum, so an unknown name is rejected rather than \
                       silently falling back to the default."
    )]
    pub async fn set_theme(
        &self,
        params: Parameters<SetThemeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;

        manifest.theme = p.theme.into_config();
        let pack = manifest.resolved_theme_pack().to_string();
        store.save_manifest(&manifest).map_err(internal)?;
        store.touch(&p.id).map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} now renders with the '{pack}' theme pack",
            p.id
        ))]))
    }

    #[tool(
        name = "declare_profiles",
        description = "Declare an app's WORKER AGENT PROFILES for multi-agent work (adversarial                        panels, collaborative pipelines). Each profile is a full alternate agent,                        with its own model, system prompt, extensions and skills, that the main agent                        reaches with `consult(agent: \"<key>\")`. Keys must be stable identifiers                        (lowercase/digits/underscore); a display name like \"Prosecutor\" is                        rejected because `consult` resolves keys exactly. Workers do NOT get the                        app's UI tools unless you explicitly grant them: the main agent owns the                        page."
    )]
    pub async fn declare_profiles(
        &self,
        params: Parameters<DeclareProfilesParams>,
        // Issue #56 (CP5). A worker profile validates against a catalog too; it
        // renders skill ids today, and the capability is read here so that stays
        // true the day a profile gains a knowledge base of its own.
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;

        let catalog = catalog::Catalog::discover(&Self::caller(&context));
        let mut profiles: std::collections::HashMap<String, store::AgentConfig> =
            std::collections::HashMap::new();

        for prof in p.agents {
            declare::validate_profile_key(&prof.key)
                .map_err(|e| err(ErrorCode::INVALID_PARAMS, e))?;
            validate::check_all(None, &prof.skills, &prof.extensions, &catalog).map_err(|e| {
                err(
                    ErrorCode::INVALID_PARAMS,
                    format!("profile '{}': {e}", prof.key),
                )
            })?;

            let cfg = store::AgentConfig {
                system_prompt: prof.system_prompt,
                greeting: prof.description,
                model: prof.model.map(ModelSelection::from).filter(|m| m.is_set()),
                extensions: prof.extensions,
                skills: prof.skills,
                max_turns: prof.max_turns,
                ..Default::default()
            };
            profiles.insert(prof.key, cfg);
        }

        let agent = manifest.agent.get_or_insert_with(Default::default);
        if p.merge.unwrap_or(false) {
            agent.orchestration.agents.extend(profiles);
        } else {
            agent.orchestration.agents = profiles;
        }
        let mut keys: Vec<String> = agent.orchestration.agents.keys().cloned().collect();
        keys.sort();

        store.save_manifest(&manifest).map_err(internal)?;
        store.touch(&p.id).map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} declares {} worker profile(s): {}. The main agent reaches them with \
             consult(agent=\"<key>\"); use these exact keys.",
            p.id,
            keys.len(),
            keys.join(", ")
        ))]))
    }

    #[tool(
        name = "set_routes",
        description = "Declare named model routes (e.g. `fast`, `deep`) an app's `call` may \
                       select per invocation."
    )]
    pub async fn set_routes(
        &self,
        params: Parameters<SetRoutesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        let mut manifest = store
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;

        let agent = manifest.agent.get_or_insert_with(Default::default);
        agent.orchestration.routes = declare::routes_from_params(p.routes);
        let n = agent.orchestration.routes.len();
        store.save_manifest(&manifest).map_err(internal)?;
        store.touch(&p.id).map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "{} now declares {n} model route(s)",
            p.id
        ))]))
    }

    #[tool(
        name = "smoke_app",
        description = "EXECUTE the app in a real browser against a mock daemon and report what                        actually happens. This is the check that catches what lint cannot: a                        control that fires and delivers NO turn (the handler completes, the                        console is clean, and nothing reaches the agent), a bound KPI that renders                        blank before any turn, a slider no arrow key can move, a drag surface only                        a human mouse can drive, and progress that displaces the result. Run it                        after build_app. A finding here is a real defect a user would hit."
    )]
    pub async fn smoke_app(
        &self,
        params: Parameters<AppIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store();
        let dir = store.artifact_dir(&params.0.id).map_err(|_| {
            err(
                ErrorCode::INVALID_PARAMS,
                format!("no app '{}'", params.0.id),
            )
        })?;
        if !dir.join("index.html").exists() {
            return Err(err(
                ErrorCode::INVALID_PARAMS,
                format!("no app '{}'", params.0.id),
            ));
        }
        let text = match run_smoke(&dir) {
            Ok(text) => text,
            Err(e) => format!(
                "smoke check could not run: {e}\n\nThis is NOT a pass: the app was never \
                 executed. Install a browser (`npx playwright install chromium`) or set \
                 BIOROUTER_APP_SMOKE=off to skip deliberately."
            ),
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "list_platform_catalog",
        description = "List what this Biorouter install ACTUALLY has: installed knowledge bases, \
                       installed skills, and available extensions. Call this BEFORE configure_app \
                       whenever an app needs a knowledge base, a skill, or an extension. Only ids \
                       returned here may be configured. An id that does not exist is rejected, \
                       because configuring one arms tools scoped to nothing and fails the app's \
                       first turn. If the app needs something this install does not have, leave \
                       the id unset and record it in `requires` instead; wanting an absent \
                       capability is legal and is reported honestly to the user."
    )]
    pub async fn list_platform_catalog(
        &self,
        // Issue #56 (CP5). This tool serialised `Catalog::discover()` whole, so
        // it handed the model `{id, name}` for every knowledge base on the
        // machine with no arguments at all — and its own description tells the
        // model to call it before `configure_app`, so it ran on every
        // app-building turn.
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let catalog = catalog::Catalog::discover(&Self::caller(&context));
        let json = serde_json::to_string_pretty(&catalog).map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        name = "list_apps",
        description = "List all Biorouter apps (optionally filtered by kind)."
    )]
    pub async fn list_apps(
        &self,
        params: Parameters<ListAppsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let filter = match params.0.kind.as_deref() {
            Some(k) => Some(ArtifactKind::parse(k).ok_or_else(|| {
                err(
                    ErrorCode::INVALID_PARAMS,
                    "kind must be 'static' or 'agentic'",
                )
            })?),
            None => None,
        };
        let list: Vec<Manifest> = self
            .store()
            .list()
            .into_iter()
            .filter(|m| filter.map(|k| k == m.kind).unwrap_or(true))
            .collect();
        if list.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No apps yet.".to_string(),
            )]));
        }
        let lines: Vec<String> = list
            .iter()
            .map(|m| {
                let model = m
                    .agent
                    .as_ref()
                    .and_then(|a| a.model.as_ref())
                    .and_then(|s| s.model.clone())
                    .unwrap_or_else(|| "default".into());
                format!("- {} [{:?}, model: {}]: {}", m.id, m.kind, model, m.title)
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        name = "read_app",
        description = "Read an app's manifest, or a specific file within it. The manifest is \
                       returned as a RESOLVED view by default: every optional block is present \
                       and the theme pack is resolved, so a field holding its default value is \
                       visible rather than absent. Edit that skeleton. Pass view=\"raw\" for the \
                       exact on-disk bytes."
    )]
    pub async fn read_app(
        &self,
        params: Parameters<ReadAppParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        match p.path {
            None => {
                let view = resolved::ManifestView::parse(p.view.as_deref())
                    .map_err(|e| err(ErrorCode::INVALID_PARAMS, e))?;
                let m = store
                    .load_manifest(&p.id)
                    .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;
                let value = match view {
                    resolved::ManifestView::Resolved => resolved::resolved_view(&m),
                    resolved::ManifestView::Raw => serde_json::to_value(&m).map_err(internal)?,
                    // Absorbed from the retired `preview_app`. It returns the
                    // ui:// card for the USER rather than JSON for the model —
                    // which is not new for this capability: create_app,
                    // configure_app, update_app, set_app_size and build_app all
                    // already return that card beside their text. `preview_app`
                    // was simply the one whose ONLY job was the card.
                    resolved::ManifestView::Card => {
                        return self.card_result(&m, &format!("Preview of '{}'.", p.id))
                    }
                };
                let json = serde_json::to_string_pretty(&value).map_err(internal)?;
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Some(path) => {
                let content = store
                    .read_file(&p.id, &path)
                    .map_err(|e| err(ErrorCode::INVALID_PARAMS, format!("read {path}: {e}")))?;
                Ok(CallToolResult::success(vec![Content::text(content)]))
            }
        }
    }

    #[tool(
        name = "preview_app",
        description = "Render an app's preview card for the user."
    )]
    pub async fn preview_app(
        &self,
        params: Parameters<AppIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let manifest = self
            .store()
            .load_manifest(&p.id)
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;
        self.card_result(&manifest, &format!("Preview of '{}'.", p.id))
    }

    #[tool(
        name = "export_app",
        description = "Export an app as a standalone folder the user can just run: double-click \
                       `run.command` (macOS) or `bash run.sh`. The launcher installs the app into \
                       the local Biorouter store, starts or reuses a `biorouterd`, and opens it, with \
                       no npm install and no build step (`dist/app.js` ships prebuilt). \
                       `npm start` additionally serves the folder and proxies the agent, for \
                       editing `src/`."
    )]
    pub async fn export_app(
        &self,
        params: Parameters<ExportAppParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // Issue #56 (CP4). Read through the SHARED reader that CP1 uses, so the
        // two sides cannot spell the meta key differently. Task 10D gave this
        // file its own one-line `Self::caller_is_private` delegate for CP5's four
        // tools; this call went through `knowledge::tier` directly, which left
        // two spellings of the same read in one file — routed through the
        // delegate so there is exactly one.
        //
        // Split into an `_inner` exactly like `create_app`/`create_app_inner`
        // above: the eight unit tests below drive the body without fabricating a
        // `RequestContext`, and the capability still enters at the one seam.
        self.export_app_inner(params.0, &Self::caller(&context))
            .await
    }

    async fn export_app_inner(
        &self,
        p: ExportAppParams,
        caller: &KbCaller,
    ) -> Result<CallToolResult, ErrorData> {
        let store = self.store();
        if !store.exists(&p.id) {
            return Err(err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)));
        }
        let app_dir = store.artifact_dir(&p.id).map_err(internal)?;
        let lint = bundle::lint_app(&app_dir);
        if lint.iter().any(|f| f.level == bundle::LintLevel::Error) {
            return Err(err(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "harness ERRORs block export for '{}':\n{}",
                    p.id,
                    bundle::format_lint(&lint)
                ),
            ));
        }
        let scaffold = export_scaffold(self.root(), &p.id, p.endpoint.as_deref())
            .map_err(|_| err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)))?;

        let target = PathBuf::from(&p.target_dir);
        for (rel, content) in &scaffold {
            let full = target.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).map_err(internal)?;
            }
            std::fs::write(&full, content).map_err(internal)?;
            // The whole point of `run.command` is that a user can double-click
            // it. Written without the exec bit, it opens in a text editor.
            if render::EXECUTABLE_EXPORT_FILES.contains(&rel.as_str()) {
                make_executable(&full);
            }
        }

        // ── Standalone export v2 (design §3.9): mode + payload + fat daemon ──
        // Launcher mode = today's scaffold exactly (above). Full mode also
        // stages the app's server-side payload and writes an audit manifest.
        let mode = match p.mode.as_deref() {
            Some("full") => "full",
            _ => "launcher", // default; unknown values degrade to launcher
        };
        let mut notes: Vec<String> = Vec::new();
        // Re-load the manifest to resolve the payload selection from the agent
        // config; export_scaffold already validated the app exists.
        let manifest = store.load_manifest(&p.id).map_err(internal)?;

        let staged = if mode == "full" {
            stage_full_payload(&manifest, &target, p.include.as_ref(), caller)
        } else {
            StagedPayload::empty()
        };
        notes.extend(staged.notes.iter().cloned());

        // Fat export: bundle the current-platform daemon (both modes may opt in).
        let bundle_mode = p.bundle_daemon.as_deref().unwrap_or("none");
        let daemon_record = match bundle_mode {
            "none" => None,
            other => {
                if other == "all" {
                    notes.push(
                        "bundle_daemon=\"all\" (universal) is out of scope in this build; \
                         staging the current platform's daemon instead"
                            .to_string(),
                    );
                } else if other != "current" {
                    notes.push(format!(
                        "unknown bundle_daemon=\"{other}\"; treating it as \"current\""
                    ));
                }
                let (rec, note) = stage_current_daemon(&target);
                notes.push(note);
                rec
            }
        };

        // Write export.json whenever the export carries a payload: full mode, or
        // a bundled daemon in launcher mode (a "fat launcher").
        let wrote_manifest = if mode == "full" || daemon_record.is_some() {
            let ejson = build_export_json(&p.id, mode, &staged, daemon_record.as_ref());
            let ejpath = target.join("export.json");
            std::fs::write(
                &ejpath,
                serde_json::to_string_pretty(&ejson).unwrap_or_default(),
            )
            .map_err(internal)?;
            true
        } else {
            false
        };

        Ok(CallToolResult::success(vec![Content::text(
            export_summary(
                &p.id,
                mode,
                &target,
                scaffold.len(),
                &staged,
                wrote_manifest,
                &notes,
            ),
        )]))
    }

    #[tool(
        name = "delete_app",
        description = "Delete an app and all of its files."
    )]
    pub async fn delete_app(
        &self,
        params: Parameters<AppIdParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let store = self.store();
        if !store.exists(&p.id) {
            return Err(err(ErrorCode::INVALID_PARAMS, format!("no app '{}'", p.id)));
        }
        store.delete(&p.id).map_err(internal)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Deleted app '{}'.",
            p.id
        ))]))
    }
}

/// Registered so the name keeps dispatching, but never advertised.
///
/// Each of these is now a REGION or a MODE of a tool that already existed:
/// the five typed writers are `declare_app`'s optional fields, `lint_app` is
/// `build_app { check_only: true }`, and `preview_app` is
/// `read_app { view: "card" }`. Dropping the route as well as the declaration
/// would turn a stored `always allow` grant, a persisted transcript's retry and
/// `biorouter-self-test.yaml`'s named steps into unknown-tool errors none of
/// them can act on — so the route stays and only the advertisement goes.
const RETIRED_TOOL_NAMES: &[&str] = &[
    "declare_surface",
    "set_theme",
    "declare_profiles",
    "set_routes",
    "set_app_size",
    "lint_app",
    "preview_app",
];

impl ServerHandler for AgentDrafterServer {
    /// `#[tool_handler]`'s generated body, plus the retired-name filter. See
    /// [`RETIRED_TOOL_NAMES`]; `crates/biorouter-mcp/src/knowledge/server.rs`
    /// does the same thing for the same reason.
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, ErrorData> {
        Ok(rmcp::model::ListToolsResult {
            tools: self
                .tool_router
                .list_all()
                .into_iter()
                .filter(|tool| !RETIRED_TOOL_NAMES.contains(&tool.name.as_ref()))
                .collect(),
            meta: None,
            next_cursor: None,
        })
    }

    /// Verbatim what `#[tool_handler]` generated; re-check when bumping rmcp.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "biorouter-agent-drafter".to_string(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: Some("Agent Drafter".to_string()),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(self.instructions.clone()),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::RawContent;
    use tempfile::TempDir;

    /// A caller the KB barrier clears on both axes: private tier, LOCAL model.
    ///
    /// ⚠ `Local` and not `Unstated`. `Unstated` is DR-26's restrictive value —
    /// it mismatches every base an institution claims — so a test that meant
    /// "a private caller may reach a private base" and reached for it would
    /// assert the opposite of what it says. `Local` transfers nothing, so it
    /// clears every base, which is the pre-DR-26 meaning of `caller_is_private
    /// = true` these fixtures were written against.
    fn private_test_caller() -> KbCaller {
        KbCaller::new(
            true,
            crate::knowledge::affiliation::CallerAffiliation::Local,
        )
    }

    #[test]
    fn agent_drafter_children_strip_daemon_credentials_only() {
        let mut command = std::process::Command::new("node");
        command
            .env("BIOROUTER_SERVER__SECRET_KEY", "daemon-private")
            .env("BIOROUTER_PORT", "4931")
            .env("SPOKEAGENT_PASSCODE", "extension-private");
        prepare_agent_drafter_child(&mut command);

        let envs = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        assert!(envs.contains(&("BIOROUTER_SERVER__SECRET_KEY".to_string(), None)));
        assert!(envs.contains(&("BIOROUTER_PORT".to_string(), Some("4931".to_string()))));
        assert!(envs.contains(&(
            "SPOKEAGENT_PASSCODE".to_string(),
            Some("extension-private".to_string())
        )));
    }

    /// Opt this test out of catalog strictness.
    ///
    /// Six tests here configure knowledge bases / skills that deliberately do not
    /// exist — `export_full_mode_skips_missing_kb` *is* the missing-KB path, and
    /// the config round-trip tests only care that the fields survive a save/load.
    /// Under the write-boundary rule (Wave 1) an id that is not installed cannot be
    /// saved, which is correct for real authoring and wrong for these fixtures.
    ///
    /// The returned guard must be BOUND for the test's duration
    /// (`let _strict = relax_catalog_strictness();`).
    ///
    /// This was a bare `set_var` that was never restored, which was safe only
    /// while no test in this binary asserted that a rejection *happens*. Issue
    /// #56's CP5 tests do exactly that — they read the kb list a rejection
    /// renders — and three of this helper's callers are not `#[serial]`, so a
    /// leaked `0` from any of them would silently make those tests vacuous.
    /// `env_lock` takes the same process-wide lock the CP5 tests take, so the two
    /// families cannot interleave, and restores the variable even from a
    /// panicking test.
    fn relax_catalog_strictness() -> env_lock::EnvGuard<'static> {
        env_lock::lock_env([("BIOROUTER_APPS_CATALOG_STRICT", Some("0".to_string()))])
    }

    fn server() -> (TempDir, AgentDrafterServer) {
        let dir = TempDir::new().unwrap();
        let s = AgentDrafterServer::with_root(dir.path().to_path_buf());
        (dir, s)
    }

    #[test]
    fn instructions_distinguish_local_controls_from_observable_agent_work() {
        let (_dir, server) = server();
        let instructions = server
            .get_info()
            .instructions
            .expect("Agent Drafter must advertise authoring instructions");
        let normalized = instructions
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let (_, harness) = normalized
            .split_once("BUILD HARNESS / guardrails:")
            .expect("authoring instructions must include the harness contract");
        for requirement in [
            "enforces five guardrails",
            "SDK wiring: agentic apps import from \"./sdk\" in `src/main.ts`",
            "Intentional local-only controls with `autoChat: false` may omit agent calls",
            "actual long-running agent work must expose a visible progress surface",
            "Wire the surface to real run events",
            "Never add empty or dummy progress elements just to satisfy lint",
            "Local-only controls need no agent-progress surface",
        ] {
            assert!(harness.contains(requirement), "missing: {requirement}");
        }
        assert!(!harness.contains("enforces three things"));
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn has_ui_resource(result: &CallToolResult) -> bool {
        result
            .content
            .iter()
            .any(|c| matches!(&c.raw, RawContent::Resource(_)))
    }

    fn create(title: &str, kind: Option<&str>) -> CreateAppParams {
        CreateAppParams {
            title: title.into(),
            id: None,
            description: String::new(),
            surface: None,
            theme: None,
            archetype: None,
            kind: kind.map(|k| k.to_string()),
            html: None,
            files: vec![],
            system_prompt: None,
            greeting: None,
            model: None,
            extensions: vec![],
            skills: vec![],
            knowledge_base: None,
            capabilities: None,
            guardrails: None,
            reliability: None,
            orchestration: None,
            output_type: None,
            durable_session: None,
        }
    }

    #[tokio::test]
    async fn create_app_writes_ts_project_and_defaults() {
        let (_d, s) = server();
        let mut p = create("Dashboard", None);
        p.system_prompt = Some("You analyze data.".into());
        p.extensions = vec!["autovisualiser".into()];
        let res = s
            .create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();
        assert!(has_ui_resource(&res));
        assert!(s.store().read_file("dashboard", "src/main.ts").is_ok());
        assert!(s.store().read_file("dashboard", "src/sdk.ts").is_ok());
        let html = s.store().read_file("dashboard", "index.html").unwrap();
        assert!(html.contains("br-container"));
        let m = s.store().load_manifest("dashboard").unwrap();
        assert_eq!(m.kind, ArtifactKind::Agentic);
        let agent = m.agent.unwrap();
        // Provider-agnostic: no model pinned unless explicitly chosen → inherits
        // the user's configured provider/model.
        assert!(agent.model.is_none());
        assert_eq!(agent.extensions, vec!["autovisualiser".to_string()]);
        assert_eq!(agent.system_prompt, "You analyze data.");
    }

    #[tokio::test]
    async fn create_app_records_session_id_when_present() {
        let (_d, s) = server();
        // Agentic app carries the session id through the agent-config save path.
        s.create_app_inner(
            create("Sessioned", None),
            Some("sess-123".into()),
            &KbCaller::restricted(),
        )
        .await
        .unwrap();
        let m = s.store().load_manifest("sessioned").unwrap();
        assert_eq!(m.session_id.as_deref(), Some("sess-123"));

        // Static app gets the id too (re-saved after the initial create).
        s.create_app_inner(
            create("StaticOne", Some("static")),
            Some("sess-999".into()),
            &KbCaller::restricted(),
        )
        .await
        .unwrap();
        let sm = s.store().load_manifest("staticone").unwrap();
        assert_eq!(sm.session_id.as_deref(), Some("sess-999"));

        // No session meta (headless/CLI) leaves it unset.
        s.create_app_inner(create("NoSession", None), None, &KbCaller::restricted())
            .await
            .unwrap();
        assert_eq!(
            s.store().load_manifest("nosession").unwrap().session_id,
            None
        );
    }

    #[tokio::test]
    async fn create_static_app_persists_its_theme_pack() {
        let (_d, s) = server();
        let mut p = create("Midnight Static", Some("static"));
        p.theme = Some(declare::ThemeParam {
            pack: declare::ThemePack::Midnight,
            accent: None,
            tokens: None,
        });

        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let manifest = s.store().load_manifest("midnight-static").unwrap();
        assert_eq!(manifest.theme.resolved_pack(), "midnight");
    }

    #[tokio::test]
    async fn configure_app_sets_model_and_extensions() {
        let _strict = relax_catalog_strictness();
        let (_d, s) = server();
        s.create_app_inner(create("Cfg", Some("static")), None, &KbCaller::restricted())
            .await
            .unwrap();
        s.configure_app_public(Parameters(ConfigureAppParams {
            id: "cfg".into(),
            system_prompt: Some("Be terse.".into()),
            greeting: None,
            model: Some(ModelParam {
                provider: Some("anthropic".into()),
                model: Some("claude-opus-4-8".into()),
                settings: None,
            }),
            extensions: Some(vec!["developer".into(), "knowledge".into()]),
            skills: Some(vec!["scientific-research".into()]),
            knowledge_base: Some("my-kb".into()),
            max_turns: Some(40),
            capabilities: None,
            guardrails: None,
            reliability: None,
            orchestration: None,
            output_type: None,
            durable_session: None,
            requires: None,
        }))
        .await
        .unwrap();
        let m = s.store().load_manifest("cfg").unwrap();
        assert_eq!(m.kind, ArtifactKind::Agentic);
        let a = m.agent.unwrap();
        assert_eq!(a.model.unwrap().provider.unwrap(), "anthropic");
        assert_eq!(a.extensions, vec!["developer", "knowledge"]);
        assert_eq!(a.skills, vec!["scientific-research"]);
        assert_eq!(a.knowledge_base.unwrap(), "my-kb");
        assert_eq!(a.max_turns, Some(40));
    }

    #[tokio::test]
    async fn configure_app_sets_advanced_agent_design_fields() {
        let _strict = relax_catalog_strictness();
        let (_d, s) = server();
        s.create_app_inner(create("Harnessed", None), None, &KbCaller::restricted())
            .await
            .unwrap();
        s.configure_app_public(Parameters(ConfigureAppParams {
            id: "harnessed".into(),
            system_prompt: Some("Use the visible workflow and cite each step.".into()),
            greeting: None,
            model: Some(ModelParam {
                provider: Some("openrouter".into()),
                model: Some("anthropic/claude-sonnet-4".into()),
                settings: Some(ModelSettingsParam {
                    temperature: Some(0.2),
                    max_tokens: Some(4096),
                    reasoning_effort: Some("medium".into()),
                    ..Default::default()
                }),
            }),
            extensions: Some(vec!["knowledge".into(), "autovisualiser".into()]),
            skills: Some(vec!["graph-visualization".into()]),
            knowledge_base: Some("kb-science".into()),
            max_turns: Some(96),
            capabilities: Some(serde_json::json!({
                "data": {
                    "sources": [
                        { "name": "science", "kind": "knowledge", "ref_id": "kb-science" }
                    ]
                },
                "memory": { "kb": "kb-science", "mode": "read_write", "distill": true },
                "events": ["tool", "handoff", "compaction", "guardrail"]
            })),
            guardrails: Some(serde_json::json!({
                "goal": "produce a knowledge-map answer with citations and a chart",
                "pii": "block",
                "needs_approval": ["developer__shell"],
                "approvals_require_persistence": true
            })),
            reliability: Some(serde_json::json!({
                "tool_timeout_s": 45,
                "parallel_tools": true,
                "error_to_output": true
            })),
            orchestration: Some(serde_json::json!({
                "sub_agents": {
                    "mapper": {
                        "description": "Knowledge graph mapper",
                        "system_prompt": "Extract entities and relationships.",
                        "extensions": ["knowledge"],
                        "max_steps": 8
                    }
                },
                "workflows": {
                    "map_then_visualize": {
                        "steps": [
                            { "type": "agent", "agent": "mapper", "input_template": "{{query}}" },
                            { "type": "tool", "tool": "autovisualiser__visualise", "args_template": { "format": "graph" } }
                        ]
                    }
                },
                "lazy_tools": true
            })),
            output_type: Some(serde_json::json!({
                "type": "object",
                "required": ["summary", "visualization"],
                "properties": {
                    "summary": { "type": "string" },
                    "visualization": { "type": "string" }
                }
            })),
            durable_session: Some(false),
            requires: None,
        }))
        .await
        .unwrap();

        let m = s.store().load_manifest("harnessed").unwrap();
        let a = m.agent.unwrap();
        let model = a.model.unwrap();
        assert_eq!(model.provider.as_deref(), Some("openrouter"));
        assert_eq!(
            model
                .settings
                .as_ref()
                .and_then(|s| s.reasoning_effort.as_deref()),
            Some("medium")
        );
        assert_eq!(
            a.capabilities.data.as_ref().unwrap().sources[0]
                .ref_id
                .as_deref(),
            Some("kb-science")
        );
        assert!(a
            .capabilities
            .advertised()
            .contains(&"event:compaction".to_string()));
        assert_eq!(
            a.guardrails.unwrap().pii,
            crate::agent_drafter::manifest::PiiMode::Block
        );
        assert!(a.reliability.unwrap().parallel_tools);
        assert!(a.orchestration.sub_agents.contains_key("mapper"));
        assert!(a.orchestration.workflows.contains_key("map_then_visualize"));
        assert!(a.output_type.is_some());
        assert_eq!(a.durable_session, Some(false));
    }

    #[tokio::test]
    async fn creates_static_and_agentic_kinds_with_expected_defaults() {
        let (_d, s) = server();
        s.create_app_inner(
            create("Plain Widget", Some("static")),
            None,
            &KbCaller::restricted(),
        )
        .await
        .unwrap();
        s.create_app_inner(
            create("Agent Workspace", Some("agentic")),
            None,
            &KbCaller::restricted(),
        )
        .await
        .unwrap();

        let static_manifest = s.store().load_manifest("plain-widget").unwrap();
        assert_eq!(static_manifest.kind, ArtifactKind::Static);
        assert!(static_manifest.agent.is_none());
        assert!(!s.store().file_exists("plain-widget", "src/main.ts"));

        let agentic_manifest = s.store().load_manifest("agent-workspace").unwrap();
        assert_eq!(agentic_manifest.kind, ArtifactKind::Agentic);
        assert!(agentic_manifest.agent.is_some());
        assert!(s.store().file_exists("agent-workspace", "src/main.ts"));
        assert!(s.store().file_exists("agent-workspace", "src/sdk.ts"));
    }

    #[tokio::test]
    async fn custom_layout_and_workflow_prompt_build_and_pass_launch_harness() {
        let _strict = relax_catalog_strictness();
        let (_d, s) = server();
        let mut p = create("Cohort Review Console", None);
        p.id = Some("cohort-review-console".into());
        p.description = "Control-driven clinical cohort review workflow".into();
        p.extensions = vec!["knowledge".into(), "developer".into()];
        p.skills = vec!["clinical-biostatistics".into()];
        p.knowledge_base = Some("trial-kb".into());
        p.system_prompt = Some(
            "Follow this workflow: 1. inspect the selected cohort; 2. search the knowledge base; \
             3. delegate statistical checks to the stats sub-agent when appropriate; 4. if context \
             usage is above 80%, summarize before continuing; 5. never download external assets."
                .into(),
        );
        p.html = Some(
            r#"<html>
              <head><title>Cohort Review Console</title></head>
              <body>
                <main class="br-container">
                  <section class="br-card">
                    <label class="br-label" for="assay">Assay</label>
                    <select id="assay" class="br-select">
                      <option>single-cell RNA-seq</option>
                      <option>flow cytometry</option>
                    </select>
                    <label class="br-label" for="cohorts">Cohorts to compare</label>
                    <input id="cohorts" class="br-input" value="responders vs non-responders" />
                    <button id="run" class="br-btn">Run workflow</button>
                  </section>
                  <section id="out" class="br-output" data-placeholder="Workflow output"></section>
                </main>
              </body>
            </html>"#
                .into(),
        );
        p.files = vec![FileSpec {
            path: "src/main.ts".into(),
            content: r##"import { createApp } from "./sdk";

const br = createApp({ autoChat: false });
const assay = document.getElementById("assay") as HTMLSelectElement;
const cohorts = document.getElementById("cohorts") as HTMLInputElement;
const run = document.getElementById("run") as HTMLButtonElement;

run.addEventListener("click", async () => {
  const ctx = await br.context.tokens();
  const contextPlan =
    ctx.ratio > 0.8
      ? "First compact/summarize the working context before continuing."
      : "Continue without compaction unless the context grows past 80%.";
  await br.run(
    `Run the cohort review workflow for ${cohorts.value} using ${assay.value}. ${contextPlan} Use the configured tools and sub-agents when useful, and do not download external assets.`,
    "#out"
  );
});
"##
            .into(),
        }];

        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();
        s.configure_app_public(Parameters(ConfigureAppParams {
            id: "cohort-review-console".into(),
            system_prompt: None,
            greeting: Some("Choose a cohort and run the review.".into()),
            model: None,
            extensions: None,
            skills: None,
            knowledge_base: None,
            max_turns: Some(72),
            capabilities: None,
            guardrails: None,
            reliability: None,
            orchestration: None,
            output_type: None,
            durable_session: None,
            requires: None,
        }))
        .await
        .unwrap();

        let build = s
            .build_app(Parameters(BuildAppParams {
                id: "cohort-review-console".into(),
                check_only: None,
            }))
            .await
            .unwrap();
        let build_text = text_of(&build);
        assert!(build_text.contains("dist/app.js"));
        assert!(build_text.contains("passes all guardrails"));
        // Building leaves the app in a new visible state, so the GUI's artifact
        // panel must get the app itself, not just the build log.
        assert!(
            has_ui_resource(&build),
            "build_app must return a preview card"
        );

        let launch = s
            .launch_app(Parameters(AppIdParams {
                id: "cohort-review-console".into(),
            }))
            .await
            .unwrap();
        assert!(text_of(&launch).contains("/apps/cohort-review-console/"));
        assert!(launch
            .content
            .iter()
            .any(|content| matches!(&content.raw, rmcp::model::RawContent::ResourceLink(_))));
        assert!(
            !has_ui_resource(&launch),
            "launch_app should expose one click-only browser link"
        );

        let manifest = s.store().load_manifest("cohort-review-console").unwrap();
        let agent = manifest.agent.unwrap();
        assert_eq!(agent.max_turns, Some(72));
        assert_eq!(agent.extensions, vec!["knowledge", "developer"]);
        assert_eq!(agent.skills, vec!["clinical-biostatistics"]);
        assert_eq!(agent.knowledge_base.as_deref(), Some("trial-kb"));
        assert!(agent.system_prompt.contains("above 80%"));
    }

    #[tokio::test]
    async fn manifest_update_supports_advanced_workflow_and_security_config() {
        use crate::agent_drafter::manifest::{
            Capabilities, ComputeCapability, FilesCapability, GuardrailsConfig, PiiMode,
            ReliabilityConfig, SubAgentManifest, WorkflowManifest, WorkflowStep,
        };
        use std::collections::HashMap;

        let (_d, s) = server();
        s.create_app_inner(
            create("Advanced Agent", None),
            None,
            &KbCaller::restricted(),
        )
        .await
        .unwrap();
        let mut manifest = s.store().load_manifest("advanced-agent").unwrap();
        let agent = manifest.agent.as_mut().unwrap();

        let capabilities = Capabilities {
            files: Some(FilesCapability {
                entries: Vec::new(),
                max_file_bytes: Some(256 * 1024),
            }),
            compute: Some(ComputeCapability {
                sandbox: "docker".into(),
                timeout_s: 45,
                network: "none".into(),
                max_mem: Some("512m".into()),
                cpus: Some(1.0),
                image: None,
            }),
            events: vec!["tool".into(), "compaction".into(), "handoff".into()],
            ..Default::default()
        };
        agent.capabilities = capabilities;
        agent.guardrails = Some(GuardrailsConfig {
            goal: Some("finish with a cited risk summary".into()),
            business_scope: Some("clinical research workflow drafting".into()),
            pii: PiiMode::Block,
            needs_approval: vec!["compute__exec".into(), "developer__shell".into()],
            approvals_require_persistence: true,
            ..Default::default()
        });
        agent.reliability = Some(ReliabilityConfig {
            tool_timeout_s: Some(30),
            parallel_tools: true,
            ..Default::default()
        });
        agent.output_type = Some(serde_json::json!({
            "type": "object",
            "required": ["summary", "next_steps"],
            "properties": {
                "summary": { "type": "string" },
                "next_steps": { "type": "array", "items": { "type": "string" } }
            }
        }));

        let mut sub_agents = HashMap::new();
        sub_agents.insert(
            "stats".into(),
            SubAgentManifest {
                description: "Biostatistics specialist".into(),
                system_prompt: "Check statistical assumptions and effect sizes.".into(),
                skills: vec!["clinical-biostatistics".into()],
                max_steps: Some(8),
                max_wall_s: Some(120),
                ..Default::default()
            },
        );
        let mut workflows = HashMap::new();
        workflows.insert(
            "triage".into(),
            WorkflowManifest {
                steps: vec![
                    WorkflowStep::Agent {
                        agent: "stats".into(),
                        input_template: "{{cohort_summary}}".into(),
                        guardrail: None,
                        on_error: "abort".into(),
                    },
                    WorkflowStep::Tool {
                        tool: "knowledge__query".into(),
                        args_template: serde_json::json!({ "q": "{{finding}}" }),
                        guardrail: Some(serde_json::json!({ "kind": "pii", "mode": "block" })),
                        on_error: "continue".into(),
                    },
                ],
            },
        );
        agent.orchestration.sub_agents = sub_agents;
        agent.orchestration.workflows = workflows;

        s.update_app_public(Parameters(UpdateAppParams {
            id: "advanced-agent".into(),
            path: Some("manifest.json".into()),
            content: Some(serde_json::to_string_pretty(&manifest).unwrap()),
            old_str: None,
            new_str: None,
        }))
        .await
        .unwrap();

        let read = s
            .read_app(Parameters(ReadAppParams {
                id: "advanced-agent".into(),
                path: None,
                view: None,
            }))
            .await
            .unwrap();
        let roundtrip: Manifest = serde_json::from_str(&text_of(&read)).unwrap();
        let cfg = roundtrip.agent.unwrap();
        assert_eq!(cfg.capabilities.compute.as_ref().unwrap().network, "none");
        assert!(cfg.capabilities.advertised().contains(&"files".to_string()));
        assert!(cfg
            .capabilities
            .advertised()
            .contains(&"compute".to_string()));
        assert!(cfg
            .capabilities
            .advertised()
            .contains(&"event:compaction".to_string()));
        assert_eq!(cfg.guardrails.as_ref().unwrap().pii, PiiMode::Block);
        assert!(cfg
            .guardrails
            .as_ref()
            .unwrap()
            .needs_approval
            .contains(&"compute__exec".to_string()));
        assert!(cfg.reliability.as_ref().unwrap().parallel_tools);
        assert!(cfg.orchestration.sub_agents.contains_key("stats"));
        assert!(cfg.orchestration.workflows.contains_key("triage"));
        assert!(cfg.output_type.is_some());
    }

    #[tokio::test]
    async fn manifest_update_rejects_invalid_json_and_id_mismatch() {
        let (_d, s) = server();
        s.create_app_inner(create("Manifest Safe", None), None, &KbCaller::restricted())
            .await
            .unwrap();
        let original = s.store().load_manifest("manifest-safe").unwrap();

        assert!(s
            .update_app_public(Parameters(UpdateAppParams {
                id: "manifest-safe".into(),
                path: Some("manifest.json".into()),
                content: Some("{ not json".into()),
                old_str: None,
                new_str: None,
            }))
            .await
            .is_err());

        let mut wrong_id = original.clone();
        wrong_id.id = "other-app".into();
        assert!(s
            .update_app_public(Parameters(UpdateAppParams {
                id: "manifest-safe".into(),
                path: Some("manifest.json".into()),
                content: Some(serde_json::to_string_pretty(&wrong_id).unwrap()),
                old_str: None,
                new_str: None,
            }))
            .await
            .is_err());

        let still_valid = s.store().load_manifest("manifest-safe").unwrap();
        assert_eq!(still_valid.id, original.id);
        assert_eq!(still_valid.title, original.title);
    }

    #[tokio::test]
    async fn harness_errors_block_launch_and_export() {
        let (_d, s) = server();
        let mut p = create("Broken Harness", None);
        p.html = Some(
            r#"<html><body><main class="br-container"><button id="go" class="br-btn">Run</button></main></body></html>"#
                .into(),
        );
        p.files = vec![FileSpec {
            path: "src/main.ts".into(),
            content: r##"import { createApp } from "./sdk";
const br = createApp({ autoChat: false });
br.run("hello", "#missing");
"##
            .into(),
        }];
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let build = s
            .build_app(Parameters(BuildAppParams {
                id: "broken-harness".into(),
                check_only: None,
            }))
            .await
            .unwrap();
        let build_text = text_of(&build);
        assert!(build_text.contains("ERROR"));
        assert!(build_text.contains("#missing"));

        assert!(s
            .launch_app(Parameters(AppIdParams {
                id: "broken-harness".into(),
            }))
            .await
            .is_err());

        let out = TempDir::new().unwrap();
        assert!(s
            .export_app_public(Parameters(ExportAppParams {
                id: "broken-harness".into(),
                target_dir: out.path().to_string_lossy().to_string(),
                endpoint: None,
                ..Default::default()
            }))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn create_rejects_empty_title_and_bad_kind() {
        let (_d, s) = server();
        assert!(s
            .create_app_inner(create("  ", None), None, &KbCaller::restricted())
            .await
            .is_err());
        assert!(s
            .create_app_inner(create("X", Some("bogus")), None, &KbCaller::restricted())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn update_marks_bundle_stale_for_src() {
        let (_d, s) = server();
        let mut p = create("Edit Me", None);
        p.html = Some("<html><body>ORIGINAL</body></html>".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        s.update_app_public(Parameters(UpdateAppParams {
            id: "edit-me".into(),
            path: Some("src/main.ts".into()),
            content: Some("import './sdk'; console.log(1);".into()),
            old_str: None,
            new_str: None,
        }))
        .await
        .unwrap();
        assert_eq!(s.store().load_manifest("edit-me").unwrap().built_at, None);

        let res = s
            .update_app_public(Parameters(UpdateAppParams {
                id: "edit-me".into(),
                path: None,
                content: None,
                old_str: Some("ORIGINAL".into()),
                new_str: Some("CHANGED".into()),
            }))
            .await
            .unwrap();
        assert!(has_ui_resource(&res));
        assert!(s
            .store()
            .read_file("edit-me", "index.html")
            .unwrap()
            .contains("CHANGED"));
    }

    #[tokio::test]
    async fn build_then_launch_returns_url() {
        let (_d, s) = server();
        s.create_app_inner(create("Launchy", None), None, &KbCaller::restricted())
            .await
            .unwrap();
        let res = s
            .build_app(Parameters(BuildAppParams {
                id: "launchy".into(),
                check_only: None,
            }))
            .await
            .unwrap();
        assert!(text_of(&res).contains("dist/app.js"));
        assert!(s.store().file_exists("launchy", "dist/app.js"));
        assert!(s
            .store()
            .load_manifest("launchy")
            .unwrap()
            .built_at
            .is_some());

        let res = s
            .launch_app(Parameters(AppIdParams {
                id: "launchy".into(),
            }))
            .await
            .unwrap();
        assert!(text_of(&res).contains("/apps/launchy/"));
    }

    #[tokio::test]
    async fn list_read_delete() {
        let (_d, s) = server();
        s.create_app_inner(create("One", None), None, &KbCaller::restricted())
            .await
            .unwrap();
        let all = s
            .list_apps(Parameters(ListAppsParams { kind: None }))
            .await
            .unwrap();
        assert!(text_of(&all).contains("one"));

        let m = s
            .read_app(Parameters(ReadAppParams {
                id: "one".into(),
                path: None,
                view: None,
            }))
            .await
            .unwrap();
        assert!(text_of(&m).contains("\"title\": \"One\""));

        s.delete_app(Parameters(AppIdParams { id: "one".into() }))
            .await
            .unwrap();
        assert!(!s.store().exists("one"));
    }

    #[tokio::test]
    async fn export_writes_standalone_ts_project() {
        let (_d, s) = server();
        let mut p = create("Exporter", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let out = TempDir::new().unwrap();
        let res = s
            .export_app_public(Parameters(ExportAppParams {
                id: "exporter".into(),
                target_dir: out.path().to_string_lossy().to_string(),
                endpoint: None,
                ..Default::default()
            }))
            .await
            .unwrap();
        assert!(text_of(&res).contains("run.command"));
        for f in [
            "index.html",
            "manifest.json",
            "package.json",
            "serve.mjs",
            "run.sh",
            "run.command",
            "biorouter-launch.sh",
            "src/main.ts",
            "src/sdk.ts",
        ] {
            assert!(out.path().join(f).exists(), "export is missing {f}");
        }
        let index = std::fs::read_to_string(out.path().join("index.html")).unwrap();
        assert!(index.contains("dist/app.js"));
        // Config is the non-executable JSON island (CSP parity with served apps).
        assert!(index.contains("<script type=\"application/json\" id=\"biorouter-app-config\">"));

        // The exported manifest is what registers the app with a daemon.
        let m: Manifest = serde_json::from_str(
            &std::fs::read_to_string(out.path().join("manifest.json")).unwrap(),
        )
        .expect("exported manifest must parse");
        assert_eq!(m.id, "exporter");

        // A launcher that isn't executable isn't double-clickable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for f in ["run.command", "run.sh", "biorouter-launch.sh"] {
                let mode = std::fs::metadata(out.path().join(f))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(
                    mode & 0o111,
                    0o111,
                    "{f} must be executable (mode {mode:o})"
                );
            }
        }
    }

    /// An export must never carry the author's sealed secrets off their machine.
    #[tokio::test]
    async fn export_excludes_the_vault() {
        let (_d, s) = server();
        let mut p = create("Vaulted", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();
        let vault_dir = s.store().artifact_dir("vaulted").unwrap().join(".vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        std::fs::write(vault_dir.join("API_KEY.enc"), "sealed-bytes").unwrap();

        let out = TempDir::new().unwrap();
        s.export_app_public(Parameters(ExportAppParams {
            id: "vaulted".into(),
            target_dir: out.path().to_string_lossy().to_string(),
            endpoint: None,
            ..Default::default()
        }))
        .await
        .unwrap();
        assert!(
            !out.path().join(".vault").exists(),
            "the vault must not be exported"
        );
    }

    #[tokio::test]
    async fn export_rejects_missing_app() {
        let (_d, s) = server();
        assert!(s
            .export_app_public(Parameters(ExportAppParams {
                id: "ghost".into(),
                target_dir: "/tmp/x".into(),
                endpoint: None,
                ..Default::default()
            }))
            .await
            .is_err());
    }

    // ── Standalone export v2 (design §3.9) ─────────────────────────────────

    /// Removes an env var on drop so a panicking `#[serial]` test can't leak it
    /// into the next one.
    struct EnvGuard(&'static str);
    impl EnvGuard {
        fn set(key: &'static str, val: &std::path::Path) -> Self {
            std::env::set_var(key, val);
            EnvGuard(key)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }

    /// Launcher mode (the default) is unchanged: no `payload/`, no `export.json`.
    #[tokio::test]
    async fn export_launcher_mode_writes_no_payload() {
        let (_d, s) = server();
        let mut p = create("Launcher", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let out = TempDir::new().unwrap();
        s.export_app_public(Parameters(ExportAppParams {
            id: "launcher".into(),
            target_dir: out.path().to_string_lossy().to_string(),
            mode: Some("launcher".into()),
            ..Default::default()
        }))
        .await
        .unwrap();
        assert!(
            !out.path().join("payload").exists(),
            "no payload in launcher mode"
        );
        assert!(
            !out.path().join("export.json").exists(),
            "no export.json in launcher mode"
        );
        // The Windows launchers still ship in every export.
        assert!(out.path().join("run.ps1").exists());
        assert!(out.path().join("run.bat").exists());
    }

    /// Full mode stages the app's KB + skill payload and writes a deterministic
    /// `export.json`. Uses env overrides so the KB / skills roots point at temp
    /// fixtures (hence `#[serial]`).
    #[tokio::test]
    #[serial_test::serial]
    async fn export_full_mode_stages_payload_and_writes_export_json() {
        let _strict = relax_catalog_strictness();
        // Fake knowledge store with one KB directory the brkb exporter can walk.
        let kroot = TempDir::new().unwrap();
        let kb_dir = kroot.path().join("ms-cohort").join("knowledge");
        std::fs::create_dir_all(&kb_dir).unwrap();
        std::fs::write(kb_dir.join("index.md"), "# MS cohort\n").unwrap();
        // Fake skills dir with one installed skill.
        let sroot = TempDir::new().unwrap();
        std::fs::create_dir_all(sroot.path().join("ggplot")).unwrap();
        std::fs::write(sroot.path().join("ggplot").join("SKILL.md"), "# ggplot\n").unwrap();

        let _kg = EnvGuard::set("BIOROUTER_KNOWLEDGE_DIR", kroot.path());
        let _sg = EnvGuard::set("BIOROUTER_SKILLS_DIR", sroot.path());

        let (_d, s) = server();
        let mut p = create("Cohort", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        p.knowledge_base = Some("ms-cohort".into());
        p.skills = vec!["ggplot".into()];
        // developer is builtin (travels with the daemon); spokeagent is external.
        p.extensions = vec!["developer".into(), "spokeagent".into()];
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        // Give the app a vault to prove full mode still excludes it.
        let vault = s.store().artifact_dir("cohort").unwrap().join(".vault");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(vault.join("K.enc"), "sealed").unwrap();

        let out = TempDir::new().unwrap();
        let res = s
            .export_app_public(Parameters(ExportAppParams {
                id: "cohort".into(),
                target_dir: out.path().to_string_lossy().to_string(),
                mode: Some("full".into()),
                ..Default::default()
            }))
            .await
            .unwrap();

        // Payload dirs.
        assert!(
            out.path().join("payload/knowledge/ms-cohort.brkb").exists(),
            "KB staged as a .brkb"
        );
        assert!(
            out.path().join("payload/skills/ggplot/SKILL.md").exists(),
            "skill staged as a dir tree"
        );
        assert!(
            !out.path().join(".vault").exists(),
            "vault excluded in full mode"
        );

        // export.json structure.
        let ej: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.path().join("export.json")).unwrap())
                .expect("export.json must parse");
        assert_eq!(ej["version"], 1);
        assert_eq!(ej["app"], "cohort");
        assert_eq!(ej["mode"], "full");
        assert_eq!(ej["knowledge_bases"][0]["id"], "ms-cohort");
        assert_eq!(
            ej["knowledge_bases"][0]["file"],
            "payload/knowledge/ms-cohort.brkb"
        );
        assert!(ej["knowledge_bases"][0]["bytes"].as_u64().unwrap() > 0);
        assert_eq!(ej["skills"][0]["name"], "ggplot");
        assert_eq!(ej["skills"][0]["path"], "payload/skills/ggplot");
        // Only the external extension is recorded; the builtin is not.
        assert_eq!(ej["extensions"].as_array().unwrap().len(), 1);
        assert_eq!(ej["extensions"][0]["name"], "spokeagent");
        assert_eq!(ej["extensions"][0]["source"], "registry");
        assert!(ej["required_credentials"].as_array().unwrap().is_empty());
        assert!(ej["runtime_requirements"].as_array().unwrap().is_empty());
        assert!(ej["bundled_daemon"].is_null(), "thin export → no daemon");

        assert!(text_of(&res).contains("full mode"));
    }

    /// A KB id that doesn't exist is skipped with a note — the export still
    /// succeeds and `export.json` records an empty `knowledge_bases` list.
    #[tokio::test]
    #[serial_test::serial]
    async fn export_full_mode_skips_missing_kb() {
        let _strict = relax_catalog_strictness();
        let kroot = TempDir::new().unwrap(); // empty store — no such KB
        let _kg = EnvGuard::set("BIOROUTER_KNOWLEDGE_DIR", kroot.path());

        let (_d, s) = server();
        let mut p = create("Ghostkb", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        p.knowledge_base = Some("does-not-exist".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let out = TempDir::new().unwrap();
        let res = s
            .export_app_public(Parameters(ExportAppParams {
                id: "ghostkb".into(),
                target_dir: out.path().to_string_lossy().to_string(),
                mode: Some("full".into()),
                ..Default::default()
            }))
            .await
            .unwrap();

        let ej: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.path().join("export.json")).unwrap())
                .unwrap();
        assert!(
            ej["knowledge_bases"].as_array().unwrap().is_empty(),
            "missing KB is skipped, not staged"
        );
        assert!(text_of(&res).contains("skipped knowledge base 'does-not-exist'"));
    }

    /// Issue #56 (CP4). `export_brkb` writes the WHOLE base into the payload,
    /// and `kb_ids` comes from the model-supplied `include.knowledge_bases` — a
    /// strictly wider `kb_export` with no id gate anywhere before this task.
    /// Skip-and-note rather than hard-fail, matching `search_visible_bases`: the
    /// rest of the export is still useful and the user is told why.
    #[tokio::test]
    #[serial_test::serial]
    async fn export_app_leaves_a_private_knowledge_base_out_of_the_payload() {
        let _strict = relax_catalog_strictness();
        let kroot = TempDir::new().unwrap();
        // Real bases, not hand-made directories: `create_base` registers each id
        // PUBLIC, and a base with a directory but no tier entry would read
        // private by inference (decision 3) and make this test pass vacuously.
        let ksvc = crate::knowledge::service::KnowledgeService::new(kroot.path().to_path_buf());
        ksvc.create_base("pub-kb", "Public KB", None).unwrap();
        ksvc.create_base("priv-kb", "OMOP Cohort", None).unwrap();
        crate::knowledge::tier::raise_unlocked(kroot.path(), "priv-kb", true).unwrap();

        let _kg = EnvGuard::set("BIOROUTER_KNOWLEDGE_DIR", kroot.path());

        let (_d, s) = server();
        let mut p = create("Payload", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let include = serde_json::json!({ "knowledge_bases": ["pub-kb", "priv-kb"] });

        // A PUBLIC caller gets the public base and a note naming what was left out.
        let out = TempDir::new().unwrap();
        let res = s
            .export_app_inner(
                ExportAppParams {
                    id: "payload".into(),
                    target_dir: out.path().to_string_lossy().to_string(),
                    mode: Some("full".into()),
                    include: Some(include.clone()),
                    ..Default::default()
                },
                &KbCaller::restricted(),
            )
            .await
            .unwrap();

        let ej: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.path().join("export.json")).unwrap())
                .unwrap();
        let ids: Vec<&str> = ej["knowledge_bases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["pub-kb"]);
        let text = text_of(&res);
        assert!(
            text.contains("priv-kb") && text.contains("private"),
            "the export must say what it left out and why: {text}"
        );
        assert!(!out.path().join("payload/knowledge/priv-kb.brkb").exists());
        assert!(out.path().join("payload/knowledge/pub-kb.brkb").exists());

        // A PRIVATE caller still gets both, or "no leak" is satisfied by
        // "the export stages nothing".
        let out2 = TempDir::new().unwrap();
        s.export_app_inner(
            ExportAppParams {
                id: "payload".into(),
                target_dir: out2.path().to_string_lossy().to_string(),
                mode: Some("full".into()),
                include: Some(include),
                ..Default::default()
            },
            &private_test_caller(),
        )
        .await
        .unwrap();
        assert!(out2.path().join("payload/knowledge/priv-kb.brkb").exists());
    }

    /// An explicit empty `include.knowledge_bases` selects nothing even when the
    /// agent config references a KB (per-item opt-out).
    #[tokio::test]
    #[serial_test::serial]
    async fn export_full_mode_explicit_empty_include_selects_none() {
        let _strict = relax_catalog_strictness();
        let kroot = TempDir::new().unwrap();
        std::fs::create_dir_all(kroot.path().join("kb1").join("knowledge")).unwrap();
        std::fs::write(kroot.path().join("kb1").join("knowledge").join("i.md"), "x").unwrap();
        let _kg = EnvGuard::set("BIOROUTER_KNOWLEDGE_DIR", kroot.path());

        let (_d, s) = server();
        let mut p = create("Opt Out", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        p.knowledge_base = Some("kb1".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let out = TempDir::new().unwrap();
        s.export_app_public(Parameters(ExportAppParams {
            id: "opt-out".into(),
            target_dir: out.path().to_string_lossy().to_string(),
            mode: Some("full".into()),
            include: Some(serde_json::json!({ "knowledge_bases": [] })),
            ..Default::default()
        }))
        .await
        .unwrap();

        assert!(
            !out.path().join("payload/knowledge").exists(),
            "explicit empty include stages no KB"
        );
    }

    /// `bundle_daemon="current"` copies a `biorouterd` (located via the
    /// `BIOROUTERD_BIN` hook) into `payload/bin/` and records it in export.json.
    #[tokio::test]
    #[serial_test::serial]
    async fn export_bundle_daemon_current_stages_binary() {
        // A fake daemon binary on disk; find_biorouterd_binary honours BIOROUTERD_BIN first.
        let bin_dir = TempDir::new().unwrap();
        let fake = bin_dir.path().join("biorouterd");
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
        let _bg = EnvGuard::set("BIOROUTERD_BIN", &fake);

        let (_d, s) = server();
        let mut p = create("Fat", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let out = TempDir::new().unwrap();
        let res = s
            .export_app_public(Parameters(ExportAppParams {
                id: "fat".into(),
                target_dir: out.path().to_string_lossy().to_string(),
                bundle_daemon: Some("current".into()),
                ..Default::default()
            }))
            .await
            .unwrap();

        let bin_name = if cfg!(windows) {
            "biorouterd.exe"
        } else {
            "biorouterd"
        };
        let staged = out.path().join("payload/bin").join(bin_name);
        assert!(staged.exists(), "daemon staged under payload/bin");
        // A fat launcher still writes export.json recording the bundled daemon.
        let ej: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.path().join("export.json")).unwrap())
                .unwrap();
        assert!(
            !ej["bundled_daemon"].is_null(),
            "records the bundled daemon"
        );
        assert_eq!(
            ej["bundled_daemon"]["file"],
            format!("payload/bin/{bin_name}")
        );
        assert!(text_of(&res).contains("fat export"));
    }

    /// When no daemon can be found, `bundle_daemon="current"` degrades to a thin
    /// export with a note instead of failing.
    #[tokio::test]
    #[serial_test::serial]
    async fn export_bundle_daemon_falls_back_to_thin_when_daemon_absent() {
        let _bg = EnvGuard::set(
            "BIOROUTERD_BIN",
            std::path::Path::new("/nonexistent/biorouterd-xyz"),
        );

        let (_d, s) = server();
        let mut p = create("NoDaemon", None);
        p.html = Some("<html><head></head><body>hi</body></html>".into());
        p.system_prompt = Some("help".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let out = TempDir::new().unwrap();
        let res = s
            .export_app_public(Parameters(ExportAppParams {
                id: "nodaemon".into(),
                target_dir: out.path().to_string_lossy().to_string(),
                bundle_daemon: Some("current".into()),
                ..Default::default()
            }))
            .await
            .unwrap();

        assert!(!out.path().join("payload/bin").exists(), "no daemon staged");
        assert!(text_of(&res).contains("thin export"));
    }

    // ── Archetype starters (Apps SDK v2, Pillar 6) ─────────────────────────────

    #[tokio::test]
    async fn dashboard_starter_has_initial_values_for_every_visible_metric_binding() {
        let (_d, s) = server();
        let mut params = create("Dashboard initial state", None);
        params.id = Some("dashboard-initial-state".into());
        params.archetype = Some("dashboard".into());
        s.create_app_inner(params, None, &KbCaller::restricted())
            .await
            .unwrap();

        let manifest = s.store().load_manifest("dashboard-initial-state").unwrap();
        let initial = manifest.surface.state_initial.unwrap();
        let index = s
            .store()
            .read_file("dashboard-initial-state", "index.html")
            .unwrap();
        let mut bindings = 0;
        for suffix in index.split("data-br-bind=\"").skip(1) {
            let (pointer, _) = suffix.split_once('"').unwrap();
            let value = initial.pointer(pointer).and_then(serde_json::Value::as_str);
            assert!(
                value.is_some_and(|text| !text.trim().is_empty()),
                "first-load binding {pointer} needs an explicit unavailable-data label"
            );
            bindings += 1;
        }
        assert_eq!(bindings, 6);
    }

    /// The killer test: creating an app for every archetype seeds the matching
    /// starter and the seeded project lints with **no ERROR-level findings**.
    #[tokio::test]
    async fn starters_seed_and_lint_clean_for_every_archetype() {
        let (_d, s) = server();
        // (archetype, distinguishing index badge, distinguishing main.ts marker)
        let cases = [
            ("explorer", "Explorer", "focus_node"),
            ("dashboard", "Dashboard", "set_metric"),
            ("workbench", "Workbench", "open_row"),
            ("wizard", "Wizard", "go_to_step"),
            ("canvas", "Canvas", "move_avatar"),
            ("chat", "Biorouter App", "createApp();"),
        ];
        for (arch, badge, marker) in cases {
            let id = format!("app-{arch}");
            let mut p = create("Starter", None);
            p.id = Some(id.clone());
            p.archetype = Some(arch.to_string());
            s.create_app_inner(p, None, &KbCaller::restricted())
                .await
                .unwrap();

            let index = s.store().read_file(&id, "index.html").unwrap();
            let main = s.store().read_file(&id, "src/main.ts").unwrap();
            assert!(index.contains(badge), "{arch}: index missing '{badge}'");
            assert!(main.contains(marker), "{arch}: main.ts missing '{marker}'");

            let findings = bundle::lint_app(&s.store().artifact_dir(&id).unwrap());
            let errors: Vec<String> = findings
                .iter()
                .filter(|f| f.level == bundle::LintLevel::Error)
                .map(|f| f.msg.clone())
                .collect();
            assert!(errors.is_empty(), "{arch}: lint errors: {errors:#?}");
        }
    }

    #[test]
    fn infers_archetype_from_brief() {
        assert_eq!(
            Archetype::infer("Gene network explorer", ""),
            Archetype::Explorer
        );
        assert_eq!(
            Archetype::infer("Trial metrics dashboard", ""),
            Archetype::Dashboard
        );
        assert_eq!(
            Archetype::infer("Cohort browser", "browse the sample table"),
            Archetype::Workbench
        );
        assert_eq!(
            Archetype::infer("Intake wizard", "a short survey form"),
            Archetype::Wizard
        );
        assert_eq!(
            Archetype::infer("Avatar scene", "a little game"),
            Archetype::Canvas
        );
        assert_eq!(
            Archetype::infer("Lab helper", "a chat Q&A assistant"),
            Archetype::Chat
        );
        // No keyword → dashboard, never chat by default.
        assert_eq!(
            Archetype::infer("Baranzini tool", "a helpful thing"),
            Archetype::Dashboard
        );
        // Whole-word prefix matching: "platform"/"perform" must NOT hit "form".
        assert_eq!(
            Archetype::infer("Analytics platform", "perform well"),
            Archetype::Dashboard
        );
    }

    #[tokio::test]
    async fn explicit_archetype_overrides_inference() {
        let (_d, s) = server();
        // Title screams "dashboard", but the caller explicitly asked for canvas.
        let mut p = create("Trial metrics dashboard", None);
        p.id = Some("override".into());
        p.archetype = Some("canvas".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let index = s.store().read_file("override", "index.html").unwrap();
        assert!(index.contains("Canvas"));
        let m = s.store().load_manifest("override").unwrap();
        assert!(
            m.surface.components.iter().any(|c| c.name == "scene"),
            "canvas must seed the scene component"
        );

        // A bogus archetype is a clean INVALID_PARAMS, not a panic.
        let mut bad = create("X", None);
        bad.archetype = Some("spaceship".into());
        assert!(s
            .create_app_inner(bad, None, &KbCaller::restricted())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn chat_default_preserved_for_chatty_prompts() {
        let (_d, s) = server();
        let mut p = create("Support assistant", None);
        p.id = Some("chatty".into());
        s.create_app_inner(p, None, &KbCaller::restricted())
            .await
            .unwrap();

        let index = s.store().read_file("chatty", "index.html").unwrap();
        assert!(index.contains("data-br-chat"), "chat keeps the chat card");
        let main = s.store().read_file("chatty", "src/main.ts").unwrap();
        assert!(
            main.contains("createApp();"),
            "chat keeps the default main.ts"
        );
        let m = s.store().load_manifest("chatty").unwrap();
        assert!(m.surface.is_empty(), "chat declares no v2 surface");
    }

    #[tokio::test]
    async fn surface_seeded_for_canvas_and_explorer() {
        let (_d, s) = server();

        let mut c = create("Simulation", None);
        c.id = Some("sim".into());
        c.archetype = Some("canvas".into());
        s.create_app_inner(c, None, &KbCaller::restricted())
            .await
            .unwrap();
        let cm = s.store().load_manifest("sim").unwrap();
        assert!(cm.surface.components.iter().any(|c| c.name == "scene"));
        assert!(cm.surface.actions.iter().any(|a| a.name == "move_avatar"));
        assert!(cm.surface.actions.iter().any(|a| a.name == "reset_scene"));
        assert!(cm
            .surface
            .signals
            .iter()
            .any(|sig| sig.name == "avatar_moved"));
        assert!(cm.surface.state_schema.is_some());

        let mut e = create("Graph tool", None);
        e.id = Some("graph".into());
        e.archetype = Some("explorer".into());
        s.create_app_inner(e, None, &KbCaller::restricted())
            .await
            .unwrap();
        let em = s.store().load_manifest("graph").unwrap();
        assert!(em.surface.actions.iter().any(|a| a.name == "focus_node"));
        assert!(em
            .surface
            .signals
            .iter()
            .any(|sig| sig.name == "search_submitted"));
        assert!(em.surface.state_schema.is_some());

        // A caller-supplied main.ts must NOT get a mismatched surface stamped on.
        let mut byo = create("Bring your own", None);
        byo.id = Some("byo".into());
        byo.archetype = Some("canvas".into());
        byo.files = vec![FileSpec {
            path: "src/main.ts".into(),
            content: "import { createApp } from \"./sdk\";\ncreateApp();\n".into(),
        }];
        s.create_app_inner(byo, None, &KbCaller::restricted())
            .await
            .unwrap();
        assert!(
            s.store().load_manifest("byo").unwrap().surface.is_empty(),
            "no surface when the caller supplies their own main.ts"
        );
    }

    /// Issue #62 — the environment [`run_smoke`] hands to the smoke child.
    ///
    /// `smoke_app` boots the agent-authored application in a real browser
    /// (chromium, launched with `--no-sandbox`) driven by a Node harness. That
    /// makes it the one Agent Drafter path that **executes** model-authored
    /// code rather than parsing it, so the daemon's auth secret must not be in
    /// the environment those processes are launched with — holding it makes the
    /// holder a fully authenticated client of `biorouterd`'s REST API (issue
    /// #57).
    ///
    /// Same probe shape as `developer::shell`: the
    /// leak lives in the *inherited* environment, so exercising it means
    /// controlling this process's environment, and `set_var` is unsound in a
    /// threaded test binary. The parent re-invokes this test binary with the
    /// daemon's environment exported and a `node` shim first on `PATH`, so the
    /// real `run_smoke` spawn resolves the shim and reports the environment the
    /// smoke child actually received.
    #[cfg(unix)]
    mod smoke_child_env {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        /// Child half. `#[ignore]` keeps it out of a normal run.
        #[test]
        #[ignore]
        fn leak_probe_prints_app_smoke_child_env() {
            let app = TempDir::new().unwrap();
            let raw = run_smoke(app.path()).expect("the smoke child must spawn");
            let canary = std::env::var("BR_TEST_CANARY").unwrap_or_default();
            // Report BioRouter's namespace, the variables the parent injected,
            // and any line carrying the canary under *any* key — so a copy of
            // the secret under a different name is still caught. Everything
            // else is dropped: printing the whole environment of whoever runs
            // the suite would be its own small leak.
            let report = raw
                .lines()
                .filter(|line| {
                    let key = line.split('=').next().unwrap_or("");
                    if key == "BR_TEST_CANARY" {
                        return false; // the channel carrying the canary is not the leak
                    }
                    key.starts_with("BIOROUTER_")
                        || key.starts_with("GOOSE_")
                        || matches!(key, "PATH" | "HOME" | "BR_TEST_USER_VAR")
                        || (!canary.is_empty() && line.contains(&canary))
                })
                .collect::<Vec<_>>()
                .join("\n");
            println!("BEGIN_CHILD_ENV");
            println!("{report}");
            println!("END_CHILD_ENV");
        }

        /// Run the probe in a fresh copy of this test binary whose environment
        /// is the daemon's: the auth secret, the port, and an ordinary user
        /// variable. `node` on that copy's `PATH` is a shim that prints its own
        /// environment, so the real `run_smoke` command is what gets measured.
        fn run_app_smoke_leak_probe(canary: &str) -> String {
            let shim = TempDir::new().expect("temp dir");
            let node = shim.path().join("node");
            // Absolute `printenv`, and `PATH`/`HOME` pinned below: sibling tests
            // in this binary swap those two process-wide (`env_lock`), so a shim
            // that resolved `printenv` through the ambient `PATH` failed only in
            // a full parallel run.
            std::fs::write(&node, format!("#!/bin/sh\nexec {}\n", printenv_bin()))
                .expect("write node shim");
            std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755))
                .expect("chmod node shim");
            let path = format!("{}:/usr/bin:/bin", shim.path().display());

            let m = module_path!();
            let without_crate = m.split_once("::").map(|(_, rest)| rest).unwrap_or(m);
            let probe = format!("{without_crate}::leak_probe_prints_app_smoke_child_env");
            let exe = std::env::current_exe().expect("test binary path");
            let out = std::process::Command::new(exe)
                .args([
                    "--exact",
                    &probe,
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("PATH", path)
                .env("HOME", shim.path())
                .env("BIOROUTER_SERVER__SECRET_KEY", canary)
                .env("BR_TEST_CANARY", canary)
                .env("BIOROUTER_PORT", "54321")
                .env("BIOROUTER_APP_SMOKE", "on")
                .env("BR_TEST_USER_VAR", "user-env-ok")
                .output()
                .expect("re-invoking the test binary must work");
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            stdout
                .split_once("BEGIN_CHILD_ENV\n")
                .and_then(|(_, rest)| rest.split_once("END_CHILD_ENV"))
                .map(|(body, _)| body.to_string())
                .unwrap_or_else(|| {
                    panic!(
                        "probe produced no child environment.\nstdout:\n{stdout}\nstderr:\n{}",
                        String::from_utf8_lossy(&out.stderr)
                    )
                })
        }

        /// Issue #62. Before the fix, `printenv` in the smoke child printed
        /// `BIOROUTER_SERVER__SECRET_KEY`.
        #[test]
        fn daemon_secret_never_reaches_the_app_smoke_child() {
            const CANARY: &str = "canary-daemon-secret-drafter-62";
            let child_env = run_app_smoke_leak_probe(CANARY);
            assert!(
                !child_env.contains(CANARY),
                "issue #62: the daemon's auth secret reached the app smoke child, which \
                 executes agent-authored application code, so that code's host can call \
                 biorouterd as an authenticated client.\nchild env:\n{child_env}"
            );
            assert!(
                !child_env.contains("BIOROUTER_SERVER__SECRET_KEY"),
                "the key name itself must be gone, not just the value:\n{child_env}"
            );
        }

        /// The other direction. The smoke harness boots a real browser: it
        /// resolves `node` and the browser binary through `PATH`, finds
        /// playwright's browser cache under `HOME`, and honours the user's own
        /// `PLAYWRIGHT_*`/proxy/temp settings. A truncated child environment is
        /// its own regression — issue #24 was exactly that, for `PATH`.
        #[test]
        fn app_smoke_child_still_gets_the_environment_it_needs() {
            let child_env = run_app_smoke_leak_probe("canary-unused-62");
            for expected in [
                "PATH=",
                "HOME=",
                "BIOROUTER_PORT=54321",
                "BR_TEST_USER_VAR=user-env-ok",
            ] {
                assert!(
                    child_env.lines().any(|l| l.starts_with(expected)),
                    "{expected} is missing: stripping the daemon credential must not censor the \
                     environment the smoke harness needs to boot a browser:\n{child_env}"
                );
            }
        }
    }

    // ── Issue #56, Task 10D: CP5, the metadata surface ──────────────────────

    /// Every drafter tool that can hand a knowledge base's **id or name** to the
    /// model, driven through the router with a capability-carrying request meta.
    ///
    /// Tasks 10B and 10C stop base *content*. They do not stop the id and the
    /// name, and `list_platform_catalog` handed both over for every base on the
    /// machine with no arguments at all — while its own description tells the
    /// model to call it before `configure_app`.
    mod privacy_catalog {
        use super::*;
        use crate::agent_drafter::catalog::drafter_catalog_root_with_kbs;
        use crate::knowledge::tier;
        use serde_json::json;

        /// The capability a probe drives a call with. An enum, not a `bool`, so
        /// the call sites read `Public` / `Private` and cannot be transposed
        /// silently — the same choice `knowledge::server`'s tests made.
        #[derive(Clone, Copy, PartialEq, Debug)]
        enum Caller {
            Public,
            Private,
        }
        use Caller::{Private, Public};

        impl Caller {
            fn is_private(self) -> bool {
                matches!(self, Caller::Private)
            }
        }

        /// A drafter server over a temp app store, pointed at a sandboxed
        /// knowledge root holding `kbs`.
        ///
        /// All four values are returned because all four must outlive the
        /// assertions: the `EnvGuard` is what makes `Catalog::discover` read this
        /// root at all, and either `TempDir` dropping unlinks a tree under it.
        fn drafter_at_root_with_kbs(
            kbs: &[&str],
        ) -> (
            AgentDrafterServer,
            std::path::PathBuf,
            TempDir,
            tempfile::TempDir,
            env_lock::EnvGuard<'static>,
        ) {
            let (kdir, kroot, env) = drafter_catalog_root_with_kbs(kbs);
            let apps = TempDir::new().unwrap();
            let srv = AgentDrafterServer::with_root(apps.path().to_path_buf());
            (srv, kroot, apps, kdir, env)
        }

        /// Drive a drafter tool BY NAME with a request whose meta carries the
        /// caller's capability.
        ///
        /// By name, and not by calling the `#[tool]` function: fourteen of the
        /// eighteen tools take no `RequestContext` at all, so the universal probe
        /// below could not express "as a public caller" for them any other way —
        /// and calling `Catalog::discover(false)` directly would prove the filter
        /// works while saying nothing about whether the TOOL passes the right
        /// argument, which is the whole of the bug.
        ///
        /// A `RequestContext` needs a live `Peer`, which only `serve_directly`
        /// mints; mirrors `knowledge::server`'s `call_tool_as`.
        async fn call_drafter_tool_as(
            srv: &AgentDrafterServer,
            name: &str,
            args: serde_json::Value,
            caller: Caller,
        ) -> Result<CallToolResult, ErrorData> {
            use tokio::io::AsyncReadExt as _;

            let (mut client, server_side) = tokio::io::duplex(64 * 1024);
            tokio::spawn(async move {
                let mut buffer = [0_u8; 8192];
                while client.read(&mut buffer).await.unwrap_or(0) != 0 {}
            });
            let running = rmcp::service::serve_directly(srv.clone(), server_side, None);
            let mut meta = rmcp::model::Meta::new();
            meta.0.insert(
                crate::knowledge::tier::CAPABILITY_TIER_META_KEY.to_string(),
                serde_json::Value::String(
                    crate::knowledge::tier::capability_meta_value(caller.is_private()).to_string(),
                ),
            );
            let context = RequestContext {
                ct: Default::default(),
                id: rmcp::model::NumberOrString::Number(1),
                meta,
                extensions: Default::default(),
                peer: running.peer().clone(),
            };
            let request = rmcp::model::CallToolRequestParams {
                name: name.to_string().into(),
                arguments: args.as_object().cloned(),
                task: None,
                meta: None,
            };
            let out = ServerHandler::call_tool(srv, request, context).await;
            drop(running);
            out
        }

        /// Everything a call said, whether it answered or refused — one string,
        /// so a leak assertion cannot be satisfied by the payload moving from the
        /// success branch to the error branch. The drafter's leaks are in
        /// `INVALID_PARAMS` messages, so the error branch is the important half.
        fn rendered(out: &Result<CallToolResult, ErrorData>) -> String {
            match out {
                Ok(r) => r
                    .content
                    .iter()
                    .filter_map(|c| match &c.raw {
                        RawContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(e) => e.message.to_string(),
            }
        }

        #[tokio::test]
        async fn list_platform_catalog_is_scoped_to_the_calling_sessions_capability() {
            let (srv, root, _apps, _kdir, _env) = drafter_at_root_with_kbs(&["default", "omop"]);
            tier::raise_unlocked(&root, "omop", true).unwrap();

            let public =
                call_drafter_tool_as(&srv, "list_platform_catalog", json!({}), Public).await;
            assert!(!rendered(&public).contains("omop"), "{}", rendered(&public));
            assert!(
                rendered(&public).contains("default"),
                "the public bases went too: {}",
                rendered(&public)
            );
            let private =
                call_drafter_tool_as(&srv, "list_platform_catalog", json!({}), Private).await;
            assert!(rendered(&private).contains("omop"));
        }

        #[tokio::test]
        async fn every_drafter_tool_that_builds_a_catalog_scopes_it() {
            // Parameterised, for the same reason 10B/10C parameterise over all
            // nineteen `kb_*` tools: fixing the tool whose NAME says "catalog"
            // leaves the validators enumerating the same list through their
            // error strings, which is the surface a model reads on every
            // rejected `configure_app`.
            let (srv, root, _apps, _kdir, _env) = drafter_at_root_with_kbs(&["default", "omop"]);
            tier::raise_unlocked(&root, "omop", true).unwrap();

            // A real app for the four id-addressing tools to reach validation on.
            let mut seed = create("Probe", None);
            seed.system_prompt = Some("help".into());
            seed.html = Some("<html><head></head><body>hi</body></html>".into());
            srv.create_app_inner(seed, None, &KbCaller::restricted())
                .await
                .unwrap();
            let manifest_json = srv.store().read_file("probe", "manifest.json").unwrap();
            let mut manifest: serde_json::Value = serde_json::from_str(&manifest_json).unwrap();
            manifest["agent"]["knowledge_base"] = json!("br.kb");

            // `br.kb` is the client API namespace the 100-app test drive really
            // configured, so every row below is the live rejection path.
            let rows: Vec<(&str, serde_json::Value)> = vec![
                ("list_platform_catalog", json!({})),
                (
                    "create_app",
                    json!({ "title": "Second", "knowledge_base": "br.kb" }),
                ),
                (
                    "configure_app",
                    json!({ "id": "probe", "knowledge_base": "br.kb" }),
                ),
                (
                    "update_app",
                    json!({
                        "id": "probe",
                        "path": "manifest.json",
                        "content": serde_json::to_string(&manifest).unwrap(),
                    }),
                ),
                // `declare_profiles` renders SKILL ids, not KB ids, so this row
                // cannot fail today — it is here because it CONSTRUCTS a catalog
                // (`:2544`), which is what makes it able to name a base the day
                // anyone adds a KB field to a worker profile. A row that goes red
                // then is the point.
                (
                    "declare_profiles",
                    json!({
                        "id": "probe",
                        "agents": [{
                            "key": "analyst",
                            "system_prompt": "analyse",
                            "skills": ["no-such-skill"],
                        }],
                    }),
                ),
            ];

            for (tool, args) in rows {
                let out = call_drafter_tool_as(&srv, tool, args, Public).await;
                assert!(
                    !rendered(&out).contains("omop"),
                    "{tool} leaked a private base id: {}",
                    rendered(&out)
                );
            }
        }

        #[tokio::test]
        async fn every_drafter_tool_that_can_name_a_base_is_in_the_register() {
            // The register, as a test rather than only as a table: no drafter
            // tool may produce a base id it was not given. Universal over the
            // WHOLE router, not over a hand-picked list — the leak this task
            // exists to close was a tool nobody had enumerated. Arguments name
            // only a base that does not exist here, so a hit is volunteering and
            // not echoing, and every validator takes its ENUMERATING branch.
            let (srv, root, _apps, _kdir, _env) =
                drafter_at_root_with_kbs(&["default", "omop-cohort-412"]);
            tier::raise_unlocked(&root, "omop-cohort-412", true).unwrap();

            // REAL apps for the id-addressing tools to address. Review caught
            // this fixture pointing every one of them at `no-such-app`, which
            // made sixteen of the eighteen rows bail at `no app 'no-such-app'`
            // before any catalog existed — a pass no implementation could fail.
            // See `benign_args_for` for what each row now reaches.
            seed_app(&srv, "probe", "Probe").await;
            seed_app(&srv, "disposable", "Disposable").await;
            let exports = TempDir::new().unwrap();
            let probe_manifest = srv.store().read_file("probe", "manifest.json").unwrap();

            for tool in AgentDrafterServer::tool_router().list_all() {
                let args = benign_args_for(&tool.name, exports.path(), &probe_manifest);
                let out = call_drafter_tool_as(&srv, &tool.name, args, Public).await;
                let said = rendered(&out);
                assert!(
                    !said.contains("omop-cohort-412"),
                    "{} volunteered a private base id; add it to the metadata register \
                     or scope it: {said}",
                    tool.name,
                );

                // The fixture polices ITSELF, because the way this test failed
                // review was not a wrong assertion but an inert one: pointed at
                // `no-such-app`, sixteen of eighteen rows never reached a line
                // that could leak, and the docstring still claimed the whole
                // router. A row that stops at the "no app" guard proves nothing,
                // so only the three that must not spawn a process may.
                let held = said.contains("no app '");
                assert_eq!(
                    held,
                    HELD_AT_THE_ID_GUARD.contains(&tool.name.as_ref()),
                    "{} is in the wrong half of the fixture (held at the id guard: \
                     {held}). Point it at the seeded `probe` app so its row asserts \
                     something, or add it to HELD_AT_THE_ID_GUARD with the reason: \
                     {said}",
                    tool.name,
                );
            }

            // …and a private caller still sees it, so the assertion above cannot
            // be satisfied by a router that answers nothing.
            let out = call_drafter_tool_as(&srv, "list_platform_catalog", json!({}), Private).await;
            assert!(rendered(&out).contains("omop-cohort-412"));
        }

        /// A real app for the register probe to address, so an id-addressing
        /// tool reaches its body instead of its "no app" guard.
        async fn seed_app(srv: &AgentDrafterServer, id: &str, title: &str) {
            let mut p = create(title, None);
            p.id = Some(id.to_string());
            // Agentic, because `persist_created_app` returns before the catalog
            // for any other kind (`:1088`) — a static seed would leave the very
            // tools this fixture exists to reach unable to build one.
            p.system_prompt = Some("help".into());
            p.html = Some("<html><head></head><body>hi</body></html>".into());
            srv.create_app_inner(p, None, &KbCaller::restricted())
                .await
                .unwrap();
        }

        /// An id no base on this machine has.
        ///
        /// Naming the PUBLIC base — which this fixture did until review — routes
        /// every row down its SUCCESS path, where no validator ever renders a
        /// list. The drafter's leak lives in the rejection strings
        /// (`validate.rs:33/:42/:52`), so the probe has to be rejected to reach
        /// it. It is also not the private id, so a hit is still volunteering
        /// rather than echoing. `br.kb` is the client API namespace the 100-app
        /// test drive really configured, so this is the live mistake.
        const ABSENT_KB: &str = "br.kb";

        /// The only rows allowed to stop at their `no app '<id>'` guard, because
        /// reaching their bodies means spawning a process this test must not
        /// spawn. Asserted in both directions, so neither half can drift.
        const HELD_AT_THE_ID_GUARD: [&str; 3] = ["build_app", "launch_app", "smoke_app"];

        /// Arguments that drive each tool as deep into its body as it can go
        /// without spawning a process or destroying the fixture.
        ///
        /// **Every id-addressing tool is pointed at a REAL app.** Each one's
        /// first statement is `load_manifest` / `store.exists` / `artifact_dir`
        /// and returns `no app '<id>'` on an unknown id — so the previous
        /// `"no-such-app"` meant sixteen of eighteen rows asserted only that
        /// `no app 'no-such-app'` does not contain a private base id. That is
        /// precisely the leak shape this task closes: `configure_app` (`:2168`),
        /// `update_app` (`:2310`) and `declare_profiles` (`:2623`) all build
        /// their catalog AFTER the manifest loads.
        ///
        /// **Three rows stay at the id guard, named with the reason:**
        /// `build_app` and `launch_app` run esbuild — and `find_esbuild` falls
        /// back to `npx --yes esbuild`, which DOWNLOADS — and `smoke_app` boots
        /// a browser. None of the three constructs a `Catalog` (the six
        /// production `Catalog::discover` sites are `create_app`,
        /// `configure_app`, `update_app`, `declare_profiles`,
        /// `list_platform_catalog` and `routes/apps.rs`), so what the register
        /// covers for them is the shallow row — stated here rather than implied
        /// by a docstring that claims the whole router.
        ///
        /// `delete_app` gets its OWN app, so it cannot destroy the shared
        /// fixture whatever order `list_all()` returns; `export_app` gets a real
        /// temp `target_dir`, so it writes into the fixture instead of at `/`;
        /// `create_app` gets its own id for the same ordering reason.
        fn benign_args_for(
            tool: &str,
            export_root: &std::path::Path,
            probe_manifest: &str,
        ) -> serde_json::Value {
            match tool {
                "list_platform_catalog" | "list_apps" => json!({}),
                "create_app" => json!({
                    "title": "Register Probe",
                    "id": "register-probe",
                    "system_prompt": "help",
                    "knowledge_base": ABSENT_KB,
                }),
                "build_app" | "launch_app" | "smoke_app" => json!({ "id": "no-such-app" }),
                "delete_app" => json!({ "id": "disposable" }),
                "export_app" => json!({
                    "id": "probe",
                    "target_dir": export_root.join("out").to_string_lossy(),
                }),
                // The `manifest.json` branch is the one that re-runs the
                // write-boundary check, so this row carries a real manifest
                // naming `ABSENT_KB` rather than the fallback's `"x"` — without
                // it `update_app` writes a file and never reaches `:2310`.
                "update_app" => json!({
                    "id": "probe",
                    "path": "manifest.json",
                    "content": manifest_naming_kb(probe_manifest, ABSENT_KB),
                }),
                _ => json!({
                    "id": "probe",
                    "knowledge_base": ABSENT_KB,
                    // A profile with an unresolvable skill, so `declare_profiles`
                    // reaches `validate::check_all` and renders a list; an empty
                    // `agents` skips the loop entirely.
                    "agents": [{
                        "key": "analyst",
                        "system_prompt": "analyse",
                        "skills": ["no-such-skill"],
                    }],
                    "routes": [],
                    // `surface` and `theme` are REQUIRED fields, not defaulted —
                    // without them `declare_surface` and `set_theme` fail at
                    // param decode, which is shallower still than the id guard.
                    "surface": {},
                    "theme": { "pack": "parchment" },
                    "path": "index.html",
                    "content": "x",
                }),
            }
        }

        /// The probe app's own manifest, re-pointed at `kb`, so `update_app`'s
        /// manifest branch validates a knowledge base instead of failing to
        /// parse.
        fn manifest_naming_kb(manifest_json: &str, kb: &str) -> String {
            let mut m: serde_json::Value = serde_json::from_str(manifest_json).unwrap();
            m["agent"]["knowledge_base"] = json!(kb);
            serde_json::to_string(&m).unwrap()
        }
    }
}
