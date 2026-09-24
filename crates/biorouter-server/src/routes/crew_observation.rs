//! Human room observation shared by native CLI and desktop clients.
use anyhow::{bail, ensure, Context, Result};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
    Json, Router,
};
use biorouter::crew::manager;
use biorouter::crew::observation::MessagePerson;
pub use biorouter::crew::observation::{Initial, ObserveEvent, ObserveRequest};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use bytes::Bytes;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    convert::Infallible,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

const MAX_FRAME: usize = 1_048_576;
/// Finished runs a `state` frame lists per channel, newest first. Live runs are always listed.
const FINISHED_RUNS_PER_CHANNEL: usize = 20;
/// Where a queued message keeps the display names its page gave it (see `annotate_page`). The
/// NUL makes it a key no broker message field can have; `next_page_frame` removes it before the
/// message leaves.
const PAGE_NAMES: &str = "\u{0}page_names";
/// The most messages one `messages` frame carries: the clients' own limit, and the broker's
/// largest page.
const MAX_MESSAGES_PER_FRAME: usize = 200;
/// The encoded messages (with their names) one `messages` frame carries, well under
/// `MAX_FRAME`. A single larger message still travels, alone.
const FRAME_BUDGET: usize = 256 * 1024;
static SLOTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(16)));

fn person(headers: &HeaderMap) -> Result<()> {
    ensure!(
        matches!(user_action_proof(headers), UserActionProof::Proven),
        "human_authority_required"
    );
    Ok(())
}
fn broker_code(error: &anyhow::Error) -> Option<String> {
    let text = error.to_string();
    let encoded = text.strip_prefix("Crew broker refused request: ")?;
    serde_json::from_str::<Value>(encoded)
        .ok()?
        .get("code")?
        .as_str()
        .map(str::to_owned)
}
fn observation_error_code(error: &anyhow::Error) -> String {
    if let Some(code) = broker_code(error) {
        return code;
    }
    match error.to_string().as_str() {
        "policy_changed" => "policy_changed",
        "channel_access_changed" => "channel_access_changed",
        "scope_changed" => "scope_changed",
        "human_authority_required" => "human_authority_required",
        "observer_capacity_reached" => "observer_capacity_reached",
        _ => "observation_refused",
    }
    .into()
}

/// The plain sentence an error frame carries beside its code. The desktop app words each code
/// itself; this is what a terminal (`crew watch`) prints, so it names what happened and never
/// the observer's internals.
fn observation_error_text(code: &str) -> &'static str {
    match code {
        "policy_changed" => {
            "The workspace's privacy settings, or this computer's connection to it, changed"
        }
        "scope_changed" => "You can no longer read a channel that messages here come from",
        "channel_access_changed" => "You no longer have access to this channel",
        "stale_cursor" => "A message in this view is no longer available to you",
        "human_authority_required" => {
            "Watching a channel needs you to confirm it's you in Biorouter"
        }
        "observer_capacity_reached" => "Too many channels are being watched at once",
        "response_too_large" => "An update was too large to show",
        "unauthorized" | "unknown_device" => "This computer isn't signed in to the workspace",
        "forbidden" | "access_denied" => "You no longer have access to what this view shows",
        "principal_revoked" => "You're no longer a member of this workspace",
        "privacy_denied" => "The workspace's privacy rules don't allow this view",
        _ => "Live updates stopped",
    }
}

/// Where an observer reads the saved connection and the broker: the daemon's Crew manager, or a
/// scripted broker in tests.
#[async_trait::async_trait]
trait ObservationSource: Send + Sync {
    /// The saved connection's privacy binding (see `connection_binding`).
    async fn binding(&self, connection: &str) -> Result<Value>;
    /// One read the person's own device signs.
    async fn read(&self, connection: &str, method: &str, params: Value) -> Result<Value>;
    /// The runs this device owns on the connection.
    async fn runs(&self, connection: &str) -> Result<Vec<super::crew::RunView>>;
    /// What the broker's last verified `hello` said it supports.
    fn capabilities(&self, connection: &str) -> Result<Vec<String>>;
}

/// The daemon's own Crew manager.
struct Daemon;
#[async_trait::async_trait]
impl ObservationSource for Daemon {
    async fn binding(&self, connection: &str) -> Result<Value> {
        connection_binding(&manager()?.connection(connection).await?)
    }
    async fn read(&self, connection: &str, method: &str, params: Value) -> Result<Value> {
        manager()?
            .human_request(connection, method, params, None)
            .await
    }
    async fn runs(&self, connection: &str) -> Result<Vec<super::crew::RunView>> {
        super::crew::owned_run_views(connection).await
    }
    fn capabilities(&self, connection: &str) -> Result<Vec<String>> {
        Ok(manager()?.capabilities(connection).unwrap_or_default())
    }
}

/// What an observer last verified against a fresh snapshot: the workspace policy epoch, the
/// privacy and institution the view was opened under, and every channel the person could read.
#[derive(Debug)]
struct Verified {
    epoch: Value,
    privacy: Value,
    readable: BTreeSet<String>,
}
impl Verified {
    fn from_snapshot(snapshot: &Value) -> Result<Self> {
        let workspace = &snapshot["workspace"];
        let epoch = workspace["policy_epoch"].clone();
        ensure!(!epoch.is_null(), "Invalid workspace policy");
        Ok(Self {
            epoch,
            privacy: json!([workspace["mode"], workspace["institution_id"]]),
            readable: snapshot["channels"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|channel| channel["id"].as_str().map(str::to_owned))
                .collect(),
        })
    }

    /// Moves to a newer verification.
    ///
    /// SECURITY-SENSITIVE (human review): a moved workspace policy epoch is not a reason to end.
    /// Every accepted invitation, added member or archived channel moves it, and ending there
    /// collapsed every member's open channel (P0-1). A person reads a message when they can read
    /// its channel and every channel it was derived from (`visible` in `biorouter-crew`'s
    /// broker), so someone who can still read every channel they could read can still read
    /// everything this observer already delivered. The view is re-authorized on that basis and
    /// continues. A privacy or institution change (`policy_changed`), or losing any channel
    /// (`scope_changed`), ends it instead: what is on screen may no longer be theirs to see.
    fn advance(&mut self, next: Self) -> Result<()> {
        ensure!(next.privacy == self.privacy, "policy_changed");
        ensure!(next.readable.is_superset(&self.readable), "scope_changed");
        if next.epoch != self.epoch {
            tracing::debug!(from = %self.epoch, to = %next.epoch,
                "Room observation re-authorized under a new workspace policy epoch");
        }
        *self = next;
        Ok(())
    }
}

/// What the broker's read check for a person looks at in a message: its channel and the
/// channels it was derived from. Messages that share a key share the broker's answer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ReadKey {
    Channels {
        channel: String,
        sources: BTreeSet<String>,
    },
    /// A message whose channel or sources do not parse is decided on its own, by the broker.
    Message(String),
}
impl ReadKey {
    fn of(message: &Value) -> Self {
        let sources = message["source_channels"].as_array().and_then(|sources| {
            sources
                .iter()
                .map(|source| source.as_str().map(str::to_owned))
                .collect::<Option<BTreeSet<_>>>()
        });
        match (message["channel_id"].as_str(), sources) {
            (Some(channel), Some(sources)) => Self::Channels {
                channel: channel.to_owned(),
                sources,
            },
            _ => Self::Message(message["sequence"].as_str().unwrap_or_default().to_owned()),
        }
    }

