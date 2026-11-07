//! The chat's actions the Crew view had and a chat did not (the parity review's gap list):
//! `crew__read setup_list`, `files_pending` and `connections_invitation`, and `crew__change
//! files_pause`, `files_resume`, `files_forget`, `profile_devices_acknowledge` and
//! `context_draft`.
//!
//! Each calls the function its Crew route calls: `setup_jobs` (`GET /crew/setup`), the transfer
//! service (`GET /crew/transfers`, `POST /crew/transfers/{id}/pause` and `/resume`,
//! `DELETE /crew/transfers/{id}`), `CrewManager::invitation_for`
//! (`GET /crew/connections/{id}/invitation`), `CrewManager::acknowledge_devices`
//! (`POST /crew/connections/{id}/devices/acknowledge`) and `CrewManager::context_draft`
//! (`POST /crew/connections/{id}/context/draft`).
//!
//! A read of workspace content (`files_pending`'s file names, `context_draft`'s draft, and a
//! resumed download, which brings a workspace file onto this computer) takes the read gate, the
//! consent and the raise every content read takes ([`Ctx::consented`], [`Ctx::release`]); a
//! change parks one approval ([`Ctx::approve`]), written from what the daemon resolved before
//! it was parked. A transfer change answers with states only, never the file's name: the name
//! is the workspace's, and it reaches the model only through a read.
//!
//! ⚠ **Person authority; needs human review.** Everything here acts as the person.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Result;
use biorouter::crew::{ClusterMode, Connection, NewDevice, SetupStatus};
use biorouter_server::crew::local_files::{self, Direction};
use biorouter_server::crew::transfers::{
    service, FileCapability, FilePurpose, FileRequest, Receipt, TransferService,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::places::{
    crossing, item, local_path, naming_scope, private_write, public_safe, reading, size_words,
    Place,
};
use super::setup::hosts;
use super::{job_answer, said, saved_connection, Card, ContentScope, Ctx};

/// Said for a transfer ID this computer has no receipt for.
const UNKNOWN_TRANSFER: &str =
    "No transfer with that ID on this computer. List them with crew__read action files_pending.";
/// Receipt states in which the daemon is moving the file, so it can be paused and not forgotten.
const UNDER_WAY: &[&str] = &["starting", "uploading", "downloading", "publishing"];

// ------------------------------------------------------------------------------------- reads

/// `crew__read setup_list`: the setup jobs on this computer, started from any chat, Crew or a
/// terminal, as `GET /crew/setup` lists them, each as what it is and where it stands. Nothing a
/// job found on the server is listed; `setup_status` reads one in full.
pub(super) fn setup_list(_ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let jobs: Vec<Value> = biorouter::crew::setup_jobs().iter().map(job_row).collect();
        Ok(json!({
            "jobs": jobs,
            "note": "Jobs under way, and those that ended in the last 15 minutes, newest first. Read one in full with crew__read action setup_status.",
        }))
    })
}

/// One job as `setup_list` lists it: its ID, intent, state, stage and server, with the code
/// and the step a job waiting for the person, or one that failed, carries ([`job_answer`]).
fn job_row(status: &SetupStatus) -> Value {
    let answered = job_answer(status, false);
    let mut row = json!({
        "job_id": status.job_id,
        "intent": status.intent,
        "state": status.state,
        "stage": status.stage,
        "server": status.server.login,
    });
    let code = answered
        .get("code")
        .cloned()
        .or_else(|| status.error.as_ref().map(|error| json!(error.code)));
    if let Some(code) = code {
        row["code"] = code;
    }
    if let Some(step) = answered.get("next_step") {
        row["next_step"] = step.clone();
    }
    row
}

/// `@bob` or `bob` as the account name `bob`, as the invitation route reads its `invitee`, in
/// the route's own code and sentence; none when not given or blank.
fn invitee(typed: Option<&str>) -> Result<Option<String>> {
    let Some(typed) = typed.map(str::trim).filter(|typed| !typed.is_empty()) else {
        return Ok(None);
    };
    let name = typed.strip_prefix('@').unwrap_or(typed);
    if biorouter_crew::valid_username(name) {
        Ok(Some(name.to_owned()))
    } else {
        Err(said(
            Some("crew_invalid_selector"),
            "Type the person's username on the server, like @bob.",
        ))
    }
}

/// `crew__read connections_invitation`: the invitation message for a person to join the
/// workspace, as `GET /crew/connections/{id}/invitation` builds it, for its host. It carries
/// only what an invitation carries, the fields a host sends to someone outside the workspace,
/// so it takes no read consent.
pub(super) fn connections_invitation(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let invitee = invitee(ctx.text("invitee").as_deref())?;
        let connection = ctx.connection().await?;
        let snapshot = ctx.snapshot(&connection).await?;
        if hosts(&snapshot) == Some(false) {
            return Err(said(
                Some("crew_host_required"),
                format!(
                    "Only the workspace host can send an invitation to {}. Ask the host to send it.",
                    Ctx::workspace_label(&connection, Some(&snapshot))
                ),
            ));
        }
        let text = ctx
            .crew()?
            .invitation_for(&connection.id, invitee.as_deref())
            .await?;
        Ok(json!({
            "connection_id": connection.id,
            "invitee": invitee.map(|name| format!("@{name}")),
            "message": text.message,
            "line": text.line,
            "note": "Send the whole message to the person. They join by pasting it into Crew, or into a chat of theirs.",
        }))
    })
}

