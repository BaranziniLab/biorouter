//! The person tools' agent-task actions (AO-12): what `biorouter crew tasks` and `ask` do.
//!
//! A task is started through the route's own start (`routes::crew::start_owned_run`), with its
//! own grant, after the person approves a card naming its channel, the channels it may read,
//! its model and its server access. A task asked from a chat classified Private starts with
//! `origin_restricted` and the chat's affiliations, so a public model is refused as a grant to
//! that chat would be. A task's list and output are workspace content: they go through the
//! call's privacy scope and the person's read consent like any read.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::Json;
use biorouter::config::Config;
use biorouter::crew::RemoteAccess;
use biorouter::privacy::{ProviderTier, SessionClassification};
use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::places::{
    channel_phrase, crossing, item, place, public_safe, reading, resolve, yours, Place, Reading,
};
use super::setup::CREW_TASK_MODEL;
use super::{idempotency_key, refusal_text, said, Card, Ctx, Progress};
use crate::routes::crew::names::SelectorKind;
use crate::routes::crew::{
    cancel_owned_task, owned_run_views, start_owned_run, CrewRouteError, RunOrigin, StartRunRequest,
};

/// The providers that run coding agents through a tool bridge, which no Crew task can run
/// (`uses_tool_bridge`), so neither is ever a task's model.
const CODING_AGENT_PROVIDERS: [&str; 2] = ["claude_code", "codex"];
/// The longest prompt a task takes, as the route holds it.
const MAX_PROMPT_BYTES: usize = 32_768;
/// How long `tasks_watch` waits for news at most (AO-12).
const MAX_WATCH: u64 = 20;
/// How often a watched task is asked where it stands.
const WATCH_POLL: Duration = Duration::from_millis(500);
/// How many of a task's messages one answer carries, and how much of each.
const WATCH_MESSAGES: usize = 50;
const WATCH_MESSAGE_CHARS: usize = 8000;
/// A task in one of these has ended.
const ENDED: [&str; 6] = [
    "completed",
    "failed",
    "cancelled",
    "interrupted",
    "outcome_not_durable",
    "cancellation_unconfirmed",
];

/// A route refusal as the model is told it: its sentence, code and the person's step.
fn route_refusal(error: CrewRouteError) -> anyhow::Error {
    let (code, sentence) = error.code_and_sentence();
    said(None, refusal_text(Some(code), sentence, None))
}

/// Said for a task ID that is not one of this computer's on the connection.
fn no_task(connection: &str) -> anyhow::Error {
    said(
        Some("unknown_name"),
        format!("No task of this computer on {connection} has that ID. List them with crew__read action tasks_list."),
    )
}

/// One of this computer's tasks on the connection `connection_id`.
async fn find_run(connection_id: &str, id: &str) -> Option<biorouter::crew::observation::RunView> {
    owned_run_views(connection_id)
        .await
        .ok()?
        .into_iter()
        .find(|run| run.run_id == id)
}

/// `#methods`, from what the call may see, for a task's channel.
fn channel_name(visible: &Value, id: &str) -> Value {
    item(visible, "channels", id)
        .and_then(|channel| channel["name"].as_str())
        .map_or(Value::Null, |name| json!(format!("#{name}")))
}

// ------------------------------------------------------------------------------------- reads

/// `biorouter crew tasks list`: this computer's tasks on the connection, newest first. A
/// task in a channel the call may not read, or whose conversation, a chat of its own, the call
/// may not list ([`Ctx::lists_chat`]), is counted, not listed.
pub(super) fn tasks_list(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let reading = reading(ctx).await?;
        let runs = owned_run_views(&reading.connection.id).await?;
        let mut tasks = Vec::new();
        let mut withheld = 0usize;
        for run in runs {
            if !reading.scope.channel_allowed(&run.channel_id)
                || !ctx.lists_chat(&run.session_id).await
            {
                withheld += 1;
                continue;
            }
            let mut view = serde_json::to_value(&run)?;
            view["channel"] = channel_name(&reading.visible, &run.channel_id);
            tasks.push(view);
        }
        let answer = json!({"tasks": tasks, "withheld": {"tasks": withheld}});
        ctx.release(&reading.scope, answer).await
    })
}