    /// Whether a snapshot's readable channels allow the key. A key that did not parse has only
    /// the broker's answer.
    fn readable_in(&self, readable: &BTreeSet<String>) -> bool {
        match self {
            Self::Channels { channel, sources } => {
                readable.contains(channel) && sources.is_subset(readable)
            }
            Self::Message(_) => true,
        }
    }
}

fn message_cursor(message: &Value) -> Result<String> {
    Ok(message["sequence"]
        .as_str()
        .context("Invalid message cursor")?
        .to_owned())
}

/// Each message's key and the cursor the broker resolves for it.
fn read_keys<'a>(messages: impl IntoIterator<Item = &'a Value>) -> Result<Vec<(ReadKey, String)>> {
    messages
        .into_iter()
        .map(|message| Ok((ReadKey::of(message), message_cursor(message)?)))
        .collect()
}

/// How many of the pending messages the next frame carries: every one that fits
/// `FRAME_BUDGET` and `MAX_MESSAGES_PER_FRAME`, and always at least one.
fn frame_batch_len(pending: &VecDeque<Value>) -> usize {
    let mut size = 0usize;
    let mut count = 0;
    for message in pending.iter().take(MAX_MESSAGES_PER_FRAME) {
        let length = serde_json::to_vec(message).map_or(usize::MAX, |encoded| encoded.len());
        if count > 0 && size.saturating_add(length) > FRAME_BUDGET {
            break;
        }
        size = size.saturating_add(length);
        count += 1;
    }
    count.max(1)
}

type Decisions = HashMap<ReadKey, std::result::Result<(), String>>;

