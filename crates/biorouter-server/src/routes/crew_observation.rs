//! Human room observation shared by native CLI and desktop clients.
use anyhow::{ensure, Context, Result};
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
    collections::{BTreeMap, HashMap, VecDeque},
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
/// NUL makes it a key no broker message field can have; `next_frame` removes it before the
/// message leaves.
const PAGE_NAMES: &str = "\u{0}page_names";
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
        "human_authority_required" => "human_authority_required",
        "observer_capacity_reached" => "observer_capacity_reached",
        _ => "observation_refused",
    }
    .into()
}
struct Observer {
    headers: HeaderMap,
    connection: String,
    request: ObserveRequest,
    cursor: Option<String>,
    pending: VecDeque<Value>,
    binding: Value,
    epoch: Option<Value>,
    first: bool,
    state_due: bool,
    sleep_due: bool,
    last_state: Option<tokio::time::Instant>,
    limit: usize,
    deadline: tokio::time::Instant,
    done: bool,
    _permit: Option<OwnedSemaphorePermit>,
}
impl Observer {
    async fn authorize(&mut self, cancel: &CancellationToken) -> Result<Value> {
        ensure!(!cancel.is_cancelled(), "observation_cancelled");
        person(&self.headers)?;
        let crew = manager()?;
        ensure!(
            connection_binding(&crew.connection(&self.connection).await?)? == self.binding,
            "policy_changed"
        );
        let snapshot = crew
            .human_request(&self.connection, "workspace.snapshot", json!({}), None)
            .await?;
        ensure!(!cancel.is_cancelled(), "observation_cancelled");
        if let Some(channel) = &self.request.channel_id {
            ensure!(
                snapshot["channels"]
                    .as_array()
                    .is_some_and(|channels| channels
                        .iter()
                        .any(|entry| entry["id"].as_str() == Some(channel))),
                "channel_access_changed"
            );
        }
        let epoch = snapshot["workspace"]["policy_epoch"].clone();
        ensure!(!epoch.is_null(), "Invalid workspace policy");
        ensure!(
            self.epoch
                .as_ref()
                .is_none_or(|previous| previous == &epoch),
            "policy_changed"
        );
        self.epoch = Some(epoch);
        ensure!(
            connection_binding(&crew.connection(&self.connection).await?)? == self.binding,
            "policy_changed"
        );
        person(&self.headers)?;
        Ok(snapshot)
    }
    async fn state_frame(&mut self, cancel: &CancellationToken) -> Result<Value> {
        self.authorize(cancel).await?;
        let mut runs = super::crew::owned_run_views(&self.connection).await?;
        let snapshot = self.authorize(cancel).await?;
        runs.retain(|run| {
            snapshot["channels"].as_array().is_some_and(|channels| {
                channels
                    .iter()
                    .any(|channel| channel["id"] == run.channel_id)
            })
        });
        let capabilities = manager()?
            .capabilities(&self.connection)
            .unwrap_or_default();
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
        self.authorize(cancel).await?;
        if frame["type"] == "messages" {
            if let Some(cursor) = frame["cursor"].as_str() {
                manager()?
                    .human_request(
                        &self.connection,
                        "messages.history",
                        json!({"channel_id":self.request.channel_id,"after":cursor,"limit":0}),
                        None,
                    )
                    .await?;
            }
        }
        self.authorize(cancel).await?;
        Ok(frame)
    }

