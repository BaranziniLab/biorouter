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
pub use biorouter::crew::observation::{Initial, ObserveEvent, ObserveRequest};
use biorouter_server::auth::{user_action_proof, UserActionProof};
use bytes::Bytes;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

const MAX_FRAME: usize = 1_048_576;
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
        person(&self.headers)?;
        Ok(
            json!({"type":"state","connection_id":self.connection,"connection_mode":self.binding["mode"],"snapshot":snapshot,"runs":runs}),
        )
    }

    async fn admit_frame(&mut self, frame: &Bytes, cancel: &CancellationToken) -> Result<Value> {
        let frame: Value = serde_json::from_slice(frame)?;
        if frame["type"] == "state" {
            return self.state_frame(cancel).await;
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
                    return Ok(messages.iter().cloned().collect());
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
                let message = self.pending.pop_front().unwrap();
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
                return Ok(
                    json!({"type":"messages","channel_id":self.request.channel_id,"messages":[message],"cursor":self.cursor,"reset":reset}),
                );
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
                    return Ok(
                        json!({"type":"messages","channel_id":self.request.channel_id,"messages":[],"cursor":self.cursor,"reset":self.request.after.is_none()}),
                    );
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
                snapshot: json!({}),
                runs: vec![],
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
#[path = "crew_observation_live_acceptance.rs"]
mod live_acceptance;