struct Observer {
    headers: HeaderMap,
    connection: String,
    request: ObserveRequest,
    cursor: Option<String>,
    pending: VecDeque<Value>,
    binding: Value,
    verified: Option<Verified>,
    first: bool,
    state_due: bool,
    sleep_due: bool,
    last_state: Option<tokio::time::Instant>,
    limit: usize,
    deadline: tokio::time::Instant,
    done: bool,
    _permit: Option<OwnedSemaphorePermit>,
    source: Arc<dyn ObservationSource>,
}
impl Observer {
    fn new(
        headers: HeaderMap,
        connection: String,
        request: ObserveRequest,
        binding: Value,
        source: Arc<dyn ObservationSource>,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            headers,
            connection,
            cursor: request.after.clone(),
            request,
            pending: VecDeque::new(),
            binding,
            verified: None,
            first: true,
            state_due: true,
            sleep_due: false,
            last_state: None,
            limit: 200,
            deadline: tokio::time::Instant::now() + Duration::from_secs(600),
            done: false,
            _permit: permit,
            source,
        }
    }

    /// The checks that need no broker: cancellation, the person's proof, and that the saved
    /// connection still has the privacy binding the observation was opened under.
    async fn check_local(&self, cancel: &CancellationToken) -> Result<()> {
        ensure!(!cancel.is_cancelled(), "observation_cancelled");
        person(&self.headers)?;
        ensure!(
            self.source.binding(&self.connection).await? == self.binding,
            "policy_changed"
        );
        Ok(())
    }

    /// Verifies the observation against a fresh snapshot, and returns that snapshot. A policy
    /// epoch that moved is re-authorized under the new one (see [`Verified::advance`]).
    async fn authorize(&mut self, cancel: &CancellationToken) -> Result<Value> {
        self.check_local(cancel).await?;
        let snapshot = self
            .source
            .read(&self.connection, "workspace.snapshot", json!({}))
            .await?;
        ensure!(!cancel.is_cancelled(), "observation_cancelled");
        let next = Verified::from_snapshot(&snapshot)?;
        if let Some(channel) = &self.request.channel_id {
            ensure!(next.readable.contains(channel), "channel_access_changed");
        }
        match &mut self.verified {
            Some(verified) => verified.advance(next)?,
            None => self.verified = Some(next),
        }
        self.check_local(cancel).await?;
        Ok(snapshot)
    }

    /// Resolves a message cursor in the observed channel. The broker refuses one the person can
    /// no longer read (`stale_cursor`), including after a revocation that moved no epoch.
    async fn resolve(&self, cursor: &str) -> Result<Value> {
        self.source
            .read(
                &self.connection,
                "messages.history",
                json!({"channel_id":self.request.channel_id,"after":cursor,"limit":0}),
            )
            .await
    }

    /// The broker's decision for each distinct key among `messages`, taken between two
    /// snapshots.
    ///
    /// SECURITY-SENSITIVE (human review): this replaces one cursor check per message, which kept
    /// a busy channel on its skeleton for seconds. The broker decides a person's read from the
    /// message's channel and source channels alone (`visible` in `biorouter-crew`'s broker), so
    /// it resolves one message per distinct key and that answer is every such message's answer.
    /// An allowed key must also be readable in the second snapshot, so a revocation that lands
    /// while the broker answers still withholds what it covers.
    async fn decide(
        &mut self,
        messages: &[(ReadKey, String)],
        cancel: &CancellationToken,
    ) -> Result<Decisions> {
        self.authorize(cancel).await?;
        let mut decisions = Decisions::new();
        for (key, cursor) in messages {
            if decisions.contains_key(key) {
                continue;
            }
            ensure!(!cancel.is_cancelled(), "observation_cancelled");
            let decision = self
                .resolve(cursor)
                .await
                .map(drop)
                .map_err(|error| error.to_string());
            decisions.insert(key.clone(), decision);
        }
        self.authorize(cancel).await?;
        let readable = &self
            .verified
            .as_ref()
            .context("The view was not verified")?
            .readable;
        for (key, decision) in &mut decisions {
            if decision.is_ok() && !key.readable_in(readable) {
                *decision = Err("scope_changed".into());
            }
        }
        Ok(decisions)
    }

    async fn state_frame(&mut self, cancel: &CancellationToken) -> Result<Value> {
        self.authorize(cancel).await?;
        let mut runs = self.source.runs(&self.connection).await?;
        let snapshot = self.authorize(cancel).await?;
        runs.retain(|run| {
            snapshot["channels"].as_array().is_some_and(|channels| {
                channels
                    .iter()
                    .any(|channel| channel["id"] == run.channel_id)
            })
        });
        let capabilities = self.source.capabilities(&self.connection)?;
        person(&self.headers)?;
        Ok(state_frame_json(
            &self.connection,
            &self.binding,
            snapshot,
            runs,
            capabilities,
        ))
    }

    async fn admit_frame(&mut self, frame: &Bytes, cancel: &CancellationToken) -> Result<Value> {
        let frame: Value = serde_json::from_slice(frame)?;
        if frame["type"] == "state" {
            let frame = self.state_frame(cancel).await?;
            self.last_state = Some(tokio::time::Instant::now());
            return Ok(frame);
        }
        if frame["type"] == "messages" {
            if let Some(messages) = frame["messages"].as_array().filter(|m| !m.is_empty()) {
                self.admit_messages(messages, &frame["cursor"], cancel)
                    .await?;
                return Ok(frame);
            }
        }
        self.authorize(cancel).await?;
        if frame["type"] == "messages" {
            if let Some(cursor) = frame["cursor"].as_str() {
                self.resolve(cursor).await?;
            }
        }
        self.authorize(cancel).await?;
        Ok(frame)
    }

    /// A queued frame's messages, decided again now: the frame may have waited in the queue
    /// across a revocation. Nothing decided when it was made is reused.
    async fn admit_messages(
        &mut self,
        messages: &[Value],
        cursor: &Value,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.check_local(cancel).await?;
        let mut keys = read_keys(messages)?;
        // The frame's resume cursor is its last message's; one that is not is decided alone.
        if let Some(cursor) = cursor.as_str() {
            if !keys.iter().any(|(_, known)| known == cursor) {
                keys.push((ReadKey::Message(cursor.to_owned()), cursor.to_owned()));
            }
        }
        let decisions = self.decide(&keys, cancel).await?;
        for (key, _) in &keys {
            match decisions.get(key) {
                Some(Ok(())) => {}
                Some(Err(refusal)) => bail!("{refusal}"),
                None => bail!("observation_refused"),
            }
        }
        Ok(())
    }

    async fn history(&mut self, cancel: &CancellationToken) -> Result<VecDeque<Value>> {
        let channel = self
            .request
            .channel_id
            .clone()
            .context("No channel selected")?;
        loop {
            ensure!(!cancel.is_cancelled(), "observation_cancelled");
            let mut params = json!({"channel_id":channel,"limit":self.limit,
                "latest":self.first && self.cursor.is_none() && matches!(self.request.initial, Initial::Latest)});
            if let Some(cursor) = &self.cursor {
                params["after"] = json!(cursor);
            }
            match self
                .source
                .read(&self.connection, "messages.history", params)
                .await
            {
                Ok(page) => {
                    let messages = page["messages"]
                        .as_array()
                        .context("Invalid history page")?;
                    ensure!(messages.len() <= self.limit, "Invalid history page size");
                    return Ok(annotate_page(messages, &page));
                }
                Err(error)
                    if self.limit > 1
                        && broker_code(&error).as_deref() == Some("response_too_large") =>
                {
                    self.limit = (self.limit / 2).max(1)
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// The next frame of the current page: as many of its messages as fit one frame, each
    /// decided by the broker now (see [`Self::decide`]). A message the broker refuses is never
    /// sent: the frame stops before it, and it ends the observation when it is the next to go,
    /// as when each message was checked alone, so nothing after it is sent either.
    async fn next_page_frame(&mut self, cancel: &CancellationToken) -> Result<Value> {
        let count = frame_batch_len(&self.pending);
        let keys = read_keys(self.pending.iter().take(count))?;
        let decisions = self.decide(&keys, cancel).await?;
        let allowed = keys
            .iter()
            .take_while(|(key, _)| matches!(decisions.get(key), Some(Ok(()))))
            .count();
        if allowed == 0 {
            match decisions.get(&keys[0].0) {
                Some(Err(refusal)) => bail!("{refusal}"),
                _ => bail!("observation_refused"),
            }
        }
        let mut messages = Vec::with_capacity(allowed);
        let mut names = Vec::new();
        for (_, cursor) in &keys[..allowed] {
            ensure!(
                self.cursor.as_ref() != Some(cursor),
                "History cursor did not advance"
            );
            let mut message = self.pending.pop_front().context("Invalid history page")?;
            if let Some(page_names) = message
                .as_object_mut()
                .and_then(|fields| fields.remove(PAGE_NAMES))
            {
                names.push(page_names);
            }
            messages.push(message);
            self.cursor = Some(cursor.clone());
        }
        let reset = self.first && self.request.after.is_none();
        self.first = false;
        Ok(messages_frame(
            &self.request.channel_id,
            messages,
            &self.cursor,
            reset,
            self.pending.len(),
            self.limit,
            names,
        ))
    }

    /// The next frame, after the pause between two state frames when one is due. `produce`
    /// steps itself, so it can wait out the pause without holding this observer.
    #[cfg(test)]
    async fn next_frame(&mut self, cancel: &CancellationToken) -> Result<Value> {
        loop {
            match self.step(cancel).await? {
                Step::Frame(frame) => return Ok(frame),
                Step::Idle => idle_wait(cancel).await?,
            }
        }
    }

    /// The next frame, or `Idle` when nothing changed and the next state frame waits out the
    /// pause. The pause is the caller's, so it can wait without holding this observer.
    async fn step(&mut self, cancel: &CancellationToken) -> Result<Step> {
        loop {
            if tokio::time::Instant::now() >= self.deadline {
                self.done = true;
                return Ok(Step::Frame(
                    json!({"type":"reconnect","cursor":self.cursor}),
                ));
            }
            ensure!(!cancel.is_cancelled(), "observation_cancelled");
            if self.state_due
                || self
                    .last_state
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            {
                if self.state_due && self.sleep_due {
                    self.sleep_due = false;
                    return Ok(Step::Idle);
                }
                let frame = self.state_frame(cancel).await?;
                self.last_state = Some(tokio::time::Instant::now());
                self.state_due = false;
                self.sleep_due = true;
                return Ok(Step::Frame(frame));
            }
            if !self.pending.is_empty() {
                return self.next_page_frame(cancel).await.map(Step::Frame);
            }
            if self.request.channel_id.is_none() {
                self.state_due = true;
                continue;
            }
            self.pending = self.history(cancel).await?;
            if self.pending.is_empty() {
                self.authorize(cancel).await?;
                self.state_due = true;
                if self.first {
                    self.first = false;
                    return Ok(Step::Frame(messages_frame(
                        &self.request.channel_id,
                        Vec::new(),
                        &self.cursor,
                        self.request.after.is_none(),
                        0,
                        self.limit,
                        Vec::new(),
                    )));
                }
            }
        }
    }
}

/// What an observer's producer does next.
enum Step {
    Frame(Value),
    /// Wait out the pause between two state frames, then step again.
    Idle,
}

/// The pause between two state frames when nothing else is due.
async fn idle_wait(cancel: &CancellationToken) -> Result<()> {
    tokio::select! {
        () = tokio::time::sleep(Duration::from_secs(2)) => Ok(()),
        () = cancel.cancelled() => bail!("observation_cancelled"),
    }
}

#[utoipa::path(post, path = "/crew/connections/{id}/observe", params(("id" = String, Path, description = "Saved Crew connection")), request_body = ObserveRequest, responses((status = 200, description = "Bounded NDJSON room events (one schema instance per line)", body = ObserveEvent, content_type = "application/x-ndjson")), tag = "Crew")]
pub async fn observe(
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<ObserveRequest>,
) -> Result<Response, (StatusCode, Json<Value>)> {
    person(&headers).map_err(|_| {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"Verified human Crew authority is required"})),
        )
    })?;
    if request
        .channel_id
        .as_ref()
        .is_some_and(|v| v.is_empty() || v.len() > 128)
        || request
            .after
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 128)
        || (request.after.is_some() && request.channel_id.is_none())
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Invalid room observation selection"})),
        ));
    }
    let permit = SLOTS.clone().try_acquire_owned().map_err(|_| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"Too many room observers"})),
        )
    })?;
    let connection = manager()
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"Crew unavailable"})),
            )
        })?
        .connection(&id)
        .await
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"Crew connection unavailable"})),
            )
        })?;
    let binding = connection_binding(&connection).unwrap();
    let observer = Observer::new(
        headers,
        id,
        request,
        binding,
        Arc::new(Daemon),
        Some(permit),
    );
    let stream = observation_stream(observer);
    Ok(Response::builder()
        .header("Content-Type", "application/x-ndjson")
        .header("Cache-Control", "no-store")
        .body(Body::from_stream(stream))
        .unwrap())
}

