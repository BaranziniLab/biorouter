use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
use crate::session::extension_data;
use crate::session::extension_data::{
    ExpandError, ExtensionState, TodoState, TodoStatus, MOIM_PLAN_CHARS,
};
use crate::session::SessionManager;
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ProtocolVersion, Role, ServerCapabilities, Tool, ToolAnnotations, ToolsCapability,
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

// ---------------------------------------------------------------------------
// The ride-along sidecar
// ---------------------------------------------------------------------------

/// The argument the host adds to a workhorse tool's schema so checklist
/// progress rides on a call the model is making anyway.
///
/// Why it exists: the model emits one tool call per message. In the chat that
/// motivated this, 239 of 575 requests (38% of all prompt tokens) were a todo
/// call made on its own, each one re-sending a ~235k-token conversation to flip
/// one checkbox. Prompt text asking for batching did not change that; a field
/// on the next real call removes the request instead of asking the model to.
pub const TODO_SIDECAR_FIELD: &str = "todo";

/// The tools whose schema carries [`TODO_SIDECAR_FIELD`]: the ones a model
/// actually does its work with. Exact model-facing names, for the same reason
/// as [`TODO_TOOL_NAMES`]. Kept small on purpose: every listed schema grows by
/// the field's description in every request, and a field on a tool the model
/// rarely calls buys nothing.
pub const TODO_SIDECAR_TOOLS: [&str; 3] = [
    "code_execution__execute_code",
    "developer__shell",
    "developer__text_editor",
];

/// Does `tool_name`'s schema carry the sidecar? The host strips the field from
/// such a call before dispatch ([`take_todo_sidecar`]) and applies it after
/// ([`apply_todo_sidecar`]).
pub fn tool_takes_todo_sidecar(tool_name: &str) -> bool {
    TODO_SIDECAR_TOOLS.contains(&tool_name)
}

/// The JSON schema of [`TODO_SIDECAR_FIELD`], one line of description and
/// five optional keys.
pub fn todo_sidecar_schema() -> serde_json::Value {
    let ids = serde_json::json!({
        "type": "array",
        "items": {"type": ["integer", "string"]}
    });
    serde_json::json!({
        "type": "object",
        "description": "Optional: checklist updates to apply with this call, e.g. \
            {\"done\":[3,4],\"add\":[\"write the report\"]}. done/blocked: item ids; \
            add: new items; plan: replaces the plan. The next open item becomes current \
            on its own. Prefer this to a separate todo call.",
        "properties": {
            "done": ids.clone(),
            "blocked": ids,
            "start": {"type": ["integer", "string"]},
            "add": {"type": "array", "items": {"type": "string"}},
            "plan": {"type": "string"}
        }
    })
}

/// Remove a host-injected [`TODO_SIDECAR_FIELD`] from a tool call's arguments
/// and return it, so the tool itself never sees an argument it did not
/// declare. `None` when the field is absent or `null`.
pub fn take_todo_sidecar(arguments: &mut Option<JsonObject>) -> Option<serde_json::Value> {
    arguments
        .as_mut()?
        .remove(TODO_SIDECAR_FIELD)
        .filter(|value| !value.is_null())
}

/// One parsed sidecar.
#[derive(Debug, Default, PartialEq, Eq)]
struct Sidecar {
    done: Vec<String>,
    blocked: Vec<String>,
    start: Option<String>,
    add: Vec<String>,
    plan: Option<String>,
}

impl Sidecar {
    fn is_empty(&self) -> bool {
        self.done.is_empty()
            && self.blocked.is_empty()
            && self.start.is_none()
            && self.add.is_empty()
            && self.plan.is_none()
    }
}

/// Read a sidecar, accepting what models send: the object, the object
/// stringified, a lone id where a list is expected, and a few key aliases.
/// An unknown key is refused rather than ignored, so a misspelling is logged
/// instead of silently doing nothing.
fn parse_sidecar(value: &serde_json::Value) -> Result<Sidecar, String> {
    let parsed;
    let value = match value {
        serde_json::Value::String(raw) => {
            parsed = serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|_| format!("todo must be an object; got {}", describe_json(value)))?;
            &parsed
        }
        other => other,
    };
    let object = value
        .as_object()
        .ok_or_else(|| format!("todo must be an object; got {}", describe_json(value)))?;
    let mut sidecar = Sidecar::default();
    for (key, entry) in object {
        if entry.is_null() {
            continue;
        }
        match key.as_str() {
            "done" | "completed" | "complete" => sidecar.done.extend(sidecar_ids(key, entry)?),
            "blocked" => sidecar.blocked.extend(sidecar_ids(key, entry)?),
            "start" | "current" | "in_progress" => {
                sidecar.start = Some(match entry {
                    serde_json::Value::Array(list) if list.len() == 1 => {
                        item_id_value(key, &list[0])?
                    }
                    one => item_id_value(key, one)?,
                })
            }
            "add" => {
                let wrapped = Some(JsonObject::from_iter([(key.clone(), entry.clone())]));
                let texts = match entry {
                    serde_json::Value::String(text) => vec![text.trim().to_string()],
                    _ => item_texts_arg(&wrapped, key)?,
                };
                sidecar
                    .add
                    .extend(texts.into_iter().filter(|text| !text.is_empty()));
            }
            "plan" => {
                sidecar.plan = Some(
                    entry
                        .as_str()
                        .ok_or_else(|| {
                            format!("todo.plan must be a string; got {}", describe_json(entry))
                        })?
                        .to_string(),
                )
            }
            other => {
                return Err(format!(
                    "todo has an unknown key `{other}` (use done, blocked, start, add, plan)"
                ))
            }
        }
    }
    Ok(sidecar)
}