/// `crew__read files_pending`: this computer's transfers through a connection, finished or
/// not, as `GET /crew/transfers?connection_id=` lists them. A file's name is the workspace's
/// content, so this is a content read: the read gate, the person's consent, and the raise. A
/// transfer in a channel the read may not see is counted, not listed. Of the rest, a name is
/// given only where the read may carry it ([`may_name`]); every other is withheld and counted,
/// and the transfer is listed by its ID, state, direction and size alone ([`receipt_view`]).
pub(super) fn files_pending(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let reading = reading(ctx).await?;
        let receipts: Vec<Receipt> = transfers()
            .await?
            .list()
            .await
            .into_iter()
            .filter(|receipt| receipt.connection_id == reading.connection.id)
            .collect();
        let mut listed = Vec::with_capacity(receipts.len());
        let (mut names, mut unlisted) = (0u64, 0u64);
        for receipt in &receipts {
            if !reading.scope.channel_allowed(&receipt.channel_id) {
                unlisted += 1;
                continue;
            }
            let named = may_name(ctx, &reading.scope, receipt).await;
            if !named {
                names += 1;
            }
            listed.push(receipt_view(receipt, named));
        }
        let answer = json!({
            "workspace": reading.workspace,
            "connection_id": reading.connection.id,
            "transfers": listed,
            "withheld": {"names": names, "transfers": unlisted},
            "note": "Pause one under way with crew__change action files_pause, resume a stopped one with files_resume, or remove one from the list with files_forget.",
        });
        ctx.release(&reading.scope, answer).await
    })
}

/// Whether `receipt`'s file may be named to a call whose read of its workspace has `scope`:
/// the one rule every read that could name a transfer asks (`files_pending`, `files_status`,
/// and `files_resume` before it compares a file with it). Its channel is one the read may see,
/// and the file is not one the workspace restricts, unless the read is a private model's the
/// chat is raised for. An upload is restricted when its selection required Private; for a
/// download the workspace is asked, and an answer that can't be had withholds it.
pub(super) async fn may_name(ctx: &Ctx, scope: &ContentScope, receipt: &Receipt) -> bool {
    if !scope.channel_allowed(&receipt.channel_id) {
        return false;
    }
    if !scope.public_only && scope.raise {
        return true;
    }
    match receipt.direction {
        Direction::Upload => !requires_private(receipt),
        Direction::Download => {
            let Some(blob_id) = receipt.blob_id.as_deref() else {
                return false;
            };
            let Ok(crew) = ctx.crew() else {
                return false;
            };
            crew.human_request(
                &receipt.connection_id,
                "blob.status",
                json!({"blob_id": blob_id}),
                None,
            )
            .await
            .is_ok_and(|blob| blob["restricted"] == false)
        }
    }
}

/// A receipt field the transfer service keeps for itself, as it serializes it.
fn bookkeeping(receipt: &Receipt, key: &str) -> Value {
    serde_json::to_value(receipt)
        .map(|value| value[key].clone())
        .unwrap_or(Value::Null)
}

/// Whether an upload's selection required Private; unreadable reads as required (fail closed).
fn requires_private(receipt: &Receipt) -> bool {
    bookkeeping(receipt, "requires_private")
        .as_bool()
        .unwrap_or(true)
}

/// The identity of a download's destination folder and name, once the download began writing.
fn destination_identity(receipt: &Receipt) -> Option<String> {
    bookkeeping(receipt, "destination_identity")
        .as_str()
        .map(str::to_owned)
}

/// A transfer as the model is told it, as `GET /crew/transfers/{id}` records it, without the
/// daemon's own bookkeeping. Only when `named` ([`may_name`]) does it carry its file's name, why
/// it stopped, and the channel and attachment it went through; otherwise it is its ID, state,
/// direction and size, and nothing that would lead to the file another way.
pub(super) fn receipt_view(receipt: &Receipt, named: bool) -> Value {
    let mut view = json!({
        "transfer_id": receipt.id,
        "state": receipt.state,
        "direction": receipt.direction,
        "size": receipt.size,
    });
    if named {
        for (key, value) in [
            ("job_id", json!(receipt.id)),
            ("stage", json!(receipt.state)),
            ("connection_id", json!(receipt.connection_id)),
            ("channel_id", json!(receipt.channel_id)),
            ("offset", json!(receipt.offset)),
            ("blob_id", json!(receipt.blob_id)),
            ("pause_reason", json!(receipt.pause_reason)),
            ("name", json!(receipt.name)),
            ("error", json!(receipt.error)),
        ] {
            view[key] = value;
        }
    }
    view
}

// ----------------------------------------------------------------------------------- changes

/// `crew__change context_draft`: the fixed probe of the server, run in the person's account as
/// `POST /crew/connections/{id}/context/draft` runs it, and the context document drafted from
/// what the server said. The draft is about the workspace's server, so it is read as the
/// workspace's content is: the read gate and the person's consent before the card, and judged
/// again and the chat raised before it returns. Nothing is saved: `context_set` saves it.
pub(super) fn context_draft(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let connection = ctx.connection().await?;
        let (connection, snapshot, _) = ctx.consented(connection).await?;
        let workspace = Ctx::workspace_label(&connection, Some(&snapshot));
        let server = Ctx::server_label(&connection).await;
        ctx.approve(
            Card::at(&workspace, &server, |here| {
                format!("Draft the context document for {here} from what the server reports about itself")
            })
            .line("Crew runs its fixed check of the server as you. It changes nothing there.")
            .line("The draft comes back to this chat. Nothing is saved until you approve saving it."),
        )
        .await?;
        let drafted = ctx.crew()?.context_draft(&connection.id).await?;
        // The person may have taken minutes to answer: judged on the workspace as it is now.
        let (connection, snapshot, scope) = ctx.rescoped(&connection.id).await?;
        let answer = json!({
            "connection_id": connection.id,
            "workspace": Ctx::workspace_label(&connection, Some(&snapshot)),
            "draft": drafted.draft,
            "saved": false,
            "note": "Nothing is saved yet. Show the person the draft, then save it, edited or not, with crew__change action context_set.",
        });
        ctx.release(&scope, answer).await
    })
}

