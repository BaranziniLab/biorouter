//! The person tools' message, context and file actions (AO-12): what `biorouter crew history`,
//! `search`, `send`, `workspace context` and `files` do.
//!
//! A read of messages, files or the context document goes through [`reading`]: the call's
//! privacy scope, the person's read consent, and the channels the call may see. A public model
//! gets only messages, files and references the workspace does not restrict, in channels that
//! are public-safe; everything else is withheld and counted. The chat is raised Private before
//! anything protected returns ([`Ctx::release`]).
//!
//! A write from a chat classified Private is marked Private (`personal_mode`), and its card says
//! when it crosses into something Public. A file is shared and downloaded through the same
//! transfer service `POST /crew/files` and `POST /crew/transfers` use; the card names the file's
//! resolved path and size.

use std::collections::BTreeSet;

use anyhow::Result;
use biorouter_server::crew::local_files::Direction;
use biorouter_server::crew::transfers::{
    service, FileCapability, FilePurpose, FileRequest, Receipt, StartRequest, TransferService,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::places::{
    channel_phrase, check_message_size, crossing, item, local_path, place, post_params, posted,
    private_where_kept, private_write, public_safe, reading, resolve, send, size_words, yours,
    Reading,
};
use super::{idempotency_key, naming, said, Card, Ctx, Progress};
use crate::routes::crew::names::SelectorKind;

/// The most messages a page reads (the broker's own limit).
const PAGE: u64 = 200;
/// How many messages `files_list` and a download by name read at most, newest first.
const FILE_SCAN_MESSAGES: usize = 2000;
/// How often a transfer the action waits on is asked where it stands.
const TRANSFER_POLL: std::time::Duration = std::time::Duration::from_millis(250);

// ------------------------------------------------------------------------------------- reads

/// `page` as `reading` lets the call see it: messages it may not read withheld and counted, and
/// the names beside them only for what is kept.
fn narrowed(reading: &Reading, mut page: Value) -> Value {
    let messages = page["messages"].as_array().cloned().unwrap_or_default();
    let (kept, withheld): (Vec<Value>, Vec<Value>) = messages
        .into_iter()
        .partition(|message| reading.keeps(message));
    let authors: BTreeSet<String> = kept
        .iter()
        .filter_map(|message| message["actor_id"].as_str())
        .map(str::to_owned)
        .collect();
    if let Some(people) = page["people"].as_object_mut() {
        people.retain(|id, _| authors.contains(id));
    }
    if let Some(names) = page["channel_names"].as_object_mut() {
        names.retain(|id, _| reading.scope.channel_allowed(id));
    }
    page["messages"] = json!(kept);
    page["withheld"] = json!({"messages": withheld.len()});
    page
}

/// `biorouter crew history CHANNEL` (and `watch`, with `after`): a channel's messages, newest
/// last; with `after`, only those after it, oldest first.
pub(super) fn history(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let reading = reading(ctx).await?;
        let channel = reading.channel(&typed)?;
        let limit = ctx.number("limit").unwrap_or(50).clamp(1, PAGE);
        let mut params = json!({"channel_id": channel.id, "limit": limit});
        match ctx.text("after") {
            Some(after) => params["after"] = json!(after),
            None => params["latest"] = json!(true),
        }
        if let Some(before) = ctx.text("before") {
            params["before"] = json!(before);
        }
        let page = ctx
            .crew()?
            .human_request(&reading.connection.id, "messages.history", params, None)
            .await?;
        let mut answer = narrowed(&reading, page);
        answer["channel"] = json!(reading.phrase(&channel));
        answer["channel_id"] = json!(channel.id);
        ctx.release(&reading.scope, answer).await
    })
}

/// `biorouter crew search CHANNEL QUERY`: one channel's messages that contain the words.
pub(super) fn search(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let query = ctx.required("query")?;
        let reading = reading(ctx).await?;
        let channel = reading.channel(&typed)?;
        let limit = ctx.number("limit").unwrap_or(50).clamp(1, PAGE);
        let mut params = json!({"channel_id": channel.id, "query": query, "limit": limit});
        if let Some(after) = ctx.text("after") {
            params["after"] = json!(after);
        }
        let page = ctx
            .crew()?
            .human_request(&reading.connection.id, "messages.search", params, None)
            .await?;
        let mut answer = narrowed(&reading, page);
        answer["channel"] = json!(reading.phrase(&channel));
        answer["channel_id"] = json!(channel.id);
        ctx.release(&reading.scope, answer).await
    })
}