/// A sidecar id list: an array of ids, or one id on its own.
fn sidecar_ids(key: &str, value: &serde_json::Value) -> Result<Vec<String>, String> {
    match value {
        serde_json::Value::Array(list) => list
            .iter()
            .enumerate()
            .map(|(index, id)| item_id_value(&format!("todo.{key}[{index}]"), id))
            .collect(),
        one => Ok(vec![item_id_value(&format!("todo.{key}"), one)?]),
    }
}

/// Apply a sidecar taken with [`take_todo_sidecar`] to the chat's checklist.
///
/// Never fails the tool call it rode on: the caller logs an `Err` (and may
/// show its sentence to the model). Valid parts are applied even when some
/// are not, because the model gets no separate answer for a sidecar and would
/// otherwise lose every change for one stale id; what was skipped is named in
/// the error. A malformed sidecar (wrong shape, unknown key) changes nothing.
/// A checklist this build cannot read is left untouched, as in every todo
/// tool.
pub async fn apply_todo_sidecar(
    session_manager: &SessionManager,
    session_id: &str,
    sidecar: serde_json::Value,
) -> anyhow::Result<()> {
    let sidecar = parse_sidecar(&sidecar).map_err(|reason| anyhow::anyhow!(reason))?;
    if sidecar.is_empty() {
        return Ok(());
    }
    let max_items = TodoClient::max_items();
    let plan_chars = sidecar.plan.as_deref().map(|plan| plan.chars().count());
    let max_chars = TodoClient::max_chars();
    if let Some(chars) = plan_chars.filter(|chars| max_chars > 0 && *chars > max_chars) {
        anyhow::bail!("todo.plan too large: {chars} chars (max: {max_chars})");
    }
    let skipped = update_todo_state(session_manager, session_id, move |state| {
        let mut skipped = Vec::new();
        for (ids, status) in [
            (&sidecar.done, TodoStatus::Completed),
            (&sidecar.blocked, TodoStatus::Blocked),
        ] {
            for id in ids {
                if !state.update_item(id, Some(status), None) {
                    skipped.push(format!("#{id}"));
                }
            }
        }
        if let Some(id) = sidecar.start.as_deref() {
            if !state.update_item(id, Some(TodoStatus::InProgress), None) {
                skipped.push(format!("#{id}"));
            }
        }
        if !sidecar.add.is_empty() {
            let room = max_items.saturating_sub(state.items.len());
            if max_items > 0 && sidecar.add.len() > room {
                return Err(format!(
                    "todo.add would make the checklist {} items (max: {max_items})",
                    state.items.len() + sidecar.add.len()
                ));
            }
            state.add_items(sidecar.add);
        }
        if let Some(plan) = sidecar.plan {
            let trimmed = plan.trim();
            state.plan = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        Ok(skipped)
    })
    .await
    .map_err(|reason| anyhow::anyhow!(reason))?;
    if skipped.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "todo: no checklist item {}; the rest was applied",
            skipped.join(", ")
        )
    }
}

