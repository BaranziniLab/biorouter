use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
use crate::session::extension_data;
use crate::session::extension_data::{ExpandError, ExtensionState, TodoState, TodoStatus};
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ProtocolVersion, ServerCapabilities, Tool, ToolAnnotations, ToolsCapability,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "todo";

/// The Todo tools as the model calls them: the extension key, `__`, the tool.
///
/// Spelled out rather than matched by a `todo__` prefix, because the prefix is
/// not this builtin's to own: a user who disables the capability and installs
/// an MCP server keyed `todo` would otherwise inherit every exemption these
/// names carry. `the_todo_tool_names_are_the_tools_this_extension_lists` pins
/// the list to [`TodoClient::get_tools`], so a sixth tool cannot ship unnamed.
pub const TODO_TOOL_NAMES: [&str; 5] = [
    "todo__todo_write",
    "todo__todo_add",
    "todo__todo_expand",
    "todo__todo_update",
    "todo__plan_write",
];

/// The tool that seeds a checklist, and the one the planning gate points a
/// multi-step turn at (`agents::planning_gate`).
pub const TODO_WRITE_TOOL_NAME: &str = "todo__todo_write";

/// Is `tool_name` one of this capability's tools, as the model calls it?
pub fn is_todo_tool_name(tool_name: &str) -> bool {
    TODO_TOOL_NAMES.contains(&tool_name)
}

/// Default cap on the number of items a checklist may hold (per session).
const DEFAULT_MAX_ITEMS: usize = 200;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoWriteParams {
    /// The full checklist as markdown. One item per line, e.g.
    /// `- [ ] task`, `- [~] in progress`, `- [!] blocked`, `- [x] done`.
    /// Discards every existing item and renumbers the ids — use
    /// `todo_expand`/`todo_add`/`todo_update` to change an existing list.
    content: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoAddParams {
    /// New tasks. Each becomes a pending item with a fresh id.
    items: Vec<String>,
    /// Insert directly after this item's `#N` id instead of at the end of the
    /// list. Existing ids are never renumbered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoExpandParams {
    /// The `#N` id of the coarse item that turned out to be several steps.
    id: String,
    /// The concrete steps replacing it, in order. Each becomes a pending item.
    items: Vec<String>,
    /// Keep the original item as a grouping row with the new steps nested under
    /// it (the default). `false` removes it and the steps take its place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keep_parent: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoUpdateParams {
    /// The id (the `#N` shown in the checklist) of the item to update.
    id: String,
    /// New status: `pending`, `in_progress`, `blocked`, or `completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    /// Optional replacement text for the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PlanWriteParams {
    /// The living plan: a maintained, step-by-step plan you keep updated as you
    /// work. Pass an empty string to clear it.
    plan: String,
}

pub struct TodoClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
    /// Serializes [`TodoClient::with_state`]'s read-modify-write.
    ///
    /// H6: until the client-wide `McpClientBox` mutex was removed, every
    /// `call_tool` on this extension was serialized for free, which is what made
    /// `with_state` safe. It loads the session's `extension_data`, mutates it,
    /// and writes the WHOLE blob back — with two `.await` points in between, so
    /// two concurrent `todo_add`s could both read `{a}`, then write `{a,x}` and
    /// `{a,y}`: a lost update. Now that tool calls on one extension overlap,
    /// this extension carries its own lock instead. It is deliberately narrow —
    /// it guards only the session-metadata round trip (sub-millisecond, local),
    /// not the MCP dispatch path, so it costs nothing and cannot reintroduce H6.
    state_lock: tokio::sync::Mutex<()>,
}