    async fn history(&mut self, cancel: &CancellationToken) -> Result<VecDeque<Value>> {
        let channel = self
            .request
            .channel_id
            .as_ref()
            .context("No channel selected")?;
        loop {
            ensure!(!cancel.is_cancelled(), "observation_cancelled");
            let mut params = json!({"channel_id":channel,"limit":self.limit,
                "latest":self.first && self.cursor.is_none() && matches!(self.request.initial, Initial::Latest)});
            if let Some(cursor) = &self.cursor {
                params["after"] = json!(cursor);
            }
            match manager()?
                .human_request(&self.connection, "messages.history", params, None)
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
    async fn next_frame(&mut self, cancel: &CancellationToken) -> Result<Value> {
        loop {
            if tokio::time::Instant::now() >= self.deadline {
                self.done = true;
                return Ok(json!({"type":"reconnect","cursor":self.cursor}));
            }
            ensure!(!cancel.is_cancelled(), "observation_cancelled");
            if self.state_due
                || self
                    .last_state
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(2))
            {
                if self.state_due && self.sleep_due {
                    tokio::select! {
                        () = tokio::time::sleep(Duration::from_secs(2)) => (),
                        () = cancel.cancelled() => anyhow::bail!("observation_cancelled"),
                    }
                }
                let frame = self.state_frame(cancel).await?;
                self.last_state = Some(tokio::time::Instant::now());
                self.state_due = false;
                self.sleep_due = true;
                return Ok(frame);
            }
            if !self.pending.is_empty() {
                self.authorize(cancel).await?;
                let mut message = self.pending.pop_front().unwrap();
                let names = message
                    .as_object_mut()
                    .and_then(|fields| fields.remove(PAGE_NAMES));
                let cursor = message["sequence"]
                    .as_str()
                    .context("Invalid message cursor")?
                    .to_owned();
                ensure!(
                    self.cursor.as_ref() != Some(&cursor),
                    "History cursor did not advance"
                );
                // Resolving the cursor rechecks every inherited source-channel ACL,
                // including revocations that do not change the workspace epoch.
                manager()?
                    .human_request(
                        &self.connection,
                        "messages.history",
                        json!({"channel_id":self.request.channel_id,"after":cursor,"limit":0}),
                        None,
                    )
                    .await?;
                self.authorize(cancel).await?;
                self.cursor = Some(cursor);
                let reset = self.first && self.request.after.is_none();
                self.first = false;
                return Ok(messages_frame(
                    &self.request.channel_id,
                    Some(message),
                    &self.cursor,
                    reset,
                    self.pending.len(),
                    self.limit,
                    names,
                ));
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
                    return Ok(messages_frame(
                        &self.request.channel_id,
                        None,
                        &self.cursor,
                        self.request.after.is_none(),
                        0,
                        self.limit,
                        None,
                    ));
                }
            }
        }
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
    let observer = Observer {
        headers,
        connection: id,
        cursor: request.after.clone(),
        request,
        pending: VecDeque::new(),
        binding,
        epoch: None,
        first: true,
        state_due: true,
        sleep_due: false,
        last_state: None,
        limit: 200,
        deadline: tokio::time::Instant::now() + Duration::from_secs(600),
        done: false,
        _permit: Some(permit),
    };
    let stream = observation_stream(observer);
    Ok(Response::builder()
        .header("Content-Type", "application/x-ndjson")
        .header("Cache-Control", "no-store")
        .body(Body::from_stream(stream))
        .unwrap())
}

/// A `messages` frame: at most one message, how many of its page are still to come (`0` ends
/// the page, and so the opening backlog), the page size the observer asks for now, and the
/// display names the message's page gave it (`annotate_page`), limited to `people` and
/// `channel_names`.
fn messages_frame(
    channel_id: &Option<String>,
    message: Option<Value>,
    cursor: &Option<String>,
    reset: bool,
    remaining: usize,
    page_size: usize,
    names: Option<Value>,
) -> Value {
    let mut frame = json!({"type":"messages","channel_id":channel_id,
        "messages":message.into_iter().collect::<Vec<_>>(),"cursor":cursor,"reset":reset,
        "remaining":u32::try_from(remaining).unwrap_or(u32::MAX),
        "page_size":u32::try_from(page_size).unwrap_or(u32::MAX)});
    if let Some(Value::Object(mut names)) = names {
        for key in ["people", "channel_names"] {
            if let Some(value) = names.remove(key) {
                frame[key] = value;
            }
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
                return Some(Bytes::from_static(b"{\"type\":\"error\",\"code\":\"observation_refused\",\"error\":\"Room observation admission failed. Clear cached room content and refresh authorized access.\",\"clear\":true}\n"));
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
            json!({"type":"error","code":code,"error":"Room observation ended. Clear cached room content and refresh authorized access; a stale cursor requires an explicit fresh history selection.","clear":true})
        }
    };
    let mut encoded = serde_json::to_vec(&frame).unwrap();
    if encoded.len() >= MAX_FRAME {
        observer.done = true;
        encoded = br#"{"type":"error","code":"response_too_large","error":"Room update exceeds the bounded frame limit","clear":true}"#.to_vec();
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
        let mut observer = observer.lock().await;
        if observer.done {
            break;
        }
        let delivered_cursor = observer.cursor.clone();
        let result = observer.next_frame(&cancel).await;
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
    use super::*;

    fn test_observer() -> Arc<Mutex<Observer>> {
        Arc::new(Mutex::new(Observer {
            headers: HeaderMap::new(),
            connection: "revoked-derived-source".into(),
            request: ObserveRequest {
                channel_id: Some("channel".into()),
                after: Some("cursor".into()),
                initial: Initial::Latest,
            },
            cursor: Some("cursor".into()),
            pending: VecDeque::new(),
            binding: json!({}),
            epoch: None,
            first: false,
            state_due: false,
            sleep_due: false,
            last_state: None,
            limit: 200,
            deadline: tokio::time::Instant::now() + Duration::from_secs(600),
            done: false,
            _permit: None,
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
        let mut observer = Observer {
            headers: HeaderMap::new(),
            connection: "connection".into(),
            request: ObserveRequest {
                channel_id: None,
                after: None,
                initial: Initial::Latest,
            },
            cursor: None,
            pending: VecDeque::new(),
            binding: json!({}),
            epoch: None,
            first: true,
            state_due: false,
            sleep_due: false,
            last_state: None,
            limit: 200,
            deadline: tokio::time::Instant::now() + Duration::from_secs(600),
            done: false,
            _permit: Some(permit),
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
            Some(message),
            &Some("m1".into()),
            true,
            pending.len(),
            100,
            names,
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
            None,
            &None,
            false,
            0,
            200,
            Some(json!({"type": "state", "messages": [{"id": "x"}], "cursor": "forged"})),
        );
        assert_eq!(forged["type"], "messages");
        assert_eq!(forged["messages"], json!([]));
        assert_eq!(forged["cursor"], Value::Null);
    }

    #[test]
    fn an_empty_opening_frame_ends_the_backlog_and_old_frames_still_parse() {
        let frame = messages_frame(&Some("c".into()), None, &None, true, 0, 200, None);
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
        let observer = Observer {
            headers: HeaderMap::new(),
            connection: "connection".into(),
            request: ObserveRequest {
                channel_id: Some("channel".into()),
                after: Some("cursor".into()),
                initial: Initial::Latest,
            },
            cursor: Some("cursor".into()),
            pending: VecDeque::new(),
            binding: json!({}),
            epoch: None,
            first: true,
            state_due: false,
            sleep_due: false,
            last_state: None,
            limit: 200,
            deadline: tokio::time::Instant::now() - Duration::from_secs(1),
            done: false,
            _permit: Some(permit),
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
        let observer = Arc::new(Mutex::new(Observer {
            headers: HeaderMap::new(),
            connection: "revoked-derived-source".into(),
            request: ObserveRequest {
                channel_id: Some("channel".into()),
                after: Some("cursor".into()),
                initial: Initial::Latest,
            },
            cursor: Some("cursor".into()),
            pending: VecDeque::new(),
            binding: json!({}),
            epoch: None,
            first: false,
            state_due: false,
            sleep_due: false,
            last_state: None,
            limit: 200,
            deadline: tokio::time::Instant::now() + Duration::from_secs(600),
            done: false,
            _permit: Some(SLOTS.clone().try_acquire_owned().unwrap()),
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
            observer: Arc::new(Mutex::new(Observer {
                headers: HeaderMap::new(),
                connection: "connection".into(),
                request: ObserveRequest {
                    channel_id: None,
                    after: None,
                    initial: Initial::Latest,
                },
                cursor: None,
                pending: VecDeque::new(),
                binding: json!({}),
                epoch: None,
                first: false,
                state_due: false,
                sleep_due: false,
                last_state: None,
                limit: 200,
                deadline: tokio::time::Instant::now() + Duration::from_secs(600),
                done: false,
                _permit: None,
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
        let observer = Arc::new(Mutex::new(Observer {
            headers: HeaderMap::new(),
            connection: "connection".into(),
            request: ObserveRequest {
                channel_id: None,
                after: None,
                initial: Initial::Latest,
            },
            cursor: Some("cursor".into()),
            pending: VecDeque::new(),
            binding: json!({}),
            epoch: None,
            first: true,
            state_due: false,
            sleep_due: false,
            last_state: None,
            limit: 200,
            deadline: tokio::time::Instant::now() - Duration::from_secs(1),
            done: false,
            _permit: Some(permit),
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
#[path = "crew_observation_live_acceptance_tests.rs"]
mod live_acceptance;