/// A fingerprint as letters and digits only, upper case, so `3f2a-9c1e…` and
/// `3F2A 9C1E …` name the same computer.
fn fingerprint_key(typed: &str) -> String {
    typed
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// The new computers `given` names, each once, as the daemon reports them. A fingerprint that
/// is not one of `new` is refused, naming the connection by its own name.
fn chosen_devices(new: &[NewDevice], given: &[String], connection: &str) -> Result<Vec<NewDevice>> {
    let mut chosen: Vec<NewDevice> = Vec::new();
    for typed in given {
        let key = fingerprint_key(typed);
        let Some(device) = new
            .iter()
            .find(|device| fingerprint_key(&device.fingerprint) == key)
        else {
            return Err(said(
                None,
                format!("{typed} isn't one of the new computers on {connection}. See them with crew__read action profile_devices, under new_devices."),
            ));
        };
        if !chosen
            .iter()
            .any(|seen| seen.fingerprint == device.fingerprint)
        {
            chosen.push(device.clone());
        }
    }
    Ok(chosen)
}

/// A new computer as the card lists it, in the words `biorouter crew` and the desktop use: its
/// fingerprint, then when and how it was added, as known ("3F2A 9C1E 77B0 D4E1, added
/// Sep 23, 2026 by signing in to the server").
fn device_line(device: &NewDevice) -> String {
    let when = device
        .added_at
        .and_then(|at| i64::try_from(at).ok())
        .and_then(|at| chrono::DateTime::from_timestamp(at, 0))
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%b %-d, %Y")
                .to_string()
        });
    let how = device
        .added_via
        .as_deref()
        .and_then(biorouter::crew::added_via_words);
    let added: Vec<String> = when.into_iter().chain(how.map(str::to_owned)).collect();
    if added.is_empty() {
        device.fingerprint.clone()
    } else {
        format!("{}, added {}", device.fingerprint, added.join(" "))
    }
}

/// `crew__change profile_devices_acknowledge`: mark the new computers added to the person's
/// account as seen, as `POST /crew/connections/{id}/devices/acknowledge` does. The card lists
/// each one the call names, with how and when it was added, and only those are acknowledged.
/// A read never acknowledges anything.
pub(super) fn profile_devices_acknowledge(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let given = ctx.texts("fingerprints");
        if given.is_empty() {
            return Err(said(
                None,
                "Name the new computers to mark as seen: their fingerprints, as crew__read action profile_devices lists them under new_devices.",
            ));
        }
        let connection = ctx.connection().await?;
        let crew = ctx.crew()?;
        let new = crew.new_devices(&connection.id).await?;
        if new.is_empty() {
            return Err(said(
                None,
                format!(
                    "No new computers are waiting to be seen on {}.",
                    connection.name
                ),
            ));
        }
        let chosen = chosen_devices(&new, &given, &connection.name)?;
        let fingerprints: Vec<String> = chosen
            .iter()
            .map(|device| device.fingerprint.clone())
            .collect();
        let server = Ctx::server_label(&connection).await;
        let what = match chosen.len() {
            1 => "a new computer".to_owned(),
            count => format!("{count} new computers"),
        };
        let mut card = Card::at(&connection.name, &server, |here| {
            format!("Mark {what} added to your account in {here} as seen")
        });
        for device in &chosen {
            card = card.line(device_line(device));
        }
        card = card
            .line("Crew then stops pointing them out. If one isn't yours, deny this. Take its SSH access to the server away first, then remove it in Crew, under Keys.")
            .show("fingerprints", fingerprints.clone());
        ctx.approve(card).await?;
        crew.acknowledge_devices(&connection.id, &fingerprints)
            .await?;
        Ok(json!({
            "acknowledged": fingerprints,
            "connection_id": connection.id,
        }))
    })
}

// --------------------------------------------------------------------------------- transfers

/// The transfer service, its refusal said in its own code and sentence.
async fn transfers() -> Result<Arc<TransferService>> {
    service().await.map_err(transfer_refusal)
}

/// A transfer refusal in the transfer routes' own code and sentence: a credential file, a
/// folder others can change, an existing destination.
fn transfer_refusal(error: anyhow::Error) -> anyhow::Error {
    let refused = crate::routes::crew_transfers::TransferError::from(error);
    let (code, sentence) = refused.code_and_sentence();
    said(Some(code), sentence.to_owned())
}

/// The receipt for `id`, as `GET /crew/transfers/{id}` reads it.
async fn receipt_of(id: &str) -> Result<Receipt> {
    transfers()
        .await?
        .get(id)
        .await
        .map_err(|_| said(None, UNKNOWN_TRANSFER))
}

/// A transfer's state in a person's words, for a refusal.
fn state_words(state: &str) -> &'static str {
    match state {
        "completed" => "finished",
        "failed" => "stopped with an error",
        "needs_file_selection" => "paused",
        "pause_requested" => "stopping",
        "publication_unconfirmed" => {
            "waiting to be checked, because Crew couldn't confirm the file was saved"
        }
        "starting" | "uploading" | "downloading" | "publishing" => "under way",
        _ => "in a state this Biorouter doesn't know",
    }
}

/// `the upload of plot.png (1.2 MB) to`, or `the download of … from`, as a card names a
/// transfer before where it goes or comes from.
fn transfer_phrase(receipt: &Receipt) -> (String, &'static str) {
    let (kind, toward) = match receipt.direction {
        Direction::Upload => ("upload", "to"),
        Direction::Download => ("download", "from"),
    };
    let size = if receipt.size > 0 {
        format!(" ({})", size_words(receipt.size))
    } else {
        String::new()
    };
    (format!("the {kind} of {}{size}", receipt.name), toward)
}

/// The card for a change to `receipt`, its summary written around the transfer
/// (`summary("the upload of plot.png (4 KB) to lab on leo")`): to or from its connection's
/// workspace and server, or on this computer when its connection is no longer saved. The
/// payload shows the transfer whole.
async fn transfer_card(
    ctx: &Ctx,
    receipt: &Receipt,
    summary: impl FnOnce(String) -> String,
) -> Result<Card> {
    let (what, toward) = transfer_phrase(receipt);
    let card = match ctx.crew()?.connection(&receipt.connection_id).await {
        Ok(connection) => {
            let server = Ctx::server_label(&connection).await;
            Card::at(&connection.name, &server, |here| {
                summary(format!("{what} {toward} {here}"))
            })
        }
        Err(_) => Card::new(summary(format!("{what} on this computer"))),
    };
    Ok(card
        .show("transfer_id", receipt.id.clone())
        .show("file", receipt.name.clone())
        .show("direction", json!(receipt.direction))
        .show("size_bytes", receipt.size)
        .show("state", receipt.state.clone()))
}

/// How far a transfer got, for its card.
fn progress_line(receipt: &Receipt) -> String {
    let moved = match receipt.direction {
        Direction::Upload => "sent",
        Direction::Download => "received",
    };
    format!(
        "{} of {} {moved} so far.",
        size_words(receipt.offset),
        size_words(receipt.size)
    )
}