/// `biorouter crew workspace context show`: the document members wrote about the server, and
/// its revision.
pub(super) fn context_show(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let reading = reading(ctx).await?;
        let current = ctx
            .crew()?
            .human_request(
                &reading.connection.id,
                "workspace.context.get",
                json!({}),
                None,
            )
            .await?;
        let answer = json!({"workspace": reading.workspace, "context": current["context"]});
        ctx.release(&reading.scope, answer).await
    })
}

/// One attachment `files_list` found, as it lists it.
fn listed_file(id: &str, blob: &Value, message: &Value) -> Value {
    json!({
        "attachment_id": id,
        "name": blob["name"],
        "size": blob["size"],
        "media_type": blob["media_type"],
        "shared_by": message["actor_id"],
        "shared_at": message["created_at"],
        "message_id": message["id"],
    })
}

/// The files shared in `channel_id`, newest first, each attachment once at its newest post, as
/// `biorouter crew files list` reads them from the channel's history: at most `limit`, and
/// stopping at `wanted` when a name is looked for. Answers the files, how many could not be
/// opened, how many were withheld from this call, and whether the whole history was read.
async fn shared_files(
    ctx: &Ctx,
    reading: &Reading,
    channel_id: &str,
    limit: usize,
    wanted: Option<&str>,
) -> Result<(Vec<Value>, usize, usize, bool)> {
    let crew = ctx.crew()?;
    let connection = &reading.connection.id;
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    let (mut unavailable, mut withheld, mut read) = (0usize, 0usize, 0usize);
    let mut before: Option<String> = None;
    let complete = loop {
        let mut params = json!({"channel_id": channel_id, "latest": true, "limit": PAGE});
        if let Some(before) = &before {
            params["before"] = json!(before);
        }
        let page = crew
            .human_request(connection, "messages.history", params, None)
            .await?;
        let messages = page["messages"].as_array().cloned().unwrap_or_default();
        read += messages.len();
        // A latest page runs oldest to newest; the newest is wanted first.
        for message in messages.iter().rev() {
            let attachments: Vec<&str> = message["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if !reading.keeps(message) {
                withheld += attachments.len();
                continue;
            }
            for id in attachments {
                if files.len() >= limit || !seen.insert(id.to_owned()) {
                    continue;
                }
                match crew
                    .human_request(connection, "blob.status", json!({"blob_id": id}), None)
                    .await
                {
                    Ok(blob) if blob["name"].is_string() => {
                        if !reading.keeps(&blob) {
                            withheld += 1;
                            continue;
                        }
                        let found = blob["name"].as_str() == wanted;
                        files.push(listed_file(id, &blob, message));
                        if found {
                            return Ok((files, unavailable, withheld, false));
                        }
                    }
                    _ => unavailable += 1,
                }
            }
        }
        if files.len() >= limit || messages.len() < PAGE as usize {
            break true;
        }
        if read >= FILE_SCAN_MESSAGES {
            break false;
        }
        match messages.first().and_then(|message| message["id"].as_str()) {
            Some(oldest) => before = Some(oldest.to_owned()),
            None => break true,
        }
    };
    Ok((files, unavailable, withheld, complete))
}

/// `biorouter crew files list CHANNEL`: the files shared in a channel, newest first. Files this
/// call may not read are counted, not listed; `complete` is false when older messages were left
/// unread.
pub(super) fn files_list(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let reading = reading(ctx).await?;
        let channel = reading.channel(&typed)?;
        let limit = usize::try_from(ctx.number("limit").unwrap_or(50).clamp(1, PAGE))?;
        let (files, unavailable, withheld, complete) =
            shared_files(ctx, &reading, &channel.id, limit, None).await?;
        let answer = json!({
            "channel": reading.phrase(&channel),
            "channel_id": channel.id,
            "files": files,
            "unavailable": unavailable,
            "withheld": {"files": withheld},
            "complete": complete,
        });
        ctx.release(&reading.scope, answer).await
    })
}