impl TodoClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tasks: None,
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
            },
            server_info: Implementation {
                name: EXTENSION_NAME.to_string(),
                title: Some("Todo".to_string()),
                version: "1.1.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                indoc! {r#"
                Your plan and todo checklist are automatically re-injected into
                your context each turn, so you don't need to repeat them.

                Structured, per-item workflow:
                - Start: `plan_write` the approach, then `todo_write` the initial
                  checklist (one `- [ ]` line per task).
                - During: `todo_update` a single item's status as you go
                  (`in_progress` when you pick it up, `completed` when done).
                - When you reach a broad item that turns out to be several
                  steps, `todo_expand` it into the concrete steps before you
                  start it. Do not rewrite the list, and do not append the steps
                  to the end — that puts them after work they come before.
                - `todo_add` with `after` inserts mid-list; without it, appends.
                - When an item is stuck on something outside your control, set
                  its status to `blocked` rather than writing "BLOCKED" into its
                  text.
                - End: verify every item is `completed` (or explain why not).

                Statuses: pending, in_progress, blocked, completed. Items are
                addressed by the `#N` id shown next to each line, and those ids
                are stable — only `todo_write` renumbers them, which is the
                reason the incremental tools exist.
            "#}
                .to_string(),
            ),
        };

        Ok(Self {
            info,
            context,
            state_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// Load the session's todo state (migrating any legacy blob), let `f`
    /// mutate it, then persist. Returns `f`'s success message.
    async fn with_state<F>(&self, session_id: &str, f: F) -> Result<String, String>
    where
        F: FnOnce(&mut TodoState) -> Result<String, String>,
    {
        // Held across the whole read-modify-write; see `state_lock`.
        let _state_guard = self.state_lock.lock().await;

        let manager = &self.context.session_manager;
        let mut session = manager
            .get_session(session_id, false)
            .await
            .map_err(|_| "Failed to read session metadata".to_string())?;

        // ⚠ Never `TodoState::load(..).unwrap_or_default()` here. `load` reports
        // a blob this build cannot parse as `None`, and the write below replaces
        // the WHOLE `todo.v1` key — so one `todo_add` against a checklist a
        // newer build wrote would silently destroy it. Refuse instead: the blob
        // stays exactly where it is.
        let mut state = TodoState::try_load(&session.extension_data)
            .map_err(|unreadable| {
                format!(
                    "This session's stored checklist (`{}`) was written by a newer build of \
                     Biorouter and cannot be read here. Leaving it untouched rather than \
                     overwriting it — update Biorouter to change this checklist.",
                    unreadable.key
                )
            })?
            .unwrap_or_default();
        let message = f(&mut state)?;

        state
            .to_extension_data(&mut session.extension_data)
            .map_err(|_| "Failed to serialize TODO state".to_string())?;

        manager
            .update(session_id)
            .extension_data(session.extension_data)
            .apply()
            .await
            .map_err(|_| "Failed to update session metadata".to_string())?;

        Ok(message)
    }

    fn max_items() -> usize {
        std::env::var("BIOROUTER_TODO_MAX_ITEMS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MAX_ITEMS)
    }

    async fn handle_write(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let content = string_arg(&arguments, "content")?;

        let char_count = content.chars().count();
        let max_chars = std::env::var("BIOROUTER_TODO_MAX_CHARS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50_000);
        if max_chars > 0 && char_count > max_chars {
            return Err(format!(
                "Todo list too large: {} chars (max: {})",
                char_count, max_chars
            ));
        }

        let max_items = Self::max_items();
        let message = self
            .with_state(session_id, move |state| {
                state.set_from_markdown(&content);
                if max_items > 0 && state.items.len() > max_items {
                    return Err(format!(
                        "Todo list too long: {} items (max: {})",
                        state.items.len(),
                        max_items
                    ));
                }
                Ok(format!("Todo list set: {} item(s)", state.items.len()))
            })
            .await?;
        Ok(vec![Content::text(message)])
    }

    async fn handle_add(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let items = item_texts_arg(&arguments, "items")?;
        if items.is_empty() {
            return Err("items is empty; pass at least one task to add".to_string());
        }
        let after = match arguments.as_ref().and_then(|args| args.get("after")) {
            Some(value) if !value.is_null() => Some(item_id_value("after", value)?),
            _ => None,
        };
        let max_items = Self::max_items();
        let message = self
            .with_state(session_id, move |state| {
                let ids = match after.as_deref() {
                    Some(anchor) => state
                        .add_items_after(items, anchor)
                        .ok_or_else(|| unknown_id_message(state, anchor))?,
                    None => state.add_items(items),
                };
                if max_items > 0 && state.items.len() > max_items {
                    return Err(format!(
                        "Todo list too long: {} items (max: {})",
                        state.items.len(),
                        max_items
                    ));
                }
                let placement = match after.as_deref() {
                    Some(anchor) => format!(" after #{anchor}"),
                    None => String::new(),
                };
                Ok(format!(
                    "Added {} item(s){placement}: #{}",
                    ids.len(),
                    ids.join(", #")
                ))
            })
            .await?;
        Ok(vec![Content::text(message)])
    }

    async fn handle_expand(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.as_ref().ok_or("Missing arguments")?;
        let id = item_id_arg(args, "id")?;
        let items = item_texts_arg(&arguments, "items")?;
        if items.is_empty() {
            return Err("items is empty; pass the steps that replace this item".to_string());
        }
        // Keeping the coarse item as a heading is the readable default; the plan
        // still shows the shape the user agreed to.
        let keep_parent = match args.get("keep_parent") {
            None => true,
            Some(value) if value.is_null() => true,
            Some(value) => bool_value("keep_parent", value)?,
        };

        let max_items = Self::max_items();
        let message = self
            .with_state(session_id, move |state| {
                let expanded = state
                    .expand_item(&id, items, keep_parent)
                    .map_err(|error| expand_error_message(state, &id, error))?;
                if max_items > 0 && state.items.len() > max_items {
                    return Err(format!(
                        "Todo list too long: {} items (max: {})",
                        state.items.len(),
                        max_items
                    ));
                }
                let mut message = format!(
                    "Expanded #{id} into {} step(s) in place: #{}",
                    expanded.ids.len(),
                    expanded.ids.join(", #")
                );
                if expanded.kept_parent {
                    message.push_str(&format!(" (nested under #{id})"));
                } else if let Some(parent) = expanded.nested_under.as_deref() {
                    message.push_str(&format!(" (as siblings under #{parent}; #{id} removed"));
                    // Explain the shape only when the caller asked for a level
                    // it cannot have; saying it after an explicit
                    // `keep_parent: false` would imply a constraint was applied
                    // when the caller got exactly what it requested.
                    if keep_parent {
                        message.push_str(" — the checklist nests one level only");
                    }
                    message.push(')');
                } else {
                    message.push_str(&format!(" (#{id} removed)"));
                }
                Ok(serde_json::json!({
                    "message": message,
                    "ids": expanded.ids,
                    "checklist": state.render(),
                })
                .to_string())
            })
            .await?;
        Ok(vec![Content::text(message)])
    }

    async fn handle_update(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.as_ref().ok_or("Missing arguments")?;
        let id = item_id_arg(args, "id")?;

        let status = match args.get("status").and_then(|v| v.as_str()) {
            Some(raw) => Some(TodoStatus::parse(raw).ok_or_else(|| {
                format!("Unknown status: {raw} (use pending/in_progress/blocked/completed)")
            })?),
            None => None,
        };
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if status.is_none() && text.is_none() {
            return Err("Provide a new status and/or text to update".to_string());
        }

        let message = self
            .with_state(session_id, move |state| {
                if state.update_item(&id, status, text) {
                    Ok(serde_json::json!({
                        "message": format!("Updated item #{id}"),
                        "task": state.items.iter().find(|item| item.id == id),
                    })
                    .to_string())
                } else {
                    Err(unknown_id_message(state, &id))
                }
            })
            .await?;
        Ok(vec![Content::text(message)])
    }

    async fn handle_plan_write(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let plan = string_arg(&arguments, "plan")?;

        let char_count = plan.chars().count();
        let max_chars = std::env::var("BIOROUTER_TODO_MAX_CHARS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50_000);
        if max_chars > 0 && char_count > max_chars {
            return Err(format!(
                "Plan too large: {} chars (max: {})",
                char_count, max_chars
            ));
        }

        let message = self
            .with_state(session_id, move |state| {
                let trimmed = plan.trim();
                if trimmed.is_empty() {
                    state.plan = None;
                    Ok("Plan cleared".to_string())
                } else {
                    state.plan = Some(trimmed.to_string());
                    Ok("Plan updated".to_string())
                }
            })
            .await?;
        Ok(vec![Content::text(message)])
    }

    fn get_tools() -> Vec<Tool> {
        vec![
            tool_from_schema::<TodoWriteParams>(
                "todo_write",
                indoc! {r#"
                    Replace the ENTIRE checklist. Seeding a new list is what this
                    is for; changing an existing one is not.

                    It discards every current item and renumbers the `#N` ids, so
                    any id you were tracking shifts underneath you and a
                    completed item survives only if you re-emit it here from
                    memory. To change a list that already exists, use the
                    incremental tools, which keep ids stable:
                      - `todo_expand` — one coarse item becomes several concrete
                        steps, in place, in the right position.
                      - `todo_add` — append, or insert after a given item.
                      - `todo_update` — flip one item's status or text.

                    Format: one item per line — `- [ ] task`,
                    `- [~] in progress`, `- [!] blocked`, `- [x] done`.
                "#},
                true,
            ),
            tool_from_schema::<TodoExpandParams>(
                "todo_expand",
                indoc! {r#"
                    Break one coarse checklist item into the concrete steps it
                    turned out to be, replacing it IN PLACE.

                    Reach for this the moment you realise an item is really
                    several steps — before you start it. The steps take that
                    item's position, so the list still reads in the order the
                    work happens, and no surrounding id is renumbered. By
                    default the original stays as a grouping row with the steps
                    nested under it; pass `keep_parent: false` to have them
                    replace it outright.

                    Do NOT use `todo_add` for this — it appends, so the steps
                    land after work they come before — and do NOT rewrite the
                    list with `todo_write`.
                "#},
                false,
            ),
            tool_from_schema::<TodoAddParams>(
                "todo_add",
                indoc! {r#"
                    Add new pending items without rewriting the existing ones.
                    Appends by default; pass `after` with an item's `#N` id to
                    insert directly after it. Each new item gets a fresh id and
                    no existing id changes.

                    If the new items are the breakdown of an item already on the
                    list, use `todo_expand` instead.
                "#},
                false,
            ),
            tool_from_schema::<TodoUpdateParams>(
                "todo_update",
                indoc! {r#"
                    Update a single checklist item by its `#N` id: change its
                    status (pending/in_progress/blocked/completed) and/or its
                    text, without touching the rest of the list.

                    Use `blocked` when the item is stuck on something outside
                    your control, rather than writing that into its text.
                "#},
                false,
            ),
            tool_from_schema::<PlanWriteParams>(
                "plan_write",
                indoc! {r#"
                    Set or update the living plan: a maintained, step-by-step
                    plan you keep current as you work. Re-injected into your
                    context each turn alongside the checklist. Empty string
                    clears it.
                "#},
                false,
            ),
        ]
    }
}

/// Read a required string argument.
fn string_arg(arguments: &Option<JsonObject>, key: &str) -> Result<String, String> {
    arguments
        .as_ref()
        .ok_or("Missing arguments")?
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Missing required parameter: {key}"))
        .map(|s| s.to_string())
}

/// Keys a model plausibly hangs an item's text off when it sends objects
/// instead of bare strings.
const ITEM_TEXT_KEYS: &[&str] = &["text", "task", "title"];

/// Describe a JSON value for an error message, so a rejection says what
/// actually arrived instead of only what was wanted.
fn describe_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => format!("the boolean {b}"),
        serde_json::Value::Number(n) => format!("the number {n}"),
        serde_json::Value::String(s) => format!("the string {s:?}"),
        serde_json::Value::Array(a) => format!("an array of {} element(s)", a.len()),
        serde_json::Value::Object(o) => format!(
            "an object with keys [{}]",
            o.keys().cloned().collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Read an item id, accepting the shapes models actually send: the displayed
/// `"#3"`, the bare `"3"`, and — just as commonly for a field named `id` — the
/// JSON number `3`.
///
/// The string-only read this replaced answered `{"id": 3}` with *"Missing
/// required parameter: id"*, which was false: the parameter was there, in
/// another JSON type. Leniency was already the intent here (the `#` prefix is
/// stripped) and matches `TodoStatus::parse`; it just never covered numbers.
fn item_id_value(key: &str, value: &serde_json::Value) -> Result<String, String> {
    let raw = match value {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Number(n) if n.is_u64() || n.is_i64() => n.to_string(),
        other => {
            return Err(format!(
                "{key} must be an item id like \"3\" or \"#3\"; got {}",
                describe_json(other)
            ))
        }
    };
    let id = raw.strip_prefix('#').unwrap_or(&raw).trim().to_string();
    if id.is_empty() {
        return Err(format!(
            "{key} must be an item id like \"3\" or \"#3\"; got an empty string"
        ));
    }
    Ok(id)
}

/// Read a required item-id argument.
fn item_id_arg(args: &JsonObject, key: &str) -> Result<String, String> {
    let value = args
        .get(key)
        .ok_or_else(|| format!("Missing required parameter: {key}"))?;
    item_id_value(key, value)
}

/// Read a boolean argument, tolerating the stringified form.
fn bool_value(key: &str, value: &serde_json::Value) -> Result<bool, String> {
    match value {
        serde_json::Value::Bool(b) => Ok(*b),
        serde_json::Value::String(s) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" => Ok(true),
            "false" | "no" => Ok(false),
            _ => Err(format!("{key} must be true or false; got {s:?}")),
        },
        other => Err(format!(
            "{key} must be true or false; got {}",
            describe_json(other)
        )),
    }
}

/// Read a required list of task texts, accepting both shapes models send: a
/// bare string, or an object carrying the text under `text`/`task`/`title`.
///
/// Nothing is ever dropped. The `filter_map(as_str)` this replaced silently
/// discarded every non-string element, so `items: [{"text": "…"}]` lost the
/// whole list and reported *"No non-empty items to add"* — which reads as "your
/// list was empty" rather than "wrong element shape". A dropped item is a task
/// the user believes is tracked and is not, so an unusable element is named by
/// index and refuses the call.
fn item_texts_arg(arguments: &Option<JsonObject>, key: &str) -> Result<Vec<String>, String> {
    let value = arguments
        .as_ref()
        .ok_or("Missing arguments")?
        .get(key)
        .ok_or_else(|| format!("Missing required parameter: {key}"))?;
    let array = value.as_array().ok_or_else(|| {
        format!(
            "Parameter {key} must be an array of strings; got {}",
            describe_json(value)
        )
    })?;

    let mut texts = Vec::with_capacity(array.len());
    for (index, element) in array.iter().enumerate() {
        let bad_shape = || {
            format!(
                "{key}[{index}] must be a string or {{text: …}}; got {}",
                describe_json(element)
            )
        };
        let text = match element {
            serde_json::Value::String(s) => s.trim(),
            serde_json::Value::Object(object) => ITEM_TEXT_KEYS
                .iter()
                .find_map(|k| object.get(*k))
                .and_then(|v| v.as_str())
                .ok_or_else(bad_shape)?
                .trim(),
            _ => return Err(bad_shape()),
        };
        if text.is_empty() {
            return Err(format!("{key}[{index}] is empty; every item needs text"));
        }
        texts.push(text.to_string());
    }
    Ok(texts)
}

/// A refusal that names the ids that DO exist, so the next call can be right.
fn unknown_id_message(state: &TodoState, id: &str) -> String {
    if state.items.is_empty() {
        return format!("No todo item with id #{id}; the checklist is empty");
    }
    let known = state
        .items
        .iter()
        .map(|item| format!("#{}", item.id))
        .collect::<Vec<_>>()
        .join(", ");
    format!("No todo item with id #{id}; the checklist has {known}")
}

fn expand_error_message(state: &TodoState, id: &str, error: ExpandError) -> String {
    match error {
        ExpandError::UnknownId => unknown_id_message(state, id),
        ExpandError::AlreadyCompleted => format!(
            "Item #{id} is already completed; expanding it would discard that. \
             Add the follow-up work with todo_add {{ after: \"{id}\" }} instead."
        ),
        ExpandError::AlreadyExpanded { children } => format!(
            "Item #{id} has already been expanded into #{}. Add to it with \
             todo_add {{ after: \"{}\" }}, or update those items.",
            children.join(", #"),
            children.last().map(String::as_str).unwrap_or(id)
        ),
        ExpandError::NoItems => {
            format!("No non-empty steps to expand #{id} into")
        }
    }
}

fn tool_from_schema<T: JsonSchema>(name: &str, description: &str, destructive: bool) -> Tool {
    let schema = schema_for!(T);
    let schema_value = serde_json::to_value(schema).expect("Failed to serialize schema");
    Tool::new(
        name.to_string(),
        description.to_string(),
        schema_value.as_object().unwrap().clone(),
    )
    .annotate(ToolAnnotations {
        title: Some(name.to_string()),
        read_only_hint: Some(false),
        destructive_hint: Some(destructive),
        idempotent_hint: Some(false),
        open_world_hint: Some(false),
    })
}

#[async_trait]
impl McpClientTrait for TodoClient {
    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
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
        let session_id = &meta.session_id;
        let content = match name {
            "todo_write" => self.handle_write(session_id, arguments).await,
            "todo_add" => self.handle_add(session_id, arguments).await,
            "todo_expand" => self.handle_expand(session_id, arguments).await,
            "todo_update" => self.handle_update(session_id, arguments).await,
            "plan_write" => self.handle_plan_write(session_id, arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };

        match content {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                error
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn get_moim(&self, session_id: &str) -> Option<String> {
        let metadata = self
            .context
            .session_manager
            .get_session(session_id, false)
            .await
            .ok()?;

        // Only the live plan/task state belongs here. The behavioural rule
        // lives in two places, neither of them this extension: system.md
        // states it, so it holds even without this extension (BR-4 / BR-60),
        // and the planning gate (`agents::planning_gate`) enforces it for a
        // multi-step turn — adding its own "write the checklist first" line to
        // the same MOIM block while this one has nothing to render.
        let state = extension_data::TodoState::load(&metadata.extension_data)?;
        if state.is_empty() {
            return None;
        }
        Some(format!("Current tasks and notes:\n{}", state.render()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::privacy::{CallCapability, ProviderTier};
    use crate::session::{SessionManager, SessionType};

    fn meta(session_id: &str) -> McpMeta {
        McpMeta::new(
            session_id,
            CallCapability::for_test(ProviderTier::Public, true),
        )
    }

    async fn call(
        client: &TodoClient,
        session_id: &str,
        name: &str,
        args: serde_json::Value,
    ) -> CallToolResult {
        client
            .call_tool(
                name,
                args.as_object().cloned(),
                meta(session_id),
                CancellationToken::default(),
            )
            .await
            .unwrap()
    }

    fn text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `TODO_TOOL_NAMES` carries exemptions (the Code Execution filter, the
    /// planning gate), so it must be exactly this extension's tools under the
    /// key the manager gives them — no more, no fewer.
    #[test]
    fn the_todo_tool_names_are_the_tools_this_extension_lists() {
        let key = crate::config::extensions::name_to_key(EXTENSION_NAME);
        let mut listed: Vec<String> = TodoClient::get_tools()
            .iter()
            .map(|tool| format!("{key}__{}", tool.name))
            .collect();
        listed.sort();
        let mut named: Vec<String> = TODO_TOOL_NAMES.iter().map(|n| n.to_string()).collect();
        named.sort();
        assert_eq!(listed, named);
        assert!(is_todo_tool_name(TODO_WRITE_TOOL_NAME));
        assert!(
            !is_todo_tool_name("todo_write"),
            "only the name the model calls"
        );
        assert!(!is_todo_tool_name("todo__something_else"));
    }

    #[tokio::test]
    async fn all_advertised_todo_tools_dispatch_and_reinject_their_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(temp.path().join("sessions")));
        let session = manager
            .create_session(
                temp.path().to_path_buf(),
                "todo dispatch".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        let client = TodoClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::clone(&manager),
        })
        .unwrap();

        let listed = client
            .list_tools(None, CancellationToken::default())
            .await
            .unwrap();
        let mut names = listed
            .tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "plan_write",
                "todo_add",
                "todo_expand",
                "todo_update",
                "todo_write"
            ]
        );

        assert!(text(
            &call(
                &client,
                &session.id,
                "plan_write",
                serde_json::json!({"plan": "Build, verify, report"}),
            )
            .await
        )
        .contains("Plan updated"));
        assert!(text(
            &call(
                &client,
                &session.id,
                "todo_write",
                serde_json::json!({"content": "- [ ] build\n- [ ] verify"}),
            )
            .await
        )
        .contains("2 item"));
        assert!(text(
            &call(
                &client,
                &session.id,
                "todo_add",
                serde_json::json!({"items": ["report"]}),
            )
            .await
        )
        .contains("#3"));
        assert!(text(
            &call(
                &client,
                &session.id,
                "todo_update",
                serde_json::json!({"id": "1", "status": "completed"}),
            )
            .await
        )
        .contains("#1"));

        let reinjected = client.get_moim(&session.id).await.unwrap();
        assert!(reinjected.contains("Build, verify, report"), "{reinjected}");
        assert!(reinjected.contains("build"), "{reinjected}");
        assert!(reinjected.contains("report"), "{reinjected}");
        assert!(reinjected.contains("[x]"), "{reinjected}");

        let invalid = call(
            &client,
            &session.id,
            "todo_update",
            serde_json::json!({"id": "999", "status": "completed"}),
        )
        .await;
        assert_eq!(invalid.is_error, Some(true));
        assert!(text(&invalid).contains("No todo item"));
    }

    #[tokio::test]
    async fn todo_update_accepts_the_displayed_hash_prefixed_id() {
        let temp = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(temp.path().join("sessions")));
        let session = manager
            .create_session(
                temp.path().to_path_buf(),
                "todo displayed id".into(),
                SessionType::User,
            )
            .await
            .unwrap();
        let client = TodoClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::clone(&manager),
        })
        .unwrap();

        call(
            &client,
            &session.id,
            "todo_write",
            serde_json::json!({"content": "- [ ] verify displayed id"}),
        )
        .await;
        let updated = call(
            &client,
            &session.id,
            "todo_update",
            serde_json::json!({"id": "#1", "status": "completed"}),
        )
        .await;

        assert_eq!(updated.is_error, Some(false), "{}", text(&updated));
        assert!(text(&updated).contains("Updated item #1"));
        let result: serde_json::Value = serde_json::from_str(&text(&updated)).unwrap();
        assert_eq!(result["task"]["id"], "1");
        assert_eq!(result["task"]["text"], "verify displayed id");
        assert_eq!(result["task"]["status"], "completed");
        let reinjected = client.get_moim(&session.id).await.unwrap();
        assert!(
            reinjected.contains("- [x] (#1) verify displayed id"),
            "{reinjected}"
        );
    }

    /// A fresh session + client; every tool test needs the pair.
    async fn client_and_session(temp: &tempfile::TempDir, name: &str) -> (TodoClient, String) {
        let manager = Arc::new(SessionManager::new(temp.path().join("sessions")));
        let session = manager
            .create_session(temp.path().to_path_buf(), name.into(), SessionType::User)
            .await
            .unwrap();
        let client = TodoClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::clone(&manager),
        })
        .unwrap();
        (client, session.id)
    }

    #[tokio::test]
    async fn todo_expand_replaces_a_coarse_item_in_place_and_keeps_the_other_ids() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo expand").await;

        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({
                "content": "- [ ] survey the inputs\n- [ ] set up the workspace\n\
                            - [ ] do the actual work\n- [ ] test and verify",
            }),
        )
        .await;
        call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"id": "1", "status": "completed"}),
        )
        .await;

        let expanded = call(
            &client,
            &id,
            "todo_expand",
            serde_json::json!({"id": "3", "items": ["write it", "wire it up"]}),
        )
        .await;
        assert_eq!(expanded.is_error, Some(false), "{}", text(&expanded));

        let reinjected = client.get_moim(&id).await.unwrap();
        // The steps land where #3 was — before "test and verify", which keeps
        // the id it always had — and the completed item is untouched.
        let checklist: Vec<&str> = reinjected
            .lines()
            .filter(|line| line.contains("(#"))
            .collect();
        assert_eq!(
            checklist,
            vec![
                "- [x] (#1) survey the inputs",
                "- [ ] (#2) set up the workspace",
                "- [ ] (#3) do the actual work",
                "  - [ ] (#5) write it",
                "  - [ ] (#6) wire it up",
                "- [ ] (#4) test and verify",
            ],
            "{reinjected}"
        );

        // The id the caller was tracking still addresses the same task.
        let updated = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"id": 4, "status": "in_progress"}),
        )
        .await;
        let result: serde_json::Value = serde_json::from_str(&text(&updated)).unwrap();
        assert_eq!(result["task"]["text"], "test and verify");
    }

    #[tokio::test]
    async fn todo_expand_refuses_completed_and_already_expanded_items_with_a_usable_message() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo expand refusals").await;

        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "- [x] shipped\n- [ ] coarse"}),
        )
        .await;

        let completed = call(
            &client,
            &id,
            "todo_expand",
            serde_json::json!({"id": "1", "items": ["a"]}),
        )
        .await;
        assert_eq!(completed.is_error, Some(true));
        assert!(
            text(&completed).contains("already completed"),
            "{}",
            text(&completed)
        );
        assert!(
            text(&completed).contains("todo_add"),
            "{}",
            text(&completed)
        );

        call(
            &client,
            &id,
            "todo_expand",
            serde_json::json!({"id": "2", "items": ["step one"]}),
        )
        .await;
        let again = call(
            &client,
            &id,
            "todo_expand",
            serde_json::json!({"id": "2", "items": ["step two"]}),
        )
        .await;
        assert_eq!(again.is_error, Some(true));
        assert!(
            text(&again).contains("already been expanded into #3"),
            "{}",
            text(&again)
        );

        // Nothing was lost to either refusal.
        let reinjected = client.get_moim(&id).await.unwrap();
        assert!(reinjected.contains("- [x] (#1) shipped"), "{reinjected}");
        assert!(reinjected.contains("  - [ ] (#3) step one"), "{reinjected}");
    }

    #[tokio::test]
    async fn todo_update_accepts_a_bare_number_id_and_names_an_unusable_one() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo lenient id").await;
        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "- [ ] one\n- [ ] two\n- [ ] three"}),
        )
        .await;

        // `3`, `"3"` and `"#3"` all address the same item. The number used to
        // answer "Missing required parameter: id", which was simply false.
        for (index, sent) in [
            serde_json::json!(3),
            serde_json::json!("3"),
            serde_json::json!("#3"),
        ]
        .into_iter()
        .enumerate()
        {
            let status = ["in_progress", "completed", "pending"][index];
            let result = call(
                &client,
                &id,
                "todo_update",
                serde_json::json!({"id": sent, "status": status}),
            )
            .await;
            assert_eq!(result.is_error, Some(false), "{}", text(&result));
            let parsed: serde_json::Value = serde_json::from_str(&text(&result)).unwrap();
            assert_eq!(parsed["task"]["id"], "3");
            assert_eq!(parsed["task"]["status"], status);
        }

        let bad = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"id": {"n": 3}, "status": "completed"}),
        )
        .await;
        assert_eq!(bad.is_error, Some(true));
        let message = text(&bad);
        assert!(!message.contains("Missing required parameter"), "{message}");
        assert!(message.contains("must be an item id"), "{message}");
        assert!(message.contains("an object with keys [n]"), "{message}");

        // An unknown id names what the list actually holds.
        let unknown = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"id": 99, "status": "completed"}),
        )
        .await;
        assert!(text(&unknown).contains("#1, #2, #3"), "{}", text(&unknown));
    }

    #[tokio::test]
    async fn todo_add_accepts_object_items_and_never_silently_drops_one() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo lenient items").await;

        let strings = call(
            &client,
            &id,
            "todo_add",
            serde_json::json!({"items": ["a"]}),
        )
        .await;
        assert_eq!(strings.is_error, Some(false), "{}", text(&strings));

        // `[{"text": …}]` used to lose the WHOLE list and report "No non-empty
        // items to add" — which reads as "your list was empty".
        for key in ["text", "task", "title"] {
            let objects = call(
                &client,
                &id,
                "todo_add",
                serde_json::json!({"items": [{key: format!("via {key}")}]}),
            )
            .await;
            assert_eq!(objects.is_error, Some(false), "{}", text(&objects));
        }

        let reinjected = client.get_moim(&id).await.unwrap();
        for expected in ["(#1) a", "(#2) via text", "(#3) via task", "(#4) via title"] {
            assert!(reinjected.contains(expected), "{reinjected}");
        }

        // A genuinely unusable element is named by index and refuses the call
        // rather than vanishing from a list the user believes is tracked.
        let bad = call(
            &client,
            &id,
            "todo_add",
            serde_json::json!({"items": ["fine", "also fine", {"foo": 1}]}),
        )
        .await;
        assert_eq!(bad.is_error, Some(true));
        let message = text(&bad);
        assert!(message.contains("items[2]"), "{message}");
        assert!(message.contains("an object with keys [foo]"), "{message}");
        assert!(!message.contains("No non-empty items"), "{message}");
        // The two good elements were not half-applied either.
        assert!(!client.get_moim(&id).await.unwrap().contains("also fine"));
    }

    #[tokio::test]
    async fn todo_add_after_inserts_mid_list_and_blocked_survives_the_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo add after").await;
        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "- [ ] one\n- [ ] two\n- [ ] three"}),
        )
        .await;

        let inserted = call(
            &client,
            &id,
            "todo_add",
            serde_json::json!({"items": ["one and a half"], "after": 1}),
        )
        .await;
        assert_eq!(inserted.is_error, Some(false), "{}", text(&inserted));
        assert!(text(&inserted).contains("after #1"), "{}", text(&inserted));

        let blocked = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"id": "4", "status": "waiting"}),
        )
        .await;
        assert_eq!(blocked.is_error, Some(false), "{}", text(&blocked));

        let checklist: Vec<String> = client
            .get_moim(&id)
            .await
            .unwrap()
            .lines()
            .filter(|line| line.contains("(#"))
            .map(str::to_string)
            .collect();
        assert_eq!(
            checklist,
            vec![
                "- [ ] (#1) one",
                "- [!] (#4) one and a half",
                "- [ ] (#2) two",
                "- [ ] (#3) three",
            ]
        );

        // An unknown anchor is refused, not quietly appended.
        let bad_anchor = call(
            &client,
            &id,
            "todo_add",
            serde_json::json!({"items": ["nope"], "after": "99"}),
        )
        .await;
        assert_eq!(bad_anchor.is_error, Some(true));
        assert!(!client.get_moim(&id).await.unwrap().contains("nope"));
    }

    #[tokio::test]
    async fn a_checklist_this_build_cannot_parse_is_refused_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "unreadable checklist").await;

        // A `todo.v1` blob carrying a status word only a NEWER build knows —
        // the shape this build was destroying, because an unparseable blob and
        // an absent one both arrived as `None` and were then written over.
        let future = serde_json::json!({
            "items": [{"id": "1", "text": "the user's real checklist", "status": "deferred"}],
            "plan": "the user's real plan"
        });
        let manager = &client.context.session_manager;
        let mut session = manager.get_session(&id, false).await.unwrap();
        session
            .extension_data
            .set_extension_state("todo", "v1", future.clone());
        manager
            .update(&id)
            .extension_data(session.extension_data)
            .apply()
            .await
            .unwrap();

        for (tool, args) in [
            ("todo_add", serde_json::json!({"items": ["something new"]})),
            (
                "todo_write",
                serde_json::json!({"content": "- [ ] wipe it"}),
            ),
            (
                "todo_update",
                serde_json::json!({"id": "1", "status": "completed"}),
            ),
            ("plan_write", serde_json::json!({"plan": "a new plan"})),
        ] {
            let refused = call(&client, &id, tool, args).await;
            assert_eq!(refused.is_error, Some(true), "{tool}: {}", text(&refused));
            assert!(
                text(&refused).contains("newer build"),
                "{tool}: {}",
                text(&refused)
            );

            // The whole point: the stored blob is untouched after the refusal.
            let after = manager.get_session(&id, false).await.unwrap();
            assert_eq!(
                after.extension_data.get_extension_state("todo", "v1"),
                Some(&future),
                "{tool} overwrote a checklist it could not read"
            );
        }
    }
}