/// `crew__change files_pause`: pause a transfer under way, as
/// `POST /crew/transfers/{id}/pause` does. It stops where it is and can be resumed from there.
pub(super) fn files_pause(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let id = ctx.required("transfer_id")?;
        let receipt = receipt_of(&id).await?;
        if !UNDER_WAY.contains(&receipt.state.as_str()) {
            return Err(said(
                None,
                format!(
                    "That transfer isn't moving, so there is nothing to pause. It is {}.",
                    state_words(&receipt.state)
                ),
            ));
        }
        let card = transfer_card(ctx, &receipt, |transfer| format!("Pause {transfer}"))
            .await?
            .line(progress_line(&receipt))
            .line("It stops where it is, and you can resume it from there later.");
        ctx.approve(card).await?;
        let paused = transfers()
            .await?
            .pause(&id)
            .await
            .map_err(transfer_refusal)?;
        Ok(pause_answer(&paused))
    })
}

/// What `files_pause` answers for the receipt the pause left: paused, or, when the transfer
/// stopped on its own while the card waited (it finished, or failed), how to see where it is.
fn pause_answer(paused: &Receipt) -> Value {
    let mut answer = receipt_view(paused, false);
    let stopping = matches!(
        paused.state.as_str(),
        "pause_requested" | "needs_file_selection"
    );
    answer["paused"] = json!(stopping);
    answer["note"] = json!(if stopping {
        "It stops in a moment. Resume it with crew__change action files_resume and the file's path on this computer."
    } else {
        "It stopped moving before it could be paused. See where it stands with crew__read action files_status."
    });
    answer
}

/// Refuse a transfer that resuming cannot help: one finished, one whose publication needs a
/// person to look first, or one still moving.
fn resumable(receipt: &Receipt) -> Result<()> {
    let state = receipt.state.as_str();
    let refused = match state {
        "completed" => "That transfer finished, so there is nothing to resume.".to_owned(),
        "publication_unconfirmed" => "Crew couldn't confirm whether that download was saved, so it won't resume it. Check the file in Crew, under Files.".to_owned(),
        _ if UNDER_WAY.contains(&state) || state == "pause_requested" => format!(
            "That transfer is {}, so there is nothing to resume yet.",
            state_words(state)
        ),
        _ => return Ok(()),
    };
    Err(said(None, refused))
}

/// The saved connection `id`'s workspace, as a change card names it ([`Place`]), read for the
/// transfer that names the connection.
async fn place_of(ctx: &Ctx, id: &str) -> Result<Place> {
    let connection: Connection = saved_connection(&*ctx.crew()?, id).await?;
    let snapshot = ctx.snapshot(&connection).await?;
    let workspace = Ctx::workspace_label(&connection, Some(&snapshot));
    let server = Ctx::server_label(&connection).await;
    let names = naming_scope(ctx, &connection, &snapshot).await;
    Ok(Place {
        connection,
        snapshot,
        workspace,
        server,
        names,
    })
}

/// A local file as the card showed it: where it sits on disk, its size and its last change.
#[derive(Debug, PartialEq, Eq)]
struct Shown {
    identity: Option<(u64, u64)>,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

impl Shown {
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

    fn still_at(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|now| Self::of(&now) == *self)
    }
}

/// The channel a transfer went through, as a card names it: `#methods`, or `its channel`.
fn channel_words(place: &Place, channel_id: &str) -> String {
    item(&place.snapshot, "channels", channel_id)
        .and_then(|channel| channel["name"].as_str())
        .map_or_else(|| "its channel".to_owned(), |name| format!("#{name}"))
}

/// What resuming an upload sends, checked before its card: one file at `path`, with the name
/// and size the transfer started with (the transfer service then checks its digest). Answers
/// the card, the file as shown, and whether the rest goes marked Private, as every write from
/// a Private chat does.
async fn upload_resume(
    ctx: &Ctx,
    place: &Place,
    receipt: &Receipt,
    path: &Path,
) -> Result<(Card, Option<Shown>, bool)> {
    let metadata = std::fs::metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .ok_or_else(|| {
            said(
                None,
                format!("There is no file at {} on this computer.", path.display()),
            )
        })?;
    let same_name = path.file_name().and_then(|name| name.to_str()) == Some(&receipt.name);
    let same_size = receipt.size == 0 || metadata.len() == receipt.size;
    if !same_name || !same_size {
        return Err(said(
            None,
            format!(
                "{} isn't the file this transfer was sharing. Give the path of that same file.",
                path.display()
            ),
        ));
    }
    let private = private_write(ctx, place).await?;
    let channel = channel_words(place, &receipt.channel_id);
    let size = size_words(metadata.len());
    let mut card = place
        .card(|here| {
            format!(
                "Resume sharing {} ({size}) from this computer in {channel} in {here} as you",
                receipt.name
            )
        })
        .line(format!("From {}.", path.display()))
        .line(progress_line(receipt))
        .show("path", path.display().to_string())
        .show("size_bytes", metadata.len());
    if let Some(line) = private_line(private, receipt) {
        card = card.line(line);
    }
    if let Some(line) = crossing(ctx, place, public_safe(place, &receipt.channel_id)).await {
        card = card.line(line);
    }
    Ok((card, Some(Shown::of(&metadata)), private))
}

/// The resume card's Private line: the rest goes marked Private from a Private chat, and a
/// transfer whose selection once required Private keeps it, whichever chat resumes it.
fn private_line(private: bool, receipt: &Receipt) -> Option<&'static str> {
    (private || requires_private(receipt))
        .then_some("It is marked Private, so public models never read it.")
}

/// The identity of `path` as a download destination: its folder and name, as the transfer
/// service records it. `None` where the folder can't be opened as Crew opens one.
fn identity_at(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let directory = local_files::open_directory(path.parent()?, false).ok()?;
    local_files::destination_identity(&directory, name).ok()
}

/// Said for a download the chat may not bring onto this computer, in `files_download`'s words.
const DOWNLOAD_NOT_READABLE: &str =
    "This chat can't download that file: it is restricted, or in a channel this chat can't read.";