/// The task's conversation as the person reads it, from message `after` on: each message's
/// place, role and text. Answers the messages and where the next watch starts.
async fn task_output(ctx: &Ctx, session: &str, after: usize) -> (Vec<Value>, usize) {
    let Ok(found) = ctx.state.session_manager().get_session(session, true).await else {
        return (Vec::new(), after);
    };
    let Some(conversation) = found.conversation else {
        return (Vec::new(), after);
    };
    let all = conversation.messages();
    let mut messages = Vec::new();
    let mut next = after;
    for (index, message) in all.iter().enumerate().skip(after) {
        if messages.len() >= WATCH_MESSAGES {
            break;
        }
        next = index + 1;
        if !message.is_user_visible() {
            continue;
        }
        let text = message.as_concat_text();
        if text.trim().is_empty() {
            continue;
        }
        let shown: String = text.chars().take(WATCH_MESSAGE_CHARS).collect();
        messages.push(json!({
            "index": index,
            "role": message.role,
            "text": shown,
            "truncated": shown.len() < text.len(),
        }));
    }
    (messages, next)
}

/// Whether a task's output may reach this call, and what reading it does to the chat: a public
/// model never reads a Private task's output; a private one reads it when its affiliation
/// covers the task's institutions, and the chat is raised Private with them.
async fn output_allowed(ctx: &Ctx, reading: &mut Reading, session: &str) -> Result<bool> {
    let sessions = ctx.state.session_manager();
    let private = sessions
        .get_session(session, false)
        .await
        .map_or(true, |found| {
            found.privacy_tier == SessionClassification::Private
        });
    let capability = ctx.call.capability;
    if !capability.enforced() {
        return Ok(true);
    }
    if capability.tier() == ProviderTier::Public {
        return Ok(!private);
    }
    let owners: BTreeSet<String> = sessions
        .session_affiliations(session)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    // The check a grant passes (`institution::check_provider`), on this call's model.
    capability.check_crew_institutions(&owners)?;
    if private {
        reading.scope.raise = true;
        reading.scope.institutions.extend(owners);
    }
    Ok(true)
}

/// `biorouter crew tasks watch TASK`: where a task stands and what it said since `after`,
/// waiting up to `wait_seconds` (at most 20) for news. Stop ends the wait.
pub(super) fn tasks_watch(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let id = ctx.required("task")?;
        let wait = ctx.number("wait_seconds").unwrap_or(10).min(MAX_WATCH);
        let after: usize = match ctx.text("after") {
            Some(token) => token.parse().map_err(|_| {
                said(
                    None,
                    "after is the token a previous tasks_watch answered, like 12.",
                )
            })?,
            None => 0,
        };
        let mut reading = reading(ctx).await?;
        let connection = reading.connection.clone();
        let run = find_run(&connection.id, &id)
            .await
            .ok_or_else(|| no_task(&connection.name))?;
        if !reading.scope.channel_allowed(&run.channel_id) || !ctx.lists_chat(&run.session_id).await
        {
            return Err(no_task(&connection.name));
        }
        let readable = output_allowed(ctx, &mut reading, &run.session_id).await?;
        let deadline = Instant::now() + Duration::from_secs(wait);
        let (current, messages, next) = loop {
            let current = find_run(&connection.id, &id)
                .await
                .unwrap_or_else(|| run.clone());
            let (messages, next) = if readable {
                task_output(ctx, &run.session_id, after).await
            } else {
                (Vec::new(), after)
            };
            let news = current.status != run.status || !messages.is_empty();
            if news
                || ENDED.contains(&current.status.as_str())
                || Instant::now() >= deadline
                || ctx.call.cancel.is_cancelled()
            {
                break (current, messages, next);
            }
            tokio::select! {
                () = ctx.call.cancel.cancelled() => {}
                () = tokio::time::sleep(WATCH_POLL) => {}
            }
        };
        let mut task = serde_json::to_value(&current)?;
        task["channel"] = channel_name(&reading.visible, &current.channel_id);
        let mut answer = json!({
            "task": task,
            "ended": ENDED.contains(&current.status.as_str()),
            "messages": messages,
            "after": next.to_string(),
        });
        if !readable {
            answer["withheld"] = json!({"output": true});
            answer["note"] = json!("This task's output is private, so a public model can't read it. Its status is shown.");
        }
        ctx.release(&reading.scope, answer).await
    })
}