/// A transfer refusal in the transfer routes' own code and sentence: a credential file, a
/// folder others can change, an existing destination.
fn transfer_refusal(error: anyhow::Error) -> anyhow::Error {
    let refused = crate::routes::crew_transfers::TransferError::from(error);
    let (code, sentence) = refused.code_and_sentence();
    said(Some(code), sentence.to_owned())
}

/// What a transfer answers the model, as `GET /crew/transfers/{id}` records it, without the
/// daemon's own bookkeeping. Its file's name, and why it stopped, only when `named`: once a file
/// is shared, its name is the workspace's.
fn transfer_view(receipt: &Receipt, named: bool) -> Value {
    let mut view = json!({
        "job_id": receipt.id,
        "transfer_id": receipt.id,
        "state": receipt.state,
        "stage": receipt.state,
        "direction": receipt.direction,
        "connection_id": receipt.connection_id,
        "channel_id": receipt.channel_id,
        "size": receipt.size,
        "offset": receipt.offset,
        "blob_id": receipt.blob_id,
        "pause_reason": receipt.pause_reason,
    });
    if named {
        view["name"] = json!(receipt.name);
        view["error"] = json!(receipt.error);
    }
    view
}

/// `biorouter crew files status TRANSFER`: where a transfer stands. A shared file's name is
/// workspace content, either way it went, so it is returned only to a call that may read that
/// workspace's channel, after the person's consent, and the chat is raised as a read raises it.
pub(super) fn files_status(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let id = ctx.required("transfer_id")?;
        let receipt = service()
            .await
            .map_err(transfer_refusal)?
            .get(&id)
            .await
            .map_err(|_| {
                said(
                    None,
                    "No transfer with that ID on this computer. Crew lists them under Files.",
                )
            })?;
        let crew = ctx.crew()?;
        let Ok(connection) = crew.connection(&receipt.connection_id).await else {
            return Ok(transfer_view(&receipt, false));
        };
        let Ok(snapshot) = ctx.snapshot(&connection).await else {
            return Ok(transfer_view(&receipt, false));
        };
        let scope = match ctx.content_scope(&connection, &snapshot) {
            Ok(scope) if scope.channel_allowed(&receipt.channel_id) => scope,
            _ => return Ok(transfer_view(&receipt, false)),
        };
        let workspace = Ctx::workspace_label(&connection, Some(&snapshot));
        let scope = if ctx.read_consent(&connection, &workspace).await? {
            // The card waited for the person: judged again on the workspace as it is now.
            match ctx.rescoped(&connection.id).await {
                Ok((_, _, scope)) if scope.channel_allowed(&receipt.channel_id) => scope,
                _ => return Ok(transfer_view(&receipt, false)),
            }
        } else {
            scope
        };
        ctx.release(&scope, transfer_view(&receipt, true)).await
    })
}

// ----------------------------------------------------------------------------------- changes

/// `biorouter crew send CHANNEL --text TEXT`: post a message as the person. The card shows the
/// whole message; from a Private chat it is marked Private.
pub(super) fn send_message(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        ctx.required("text")?;
        let text = ctx.raw_text("text").unwrap_or_default();
        check_message_size(&text)?;
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        yours(&place.snapshot, &channel, &typed)?;
        let private = private_write(ctx, &place).await?;
        let mut card = Card::new(format!(
            "Post this in {} as you",
            channel_phrase(&place.snapshot, &channel)
        ))
        .show("channel", channel.label.clone())
        .show("text", text.clone());
        if private {
            card = card.line("It is marked Private, so public models never read it.");
        }
        if let Some(line) = crossing(ctx, &place, public_safe(&place, &channel.id)).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let result = send(
            ctx,
            &place.connection,
            "message.post",
            post_params(&channel.id, &text, &[], &[], private),
            0,
        )
        .await?;
        Ok(posted(&result))
    })
}