/// Said, whatever path was given, for an upload whose file this chat may not be told.
const UPLOAD_NOT_NAMEABLE: &str =
    "This chat can't resume that upload, because it may not see which file it was sharing. Resume it in Crew, under Files.";

/// Resuming an upload compares the file at the model's path with the transfer's own name and
/// size, and a mismatch is refused before any card. So that comparison never tells a chat
/// whether a file of its choosing is one it may not be told of, an upload this call may not name
/// ([`may_name`], judged on the workspace's scope as it is now, with no consent asked, since
/// nothing of the workspace returns) is refused here, in one sentence, before the path is looked
/// at.
async fn upload_gate(ctx: &Ctx, receipt: &Receipt) -> Result<()> {
    let connection = saved_connection(&*ctx.crew()?, &receipt.connection_id).await?;
    let snapshot = ctx.snapshot(&connection).await?;
    let named = match ctx.content_scope(&connection, &snapshot).await {
        Ok(scope) => may_name(ctx, &scope, receipt).await,
        Err(_) => false,
    };
    if named {
        Ok(())
    } else {
        Err(said(None, UPLOAD_NOT_NAMEABLE))
    }
}

/// A download brings the workspace's file onto this computer, where the chat's own tools can
/// read it, so resuming one is gated as `files_download` starts one: the content read's scope
/// and the person's consent, and a file in a channel the chat may not read, or one the
/// workspace restricts (unless the read raises the chat), is refused before any card. A file
/// whose restriction can't be read counts as restricted. Answers the scope the answer is
/// released under ([`Ctx::release`]).
async fn download_gate(ctx: &Ctx, receipt: &Receipt) -> Result<ContentScope> {
    let crew = ctx.crew()?;
    let connection = saved_connection(&crew, &receipt.connection_id).await?;
    let (connection, _, scope) = ctx.consented(connection).await?;
    if !scope.channel_allowed(&receipt.channel_id) {
        return Err(said(None, DOWNLOAD_NOT_READABLE));
    }
    let Some(blob_id) = receipt.blob_id.as_deref() else {
        return Err(said(None, DOWNLOAD_NOT_READABLE));
    };
    let blob = crew
        .human_request(
            &connection.id,
            "blob.status",
            json!({"blob_id": blob_id}),
            None,
        )
        .await?;
    let restricted = blob["restricted"].as_bool() != Some(false);
    if restricted && (scope.public_only || !scope.raise) {
        return Err(said(None, DOWNLOAD_NOT_READABLE));
    }
    Ok(scope)
}

/// What resuming a download writes, checked before its card: to the destination it was
/// saving to, once it began writing there.
fn download_resume(place: &Place, receipt: &Receipt, path: &Path) -> Result<Card> {
    if let Some(original) = destination_identity(receipt) {
        if identity_at(path).as_deref() != Some(original.as_str()) {
            return Err(said(
                None,
                format!(
                    "{} isn't where this download was saving to. Give the same path it had.",
                    path.display()
                ),
            ));
        }
    }
    let channel = channel_words(place, &receipt.channel_id);
    Ok(place
        .card(|here| {
            format!(
                "Resume downloading {} from {channel} in {here} to {} on this computer",
                receipt.name,
                path.display()
            )
        })
        .line(progress_line(receipt))
        .show("destination", path.display().to_string())
        .show("size_bytes", receipt.size))
}

/// Download destinations a chat resumed to, by transfer, so forgetting one later finds its
/// partial file ([`partial_of`]). Kept in memory only.
static DESTINATIONS: LazyLock<Mutex<std::collections::HashMap<String, PathBuf>>> =
    LazyLock::new(Mutex::default);
/// How many destinations are remembered.
const DESTINATIONS_KEPT: usize = 64;

fn remember_destination(id: &str, path: &Path) {
    if let Ok(mut kept) = DESTINATIONS.lock() {
        if kept.len() >= DESTINATIONS_KEPT {
            kept.clear();
        }
        kept.insert(id.to_owned(), path.to_owned());
    }
}

/// `crew__change files_resume`: resume a stopped transfer from where it stopped, as
/// `POST /crew/transfers/{id}/resume` does, with the file selected again from `path`: the file
/// being shared, or where the download goes. The card names the path and the size. A download
/// is read as content ([`download_gate`]), and the chat is raised as that read raises it; an
/// upload this chat may not name is refused before its file is compared ([`upload_gate`]).
pub(super) fn files_resume(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let id = ctx.required("transfer_id")?;
        let path = local_path(&ctx.required("path")?)?;
        let receipt = receipt_of(&id).await?;
        resumable(&receipt)?;
        let scope = match receipt.direction {
            Direction::Upload => {
                upload_gate(ctx, &receipt).await?;
                None
            }
            Direction::Download => Some(download_gate(ctx, &receipt).await?),
        };
        let place = place_of(ctx, &receipt.connection_id).await?;
        let (card, shown, private) = match receipt.direction {
            Direction::Upload => upload_resume(ctx, &place, &receipt, &path).await?,
            Direction::Download => (download_resume(&place, &receipt, &path)?, None, false),
        };
        ctx.approve(card.show("transfer_id", id.clone())).await?;
        let service = transfers().await?;
        let capability = select_again(&service, &receipt, &path, shown.as_ref(), private).await?;
        let resumed = match service.resume(&id, &capability.capability_id).await {
            Ok(resumed) => resumed,
            Err(error) => {
                let _ = service.discard(&capability.capability_id).await;
                return Err(transfer_refusal(error));
            }
        };
        if receipt.direction == Direction::Download {
            remember_destination(&id, &path);
        }
        let mut answer = receipt_view(&resumed, false);
        answer["resumed"] = json!(true);
        answer["note"] = json!("It goes on from where it stopped. Follow it with crew__read action files_status, or in Crew under Files.");
        match scope {
            Some(scope) => ctx.release(&scope, answer).await,
            None => Ok(answer),
        }
    })
}