// ----------------------------------------------------------------------------------- changes

/// `{provider, model}` when both are words.
fn model_choice(value: &Value) -> Option<(String, String)> {
    let field = |key: &str| {
        value[key]
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    field("provider").zip(field("model"))
}

/// The model a task starts on: the one the call names, else the model for Crew tasks, else the
/// default model for new chats, as `biorouter crew ask` resolves it. A coding agent can never
/// run a task.
fn task_model(ctx: &Ctx) -> Result<(String, String)> {
    let usable = |choice: &(String, String)| !CODING_AGENT_PROVIDERS.contains(&choice.0.as_str());
    let choice = match (ctx.text("provider"), ctx.text("model")) {
        (Some(provider), Some(model)) => (provider, model),
        (None, None) => {
            let config = Config::global();
            let crew = config
                .get_param::<Value>(CREW_TASK_MODEL)
                .ok()
                .as_ref()
                .and_then(model_choice)
                .filter(usable);
            let chats = model_choice(&json!({
                "provider": config.get_param::<String>("BIOROUTER_PROVIDER").ok(),
                "model": config.get_param::<String>("BIOROUTER_MODEL").ok(),
            }))
            .filter(usable);
            crew.or(chats).ok_or_else(|| {
                said(
                    Some("crew_task_model_required"),
                    "No model is set for Crew tasks. Name a provider and a model, or choose one with crew__change action defaults_set.",
                )
            })?
        }
        _ => {
            return Err(said(
                None,
                "Name both the provider and the model, or neither to use the model for Crew tasks.",
            ))
        }
    };
    if !usable(&choice) {
        return Err(said(
            None,
            "Crew can't run a task on Claude Code or Codex. Choose another model.",
        ));
    }
    Ok(choice)
}

/// What the task may do on the server, as its card says it.
fn access_line(connection: &biorouter::crew::Connection, server: &str) -> String {
    match (&connection.remote_root, connection.remote_access) {
        (None, _) => format!("It can't work on {server}."),
        (Some(_), RemoteAccess::Folder) => format!(
            "With a private model it can run commands in its work folder on {server}, confined to that folder."
        ),
        (Some(_), RemoteAccess::Full) => format!(
            "With a private model it can run any command as you on {server}, and read and change any file your account can."
        ),
    }
}

/// `#a`, `#a and #b`, `#a, #b and #c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The other channels a task may read (`context_channels`), each resolved in the person's own
/// snapshot and one of theirs, in the order named, without the task's own `channel` or a
/// channel named twice.
fn context_sources(
    ctx: &Ctx,
    place: &Place,
    channel: &super::Resolved,
) -> Result<Vec<super::Resolved>> {
    let mut sources = Vec::new();
    for text in ctx.texts("context_channels") {
        let source = resolve(ctx, place, SelectorKind::Channel, &text)?;
        yours(&place.snapshot, &source, &text)?;
        if source.id != channel.id
            && !sources
                .iter()
                .any(|seen: &super::Resolved| seen.id == source.id)
        {
            sources.push(source);
        }
    }
    Ok(sources)
}

/// `biorouter crew ask CHANNEL PROMPT` / `tasks start`: start the person's own agent on a task
/// that posts its result in a channel, with its own grant. The card names the channel, the
/// channels it may read, its model and its server access, and shows the prompt whole.
pub(super) fn tasks_ask(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        ctx.required("prompt")?;
        let prompt = ctx.raw_text("prompt").unwrap_or_default();
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(said(
                None,
                "A task can be up to 32 KB. Shorten it, then ask again.",
            ));
        }
        let (provider, model) = task_model(ctx)?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        yours(&place.snapshot, &channel, &typed)?;
        let sources = context_sources(ctx, &place, &channel)?;
        let sessions = ctx.state.session_manager();
        let restricted = ctx.chat_classification().await == SessionClassification::Private;
        let institution_ids: BTreeSet<String> = sessions
            .session_affiliations(&ctx.call.session_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        let source_names: Vec<String> = sources
            .iter()
            .map(|source| channel_phrase(&place.snapshot, source))
            .collect();
        let mut card = place
            .card(|here| {
                format!(
                    "Start an agent task in {} in {here}, on {model} ({provider}), with its own Crew access",
                    place.channel(&channel)
                )
            })
            .line("It posts its result there as your agent.")
        .line(if source_names.is_empty() {
            "It reads only that channel.".to_owned()
        } else {
            format!("It may also read {}.", and_list(&source_names))
        })
        .line(access_line(&place.connection, &place.server))
        .show("channel", channel.label.clone())
        .show("prompt", prompt.clone())
        .show("provider", provider.clone())
        .show("model", model.clone());
        if !source_names.is_empty() {
            card = card.show("context_channels", json!(source_names));
        }
        if restricted {
            card =
                card.line("This chat is Private, so the task is too: a public model is refused.");
        }
        if let Some(line) = crossing(ctx, &place, public_safe(&place, &channel.id)).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let body = StartRunRequest {
            expected_mode: None,
            expected_policy_epoch: None,
            expected_workspace_policy_epoch: None,
            request_id: Some(idempotency_key(ctx, 0)),
            channel_id: channel.id.clone(),
            prompt,
            provider: provider.clone(),
            model: model.clone(),
            context_channels: sources.iter().map(|source| source.id.clone()).collect(),
            posting_grant: true,
        };
        let origin = RunOrigin {
            restricted,
            institution_ids,
        };
        let progress = Progress::default();
        progress.set("stage", "start");
        let state = ctx.state.clone();
        let connection_id = place.connection.id.clone();
        ctx.follow(progress, async move {
            let Json(view) = start_owned_run(state, connection_id, body, origin)
                .await
                .map_err(route_refusal)?;
            let mut answer = serde_json::to_value(&view)?;
            answer["job_id"] = json!(view.run_id);
            answer["state"] = json!(view.status);
            answer["stage"] = json!(view.status);
            answer["task_model"] = json!({"provider": provider, "model": model});
            Ok(answer)
        })
        .await
    })
}

/// `biorouter crew tasks cancel TASK`: stop one of this computer's tasks, through the route's
/// own stop.
pub(super) fn tasks_cancel(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let id = ctx.required("task")?;
        let connection = ctx.connection().await?;
        let run = owned_run_views(&connection.id)
            .await?
            .into_iter()
            .find(|run| run.run_id == id)
            .ok_or_else(|| no_task(&connection.name))?;
        let channel = match ctx.snapshot(&connection).await {
            Ok(snapshot) => channel_name(&snapshot, &run.channel_id)
                .as_str()
                .map_or_else(|| "its channel".to_owned(), str::to_owned),
            Err(_) => "its channel".to_owned(),
        };
        let server = Ctx::server_label(&connection).await;
        ctx.approve(
            Card::at(&connection.name, &server, |here| {
                format!("Stop the task in {channel} in {here} ({})", run.status)
            })
            .line("Its jobs on the server may keep running until their time limit.")
            .show("task", run.run_id.clone()),
        )
        .await?;
        let Json(cancelled) = cancel_owned_task(&connection.id, &run.run_id)
            .await
            .map_err(route_refusal)?;
        let mut answer = serde_json::to_value(&cancelled)?;
        answer["task"] = json!(run.run_id);
        Ok(answer)
    })
}