/// A `messages` frame: the messages it carries (one or more of a page, see `frame_batch_len`),
/// how many of that page are still to come (`0` ends the page, and so the opening backlog), the
/// page size the observer asks for now, and the display names the messages' page gave them
/// (`annotate_page`), limited to `people` and `channel_names`.
fn messages_frame(
    channel_id: &Option<String>,
    messages: Vec<Value>,
    cursor: &Option<String>,
    reset: bool,
    remaining: usize,
    page_size: usize,
    names: Vec<Value>,
) -> Value {
    let mut frame = json!({"type":"messages","channel_id":channel_id,
        "messages":messages,"cursor":cursor,"reset":reset,
        "remaining":u32::try_from(remaining).unwrap_or(u32::MAX),
        "page_size":u32::try_from(page_size).unwrap_or(u32::MAX)});
    for key in ["people", "channel_names"] {
        let merged: serde_json::Map<String, Value> = names
            .iter()
            .filter_map(|entry| entry.get(key)?.as_object())
            .flatten()
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        if !merged.is_empty() {
            frame[key] = Value::Object(merged);
        }
    }
    frame
}

/// The page's messages, each carrying the display names that belong to it under `PAGE_NAMES`:
/// the `people` entry for its author and the `channel_names` of the channels it names. Only
/// well-formed entries are kept, so a frame's `people` always parses as `MessagePerson`.
///
/// The names travel on the message rather than beside the queue, so a message can never be sent
/// with another page's names.
fn annotate_page(messages: &[Value], page: &Value) -> VecDeque<Value> {
    let people: HashMap<&str, MessagePerson> = page["people"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(id, person)| {
            serde_json::from_value::<MessagePerson>(person.clone())
                .ok()
                .filter(|person| !person.username.is_empty())
                .map(|person| (id.as_str(), person))
        })
        .collect();
    let channel_names: HashMap<&str, &str> = page["channel_names"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(id, name)| Some((id.as_str(), name.as_str()?)))
        .collect();
    messages
        .iter()
        .map(|message| {
            let mut message = message.clone();
            // Only this function writes the key: one the broker sent is dropped, never trusted.
            if let Some(fields) = message.as_object_mut() {
                fields.remove(PAGE_NAMES);
            }
            let mut names = serde_json::Map::new();
            let author = message["actor_id"]
                .as_str()
                .and_then(|id| people.get(id).map(|person| (id.to_owned(), person.clone())));
            if let Some((id, person)) = author {
                names.insert("people".into(), json!(BTreeMap::from([(id, person)])));
            }
            let named: BTreeMap<&str, &str> = std::iter::once(&message["channel_id"])
                .chain(message["source_channels"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .filter_map(|id| Some((id, *channel_names.get(id)?)))
                .collect();
            if !named.is_empty() {
                names.insert("channel_names".into(), json!(named));
            }
            if let (false, Some(fields)) = (names.is_empty(), message.as_object_mut()) {
                fields.insert(PAGE_NAMES.into(), Value::Object(names));
            }
            message
        })
        .collect()
}

/// A run that is still going, or waiting on a person: always listed.
fn run_is_live(status: &str) -> bool {
    matches!(
        status,
        "starting"
            | "running"
            | "waiting_for_approval"
            | "cancellation_pending"
            | "cancellation_unconfirmed"
    )
}

/// The runs a `state` frame lists: every live run, and the `FINISHED_RUNS_PER_CHANNEL` newest
/// finished runs of each channel, newest first. The ledger keeps every run a device ever started;
/// the frame must not grow with it.
fn bounded_runs(mut runs: Vec<super::crew::RunView>) -> Vec<super::crew::RunView> {
    super::crew::sort_newest_first(&mut runs);
    let mut finished: HashMap<String, usize> = HashMap::new();
    runs.retain(|run| {
        if run_is_live(&run.status) {
            return true;
        }
        let listed = finished.entry(run.channel_id.clone()).or_default();
        *listed += 1;
        *listed <= FINISHED_RUNS_PER_CHANNEL
    });
    runs
}

/// A `state` frame. Its `labels` are computed from the very snapshot it carries, so a label
/// never names someone the frame does not.
fn state_frame_json(
    connection: &str,
    binding: &Value,
    snapshot: Value,
    runs: Vec<super::crew::RunView>,
    capabilities: Vec<String>,
) -> Value {
    let labels = super::crew::names::project_labels(&snapshot);
    let runs = bounded_runs(runs);
    let mut frame = json!({"type":"state","connection_id":connection,"connection_mode":binding["mode"],"connection_policy_epoch":binding["policy_epoch"],"connection_institution_id":binding["institution_id"],"snapshot":snapshot,"runs":runs,"labels":labels});
    if !capabilities.is_empty() {
        frame["capabilities"] = json!(capabilities);
    }
    frame
}

fn connection_binding(connection: &biorouter::crew::Connection) -> Result<Value> {
    let mut binding = serde_json::to_value(connection)?;
    let fields = binding.as_object_mut().context("Invalid connection")?;
    fields.remove("status");
    fields.remove("last_error");
    Ok(binding)
}

struct ObservationReceiver {
    receiver: mpsc::Receiver<Bytes>,
    terminal: Option<oneshot::Receiver<Bytes>>,
    deferred_terminal: Option<Bytes>,
    observer: Arc<Mutex<Observer>>,
    cancel: CancellationToken,
    finished: bool,
}
impl ObservationReceiver {
    fn finish(&mut self) {
        self.finished = true;
        self.cancel.cancel();
        self.receiver.close();
        while self.receiver.try_recv().is_ok() {}
        self.terminal = None;
        self.deferred_terminal = None;
    }

    async fn next_queued_frame(&mut self) -> Option<Bytes> {
        loop {
            tokio::select! {
                biased;
                result = async { self.terminal.as_mut().unwrap().await }, if self.terminal.is_some() => {
                    self.terminal = None;
                    if let Ok(frame) = result {
                        if clears_cached_content(&frame) {
                            return Some(frame);
                        }
                        self.deferred_terminal = Some(frame);
                    }
                }
                frame = self.receiver.recv() => {
                    if frame.is_some() {
                        return frame;
                    }
                    if let Some(frame) = self.deferred_terminal.take() {
                        return Some(frame);
                    }
                    return self.terminal.take()?.await.ok();
                }
            }
        }
    }

    async fn next_frame(&mut self) -> Option<Bytes> {
        if self.finished {
            return None;
        }
        let frame = self.next_queued_frame().await?;
        if clears_cached_content(&frame) {
            self.finish();
            return Some(frame);
        }
        let observer = self.observer.clone();
        let cancel = self.cancel.clone();
        // The task retains the observer permit and drains any broker exchange
        // even if HTTP drops this stream while admission is awaiting a reply.
        let admitted = tokio::spawn(async move {
            let mut observer = observer.lock().await;
            let _admission_permit = if observer._permit.is_none() {
                match SLOTS.clone().try_acquire_owned() {
                    Ok(permit) => Some(permit),
                    Err(_) => {
                        return encode_frame(
                            &mut observer,
                            Err(anyhow::anyhow!("observer_capacity_reached")),
                        );
                    }
                }
            } else {
                None
            };
            let result = observer.admit_frame(&frame, &cancel).await;
            encode_frame(&mut observer, result)
        })
        .await;
        let frame = match admitted {
            Ok(frame) => frame,
            Err(_) => {
                self.finish();
                return Some(Bytes::from_static(b"{\"type\":\"error\",\"code\":\"observation_refused\",\"error\":\"Live updates stopped\",\"clear\":true}\n"));
            }
        };
        if let Some(terminal) = &mut self.terminal {
            match terminal.try_recv() {
                Ok(terminal_frame) => {
                    self.terminal = None;
                    if clears_cached_content(&terminal_frame) {
                        self.finish();
                        return Some(terminal_frame);
                    }
                    self.deferred_terminal = Some(terminal_frame);
                }
                Err(oneshot::error::TryRecvError::Closed) => self.terminal = None,
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if clears_cached_content(&frame) {
            self.finish();
        }
        Some(frame)
    }
}

fn clears_cached_content(frame: &Bytes) -> bool {
    serde_json::from_slice::<Value>(frame)
        .ok()
        .is_some_and(|frame| frame["clear"] == true)
}

impl Drop for ObservationReceiver {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn encode_frame(observer: &mut Observer, result: Result<Value>) -> Bytes {
    let frame = match result {
        Ok(frame) => frame,
        Err(error) => {
            observer.done = true;
            let code = observation_error_code(&error);
            // `clear`: whatever this observation showed must leave the screen with it.
            json!({"type":"error","code":code,"error":observation_error_text(&code),"clear":true})
        }
    };
    let mut encoded = serde_json::to_vec(&frame).unwrap();
    if encoded.len() >= MAX_FRAME {
        observer.done = true;
        encoded = br#"{"type":"error","code":"response_too_large","error":"An update was too large to show","clear":true}"#.to_vec();
    }
    encoded.push(b'\n');
    Bytes::from(encoded)
}

async fn deliver_frame(
    sender: &mpsc::Sender<Bytes>,
    frame: Bytes,
    terminal: bool,
    delivered_cursor: Option<&str>,
    timeout: Duration,
) -> std::result::Result<(), Option<Bytes>> {
    match sender.send_timeout(frame, timeout).await {
        Ok(()) => Ok(()),
        Err(mpsc::error::SendTimeoutError::Closed(_)) => Err(None),
        Err(mpsc::error::SendTimeoutError::Timeout(frame)) => {
            if terminal {
                Err(Some(frame))
            } else {
                let mut fallback = serde_json::to_vec(&json!({
                    "type":"reconnect", "cursor":delivered_cursor,
                }))
                .unwrap();
                fallback.push(b'\n');
                Err(Some(Bytes::from(fallback)))
            }
        }
    }
}

fn planned_expiry_reconnect(expired: bool, result: &Result<Value>) -> bool {
    expired
        && match result {
            Ok(_) => true,
            Err(error) => error.to_string() == "observation_cancelled",
        }
}

async fn produce(
    observer: Arc<Mutex<Observer>>,
    sender: mpsc::Sender<Bytes>,
    terminal: oneshot::Sender<Bytes>,
    cancel: CancellationToken,
) {
    let mut terminal = Some(terminal);
    let lifetime_cancel = cancel.clone();
    let deadline = observer.lock().await.deadline;
    let watchdog = tokio::spawn(async move {
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => lifetime_cancel.cancel(),
            () = lifetime_cancel.cancelled() => (),
        }
    });
    loop {
        // Never cancel a broker future mid-exchange: late JSONL replies must be
        // drained before another caller uses this shared SSH transport. The
        // observer permit stays owned until that bounded operation finishes.
        let mut guard = observer.lock().await;
        if guard.done {
            break;
        }
        let delivered_cursor = guard.cursor.clone();
        let result = match guard.step(&cancel).await {
            Ok(Step::Frame(frame)) => Ok(frame),
            Ok(Step::Idle) => {
                // The pause holds nothing: a queued frame's admission, which takes this same
                // observer, must not wait behind it (it held every channel open for 2 s).
                drop(guard);
                match idle_wait(&cancel).await {
                    Ok(()) => continue,
                    Err(error) => {
                        guard = observer.lock().await;
                        Err(error)
                    }
                }
            }
            Err(error) => Err(error),
        };
        let mut observer = guard;
        let expired = tokio::time::Instant::now() >= observer.deadline;
        if cancel.is_cancelled() && !expired {
            break;
        }
        let result = if planned_expiry_reconnect(expired, &result) {
            observer.cursor = delivered_cursor.clone();
            observer.done = true;
            Ok(json!({"type":"reconnect","cursor":observer.cursor}))
        } else {
            result
        };
        let frame = encode_frame(&mut observer, result);
        let done = observer.done;
        drop(observer);
        if clears_cached_content(&frame) {
            if let Some(terminal) = terminal.take() {
                let _ = terminal.send(frame);
            }
            break;
        }
        if let Err(fallback) = deliver_frame(
            &sender,
            frame,
            done,
            delivered_cursor.as_deref(),
            Duration::from_secs(5),
        )
        .await
        {
            if let (Some(sender), Some(frame)) = (terminal.take(), fallback) {
                let _ = sender.send(frame);
            }
            break;
        }
        if done {
            break;
        }
    }
    cancel.cancel();
    watchdog.abort();
    // Idle HTTP bodies must not retain a slot after producer expiry. Taking it
    // under the same lock waits for an active admission exchange to drain.
    drop(observer.lock().await._permit.take());
}

fn observation_stream(
    observer: Observer,
) -> impl futures::Stream<Item = Result<Bytes, Infallible>> {
    let (sender, receiver) = mpsc::channel(1);
    let (terminal_sender, terminal_receiver) = oneshot::channel();
    let cancel = CancellationToken::new();
    let observer = Arc::new(Mutex::new(observer));
    tokio::spawn(produce(
        observer.clone(),
        sender,
        terminal_sender,
        cancel.child_token(),
    ));
    futures::stream::unfold(
        ObservationReceiver {
            receiver,
            terminal: Some(terminal_receiver),
            deferred_terminal: None,
            observer,
            cancel,
            finished: false,
        },
        |mut state| async move { state.next_frame().await.map(|frame| (Ok(frame), state)) },
    )
}

pub fn routes() -> Router {
    Router::new()
        .route("/crew/connections/{id}/observe", post(observe))
        .layer(DefaultBodyLimit::max(4096))
}

#[cfg(test)]
mod tests {
    use super::reauthorize::plain_observer;
    use super::*;

    fn test_observer() -> Arc<Mutex<Observer>> {
        Arc::new(Mutex::new({
            let mut observer = plain_observer(Some("channel"), Some("cursor"), None);
            observer.connection = "revoked-derived-source".into();
            observer.first = false;
            observer.state_due = false;
            observer
        }))
    }

    fn observation_receiver(receiver: mpsc::Receiver<Bytes>) -> ObservationReceiver {
        ObservationReceiver {
            receiver,
            terminal: None,
            deferred_terminal: None,
            observer: test_observer(),
            cancel: CancellationToken::new(),
            finished: false,
        }
    }

    #[test]
    fn observation_error_codes_preserve_policy_and_broker_boundaries() {
        assert_eq!(
            observation_error_code(&anyhow::anyhow!("policy_changed")),
            "policy_changed"
        );
        assert_eq!(
            observation_error_code(&anyhow::anyhow!("channel_access_changed")),
            "channel_access_changed"
        );
        assert_eq!(
            observation_error_code(&anyhow::anyhow!("human_authority_required")),
            "human_authority_required"
        );
        assert_eq!(
            observation_error_code(&anyhow::anyhow!(
                "Crew broker refused request: {{\"code\":\"stale_cursor\"}}"
            )),
            "stale_cursor"
        );
        assert_eq!(
            observation_error_code(&anyhow::anyhow!("unexpected observer failure")),
            "observation_refused"
        );
    }

    #[test]
    fn oversized_encoded_frame_is_replaced_by_bounded_error_frame() {
        let permit = SLOTS.clone().try_acquire_owned().unwrap();
        let mut observer = {
            let mut observer = plain_observer(None, None, Some(permit));
            observer.state_due = false;
            observer
        };
        let frame = encode_frame(&mut observer, Ok(json!({"payload": "x".repeat(MAX_FRAME)})));
        let value: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(value["type"], "error");
        assert_eq!(value["code"], "response_too_large");
        assert!(value["clear"].as_bool().unwrap_or(false));
        assert!(observer.done);
    }

    #[test]
    fn state_wire_contract_requires_connection_identity_and_mode() {
        for mode in [
            biorouter::crew::ClusterMode::Public,
            biorouter::crew::ClusterMode::Private,
        ] {
            let event = ObserveEvent::State {
                connection_id: "connection-123".into(),
                connection_mode: mode,
                connection_policy_epoch: 1,
                connection_institution_id: Some("ucsf".into()),
                snapshot: json!({}),
                runs: vec![],
                labels: Default::default(),
                capabilities: vec![],
            };
            let encoded = serde_json::to_vec(&event).unwrap();
            let decoded: ObserveEvent = serde_json::from_slice(&encoded).unwrap();
            match decoded {
                ObserveEvent::State {
                    connection_id,
                    connection_mode,
                    ..
                } => {
                    assert_eq!(connection_id, "connection-123");
                    assert_eq!(connection_mode, mode);
                }
                _ => panic!("unexpected observer event type"),
            }
        }

        for invalid in [
            json!({"type":"state","snapshot":{},"runs":[]}),
            json!({"type":"state","connection_id":"connection-123","connection_mode":"restricted","snapshot":{},"runs":[]}),
        ] {
            assert!(
                serde_json::from_value::<ObserveEvent>(invalid).is_err(),
                "state without a valid connection mode must be rejected"
            );
        }
    }

    #[test]
    fn state_frames_carry_labels_computed_from_their_own_snapshot() {
        let person = |id: &str, username: &str, display_name: &str| {
            json!({"id": id, "username": username, "nickname": display_name,
                "display_name": display_name, "active": true})
        };
        let snapshot = json!({
            "actor": person("p-alice", "alice", "Alice Chen"),
            "principals": [
                person("p-alice", "alice", "Alice Chen"),
                person("p-spark", "spark", "Sam Park"),
                person("p-sampark", "sampark", "Sam Park"),
                person("p-carol", "carol", "carol"),
            ],
            "former_principals": [
                {"id": "p-dave", "username": "dave", "display_name": "Dave Old", "active": false},
            ],
        });
        let binding = json!({"mode": "private", "policy_epoch": 3, "institution_id": "ucsf"});
        let frame = state_frame_json("connection-1", &binding, snapshot, vec![], vec![]);
        assert_eq!(
            frame["labels"]["p-spark"],
            json!({"full": "Sam Park (@spark)", "short": "Sam Park (@spark)", "collides": true})
        );
        assert_eq!(
            frame["labels"]["p-alice"],
            json!({"full": "Alice Chen (@alice)", "short": "Alice Chen", "collides": false})
        );
        assert_eq!(frame["labels"]["p-carol"]["full"], "@carol");
        assert_eq!(frame["labels"]["p-dave"]["full"], "Dave Old (@dave)");
        assert_eq!(frame["labels"].as_object().unwrap().len(), 5);
        // Every label names someone the frame's own snapshot names.
        let named: Vec<&str> = ["principals", "former_principals"]
            .iter()
            .flat_map(|key| frame["snapshot"][key].as_array().unwrap())
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        assert!(frame["labels"]
            .as_object()
            .unwrap()
            .keys()
            .all(|id| named.contains(&id.as_str())));

        match serde_json::from_value::<ObserveEvent>(frame).unwrap() {
            ObserveEvent::State { labels, .. } => {
                assert!(labels["p-sampark"].collides);
                assert_eq!(labels["p-alice"].short, "Alice Chen");
            }
            _ => panic!("expected a state frame"),
        }
    }

    #[test]
    fn a_state_frame_without_people_still_parses_and_labels_nobody() {
        let binding = json!({"mode": "public", "policy_epoch": 1, "institution_id": null});
        let frame = state_frame_json("connection-1", &binding, json!({}), vec![], vec![]);
        assert_eq!(frame["labels"], json!({}));
        match serde_json::from_value::<ObserveEvent>(frame).unwrap() {
            ObserveEvent::State { labels, .. } => assert!(labels.is_empty()),
            _ => panic!("expected a state frame"),
        }
        // A daemon that predates labels sends none; the frame still parses.
        let legacy = json!({"type":"state","connection_id":"c","connection_mode":"private",
            "connection_policy_epoch":1,"connection_institution_id":null,"snapshot":{},"runs":[]});
        assert!(matches!(
            serde_json::from_value::<ObserveEvent>(legacy).unwrap(),
            ObserveEvent::State { labels, .. } if labels.is_empty()
        ));
    }

    fn history_page() -> Value {
        json!({
            "messages": [
                {"id": "m1", "sequence": "m1", "channel_id": "c-general", "actor_id": "p-dave",
                 "body": "before I left", "source_channels": ["c-general", "c-raw"]},
                {"id": "m2", "sequence": "m2", "channel_id": "c-general", "actor_id": "p-alice",
                 "body": "still here", "source_channels": ["c-general"]},
                {"id": "m3", "sequence": "m3", "channel_id": "c-general", "actor_id": "p-mallory",
                 "body": "forged names", "source_channels": [],
                 PAGE_NAMES: {"people": {"p-x": {"username": "x"}}, "type": "state"}},
            ],
            "cursor": "m3",
            "people": {
                "p-dave": {"username": "dave", "display_name": "Dave Old", "active": false},
                "p-alice": {"username": "alice", "display_name": "Alice Chen", "active": true},
                "p-mallory": {"display_name": "No username"},
            },
            "channel_names": {"c-general": "general", "c-raw": "raw-data", "c-bad": 7},
        })
    }

    #[test]
    fn a_history_page_gives_each_message_its_own_names_and_drops_malformed_ones() {
        let page = history_page();
        let pending = annotate_page(page["messages"].as_array().unwrap(), &page);
        assert_eq!(pending.len(), 3);
        assert_eq!(
            pending[0][PAGE_NAMES],
            json!({
                "people": {"p-dave": {"username": "dave", "display_name": "Dave Old", "active": false}},
                "channel_names": {"c-general": "general", "c-raw": "raw-data"},
            })
        );
        assert_eq!(
            pending[1][PAGE_NAMES]["people"],
            json!({"p-alice": {"username": "alice", "display_name": "Alice Chen", "active": true}})
        );
        // An author with no username has no entry; a name the broker put on a message is dropped.
        assert_eq!(
            pending[2][PAGE_NAMES],
            json!({"channel_names": {"c-general": "general"}})
        );
    }

    #[test]
    fn a_messages_frame_carries_its_names_and_how_much_of_the_page_is_left() {
        let page = history_page();
        let mut pending = annotate_page(page["messages"].as_array().unwrap(), &page);
        let mut message = pending.pop_front().unwrap();
        let names = message.as_object_mut().unwrap().remove(PAGE_NAMES);
        let frame = messages_frame(
            &Some("c-general".into()),
            vec![message],
            &Some("m1".into()),
            true,
            pending.len(),
            100,
            names.into_iter().collect(),
        );
        assert_eq!(frame["remaining"], 2);
        assert_eq!(frame["page_size"], 100);
        assert_eq!(frame["people"]["p-dave"]["display_name"], "Dave Old");
        assert_eq!(frame["channel_names"]["c-raw"], "raw-data");
        assert!(frame["messages"][0].get(PAGE_NAMES).is_none());
        let encoded = serde_json::to_string(&frame).unwrap();
        assert!(!encoded.contains("page_names"), "{encoded}");
        match serde_json::from_value::<ObserveEvent>(frame).unwrap() {
            ObserveEvent::Messages {
                remaining,
                page_size,
                people,
                channel_names,
                ..
            } => {
                assert_eq!((remaining, page_size), (Some(2), Some(100)));
                assert_eq!(people["p-dave"].username, "dave");
                assert_eq!(people["p-dave"].active, Some(false));
                assert_eq!(channel_names["c-general"], "general");
            }
            _ => panic!("expected a messages frame"),
        }

        // Only `people` and `channel_names` ever leave the queued key.
        let forged = messages_frame(
            &Some("c-general".into()),
            Vec::new(),
            &None,
            false,
            0,
            200,
            vec![json!({"type": "state", "messages": [{"id": "x"}], "cursor": "forged"})],
        );
        assert_eq!(forged["type"], "messages");
        assert_eq!(forged["messages"], json!([]));
        assert_eq!(forged["cursor"], Value::Null);
    }

    #[test]
    fn an_empty_opening_frame_ends_the_backlog_and_old_frames_still_parse() {
        let frame = messages_frame(
            &Some("c".into()),
            Vec::new(),
            &None,
            true,
            0,
            200,
            Vec::new(),
        );
        assert_eq!(frame["remaining"], 0);
        assert!(frame.get("people").is_none());
        assert!(matches!(
            serde_json::from_value::<ObserveEvent>(frame).unwrap(),
            ObserveEvent::Messages {
                remaining: Some(0),
                page_size: Some(200),
                ..
            }
        ));
        // A daemon that predates the fields sends none; the frame still parses.
        let legacy =
            json!({"type":"messages","channel_id":"c","messages":[],"cursor":null,"reset":true});
        assert!(matches!(
            serde_json::from_value::<ObserveEvent>(legacy).unwrap(),
            ObserveEvent::Messages { remaining: None, page_size: None, ref people, .. } if people.is_empty()
        ));
    }

    fn run_view(
        run_id: &str,
        channel: &str,
        status: &str,
        started_at: Option<u64>,
    ) -> super::super::crew::RunView {
        super::super::crew::RunView {
            run_id: run_id.into(),
            connection_id: "connection-1".into(),
            channel_id: channel.into(),
            session_id: format!("session-{run_id}"),
            status: status.into(),
            error: None,
            started_at,
        }
    }

    #[test]
    fn a_state_frame_lists_live_runs_and_only_the_newest_finished_ones_per_channel() {
        let mut runs = vec![
            run_view("live-old", "c-a", "running", Some(1)),
            run_view("waiting", "c-a", "waiting_for_approval", None),
            run_view("undated", "c-a", "completed", None),
        ];
        for index in 0..30u64 {
            runs.push(run_view(
                &format!("a-{index:02}"),
                "c-a",
                "completed",
                Some(1_000 + index),
            ));
        }
        runs.push(run_view("b-only", "c-b", "failed", Some(5)));
        let binding = json!({"mode": "private", "policy_epoch": 1, "institution_id": null});
        let frame = state_frame_json("connection-1", &binding, json!({}), runs, vec![]);
        let listed: Vec<&str> = frame["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| run["run_id"].as_str().unwrap())
            .collect();
        assert!(listed.contains(&"live-old") && listed.contains(&"waiting"));
        assert!(listed.contains(&"b-only"));
        let finished_a: Vec<&str> = listed
            .iter()
            .copied()
            .filter(|id| id.starts_with("a-") || *id == "undated")
            .collect();
        let newest: Vec<String> = (10..30u64)
            .rev()
            .map(|index| format!("a-{index:02}"))
            .collect();
        assert_eq!(
            finished_a, newest,
            "the 20 newest finished runs, newest first"
        );
        // Newest first overall; a run with no start time sorts after every dated one.
        assert_eq!(listed[0], "a-29");
        assert_eq!(listed.last(), Some(&"waiting"));
        assert_eq!(frame["runs"][0]["started_at"], 1_029);
        assert!(frame["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["run_id"] == "waiting" && run.get("started_at").is_none()));
    }

    #[test]
    fn a_state_frame_carries_the_broker_capabilities_only_when_it_has_some() {
        let binding = json!({"mode": "private", "policy_epoch": 1, "institution_id": null});
        let frame = state_frame_json(
            "connection-1",
            &binding,
            json!({}),
            vec![],
            vec!["unique_names_v1".into()],
        );
        assert_eq!(frame["capabilities"], json!(["unique_names_v1"]));
        match serde_json::from_value::<ObserveEvent>(frame).unwrap() {
            ObserveEvent::State { capabilities, .. } => {
                assert_eq!(capabilities, vec!["unique_names_v1".to_owned()])
            }
            _ => panic!("expected a state frame"),
        }
        let frame = state_frame_json("connection-1", &binding, json!({}), vec![], vec![]);
        assert!(frame.get("capabilities").is_none());
        assert!(matches!(
            serde_json::from_value::<ObserveEvent>(frame).unwrap(),
            ObserveEvent::State { ref capabilities, .. } if capabilities.is_empty()
        ));
    }

    #[test]
    fn broker_code_accepts_only_the_exact_refusal_envelope() {
        let error =
            anyhow::anyhow!("Crew broker refused request: {{\"code\":\"response_too_large\"}}");
        assert_eq!(broker_code(&error).as_deref(), Some("response_too_large"));

        let unrelated = anyhow::anyhow!("response_too_large");
        assert!(broker_code(&unrelated).is_none());
        let malformed =
            anyhow::anyhow!("Crew broker refused request: {{\"message\":\"response_too_large\"}}");
        assert!(broker_code(&malformed).is_none());
    }

    #[tokio::test]
    async fn expired_observer_emits_one_reconnect_frame_and_releases_the_stream() {
        let permit = SLOTS.clone().try_acquire_owned().unwrap();
        let observer = {
            let mut observer = plain_observer(Some("channel"), Some("cursor"), Some(permit));
            observer.state_due = false;
            observer.deadline = tokio::time::Instant::now() - Duration::from_secs(1);
            observer
        };
        let observer = Arc::new(Mutex::new(observer));
        let (sender, mut receiver) = mpsc::channel(1);
        let (terminal_sender, _terminal_receiver) = oneshot::channel();
        produce(observer, sender, terminal_sender, CancellationToken::new()).await;
        let frame = receiver.recv().await.unwrap();
        let frame: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(frame["type"], "reconnect");
        assert_eq!(frame["cursor"], "cursor");
        assert!(receiver.recv().await.is_none());
    }

    #[tokio::test]
    async fn a_full_queue_regular_frame_falls_back_to_the_last_delivered_cursor() {
        let (sender, mut receiver) = mpsc::channel(1);
        sender.try_send(Bytes::from_static(b"accepted\n")).unwrap();

        let fallback = deliver_frame(
            &sender,
            Bytes::from_static(b"attempted\n"),
            false,
            Some("accepted-cursor"),
            Duration::from_millis(10),
        )
        .await
        .expect_err("a full queue must use the reconnect fallback");
        let fallback = fallback.expect("an open receiver gets a reconnect fallback");
        let value: Value = serde_json::from_slice(&fallback).unwrap();
        assert_eq!(value["type"], "reconnect");
        assert_eq!(value["cursor"], "accepted-cursor");
        assert_eq!(
            receiver.recv().await.unwrap(),
            Bytes::from_static(b"accepted\n")
        );
    }

    #[tokio::test]
    async fn a_full_queue_terminal_frame_preserves_the_exact_clear_error() {
        let (sender, _receiver) = mpsc::channel(1);
        sender.try_send(Bytes::from_static(b"accepted\n")).unwrap();
        let terminal = Bytes::from_static(
            br#"{"type":"error","code":"channel_access_changed","clear":true}
"#,
        );

        let fallback = deliver_frame(
            &sender,
            terminal.clone(),
            true,
            Some("accepted-cursor"),
            Duration::from_millis(10),
        )
        .await
        .expect_err("a full queue must preserve terminal bytes");
        assert_eq!(fallback, Some(terminal));
    }

    #[tokio::test]
    async fn clear_error_wins_over_a_queued_derived_frame_after_revocation() {
        let (sender, receiver) = mpsc::channel(1);
        sender
            .try_send(Bytes::from_static(
                br#"{"type":"messages","messages":[{"text":"derived-canary"}],"cursor":"canary"}
"#,
            ))
            .unwrap();
        drop(sender);

        let (terminal_sender, terminal_receiver) = oneshot::channel();
        terminal_sender
            .send(Bytes::from_static(
                br#"{"type":"error","code":"channel_access_changed","clear":true}
"#,
            ))
            .unwrap();
        let observer = Arc::new(Mutex::new({
            let mut observer = plain_observer(
                Some("channel"),
                Some("cursor"),
                Some(SLOTS.clone().try_acquire_owned().unwrap()),
            );
            observer.connection = "revoked-derived-source".into();
            observer.first = false;
            observer.state_due = false;
            observer
        }));
        let mut receiver = ObservationReceiver {
            receiver,
            terminal: Some(terminal_receiver),
            deferred_terminal: None,
            observer,
            cancel: CancellationToken::new(),
            finished: false,
        };

        let frame = receiver
            .next_frame()
            .await
            .expect("revocation must produce a terminal clear frame");
        let value: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(value["type"], "error");
        assert_eq!(value["code"], "channel_access_changed");
        assert_eq!(value["clear"], true);
        assert!(!frame
            .windows(b"derived-canary".len())
            .any(|window| { window == b"derived-canary" }));
        assert!(receiver.finished);
        assert!(receiver.next_frame().await.is_none());
    }

    #[tokio::test]
    async fn queued_derived_frame_is_reauthorized_before_receiver_admits_it() {
        let (sender, receiver) = mpsc::channel(1);
        sender
            .try_send(Bytes::from_static(
                br#"{"type":"messages","messages":[{"text":"derived-canary"}],"cursor":"canary"}
"#,
            ))
            .unwrap();
        drop(sender);
        let mut receiver = observation_receiver(receiver);

        let frame = receiver
            .next_frame()
            .await
            .expect("queued frame must become an admission error");
        let value: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(value["type"], "error");
        assert_eq!(value["code"], "human_authority_required");
        assert_eq!(value["clear"], true);
        assert!(!frame
            .windows(b"derived-canary".len())
            .any(|window| { window == b"derived-canary" }));
        assert!(receiver.next_frame().await.is_none());
    }

    #[tokio::test]
    async fn queued_admission_error_closes_a_waiting_second_frame_without_delivery() {
        let (sender, receiver) = mpsc::channel(1);
        sender
            .try_send(Bytes::from_static(
                br#"{"type":"messages","messages":[{"text":"derived-canary-1"}],"cursor":"canary-1"}
"#,
            ))
            .unwrap();
        let second_sender = sender.clone();
        let second_send = tokio::spawn(async move {
            second_sender
                .send(Bytes::from_static(
                    br#"{"type":"messages","messages":[{"text":"derived-canary-2"}],"cursor":"canary-2"}
"#,
                ))
                .await
        });
        drop(sender);
        let mut receiver = observation_receiver(receiver);

        let frame = receiver
            .next_frame()
            .await
            .expect("first queued frame must become an admission error");
        let value: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(value["code"], "human_authority_required");
        assert_eq!(value["clear"], true);
        assert!(receiver.next_frame().await.is_none());
        if let Ok(()) = second_send.await.unwrap() {
            assert!(receiver.receiver.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn queued_data_is_drained_before_one_terminal_fallback_then_eof() {
        let (sender, receiver) = mpsc::channel(1);
        sender.try_send(Bytes::from_static(b"data\n")).unwrap();
        drop(sender);
        let (terminal_sender, terminal_receiver) = oneshot::channel();
        terminal_sender
            .send(Bytes::from_static(b"reconnect\n"))
            .unwrap();
        let mut receiver = ObservationReceiver {
            receiver,
            terminal: Some(terminal_receiver),
            deferred_terminal: None,
            observer: Arc::new(Mutex::new({
                let mut observer = plain_observer(None, None, None);
                observer.first = false;
                observer.state_due = false;
                observer
            })),
            cancel: CancellationToken::new(),
            finished: false,
        };

        assert_eq!(
            receiver.next_queued_frame().await,
            Some(Bytes::from_static(b"data\n"))
        );
        assert_eq!(
            receiver.next_queued_frame().await,
            Some(Bytes::from_static(b"reconnect\n"))
        );
        assert_eq!(receiver.next_frame().await, None);
    }

    #[tokio::test]
    async fn a_closed_receiver_does_not_create_a_reconnect_fallback() {
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);

        let result = deliver_frame(
            &sender,
            Bytes::from_static(b"terminal\n"),
            false,
            Some("cursor"),
            Duration::from_millis(10),
        )
        .await;
        assert_eq!(result, Err(None));
    }

    #[tokio::test]
    async fn dropping_the_receiver_releases_the_producer_permit() {
        let slots = Arc::new(Semaphore::new(1));
        let permit = slots.clone().acquire_owned().await.unwrap();
        let observer = Arc::new(Mutex::new({
            let mut observer = plain_observer(None, None, Some(permit));
            observer.cursor = Some("cursor".into());
            observer.state_due = false;
            observer.deadline = tokio::time::Instant::now() - Duration::from_secs(1);
            observer
        }));
        let (sender, receiver) = mpsc::channel(1);
        let (terminal_sender, _terminal_receiver) = oneshot::channel();
        drop(receiver);
        produce(observer, sender, terminal_sender, CancellationToken::new()).await;
        assert_eq!(slots.available_permits(), 1);
    }

    #[test]
    fn expiry_reconnect_accepts_only_planned_cancellation_or_success() {
        assert!(planned_expiry_reconnect(
            true,
            &Ok(json!({"type": "state"}))
        ));
        assert!(planned_expiry_reconnect(
            true,
            &Err(anyhow::anyhow!("observation_cancelled"))
        ));
        assert!(!planned_expiry_reconnect(
            true,
            &Err(anyhow::anyhow!("policy_changed"))
        ));
        assert!(!planned_expiry_reconnect(
            true,
            &Err(anyhow::anyhow!(
                "Crew broker refused request: {{\"code\":\"channel_access_changed\"}}"
            ))
        ));
        assert!(!planned_expiry_reconnect(
            true,
            &Err(anyhow::anyhow!("other cancellation"))
        ));
        assert!(!planned_expiry_reconnect(
            false,
            &Err(anyhow::anyhow!("observation_cancelled"))
        ));
    }

    #[tokio::test]
    async fn observation_refuses_before_accessing_a_saved_connection_without_proof() {
        let result = observe(
            HeaderMap::new(),
            Path("missing-connection".into()),
            Json(ObserveRequest {
                channel_id: Some("channel".into()),
                after: None,
                initial: Initial::Latest,
            }),
        )
        .await;
        let (status, Json(body)) = result.expect_err("missing proof must refuse");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body["error"].as_str().unwrap().contains("human"));
    }
}

#[cfg(test)]
#[path = "crew_observation_reauthorize_tests.rs"]
mod reauthorize;

#[cfg(test)]
#[path = "crew_observation_live_acceptance_tests.rs"]
mod live_acceptance;