/// Select `path` again for `receipt`, as `POST /crew/files` does for a resume, and, for an
/// upload, only when it is the file the card showed (`shown`): checked again after the
/// selection, with the selection's own size, so a file changed while the card waited is
/// refused and nothing is sent.
async fn select_again(
    service: &TransferService,
    receipt: &Receipt,
    path: &Path,
    shown: Option<&Shown>,
    private: bool,
) -> Result<FileCapability> {
    let changed = || {
        said(
            None,
            format!("{} changed on this computer while this waited for your approval, so nothing was sent. Ask again to resume it as it is now.", path.display()),
        )
    };
    if shown.is_some_and(|shown| !shown.still_at(path)) {
        return Err(changed());
    }
    let capability = service
        .register(FileRequest {
            approval_pending: false,
            expected_mode: private.then_some(ClusterMode::Private),
            purpose: FilePurpose::Transfer,
            connection_id: receipt.connection_id.clone(),
            channel_id: receipt.channel_id.clone(),
            direction: receipt.direction,
            path: path.to_owned(),
            overwrite: false,
            blob_id: (receipt.direction == Direction::Download)
                .then(|| receipt.blob_id.clone())
                .flatten(),
            transfer_id: Some(receipt.id.clone()),
            request_id: None,
        })
        .await
        .map_err(transfer_refusal)?;
    if let Some(shown) = shown {
        if capability.size != Some(shown.size) || !shown.still_at(path) {
            let _ = service.discard(&capability.capability_id).await;
            return Err(changed());
        }
    }
    Ok(capability)
}

/// An unfinished download's partial file, which forgetting it deletes: where the download was
/// saving to, and the partial beside it.
struct Partial {
    destination: PathBuf,
    part: PathBuf,
}

/// The folders where a download may have been saving to, most likely first: where a chat
/// resumed it to, the chat's own folder, then the person's Downloads folder (where a chat's
/// download goes when it names no destination) and home folder. It looks nowhere else: opening
/// a folder such as Desktop or Documents can make the system ask the person for permission.
async fn destination_folders(ctx: &Ctx, id: &str) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let resumed = DESTINATIONS
        .lock()
        .ok()
        .and_then(|kept| kept.get(id).cloned());
    if let Some(parent) = resumed.as_deref().and_then(Path::parent) {
        folders.push(parent.to_owned());
    }
    if let Ok(session) = ctx
        .state
        .session_manager()
        .get_session(&ctx.call.session_id, false)
        .await
    {
        folders.push(session.working_dir);
    }
    for typed in ["~/Downloads", "~"] {
        if let Ok(folder) = local_path(typed) {
            folders.push(folder);
        }
    }
    folders
}

/// The first of `folders` that holds `name` as the download whose destination identity is
/// `identity`: the folder and the name, as the transfer service recorded them.
fn find_destination(folders: &[PathBuf], name: &str, identity: &str) -> Option<PathBuf> {
    folders
        .iter()
        .map(|folder| folder.join(name))
        .find(|candidate| identity_at(candidate).as_deref() == Some(identity))
}

/// Where an unfinished download that began writing was saving to, and its partial file. The
/// transfer service records only the folder's identity, never its path, so the folders a
/// download most likely went to are each checked against it ([`destination_folders`]); one
/// that matches is that folder for certain. Refused when none does.
async fn partial_of(ctx: &Ctx, receipt: &Receipt, identity: &str) -> Result<Partial> {
    let folders = destination_folders(ctx, &receipt.id).await;
    let destination = find_destination(&folders, &receipt.name, identity).ok_or_else(|| {
        said(
            None,
            format!(
                "Biorouter can't tell from a chat where this download was saving to, so it can't delete its partial file here. Choose Remove from list for it in Crew, under Files, where you choose its destination again, or run biorouter crew files forget {} --file with that path.",
                receipt.id
            ),
        )
    })?;
    let folder = destination.parent().map(Path::to_owned).unwrap_or_default();
    Ok(Partial {
        part: folder.join(local_files::part_name(&receipt.id)),
        destination,
    })
}

/// What removing `receipt` from the list does beyond its record, as its card says it, in the
/// words Crew's Files tab uses for Remove from list (`removeFromListHelp`,
/// `unfinishedPartStays` in `ui/desktop/src/components/crew/files/copy.ts`).
fn forget_lines(receipt: &Receipt, partial: Option<&Partial>) -> Vec<String> {
    let finished = receipt.state == "completed";
    let mut lines = vec!["Removes the record on this computer.".to_owned()];
    match (receipt.direction, finished, partial) {
        (_, true, _) => lines.push("Shared files and saved downloads stay.".to_owned()),
        (Direction::Upload, false, _) => lines.push(
            "The unfinished part stays on the server for up to a day and counts toward the workspace's file space until then.".to_owned(),
        ),
        (Direction::Download, false, Some(partial)) if partial.part.exists() => lines.push(format!(
            "Its partial file is deleted from this computer: {}.",
            partial.part.display()
        )),
        (Direction::Download, false, Some(partial)) => lines.push(format!(
            "Its partial file, {}, is already gone.",
            partial.part.display()
        )),
        (Direction::Download, false, None) => {
            lines.push("Nothing of it was saved on this computer yet.".to_owned());
        }
    }
    if !finished {
        lines.push("It can't be resumed after this.".to_owned());
    }
    lines
}

/// `crew__change files_forget`: remove a transfer that is not under way from this computer's
/// list (Crew's Remove from list), as `DELETE /crew/transfers/{id}` does. For an unfinished
/// download, its partial file is deleted too, and the card names that file's path. The
/// workspace's attachments and a finished download's file are never deleted.
pub(super) fn files_forget(ctx: &Ctx) -> BoxFuture<'_, Result<Value>> {
    Box::pin(async move {
        let id = ctx.required("transfer_id")?;
        let receipt = receipt_of(&id).await?;
        let state = receipt.state.as_str();
        if UNDER_WAY.contains(&state) || state == "pause_requested" {
            return Err(said(
                None,
                "That transfer is still under way. Pause it first with crew__change action files_pause, then remove it from the list with files_forget.",
            ));
        }
        let cleanup = (receipt.direction == Direction::Download && state != "completed")
            .then(|| destination_identity(&receipt))
            .flatten();
        let partial = match &cleanup {
            Some(identity) => Some(partial_of(ctx, &receipt, identity).await?),
            None => None,
        };
        let mut card = transfer_card(ctx, &receipt, |transfer| {
            format!("Remove from your list: {transfer}")
        })
        .await?;
        for line in forget_lines(&receipt, partial.as_ref()) {
            card = card.line(line);
        }
        if let Some(partial) = &partial {
            card = card.show("partial_file", partial.part.display().to_string());
        }
        ctx.approve(card).await?;
        let service = transfers().await?;
        let capability = match &partial {
            Some(partial) => Some(cleanup_selection(&service, &receipt, partial).await?),
            None => None,
        };
        let deleted = partial
            .as_ref()
            .is_some_and(|partial| partial.part.exists());
        service
            .forget(&id, capability.as_deref())
            .await
            .map_err(transfer_refusal)?;
        Ok(json!({
            "forgotten": true,
            "transfer_id": id,
            "partial_file_deleted": deleted,
        }))
    })
}