/// Wait for the transfer `id` to finish, recording its state in `progress`. A transfer that
/// stops (refused, or waiting for its file to be chosen again) is said, with where to resume it.
async fn finished(service: &TransferService, id: &str, progress: &Progress) -> Result<Receipt> {
    loop {
        let receipt = service.get(id).await.map_err(transfer_refusal)?;
        progress.set("stage", receipt.state.clone());
        match receipt.state.as_str() {
            "completed" => return Ok(receipt),
            "failed" | "needs_file_selection" | "paused" | "publication_unconfirmed" => {
                return Err(said(
                    None,
                    format!(
                        "The transfer stopped ({}){}. It is kept in Crew, under Files, where it can be resumed.",
                        receipt.state,
                        receipt
                            .error
                            .as_deref()
                            .map(|error| format!(": {error}"))
                            .unwrap_or_default()
                    ),
                ))
            }
            _ => tokio::time::sleep(TRANSFER_POLL).await,
        }
    }
}

/// What a share's card says the local file is: where it sits on disk, its size and its last
/// change. Compared again after the approval, so the file shared is the one the person allowed.
#[derive(Debug, PartialEq, Eq)]
struct FileStamp {
    /// Device and inode, where the platform has them.
    identity: Option<(u64, u64)>,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

impl FileStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            Some((metadata.dev(), metadata.ino()))
        };
        #[cfg(not(unix))]
        let identity = None;
        Self {
            identity,
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }

    /// Whether the file at `path` is still the one this stamp describes.
    fn still_at(&self, path: &std::path::Path) -> bool {
        std::fs::metadata(path).is_ok_and(|now| Self::of(&now) == *self)
    }
}

/// Select `request`'s file for an upload, as the transfer service holds it for the transfer,
/// only when it is the file the card showed (`shown`): checked before the selection and again
/// after it, with the selection's own size. A file replaced, rewritten or resized while the card
/// waited is refused, and nothing is shared.
async fn select_shown(
    request: FileRequest,
    shown: &FileStamp,
    name: &str,
) -> Result<FileCapability> {
    let changed = || {
        said(
            None,
            format!("{name} changed on this computer while this waited for your approval, so nothing was shared. Ask again to share it as it is now."),
        )
    };
    let path = request.path.clone();
    if !shown.still_at(&path) {
        return Err(changed());
    }
    let transfers = service().await.map_err(transfer_refusal)?;
    let capability = transfers
        .register(request)
        .await
        .map_err(transfer_refusal)?;
    if capability.size != Some(shown.size) || !shown.still_at(&path) {
        let _ = transfers.discard(&capability.capability_id).await;
        return Err(changed());
    }
    Ok(capability)
}

/// `biorouter crew files upload CHANNEL FILE`, then a post that carries it (and `text`, when
/// given). The card names the file's resolved path and size; from a Private chat the file and
/// the post are marked Private.
pub(super) fn files_share(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let path = local_path(&ctx.required("path")?)?;
        let text = ctx
            .raw_text("text")
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_default();
        check_message_size(&text)?;
        let metadata = std::fs::metadata(&path).map_err(|_| {
            said(
                None,
                format!("There is no file at {} on this computer.", path.display()),
            )
        })?;
        if !metadata.is_file() {
            return Err(said(
                None,
                format!(
                    "{} is not a file. Share one file at a time.",
                    path.display()
                ),
            ));
        }
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let shown = FileStamp::of(&metadata);
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        yours(&place.snapshot, &channel, &typed)?;
        let private = private_write(ctx, &place).await?;
        let mut card = Card::new(format!(
            "Share {name} ({}) from this computer in {} as you",
            size_words(metadata.len()),
            channel_phrase(&place.snapshot, &channel)
        ))
        .line(format!("From {}.", path.display()))
        .show("path", path.display().to_string())
        .show("size_bytes", metadata.len())
        .show("channel", channel.label.clone());
        if !text.is_empty() {
            card = card.show("text", text.clone());
        }
        if private {
            card = card.line(
                "The file and the post are marked Private, so public models never read them.",
            );
        }
        if let Some(line) = crossing(ctx, &place, public_safe(&place, &channel.id)).await {
            card = card.line(line);
        }
        ctx.approve(card).await?;
        let key = idempotency_key(ctx, 0);
        let capability = select_shown(
            FileRequest {
                approval_pending: false,
                expected_mode: private.then_some(biorouter::crew::ClusterMode::Private),
                purpose: FilePurpose::Transfer,
                connection_id: place.connection.id.clone(),
                channel_id: channel.id.clone(),
                direction: Direction::Upload,
                path,
                overwrite: false,
                blob_id: None,
                transfer_id: None,
                request_id: Some(key.clone()),
            },
            &shown,
            &name,
        )
        .await?;
        let transfers = service().await.map_err(transfer_refusal)?;
        let receipt = transfers
            .start(StartRequest {
                request_id: key,
                connection_id: place.connection.id.clone(),
                channel_id: channel.id.clone(),
                direction: Direction::Upload,
                file_capability: capability.capability_id,
                blob_id: None,
            })
            .await
            .map_err(transfer_refusal)?;
        let progress = Progress::default();
        progress.set("job_id", receipt.id.clone());
        progress.set("transfer_id", receipt.id.clone());
        progress.set("stage", receipt.state.clone());
        let watched = progress.clone();
        let crew = ctx.crew()?;
        let post_key = idempotency_key(ctx, 1);
        let (connection_id, channel_id, transfer_id) =
            (place.connection.id.clone(), channel.id.clone(), receipt.id);
        ctx.follow(progress, async move {
            let done = finished(&transfers, &transfer_id, &watched).await?;
            let blob_id = done
                .blob_id
                .clone()
                .ok_or_else(|| said(None, super::FAILED))?;
            watched.set("stage", "post");
            let mut params = post_params(
                &channel_id,
                &text,
                std::slice::from_ref(&blob_id),
                &[],
                private,
            );
            params["idempotency_key"] = json!(post_key);
            let result = crew
                .human_request(&connection_id, "message.post", params, Some(post_key))
                .await?;
            let mut answer = posted(&result);
            answer["shared"] = json!(true);
            answer["transfer_id"] = json!(transfer_id);
            answer["job_id"] = json!(transfer_id);
            answer["state"] = json!("completed");
            answer["blob_id"] = json!(blob_id);
            Ok(answer)
        })
        .await
    })
}