/// Load the session's todo state (migrating any legacy blob), let `f` mutate
/// it, then persist, in ONE `update_extension_data` transaction. Returns `f`'s
/// value.
///
/// Only `todo.v1` changes and nothing another writer committed in the
/// meantime is put back. A todo call shares a tool batch with every other
/// tool, including a spawn recording its child into the chat's orchestration
/// plan; the old `get_session` then whole-blob `update().extension_data()`
/// pair erased such a write. The whole-object form rather than
/// `update_extension_state`, because the read may migrate a legacy `todo.v0`
/// blob, a second key.
async fn update_todo_state<F, T>(
    session_manager: &SessionManager,
    session_id: &str,
    f: F,
) -> Result<T, String>
where
    F: FnOnce(&mut TodoState) -> Result<T, String> + Send,
    T: Send,
{
    // A refusal (`f`'s, or an unreadable checklist) rolls the transaction back
    // and is reported in its own words.
    let mut refusal: Option<String> = None;
    let written = session_manager
        .update_extension_data(session_id, |data| {
            // ⚠ Never `TodoState::load(..).unwrap_or_default()` here. `load`
            // reports a blob this build cannot parse as `None`, and the write
            // below replaces the WHOLE `todo.v1` key — so one `todo_add`
            // against a checklist a newer build wrote would silently destroy
            // it. Refuse instead: the blob stays exactly where it is.
            let loaded = TodoState::try_load(data).map_err(|unreadable| {
                format!(
                    "This session's stored checklist (`{}`) was written by a newer build \
                     of Biorouter and cannot be read here. Leaving it untouched rather \
                     than overwriting it — update Biorouter to change this checklist.",
                    unreadable.key
                )
            });
            let mut state = match loaded {
                Ok(state) => state.unwrap_or_default(),
                Err(sentence) => {
                    refusal = Some(sentence);
                    anyhow::bail!("todo state refused");
                }
            };
            let value = match f(&mut state) {
                Ok(value) => value,
                Err(sentence) => {
                    refusal = Some(sentence);
                    anyhow::bail!("todo change refused");
                }
            };
            if state.to_extension_data(data).is_err() {
                refusal = Some("Failed to serialize TODO state".to_string());
                anyhow::bail!("todo state did not serialize");
            }
            Ok(value)
        })
        .await;

    match written {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err("Failed to read session metadata".to_string()),
        Err(_) => Err(refusal.unwrap_or_else(|| "Failed to update session metadata".to_string())),
    }
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// A successful todo call's result: `model` for the model, `user` for the
/// person.
///
/// The model gets a few characters (Codex answers "Plan updated", and `{}`
/// inside a script): it just sent the change and the checklist reaches it in
/// every request's MOIM block, so echoing either back is paid for again in
/// every later request. The person's copy keeps the full wording and, for
/// `todo_update`, the JSON the transcript card and Code Execution's
/// `todo_task_from_result` read the item's title from. Provider formatters drop
/// a `user`-only block before the request is built (`providers::formats::
/// audience::is_for_model`), and a script receives only the model's copy.
fn acknowledged(model: impl Into<String>, user: impl Into<String>) -> Vec<Content> {
    vec![
        Content::text(model.into()).with_audience(vec![Role::Assistant]),
        Content::text(user.into()).with_audience(vec![Role::User]),
    ]
}

/// `ok #3,#5`: the model's whole answer to a change of items.
fn ok_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        return "ok".to_string();
    }
    format!(
        "ok {}",
        ids.iter()
            .map(|id| format!("#{id}"))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Default cap on the number of items a checklist may hold (per session).
const DEFAULT_MAX_ITEMS: usize = 200;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoWriteParams {
    /// The full checklist as markdown, one `- [ ] step` per line (`- [x]` done,
    /// `- [!]` blocked). Replaces every item and renumbers the ids.
    content: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoAddParams {
    /// New tasks. Each becomes a pending item with a fresh id.
    items: Vec<String>,
    /// Insert directly after this item's `#N` id instead of at the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoExpandParams {
    /// The `#N` id of the coarse item that turned out to be several steps.
    id: String,
    /// The concrete steps replacing it, in order. Each becomes a pending item.
    items: Vec<String>,
    /// Keep the original item as a grouping row with the steps nested under it
    /// (the default). `false` removes it and the steps take its place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keep_parent: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoUpdateParams {
    /// One item's `#N` id. Omit it when you pass `updates`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    /// `completed`, `blocked`, `pending`, or `in_progress` (only to work out of
    /// order: the first open item is current on its own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    /// Only to rename the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    /// Several items at once, all or nothing, e.g.
    /// `[{"id": 2, "status": "completed"}, {"id": 3, "status": "completed"}]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updates: Option<Vec<TodoItemUpdate>>,
}

/// One entry of `todo_update`'s `updates`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct TodoItemUpdate {
    /// The item's `#N` id.
    id: String,
    /// New status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    /// Only to rename the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PlanWriteParams {
    /// A short plan: the approach in a few lines. Empty string clears it.
    plan: String,
}

pub struct TodoClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
    /// Serializes [`TodoClient::with_state`]'s read-modify-write.
    ///
    /// H6: until the client-wide `McpClientBox` mutex was removed, every
    /// `call_tool` on this extension was serialized for free, which is what made
    /// `with_state` safe. Now that tool calls on one extension overlap, this
    /// extension carries its own lock. It is deliberately narrow — it guards
    /// only the session-metadata round trip (sub-millisecond, local), not the
    /// MCP dispatch path, so it costs nothing and cannot reintroduce H6.
    ///
    /// The round trip is one `update_extension_data` transaction
    /// ([`update_todo_state`]), which also serializes it against every OTHER
    /// writer of the session's `extension_data`, a sidecar
    /// ([`apply_todo_sidecar`]) included. The lock stays so concurrent todo
    /// calls queue here rather than on SQLite's busy timeout.
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
                version: "1.2.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                indoc! {r#"
                A checklist for multi-step work; the current list is shown to you
                in every request.
                - Skip it for simple tasks, and never make a one-step list. Use
                  3-7 short steps (a few words each).
                - Report progress in the `todo` field of your next real tool call,
                  e.g. `"todo": {"done": [2, 3]}`, never in a message of its own.
                  Mark several items at once, only at milestones. The first open
                  item is current on its own, so report only done and blocked.
                - A broad item that turns out to be several steps: `todo_expand`
                  it in place. `todo_add` (with `after`) inserts; `todo_write`
                  only seeds or replaces the whole list and renumbers ids.
                - Do not repeat the list in your reply; the person sees it.
                - Before finishing, every item is done, or your reply names the
                  open ones by `#N` and says why.
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

    /// [`update_todo_state`] under this client's lock; see `state_lock`.
    async fn with_state<F, T>(&self, session_id: &str, f: F) -> Result<T, String>
    where
        F: FnOnce(&mut TodoState) -> Result<T, String> + Send,
        T: Send,
    {
        let _state_guard = self.state_lock.lock().await;
        update_todo_state(&self.context.session_manager, session_id, f).await
    }

    fn max_items() -> usize {
        std::env::var("BIOROUTER_TODO_MAX_ITEMS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MAX_ITEMS)
    }

    fn max_chars() -> usize {
        std::env::var("BIOROUTER_TODO_MAX_CHARS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50_000)
    }

    async fn handle_write(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let content = string_arg(&arguments, "content")?;

        let char_count = content.chars().count();
        let max_chars = Self::max_chars();
        if max_chars > 0 && char_count > max_chars {
            return Err(format!(
                "Todo list too large: {} chars (max: {})",
                char_count, max_chars
            ));
        }

        let max_items = Self::max_items();
        let count = self
            .with_state(session_id, move |state| {
                state.set_from_markdown(&content);
                // A write that names no item is refused, and nothing is
                // persisted on an error, so the existing checklist survives.
                // It used to succeed as "Todo list set: 0 item(s)", silently
                // replacing the list with nothing (PR #393 F07 rerun,
                // ministral-3:8b: five such writes, then the repetition
                // guard). Blank content still clears the list.
                if state.items.is_empty() && !content.trim().is_empty() {
                    return Err(no_checklist_items_message(&content));
                }
                if max_items > 0 && state.items.len() > max_items {
                    return Err(format!(
                        "Todo list too long: {} items (max: {})",
                        state.items.len(),
                        max_items
                    ));
                }
                Ok(state.items.len())
            })
            .await?;
        Ok(acknowledged(
            format!("ok, {count} item(s)"),
            format!("Todo list set: {count} item(s)"),
        ))
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
        let placement = match after.as_deref() {
            Some(anchor) => format!(" after #{anchor}"),
            None => String::new(),
        };
        let ids = self
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
                Ok(ids)
            })
            .await?;
        Ok(acknowledged(
            ok_ids(&ids),
            format!(
                "Added {} item(s){placement}: #{}",
                ids.len(),
                ids.join(", #")
            ),
        ))
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
        let (model, user) = self
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
                let mut model = ok_ids(&expanded.ids);
                if expanded.kept_parent {
                    message.push_str(&format!(" (nested under #{id})"));
                } else if let Some(parent) = expanded.nested_under.as_deref() {
                    message.push_str(&format!(" (as siblings under #{parent}; #{id} removed"));
                    // Explain the shape only when the caller asked for a level
                    // it cannot have; saying it after an explicit
                    // `keep_parent: false` would imply a constraint was applied
                    // when the caller got exactly what it requested. The model
                    // is told too: it asked for a shape it did not get.
                    if keep_parent {
                        message.push_str(" — the checklist nests one level only");
                        model.push_str(&format!(
                            " (siblings under #{parent}, #{id} removed: one level only)"
                        ));
                    }
                    message.push(')');
                } else {
                    message.push_str(&format!(" (#{id} removed)"));
                }
                // Never the whole checklist: the model reads it in the next
                // request's MOIM block anyway, and this result would be re-sent
                // with every request after that.
                let user = serde_json::json!({
                    "message": message,
                    "ids": expanded.ids,
                })
                .to_string();
                Ok((model, user))
            })
            .await?;
        Ok(acknowledged(model, user))
    }

    async fn handle_update(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.as_ref().ok_or("Missing arguments")?;
        let updates = update_requests(args)?;
        let single = updates.len() == 1;

        let (ids, user) = self
            .with_state(session_id, move |state| {
                // Nothing is persisted when this returns an error, so an
                // unknown id anywhere in the batch leaves every item as it was.
                let mut ids = Vec::with_capacity(updates.len());
                for update in updates {
                    if !state.update_item(&update.id, update.status, update.text) {
                        return Err(unknown_id_message(state, &update.id));
                    }
                    ids.push(update.id);
                }
                let user = if single {
                    let id = &ids[0];
                    serde_json::json!({
                        "message": format!("Updated item #{id}"),
                        "task": state.items.iter().find(|item| &item.id == id),
                    })
                } else {
                    let tasks: Vec<_> = ids
                        .iter()
                        .filter_map(|id| state.items.iter().find(|item| &item.id == id))
                        .collect();
                    serde_json::json!({
                        "message": format!("Updated {} items: #{}", ids.len(), ids.join(", #")),
                        "tasks": tasks,
                    })
                };
                Ok((ids, user.to_string()))
            })
            .await?;
        Ok(acknowledged(ok_ids(&ids), user))
    }

    async fn handle_plan_write(
        &self,
        session_id: &str,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let plan = string_arg(&arguments, "plan")?;

        let char_count = plan.chars().count();
        let max_chars = Self::max_chars();
        if max_chars > 0 && char_count > max_chars {
            return Err(format!(
                "Plan too large: {} chars (max: {})",
                char_count, max_chars
            ));
        }

        let cleared = self
            .with_state(session_id, move |state| {
                let trimmed = plan.trim();
                if trimmed.is_empty() {
                    state.plan = None;
                    Ok(true)
                } else {
                    state.plan = Some(trimmed.to_string());
                    Ok(false)
                }
            })
            .await?;
        if cleared {
            return Ok(acknowledged("ok", "Plan cleared"));
        }
        // The one thing worth telling the model: a plan longer than the MOIM
        // block re-sends is cut there, so the rest is invisible to it later.
        let model = if char_count > MOIM_PLAN_CHARS {
            format!("ok (only the first {MOIM_PLAN_CHARS} of {char_count} characters are re-shown; shorten it)")
        } else {
            "ok".to_string()
        };
        Ok(acknowledged(model, "Plan updated"))
    }

    fn get_tools() -> Vec<Tool> {
        vec![
            tool_from_schema::<TodoWriteParams>(
                "todo_write",
                indoc! {r#"
                    Seed the checklist, or replace it whole (this renumbers every
                    id). Only for multi-step work: 3-7 short steps, never one.
                    To change an existing list use `todo_expand`, `todo_add` or
                    the `todo` field of your next tool call.
                "#},
                true,
            ),
            tool_from_schema::<TodoExpandParams>(
                "todo_expand",
                indoc! {r#"
                    Replace one coarse item, in place, with the concrete steps it
                    turned out to be (nested under it by default). Use it before
                    starting that item, instead of appending or rewriting.
                "#},
                false,
            ),
            tool_from_schema::<TodoAddParams>(
                "todo_add",
                indoc! {r#"
                    Add pending items: at the end, or after item `after`. Ids
                    never change. For the breakdown of an existing item use
                    `todo_expand`.
                "#},
                false,
            ),
            tool_from_schema::<TodoUpdateParams>(
                "todo_update",
                indoc! {r#"
                    Change items by `#N` id: `{id, status}`, or several at once
                    with `updates`. Report only `completed` and `blocked`; the
                    first open item is current on its own. Send `text` only to
                    rename. Prefer the `todo` field of your next real tool call:
                    every separate call re-sends the whole conversation.
                "#},
                false,
            ),
            tool_from_schema::<PlanWriteParams>(
                "plan_write",
                indoc! {r#"
                    Set a short plan (a few lines), shown to you with the
                    checklist in every request. Empty string clears it.
                "#},
                false,
            ),
        ]
    }
}

/// The refusal for a `todo_write` whose `content` names no checklist item:
/// the format, and the first line that was not one.
fn no_checklist_items_message(content: &str) -> String {
    let first = content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let first: String = if first.chars().count() > 80 {
        format!("{}…", first.chars().take(80).collect::<String>())
    } else {
        first.to_string()
    };
    format!(
        "0 checklist items parsed, so the checklist was left unchanged. Write one item per \
         line, like `- [ ] step` (`- [~]` in progress, `- [!]` blocked, `- [x]` done). The \
         first line was `{first}`."
    )
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

/// One parsed item change of a `todo_update` call.
struct UpdateRequest {
    id: String,
    status: Option<TodoStatus>,
    text: Option<String>,
}

/// Read one item change: an `id` plus a `status` and/or `text`. `label` names
/// where it came from in an error (`updates[1]`, or nothing for the top level).
fn update_request(object: &JsonObject, label: &str) -> Result<UpdateRequest, String> {
    let id_key = if label.is_empty() {
        "id".to_string()
    } else {
        format!("{label}.id")
    };
    let id = match object.get("id") {
        Some(value) => item_id_value(&id_key, value)?,
        None => return Err(format!("Missing required parameter: {id_key}")),
    };
    let status = match object.get("status").and_then(|v| v.as_str()) {
        Some(raw) => Some(TodoStatus::parse(raw).ok_or_else(|| {
            format!("Unknown status: {raw} (use pending/in_progress/blocked/completed)")
        })?),
        None => None,
    };
    let text = object
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if status.is_none() && text.is_none() {
        return Err(if label.is_empty() {
            "Provide a new status and/or text to update".to_string()
        } else {
            format!("{label} needs a new status and/or text")
        });
    }
    Ok(UpdateRequest { id, status, text })
}

/// Every item change one `todo_update` call asks for, in order: the top-level
/// `id` form first (the original single-item shape), then each `updates`
/// entry.
fn update_requests(args: &JsonObject) -> Result<Vec<UpdateRequest>, String> {
    let mut requests = Vec::new();
    let has_top_level = args.get("id").is_some_and(|v| !v.is_null());
    if has_top_level {
        requests.push(update_request(args, "")?);
    }
    match args.get("updates") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Array(entries)) => {
            for (index, entry) in entries.iter().enumerate() {
                let label = format!("updates[{index}]");
                let object = entry.as_object().ok_or_else(|| {
                    format!(
                        "{label} must be an object like {{\"id\": \"3\", \"status\": \"completed\"}}; got {}",
                        describe_json(entry)
                    )
                })?;
                requests.push(update_request(object, &label)?);
            }
        }
        Some(other) => {
            return Err(format!(
                "updates must be an array of {{id, status, text}} objects; got {}",
                describe_json(other)
            ))
        }
    }
    if requests.is_empty() {
        return Err(if has_top_level || args.contains_key("updates") {
            "updates is empty; pass at least one {id, status, text} entry".to_string()
        } else {
            "Missing required parameter: id (or pass `updates` to change several items)".to_string()
        });
    }
    let max_items = TodoClient::max_items();
    if max_items > 0 && requests.len() > max_items {
        return Err(format!(
            "Too many updates in one call: {} (max: {max_items})",
            requests.len()
        ));
    }
    Ok(requests)
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
        // `render_for_model`, not `render`: open items in full, old completed
        // ones as a count, the plan capped. This block is in every request.
        Some(format!("Current tasks:\n{}", state.render_for_model()))
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

    /// What the model reads: blocks with no audience or the assistant's.
    fn model_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter(|content| crate::providers::formats::audience::is_for_model(content))
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// What the person's transcript reads: blocks with no audience or the user's.
    fn user_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter(|content| {
                content
                    .audience()
                    .is_none_or(|audience| audience.contains(&Role::User))
            })
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
        let result: serde_json::Value = serde_json::from_str(&user_text(&updated)).unwrap();
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
                "- [~] (#2) set up the workspace",
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
        let result: serde_json::Value = serde_json::from_str(&user_text(&updated)).unwrap();
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
        assert!(reinjected.contains("  - [~] (#3) step one"), "{reinjected}");
    }

    /// PR #393 F07 rerun (ministral-3:8b, 2026-10-12): `-  [  ] Create…` and
    /// `-   [ ] Create…` parsed as nothing, and each write answered "Todo
    /// list set: 0 item(s)" as a success until the repetition guard fired.
    /// The spacing is read as written, and a write that still names no item
    /// is refused with the format, leaving the existing list in place.
    #[tokio::test]
    async fn todo_write_reads_loose_spacing_and_refuses_a_write_that_names_no_item() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo loose spacing").await;

        let loose = call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({
                "content": "-  [  ] Create factorial.py\n-   [ ] Run it for N=5\n- [ x ] Plan"
            }),
        )
        .await;
        assert_eq!(loose.is_error, Some(false), "{}", text(&loose));
        assert_eq!(user_text(&loose), "Todo list set: 3 item(s)");
        assert_eq!(model_text(&loose), "ok, 3 item(s)");

        let nothing = call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "1. Create factorial.py\n2. Run it"}),
        )
        .await;
        assert_eq!(nothing.is_error, Some(true));
        let refusal = text(&nothing);
        assert!(
            refusal.contains("0 checklist items parsed")
                && refusal.contains("left unchanged")
                && refusal.contains("`- [ ] step`")
                && refusal.contains("`1. Create factorial.py`"),
            "{refusal}"
        );
        let reinjected = client.get_moim(&id).await.unwrap();
        assert!(
            reinjected.contains("- [~] (#1) Create factorial.py")
                && reinjected.contains("- [x] (#3) Plan"),
            "the refused write must leave the list: {reinjected}"
        );

        // Blank content still clears it.
        let cleared = call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "  \n"}),
        )
        .await;
        assert_eq!(cleared.is_error, Some(false), "{}", text(&cleared));
        assert_eq!(user_text(&cleared), "Todo list set: 0 item(s)");
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
            let parsed: serde_json::Value = serde_json::from_str(&user_text(&result)).unwrap();
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

    /// Finishing one item and starting the next used to take two calls, each
    /// a full model request. `updates` does both in one, all or nothing.
    #[tokio::test]
    async fn todo_update_applies_several_updates_in_one_call() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo batch update").await;
        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "- [ ] one\n- [ ] two\n- [ ] three"}),
        )
        .await;

        let batch = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"updates": [
                {"id": "#1", "status": "completed"},
                {"id": 2, "status": "in_progress", "text": "two, renamed"},
            ]}),
        )
        .await;
        assert_eq!(batch.is_error, Some(false), "{}", text(&batch));
        let parsed: serde_json::Value = serde_json::from_str(&user_text(&batch)).unwrap();
        assert_eq!(model_text(&batch), "ok #1,#2");
        assert_eq!(parsed["message"], "Updated 2 items: #1, #2");
        assert_eq!(parsed["tasks"][0]["status"], "completed");
        assert_eq!(parsed["tasks"][1]["status"], "in_progress");
        assert_eq!(parsed["tasks"][1]["text"], "two, renamed");

        let checklist = client.get_moim(&id).await.unwrap();
        assert!(checklist.contains("- [x] (#1) one"), "{checklist}");
        assert!(checklist.contains("- [~] (#2) two, renamed"), "{checklist}");
        assert!(checklist.contains("- [ ] (#3) three"), "{checklist}");

        // The top-level form and `updates` combine, top level first.
        let mixed = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"id": "2", "status": "completed",
                               "updates": [{"id": "3", "status": "in_progress"}]}),
        )
        .await;
        assert_eq!(mixed.is_error, Some(false), "{}", text(&mixed));
        assert!(text(&mixed).contains("#2, #3"), "{}", text(&mixed));

        // A one-entry batch answers in the single-item shape that the code
        // execution and transcript readers parse.
        let one = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"updates": [{"id": "3", "status": "completed"}]}),
        )
        .await;
        let parsed: serde_json::Value = serde_json::from_str(&user_text(&one)).unwrap();
        assert_eq!(parsed["message"], "Updated item #3");
        assert_eq!(parsed["task"]["status"], "completed");
    }

    /// An unknown id anywhere in a batch changes nothing, and malformed
    /// entries are named by index.
    #[tokio::test]
    async fn todo_update_batch_is_all_or_nothing_and_names_bad_entries() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo batch refusal").await;
        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "- [ ] one\n- [ ] two"}),
        )
        .await;

        let refused = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"updates": [
                {"id": "1", "status": "completed"},
                {"id": "9", "status": "completed"},
            ]}),
        )
        .await;
        assert_eq!(refused.is_error, Some(true));
        assert!(
            text(&refused).contains("No todo item with id #9"),
            "{}",
            text(&refused)
        );
        let checklist = client.get_moim(&id).await.unwrap();
        assert!(
            checklist.contains("- [~] (#1) one"),
            "a refused batch must not apply its first entry: {checklist}"
        );

        for (args, expected) in [
            (serde_json::json!({"updates": []}), "updates is empty"),
            (serde_json::json!({}), "or pass `updates`"),
            (
                serde_json::json!({"updates": [{"status": "completed"}]}),
                "updates[0].id",
            ),
            (
                serde_json::json!({"updates": [{"id": "1"}]}),
                "updates[0] needs a new status and/or text",
            ),
            (
                serde_json::json!({"updates": ["1"]}),
                "updates[0] must be an object",
            ),
            (
                serde_json::json!({"updates": "1"}),
                "updates must be an array",
            ),
        ] {
            let result = call(&client, &id, "todo_update", args.clone()).await;
            assert_eq!(result.is_error, Some(true), "{args}");
            assert!(
                text(&result).contains(expected),
                "{args}: {}",
                text(&result)
            );
        }
    }

    /// The description is where the model learns to batch and to prefer the
    /// sidecar, so it says so.
    #[test]
    fn todo_update_tells_the_model_to_batch_and_ride_along() {
        let tools = TodoClient::get_tools();
        let update = tools
            .iter()
            .find(|tool| tool.name == "todo_update")
            .unwrap();
        let description = update.description.as_deref().unwrap_or_default();
        assert!(description.contains("`updates`"), "{description}");
        assert!(description.contains("`todo` field"), "{description}");
        assert!(
            description.contains("only `completed` and `blocked`"),
            "{description}"
        );
        let schema = serde_json::to_value(&*update.input_schema).unwrap();
        assert!(schema["properties"]["updates"].is_object(), "{schema}");
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert!(
            !required.iter().any(|value| value == "id"),
            "id is optional now that `updates` exists: {schema}"
        );
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
                "- [~] (#1) one",
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

    /// A todo call and a spawn into an orchestration phase can run in the same
    /// tool batch, so a todo write must never put back a blob read before the
    /// spawn's per-key write committed. Each todo write used to read the whole
    /// `extension_data`, mutate `todo.v1` in memory and write the whole blob
    /// back, which erased any other key committed in between: the plan, or a
    /// child recorded into it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_todo_write_never_erases_a_spawn_recorded_into_the_plan() {
        use crate::agents::orchestration_plan::{
            apply_plan, load_plan, mutate_plan, record_spawn, PlanPhaseInput,
        };

        const ROUNDS: usize = 24;
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "todo races a spawn").await;
        let manager = Arc::clone(&client.context.session_manager);
        let client = Arc::new(client);

        let phase = PlanPhaseInput {
            id: "build".to_string(),
            title: "Build".to_string(),
            kind: Some("implementation".to_string()),
            goal: None,
            depends_on: Vec::new(),
        };
        mutate_plan(&manager, &id, |plan| {
            apply_plan(plan, Some("Race"), None, std::slice::from_ref(&phase)).map(|p| (p, ()))
        })
        .await
        .unwrap();

        let mut tasks = Vec::new();
        for i in 0..ROUNDS {
            let todo = Arc::clone(&client);
            let session = id.clone();
            tasks.push(tokio::spawn(async move {
                let added = call(
                    &todo,
                    &session,
                    "todo_add",
                    serde_json::json!({"items": [format!("task {i}")]}),
                )
                .await;
                assert_eq!(added.is_error, Some(false), "{}", text(&added));
            }));
            let plans = Arc::clone(&manager);
            let session = id.clone();
            tasks.push(tokio::spawn(async move {
                record_spawn(&plans, &session, "build", &format!("child-{i}"), None)
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        let plan = load_plan(&manager, &id)
            .await
            .unwrap()
            .expect("a todo write erased the orchestration plan");
        let recorded = plan.phases[0].agents.len();
        assert_eq!(
            recorded,
            ROUNDS,
            "a todo write erased {} recorded spawn(s)",
            ROUNDS - recorded
        );
        let checklist = client.get_moim(&id).await.unwrap();
        for i in 0..ROUNDS {
            assert!(
                checklist.contains(&format!("task {i}")),
                "todo item {i} was lost: {checklist}"
            );
        }
    }

    /// Every successful todo call answers the model in a few characters and
    /// keeps the full wording, and `todo_update`'s task JSON, for the person.
    #[tokio::test]
    async fn every_todo_result_is_terse_for_the_model_and_whole_for_the_person() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "terse results").await;

        let cases = [
            (
                "plan_write",
                serde_json::json!({"plan": "Build, then verify"}),
                "ok",
                "Plan updated",
            ),
            (
                "todo_write",
                serde_json::json!({"content": "- [ ] build\n- [ ] verify"}),
                "ok, 2 item(s)",
                "Todo list set: 2 item(s)",
            ),
            (
                "todo_add",
                serde_json::json!({"items": ["report", "ship"]}),
                "ok #3,#4",
                "Added 2 item(s): #3, #4",
            ),
            (
                "todo_expand",
                serde_json::json!({"id": 2, "items": ["unit", "live"]}),
                "ok #5,#6",
                "Expanded #2",
            ),
            (
                "todo_update",
                serde_json::json!({"id": "1", "status": "completed"}),
                "ok #1",
                "\"task\"",
            ),
            (
                "todo_update",
                serde_json::json!({"updates": [{"id": 5, "status": "completed"},
                                               {"id": 6, "status": "blocked"}]}),
                "ok #5,#6",
                "Updated 2 items: #5, #6",
            ),
        ];
        for (tool, args, model, user) in cases {
            let result = call(&client, &id, tool, args).await;
            assert_eq!(result.is_error, Some(false), "{tool}: {}", text(&result));
            assert_eq!(model_text(&result), model, "{tool}");
            assert!(
                user_text(&result).contains(user),
                "{tool}: {}",
                user_text(&result)
            );
            assert!(
                !user_text(&result).starts_with("ok"),
                "the person's copy is not the model's: {tool}"
            );
        }

        // The expansion never echoes the checklist, to either reader.
        let expanded = call(
            &client,
            &id,
            "todo_expand",
            serde_json::json!({"id": 4, "items": ["a", "b"], "keep_parent": false}),
        )
        .await;
        assert!(
            !text(&expanded).contains("checklist"),
            "{}",
            text(&expanded)
        );
        assert!(!text(&expanded).contains("build"), "{}", text(&expanded));

        // A long plan is accepted, and the model is told it is cut in the MOIM.
        let long = call(
            &client,
            &id,
            "plan_write",
            serde_json::json!({"plan": "x".repeat(MOIM_PLAN_CHARS + 1)}),
        )
        .await;
        assert!(
            model_text(&long).contains("shorten it"),
            "{}",
            model_text(&long)
        );

        // Errors stay explicit and unaddressed, so both readers see them.
        let refused = call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"id": "99", "status": "completed"}),
        )
        .await;
        assert_eq!(refused.is_error, Some(true));
        assert!(model_text(&refused).contains("No todo item with id #99"));
        assert_eq!(model_text(&refused), user_text(&refused));
    }

    /// The MOIM block is short for a long session: finished work collapses to a
    /// count, the next open item reads as current, and no "re-injected" blurb.
    #[tokio::test]
    async fn the_moim_block_lists_open_work_and_counts_finished_work() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "compact moim").await;
        let content = (1..=12)
            .map(|n| format!("- [ ] step {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": content}),
        )
        .await;
        let done: Vec<_> = (1..=9)
            .map(|n| serde_json::json!({"id": n, "status": "completed"}))
            .collect();
        call(
            &client,
            &id,
            "todo_update",
            serde_json::json!({"updates": done}),
        )
        .await;

        let moim = client.get_moim(&id).await.unwrap();
        assert!(moim.contains("Checklist (9 of 12 done):"), "{moim}");
        assert!(moim.contains("6 earlier item(s) completed"), "{moim}");
        assert!(!moim.contains("(#1) step 1\n"), "{moim}");
        assert!(moim.contains("- [x] (#9) step 9"), "{moim}");
        assert!(moim.contains("- [~] (#10) step 10"), "{moim}");
        assert!(moim.contains("- [ ] (#11) step 11"), "{moim}");
        let instructions = client.get_info().unwrap().instructions.clone().unwrap();
        assert!(!instructions.contains("re-injected"), "{instructions}");
        assert!(instructions.contains("`todo` field"), "{instructions}");
    }

    #[test]
    fn take_todo_sidecar_removes_only_the_host_field() {
        let mut arguments = Some(
            serde_json::json!({"code": "1+1", "todo": {"done": [3]}})
                .as_object()
                .cloned()
                .unwrap(),
        );
        assert_eq!(
            take_todo_sidecar(&mut arguments),
            Some(serde_json::json!({"done": [3]}))
        );
        assert_eq!(
            arguments,
            serde_json::json!({"code": "1+1"}).as_object().cloned()
        );
        // Gone after the first take, and absent, null or no arguments read as none.
        assert_eq!(take_todo_sidecar(&mut arguments), None);
        let mut null = serde_json::json!({"todo": null}).as_object().cloned();
        assert_eq!(take_todo_sidecar(&mut null), None);
        assert_eq!(null, Some(JsonObject::new()));
        assert_eq!(take_todo_sidecar(&mut None), None);
    }

    #[test]
    fn the_sidecar_schema_is_small_and_names_its_keys() {
        let schema = todo_sidecar_schema();
        let description = schema["description"].as_str().unwrap();
        assert!(
            description.starts_with("Optional: checklist updates"),
            "{description}"
        );
        for key in ["done", "blocked", "start", "add", "plan"] {
            assert!(schema["properties"][key].is_object(), "{key}: {schema}");
        }
        assert!(schema.get("required").is_none(), "{schema}");
        assert!(
            schema.to_string().len() < 700,
            "every listed tool carries this in every request: {}",
            schema.to_string().len()
        );
        assert!(tool_takes_todo_sidecar("code_execution__execute_code"));
        assert!(tool_takes_todo_sidecar("developer__shell"));
        assert!(!tool_takes_todo_sidecar("todo__todo_update"));
        assert!(!tool_takes_todo_sidecar("execute_code"));
    }

    #[tokio::test]
    async fn a_sidecar_applies_done_blocked_add_and_plan_in_one_write() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "sidecar apply").await;
        let manager = Arc::clone(&client.context.session_manager);
        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "- [ ] one\n- [ ] two\n- [ ] three\n- [ ] four"}),
        )
        .await;

        apply_todo_sidecar(
            &manager,
            &id,
            serde_json::json!({"done": [1, "#2"], "blocked": "3", "add": ["five"],
                               "plan": "short plan"}),
        )
        .await
        .unwrap();
        let moim = client.get_moim(&id).await.unwrap();
        for expected in [
            "short plan",
            "- [x] (#1) one",
            "- [x] (#2) two",
            "- [!] (#3) three",
            "- [~] (#4) four",
            "- [ ] (#5) five",
        ] {
            assert!(moim.contains(expected), "{expected}: {moim}");
        }

        // A stringified object is read too, and `start` sets current explicitly.
        apply_todo_sidecar(&manager, &id, serde_json::json!("{\"start\": 5}"))
            .await
            .unwrap();
        let moim = client.get_moim(&id).await.unwrap();
        assert!(moim.contains("- [~] (#5) five"), "{moim}");
        assert!(moim.contains("- [ ] (#4) four"), "{moim}");

        // An empty sidecar writes nothing and is not an error.
        apply_todo_sidecar(&manager, &id, serde_json::json!({}))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_sidecar_with_a_stale_id_applies_the_rest_and_a_malformed_one_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "sidecar errors").await;
        let manager = Arc::clone(&client.context.session_manager);
        call(
            &client,
            &id,
            "todo_write",
            serde_json::json!({"content": "- [ ] one\n- [ ] two"}),
        )
        .await;

        let stale = apply_todo_sidecar(&manager, &id, serde_json::json!({"done": [1, 99]}))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            stale.contains("#99") && stale.contains("rest was applied"),
            "{stale}"
        );
        assert!(client
            .get_moim(&id)
            .await
            .unwrap()
            .contains("- [x] (#1) one"));

        for (bad, expected) in [
            (serde_json::json!(["done", 2]), "todo must be an object"),
            (serde_json::json!("not json"), "todo must be an object"),
            (
                serde_json::json!({"finished": [2]}),
                "unknown key `finished`",
            ),
            (serde_json::json!({"done": [{"id": 2}]}), "todo.done[0]"),
            (serde_json::json!({"add": [7]}), "add[0]"),
            (serde_json::json!({"plan": 3}), "todo.plan must be a string"),
        ] {
            let error = apply_todo_sidecar(&manager, &id, bad.clone())
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{bad}: {error}");
        }
        // None of the malformed ones touched #2.
        assert!(client
            .get_moim(&id)
            .await
            .unwrap()
            .contains("- [~] (#2) two"));
    }

    /// A newer build's checklist is never overwritten by a sidecar either.
    #[tokio::test]
    async fn a_sidecar_leaves_an_unreadable_checklist_alone() {
        let temp = tempfile::tempdir().unwrap();
        let (client, id) = client_and_session(&temp, "sidecar unreadable").await;
        let manager = Arc::clone(&client.context.session_manager);
        let future = serde_json::json!({
            "items": [{"id": "1", "text": "real", "status": "deferred"}]
        });
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
        let error = apply_todo_sidecar(&manager, &id, serde_json::json!({"done": [1]}))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("newer build"), "{error}");
        let after = manager.get_session(&id, false).await.unwrap();
        assert_eq!(
            after.extension_data.get_extension_state("todo", "v1"),
            Some(&future)
        );
    }

    /// The system prompt's guidance matches what the tools do: the sidecar by
    /// its field name, derived progress, no per-item calls, and the final names
    /// of the waiting tools.
    #[test]
    fn the_system_prompt_steers_todo_and_waiting_to_the_cheap_paths() {
        let prompt = include_str!("../prompts/system.md");
        for expected in [
            "never make a one-step list",
            "3-7 steps",
            "`todo` field of your next real tool call",
            "\"todo\": {\"done\": [2, 3]}",
            "report only `done` and `blocked`",
            "don't repeat the list in your reply",
            "`code_execution__watch_job` ends your turn and resumes you",
            "`code_execution__wait_for`",
            "`sleep()`",
            "280 seconds",
            "600 seconds per script",
            "every 30 seconds",
            "Data a tool returns is data to process in code",
        ] {
            assert!(prompt.contains(expected), "system.md lost: {expected}");
        }
        for retired in [
            "mark items `in_progress`",
            "living plan and a per-item checklist",
        ] {
            assert!(!prompt.contains(retired), "system.md still says: {retired}");
        }
    }
}