/// The cleanup selection `DELETE /crew/transfers/{id}` takes for an unfinished download: its
/// destination, selected again for removing its partial file, as `POST /crew/files` selects it
/// with `purpose` `cleanup`.
async fn cleanup_selection(
    service: &TransferService,
    receipt: &Receipt,
    partial: &Partial,
) -> Result<String> {
    let capability = service
        .register(FileRequest {
            approval_pending: false,
            expected_mode: None,
            purpose: FilePurpose::Cleanup,
            connection_id: receipt.connection_id.clone(),
            channel_id: receipt.channel_id.clone(),
            direction: Direction::Download,
            path: partial.destination.clone(),
            overwrite: false,
            blob_id: receipt.blob_id.clone(),
            transfer_id: Some(receipt.id.clone()),
            request_id: None,
        })
        .await
        .map_err(transfer_refusal)?;
    Ok(capability.capability_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(fingerprint: &str, via: Option<&str>, at: Option<u64>) -> NewDevice {
        NewDevice {
            fingerprint: fingerprint.into(),
            added_via: via.map(str::to_owned),
            added_at: at,
        }
    }

    /// A receipt as the transfer service serializes one, with `overrides` on top.
    fn receipt(overrides: Value) -> Receipt {
        let mut value = json!({
            "id": "0123456789abcdef0123456789abcdef",
            "request_id": "chat-call-0",
            "connection_id": "conn",
            "channel_id": "c-general",
            "direction": "upload",
            "name": "plot.png",
            "size": 2048,
            "sha256": "",
            "offset": 1024,
            "blob_id": null,
            "state": "needs_file_selection",
            "error": "stopped",
            "pause_reason": null,
            "binding": "b",
        });
        for (key, field) in overrides.as_object().unwrap() {
            value[key] = field.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    /// A fingerprint is matched however it is spaced or cased, each computer is named once,
    /// and the daemon's own spelling is what gets acknowledged; one that isn't new is refused.
    #[test]
    fn named_computers_are_matched_as_the_daemon_lists_them() {
        let new = [
            device("3F2A 9C1E 77B0 D4E1", Some("account"), Some(1)),
            device("AAAA BBBB CCCC DDDD", None, None),
        ];
        let chosen = chosen_devices(
            &new,
            &["3f2a-9c1e-77b0-d4e1".into(), "3F2A9C1E77B0D4E1".into()],
            "lab",
        )
        .unwrap();
        assert_eq!(chosen, vec![new[0].clone()]);
        let refused = chosen_devices(&new, &["1111 2222 3333 4444".into()], "lab").unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("isn't one of the new computers on lab"),
            "{refused}"
        );
    }

    /// The card lists a computer as `biorouter crew` and the desktop say it: its fingerprint,
    /// when it was added, and how in words, never a raw code; what isn't known is left unsaid.
    #[test]
    fn a_new_computer_is_listed_with_how_and_when_it_was_added() {
        let line = device_line(&device(
            "3F2A 9C1E 77B0 D4E1",
            Some("account"),
            Some(1_790_200_000),
        ));
        assert!(line.starts_with("3F2A 9C1E 77B0 D4E1, added "), "{line}");
        assert!(line.ends_with(" by signing in to the server"), "{line}");
        assert!(line.contains("2026"), "{line}");
        assert_eq!(
            device_line(&device("AAAA BBBB CCCC DDDD", Some("raw_code"), None)),
            "AAAA BBBB CCCC DDDD"
        );
    }

    /// A transfer's name reaches the model only when the read may carry it, and a transfer it
    /// may not name is its ID, state, direction and size alone: no channel or attachment ID that
    /// `files_status` or the workspace could be asked about.
    #[test]
    fn a_transfer_is_named_only_when_asked_to_be() {
        let receipt = receipt(json!({"blob_id": "6d1f2a3b-0000-4000-8000-00000000b10b"}));
        let unnamed = receipt_view(&receipt, false);
        let mut keys: Vec<&str> = unnamed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["direction", "size", "state", "transfer_id"]);
        assert_eq!(unnamed["transfer_id"], receipt.id);
        assert_eq!(unnamed["state"], "needs_file_selection");
        assert_eq!(unnamed["size"], 2048);
        let named = receipt_view(&receipt, true);
        assert_eq!(named["name"], "plot.png");
        assert_eq!(named["error"], "stopped");
        assert_eq!(named["channel_id"], "c-general");
        assert_eq!(named["blob_id"], "6d1f2a3b-0000-4000-8000-00000000b10b");
        assert_eq!(named["offset"], 1024);
    }

    /// The service's own bookkeeping is read as it serializes it, and an upload whose Private
    /// mark can't be read counts as marked.
    #[test]
    fn bookkeeping_is_read_and_fails_closed() {
        assert!(!requires_private(&receipt(json!({}))));
        assert!(requires_private(&receipt(
            json!({"requires_private": true})
        )));
        assert_eq!(destination_identity(&receipt(json!({}))), None);
        assert_eq!(
            destination_identity(&receipt(json!({"destination_identity": "abc"}))).as_deref(),
            Some("abc")
        );
    }

    /// Only a stopped transfer is resumed; one finished, one whose publication needs looking at
    /// and one still moving are each refused in words.
    #[test]
    fn only_a_stopped_transfer_is_resumed() {
        for state in ["needs_file_selection", "failed"] {
            assert!(
                resumable(&receipt(json!({"state": state}))).is_ok(),
                "{state}"
            );
        }
        for (state, words) in [
            ("completed", "finished"),
            ("publication_unconfirmed", "couldn't confirm"),
            ("uploading", "under way"),
            ("publishing", "under way"),
            ("pause_requested", "stopping"),
        ] {
            let refused = resumable(&receipt(json!({"state": state}))).unwrap_err();
            assert!(refused.to_string().contains(words), "{state}: {refused}");
        }
    }

    /// The card says what removing a transfer from the list does beyond the record, in the
    /// Files tab's words: a partial file deleted, by its path, what an unfinished upload leaves
    /// on the server, and that an unfinished transfer can't be resumed after.
    #[test]
    fn forgetting_says_what_it_deletes() {
        let download = receipt(json!({"direction": "download", "state": "failed"}));
        let partial = Partial {
            destination: PathBuf::from("/home/you/Downloads/plot.png"),
            part: PathBuf::from("/home/you/Downloads/.biorouter-crew-0123.part"),
        };
        let lines = forget_lines(&download, Some(&partial));
        assert_eq!(lines[0], "Removes the record on this computer.");
        assert!(
            lines[1].contains("/home/you/Downloads/.biorouter-crew-0123.part"),
            "{lines:?}"
        );
        assert_eq!(lines[2], "It can't be resumed after this.");
        let finished = receipt(json!({"state": "completed"}));
        assert_eq!(
            forget_lines(&finished, None),
            [
                "Removes the record on this computer.",
                "Shared files and saved downloads stay."
            ]
        );
        let unfinished = forget_lines(&receipt(json!({})), None);
        assert!(
            unfinished[1].starts_with("The unfinished part stays on the server for up to a day"),
            "{unfinished:?}"
        );
    }

    /// The card's words for removing a transfer are the Files tab's own, byte for byte but for
    /// the apostrophe, read from its copy deck: the one place both halves are visible at once.
    #[test]
    fn removing_from_the_list_is_worded_as_the_files_tab_words_it() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/crew/files/copy.ts");
        let copy = std::fs::read_to_string(&path)
            .expect("the desktop's files copy deck")
            .replace('\u{2019}', "'");
        let finished = forget_lines(&receipt(json!({"state": "completed"})), None).join(" ");
        let unfinished = forget_lines(&receipt(json!({})), None)[..2].join(" ");
        for words in [finished, unfinished] {
            assert!(
                copy.contains(&format!("'{words}'")),
                "{words:?} is not in {}",
                path.display()
            );
        }
    }

    /// A download's destination is found by the identity the transfer service recorded: the
    /// folder that holds it, never one that merely has a file of the same name.
    #[cfg(unix)]
    #[test]
    fn a_destination_is_found_by_its_recorded_identity() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let root = tempfile::tempdir_in(base).unwrap();
        let folder = |name: &str| {
            let path = root.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        };
        let (decoy, saved) = (folder("decoy"), folder("saved"));
        let identity = identity_at(&saved.join("plot.png")).unwrap();
        assert_eq!(
            find_destination(&[decoy.clone(), saved.clone()], "plot.png", &identity),
            Some(saved.join("plot.png"))
        );
        assert_eq!(find_destination(&[decoy], "plot.png", &identity), None);
        assert_eq!(find_destination(&[saved], "other.png", &identity), None);
    }

    /// The invitee is read as the invitation route reads its `invitee`: `@` and blanks dropped,
    /// none when blank, and a name that is not an account refused in the route's words.
    #[test]
    fn an_invitee_is_read_as_the_route_reads_it() {
        assert_eq!(invitee(None).unwrap(), None);
        assert_eq!(invitee(Some("  ")).unwrap(), None);
        assert_eq!(invitee(Some(" @bob ")).unwrap().as_deref(), Some("bob"));
        assert_eq!(invitee(Some("bob")).unwrap().as_deref(), Some("bob"));
        let refused = invitee(Some("not a name")).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "Type the person's username on the server, like @bob."
        );
        assert_eq!(
            refused
                .downcast_ref::<super::super::Said>()
                .and_then(|said| said.code),
            Some("crew_invalid_selector")
        );
    }

    /// Resuming says the file goes marked Private from a Private chat, and for an upload that
    /// was already marked Private, from any chat.
    #[test]
    fn a_resume_says_when_the_file_stays_private() {
        let plain = receipt(json!({}));
        let marked = receipt(json!({"requires_private": true}));
        assert_eq!(private_line(false, &plain), None);
        for (private, receipt) in [(true, &plain), (false, &marked), (true, &marked)] {
            assert_eq!(
                private_line(private, receipt),
                Some("It is marked Private, so public models never read it.")
            );
        }
    }

    /// A pause answers paused only when the transfer is stopping; one that finished or failed
    /// while the card waited is said as such, never as paused.
    #[test]
    fn a_pause_says_whether_it_paused() {
        for state in ["pause_requested", "needs_file_selection"] {
            let answer = pause_answer(&receipt(json!({"state": state})));
            assert_eq!(answer["paused"], true, "{state}");
            assert!(
                answer["note"]
                    .as_str()
                    .unwrap()
                    .starts_with("It stops in a moment."),
                "{answer}"
            );
        }
        for state in ["completed", "failed"] {
            let answer = pause_answer(&receipt(json!({"state": state})));
            assert_eq!(answer["paused"], false, "{state}");
            assert!(
                answer["note"]
                    .as_str()
                    .unwrap()
                    .contains("before it could be paused"),
                "{answer}"
            );
            assert!(answer.get("name").is_none(), "{answer}");
        }
    }

    /// A card names a transfer by its file and size, and which way it goes.
    #[test]
    fn a_transfer_is_named_on_its_card() {
        let (what, toward) = transfer_phrase(&receipt(json!({})));
        assert_eq!(
            (what.as_str(), toward),
            ("the upload of plot.png (2 KB)", "to")
        );
        let (what, toward) = transfer_phrase(&receipt(json!({"direction": "download", "size": 0})));
        assert_eq!(
            (what.as_str(), toward),
            ("the download of plot.png", "from")
        );
        assert_eq!(
            progress_line(&receipt(json!({}))),
            "1 KB of 2 KB sent so far."
        );
    }
}