/// Where a download is saved: `destination` (a folder gets the file's name), or the person's
/// Downloads folder.
fn destination(typed: Option<&str>, name: &str) -> Result<std::path::PathBuf> {
    let Some(typed) = typed else {
        return Ok(local_path("~/Downloads")?.join(name));
    };
    let path = local_path(typed)?;
    let folder = path.is_dir() || typed.ends_with(std::path::is_separator);
    Ok(if folder { path.join(name) } else { path })
}

/// `biorouter crew files download FILE`: save a file shared in a channel to this computer. Its
/// name and the destination are the workspace's content, so this asks for the person's read
/// consent like a read, and raises the chat as a read does; the card names the file, its size,
/// its channel and where it goes.
pub(super) fn files_download(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("file")?;
        let reading = reading(ctx).await?;
        let crew = ctx.crew()?;
        let connection_id = reading.connection.id.clone();
        let blob_id = if uuid::Uuid::parse_str(&typed).is_ok() {
            typed.clone()
        } else {
            let channel_text = ctx.text("channel").ok_or_else(|| {
                said(
                    None,
                    "Name the channel the file was shared in, or give the file's ID from crew__read action files_list.",
                )
            })?;
            let channel = reading.channel(&channel_text)?;
            let (files, ..) =
                shared_files(ctx, &reading, &channel.id, usize::MAX, Some(&typed)).await?;
            files
                .iter()
                .find(|file| file["name"].as_str() == Some(typed.as_str()))
                .and_then(|file| file["attachment_id"].as_str())
                .map(str::to_owned)
                .ok_or_else(|| {
                    // The phrase names the channel's team: it returns as content does.
                    naming(
                        said(
                            Some("unknown_name"),
                            format!(
                                "No file you can see in {} is named {typed}.",
                                reading.phrase(&channel)
                            ),
                        ),
                        &reading.scope,
                    )
                })?
        };
        let blob = crew
            .human_request(
                &connection_id,
                "blob.status",
                json!({"blob_id": blob_id}),
                None,
            )
            .await?;
        let channel_id = blob["channel_id"].as_str().unwrap_or_default().to_owned();
        if !reading.scope.channel_allowed(&channel_id) || !reading.keeps(&blob) {
            return Err(said(
                None,
                "This chat can't download that file: it is restricted, or in a channel this chat can't read.",
            ));
        }
        let name = blob["name"].as_str().unwrap_or("file").to_owned();
        let size = blob["size"].as_u64().unwrap_or(0);
        let saved = destination(ctx.text("destination").as_deref(), &name)?;
        let shown_channel = item(&reading.visible, "channels", &channel_id)
            .and_then(|channel| channel["name"].as_str())
            .map_or_else(|| "its channel".to_owned(), |name| format!("#{name}"));
        ctx.approve(
            Card::new(format!(
                "Download {name} ({}) from {shown_channel} to {} on this computer",
                size_words(size),
                saved.display()
            ))
            .show("file", name.clone())
            .show("size_bytes", size)
            .show("destination", saved.display().to_string()),
        )
        .await?;
        let key = idempotency_key(ctx, 0);
        let transfers = service().await.map_err(transfer_refusal)?;
        let capability = transfers
            .register(FileRequest {
                approval_pending: false,
                expected_mode: None,
                purpose: FilePurpose::Transfer,
                connection_id: connection_id.clone(),
                channel_id: channel_id.clone(),
                direction: Direction::Download,
                path: saved.clone(),
                overwrite: false,
                blob_id: Some(blob_id.clone()),
                transfer_id: None,
                request_id: Some(key.clone()),
            })
            .await
            .map_err(transfer_refusal)?;
        let receipt = transfers
            .start(StartRequest {
                request_id: key,
                connection_id,
                channel_id,
                direction: Direction::Download,
                file_capability: capability.capability_id,
                blob_id: Some(blob_id),
            })
            .await
            .map_err(transfer_refusal)?;
        let progress = Progress::default();
        progress.set("job_id", receipt.id.clone());
        progress.set("transfer_id", receipt.id.clone());
        progress.set("stage", receipt.state.clone());
        progress.set("destination", saved.display().to_string());
        let watched = progress.clone();
        let transfer_id = receipt.id;
        let answer = ctx
            .follow(progress, async move {
                let done = finished(&transfers, &transfer_id, &watched)
                    .await
                    .map_err(|_| {
                        said(
                            None,
                            "The download stopped. It is kept in Crew, under Files, where it can be resumed. See why with crew__read action files_status.",
                        )
                    })?;
                let mut answer = transfer_view(&done, true);
                answer["downloaded"] = json!(true);
                answer["destination"] = json!(saved.display().to_string());
                Ok(answer)
            })
            .await?;
        ctx.release(&reading.scope, answer).await
    })
}

/// `biorouter crew files reference CHANNEL PATH`, then a post that carries it (and `text`, when
/// given). A reference names a file on the server, which stays there; the workspace always
/// restricts it.
pub(super) fn files_reference(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let typed = ctx.required("channel")?;
        let path = ctx.required("path")?;
        let text = ctx
            .raw_text("text")
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_default();
        check_message_size(&text)?;
        let remote = std::path::Path::new(&path);
        if !path.starts_with('/')
            || remote
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(said(
                None,
                "Give the file's full path on the server, like /data/lab/results.csv, without ..",
            ));
        }
        let label = remote
            .file_name()
            .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
        let place = place(ctx).await?;
        let channel = resolve(ctx, &place, SelectorKind::Channel, &typed)?;
        yours(&place.snapshot, &channel, &typed)?;
        let private = private_where_kept(ctx, &place).await?;
        let mut card = Card::new(format!(
            "Post a reference to {path} on {} in {} as you",
            place.server,
            channel_phrase(&place.snapshot, &channel)
        ))
        .line("The file stays on the server; members who can read it there open it from the post. The post is restricted, so public models never read it.")
        .show("path", path.clone())
        .show("channel", channel.label.clone());
        if !text.is_empty() {
            card = card.show("text", text.clone());
        }
        ctx.approve(card).await?;
        let reference = send(
            ctx,
            &place.connection,
            "reference.create",
            json!({"channel_id": channel.id, "path": path, "label": label}),
            0,
        )
        .await?;
        let reference_id = reference["id"].as_str().unwrap_or_default().to_owned();
        let result = send(
            ctx,
            &place.connection,
            "message.post",
            post_params(
                &channel.id,
                &text,
                &[],
                std::slice::from_ref(&reference_id),
                private,
            ),
            1,
        )
        .await?;
        let mut answer = posted(&result);
        answer["reference_id"] = json!(reference_id);
        Ok(answer)
    })
}
