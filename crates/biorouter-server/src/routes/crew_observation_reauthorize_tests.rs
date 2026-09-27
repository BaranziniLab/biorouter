//! Re-authorization and page batching of room observation, against a scripted broker.
//!
//! `FakeBroker` decides reads the way `biorouter-crew`'s broker does for a person (`visible`): a
//! message is readable when its channel and every channel it was derived from are among the
//! person's channels. Every policy change moves the workspace epoch, as the broker's mutations do.

use super::*;
use std::sync::Mutex as StdMutex;

pub(super) struct FakeBroker {
    state: StdMutex<FakeState>,
}

/// Something that happens to the broker's state at a chosen moment.
pub(super) type Hook = Box<dyn FnMut(&mut FakeState) + Send>;

pub(super) struct FakeState {
    pub epoch: u64,
    pub mode: &'static str,
    pub institution: Option<&'static str>,
    pub member_of: BTreeSet<String>,
    /// Every message of every channel, oldest first.
    pub messages: Vec<Value>,
    pub binding: Value,
    /// Answers every cursor as resolvable: a broker whose answer disagrees with its snapshot.
    pub resolve_everything: bool,
    /// Runs before each cursor is resolved: something landing while the broker answers.
    pub before_resolve: Option<Hook>,
    /// Each read, by name: `snapshot`, `resolve:<cursor>` or `page`.
    pub calls: Vec<String>,
}

impl FakeBroker {
    pub(super) fn new(member_of: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            state: StdMutex::new(FakeState {
                epoch: 1,
                mode: "private",
                institution: Some("ucsf"),
                member_of: member_of.iter().map(|id| (*id).to_owned()).collect(),
                messages: Vec::new(),
                binding: json!({}),
                resolve_everything: false,
                before_resolve: None,
                calls: Vec::new(),
            }),
        })
    }

    pub(super) fn with<T>(&self, change: impl FnOnce(&mut FakeState) -> T) -> T {
        change(&mut self.state.lock().unwrap())
    }

    fn count(&self, prefix: &str) -> usize {
        self.with(|state| {
            state
                .calls
                .iter()
                .filter(|call| call.starts_with(prefix))
                .count()
        })
    }
}

impl FakeState {
    /// Adds a message in `channel`, derived from `sources`.
    pub fn post(&mut self, id: &str, channel: &str, sources: &[&str], body: &str) {
        self.messages
            .push(json!({"id": id, "sequence": id, "channel_id": channel,
            "actor_id": "p-alice", "body": body, "source_channels": sources}));
    }

    fn visible(&self, message: &Value) -> bool {
        ReadKey::of(message).readable_in(&self.member_of)
    }

    fn history(&mut self, params: &Value) -> Result<Value> {
        let channel = params["channel_id"].as_str().unwrap_or_default().to_owned();
        ensure!(
            self.member_of.contains(&channel),
            "Crew broker refused request: {{\"code\":\"forbidden\"}}"
        );
        let limit = params["limit"].as_u64().unwrap_or(100) as usize;
        let start = match params["after"].as_str() {
            None => 0,
            Some(cursor) => {
                if limit == 0 {
                    self.calls.push(format!("resolve:{cursor}"));
                    if let Some(mut hook) = self.before_resolve.take() {
                        hook(self);
                        self.before_resolve = Some(hook);
                    }
                }
                let position = self.messages.iter().position(|message| {
                    message["id"] == cursor
                        && message["channel_id"] == channel.as_str()
                        && (self.resolve_everything || self.visible(message))
                });
                position.context(
                    "Crew broker refused request: {\"code\":\"stale_cursor\",\"message\":\"cursor unavailable\"}",
                )? + 1
            }
        };
        if limit == 0 {
            return Ok(json!({"messages": [], "people": {}, "channel_names": {}}));
        }
        self.calls.push("page".into());
        let page: Vec<Value> = self.messages[start..]
            .iter()
            .filter(|message| message["channel_id"] == channel.as_str() && self.visible(message))
            .take(limit)
            .cloned()
            .collect();
        Ok(json!({"messages": page, "people": {}, "channel_names": {}}))
    }
}

#[async_trait::async_trait]
impl ObservationSource for FakeBroker {
    async fn binding(&self, _connection: &str) -> Result<Value> {
        Ok(self.with(|state| state.binding.clone()))
    }
    async fn read(&self, _connection: &str, method: &str, params: Value) -> Result<Value> {
        self.with(|state| match method {
            "workspace.snapshot" => {
                state.calls.push("snapshot".into());
                Ok(json!({
                    "workspace": {"mode": state.mode, "institution_id": state.institution,
                        "policy_epoch": state.epoch},
                    "channels": state.member_of.iter().map(|id| json!({"id": id})).collect::<Vec<_>>(),
                    "principals": [], "teams": [], "invitations": [], "runs": [],
                }))
            }
            "messages.history" => state.history(&params),
            _ => bail!("unsupported"),
        })
    }
    async fn runs(&self, _connection: &str) -> Result<Vec<super::super::crew::RunView>> {
        Ok(Vec::new())
    }
    fn capabilities(&self, _connection: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// An observer as `observe` builds one, over a broker that has nothing, without the person's
/// proof.
pub(super) fn plain_observer(
    channel: Option<&str>,
    after: Option<&str>,
    permit: Option<OwnedSemaphorePermit>,
) -> Observer {
    Observer::new(
        HeaderMap::new(),
        "connection".into(),
        ObserveRequest {
            channel_id: channel.map(str::to_owned),
            after: after.map(str::to_owned),
            initial: Initial::Latest,
        },
        json!({}),
        FakeBroker::new(&[]),
        permit,
    )
}

fn proven_headers() -> HeaderMap {
    crate::routes::session::diverge_tests::install_test_user_action_key();
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-User-Action",
        crate::routes::session::diverge_tests::TEST_USER_ACTION_KEY
            .parse()
            .unwrap(),
    );
    headers
}

/// A person's observer of `channel` over `broker`, as `observe` builds it.
fn observer(broker: &Arc<FakeBroker>, channel: &str) -> Observer {
    Observer::new(
        proven_headers(),
        "connection".into(),
        ObserveRequest {
            channel_id: Some(channel.into()),
            after: None,
            initial: Initial::Latest,
        },
        json!({}),
        broker.clone(),
        None,
    )
}

/// The next `next_frame` is a state frame, without the idle wait between two of them.
fn state_frame_due(observer: &mut Observer) {
    observer.last_state = Some(tokio::time::Instant::now() - Duration::from_secs(3));
}

fn texts(frame: &Value) -> Vec<&str> {
    frame["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| message["body"].as_str())
        .collect()
}

fn error_code(result: Result<Value>) -> String {
    observation_error_code(&result.expect_err("the observation must end"))
}

/// Someone else accepting an invitation, being added or removed, or a channel being archived:
/// each moves the workspace epoch and changes nothing this person may read.
fn someone_else_joined(state: &mut FakeState) {
    state.epoch += 1;
}

#[tokio::test]
async fn a_moved_policy_epoch_is_reauthorized_and_the_view_keeps_streaming() {
    let broker = FakeBroker::new(&["c-general", "c-methods"]);
    broker.with(|state| {
        state.post("m1", "c-general", &[], "first");
        state.post("m2", "c-general", &["c-general", "c-methods"], "derived");
    });
    let mut observer = observer(&broker, "c-general");
    let cancel = CancellationToken::new();
    assert_eq!(observer.next_frame(&cancel).await.unwrap()["type"], "state");

    // P0-1: an invitation accepted anywhere in the workspace used to end every open view.
    broker.with(someone_else_joined);
    let frame = observer.next_frame(&cancel).await.unwrap();
    assert_eq!(frame["type"], "messages");
    assert_eq!(texts(&frame), ["first", "derived"]);
    assert_eq!(frame["reset"], true);
    assert_eq!(frame["remaining"], 0);
    assert_eq!(observer.verified.as_ref().unwrap().epoch, json!(2));

    // The person gains a channel as the epoch moves: still the same view.
    broker.with(|state| {
        someone_else_joined(state);
        state.member_of.insert("c-new".into());
        state.post("m3", "c-general", &["c-new"], "from the new channel");
    });
    state_frame_due(&mut observer);
    let state = observer.next_frame(&cancel).await.unwrap();
    assert_eq!(state["type"], "state");
    assert_eq!(state["snapshot"]["workspace"]["policy_epoch"], 3);
    let frame = observer.next_frame(&cancel).await.unwrap();
    assert_eq!(texts(&frame), ["from the new channel"]);
    assert_eq!(frame["reset"], false);
    assert!(!observer.done);
}

#[tokio::test]
async fn a_privacy_or_institution_change_still_ends_the_view() {
    for change in [
        (|state: &mut FakeState| state.institution = Some("stanford")) as fn(&mut FakeState),
        |state: &mut FakeState| state.mode = "public",
    ] {
        let broker = FakeBroker::new(&["c-general"]);
        let mut observer = observer(&broker, "c-general");
        let cancel = CancellationToken::new();
        observer.next_frame(&cancel).await.unwrap();
        broker.with(|state| {
            someone_else_joined(state);
            change(state);
        });
        state_frame_due(&mut observer);
        let result = observer.next_frame(&cancel).await;
        assert_eq!(error_code(result), "policy_changed");
    }
}

#[tokio::test]
async fn losing_any_channel_ends_the_view_and_losing_this_one_says_so() {
    let broker = FakeBroker::new(&["c-general", "c-raw"]);
    let mut observer = observer(&broker, "c-general");
    let cancel = CancellationToken::new();
    observer.next_frame(&cancel).await.unwrap();
    broker.with(|state| {
        someone_else_joined(state);
        state.member_of.remove("c-raw");
    });
    state_frame_due(&mut observer);
    assert_eq!(
        error_code(observer.next_frame(&cancel).await),
        "scope_changed"
    );

    let broker = FakeBroker::new(&["c-general", "c-raw"]);
    let mut observer = self::observer(&broker, "c-general");
    observer.next_frame(&cancel).await.unwrap();
    broker.with(|state| {
        someone_else_joined(state);
        state.member_of.remove("c-general");
    });
    state_frame_due(&mut observer);
    assert_eq!(
        error_code(observer.next_frame(&cancel).await),
        "channel_access_changed"
    );
}

#[tokio::test]
async fn a_changed_connection_binding_still_ends_the_view() {
    let broker = FakeBroker::new(&["c-general"]);
    let mut observer = observer(&broker, "c-general");
    let cancel = CancellationToken::new();
    observer.next_frame(&cancel).await.unwrap();
    broker.with(|state| state.binding = json!({"mode": "public"}));
    state_frame_due(&mut observer);
    assert_eq!(
        error_code(observer.next_frame(&cancel).await),
        "policy_changed"
    );
}

#[tokio::test]
async fn a_page_is_decided_once_per_source_set_and_sent_as_one_frame() {
    let broker = FakeBroker::new(&["c-general", "c-raw"]);
    broker.with(|state| {
        for index in 0..60 {
            let sources: &[&str] = if index % 6 == 0 { &["c-raw"] } else { &[] };
            state.post(&format!("m{index:02}"), "c-general", sources, "hello");
        }
    });
    let mut observer = observer(&broker, "c-general");
    let cancel = CancellationToken::new();
    observer.next_frame(&cancel).await.unwrap();
    let before = (broker.count("snapshot"), broker.count("resolve:"));
    let frame = observer.next_frame(&cancel).await.unwrap();
    assert_eq!(frame["messages"].as_array().unwrap().len(), 60);
    assert_eq!(frame["cursor"], "m59");
    assert_eq!(frame["remaining"], 0);
    // One resolve per distinct source set and a snapshot either side, where checking one message
    // at a time took a resolve and two snapshots for each of the 60.
    assert_eq!(broker.count("resolve:") - before.1, 2);
    assert_eq!(broker.count("snapshot") - before.0, 2);
    assert_eq!(broker.count("page"), 1);
}

#[tokio::test]
async fn a_restricted_source_message_is_still_withheld_in_a_batched_page() {
    let broker = FakeBroker::new(&["c-general"]);
    broker.with(|state| {
        state.post("m1", "c-general", &[], "ordinary");
        state.post(
            "m2",
            "c-general",
            &["c-general", "c-raw"],
            "RESTRICTED-CANARY",
        );
        state.post("m3", "c-general", &[], "after it");
    });
    // A page fetched while the person could still read c-raw, now delivered after they lost it.
    let mut observer = observer(&broker, "c-general");
    observer.state_due = false;
    observer.last_state = Some(tokio::time::Instant::now());
    let page = json!({"messages": broker.with(|state| state.messages.clone())});
    observer.pending = annotate_page(page["messages"].as_array().unwrap(), &page);
    let cancel = CancellationToken::new();

    let frame = observer.next_frame(&cancel).await.unwrap();
    assert_eq!(texts(&frame), ["ordinary"]);
    assert_eq!(frame["remaining"], 2);
    assert_eq!(
        broker.count("resolve:"),
        2,
        "one resolve per distinct source set"
    );

    let result = observer.next_frame(&cancel).await;
    let frame = encode_frame(&mut observer, result);
    let value: Value = serde_json::from_slice(&frame).unwrap();
    assert_eq!(
        (value["code"].as_str(), value["clear"].as_bool()),
        (Some("stale_cursor"), Some(true))
    );
    assert!(!String::from_utf8_lossy(&frame).contains("RESTRICTED-CANARY"));
    assert!(observer.done);
    assert_eq!(
        observer.pending.len(),
        2,
        "nothing after it was sent either"
    );

    // A broker whose answer disagrees with its own snapshot does not let it through either.
    let broker = FakeBroker::new(&["c-general"]);
    broker.with(|state| {
        state.post("m2", "c-general", &["c-raw"], "RESTRICTED-CANARY");
        state.resolve_everything = true;
    });
    let mut observer = self::observer(&broker, "c-general");
    observer.state_due = false;
    observer.last_state = Some(tokio::time::Instant::now());
    observer.pending = broker.with(|state| state.messages.iter().cloned().collect());
    assert_eq!(
        error_code(observer.next_frame(&cancel).await),
        "scope_changed"
    );
}

#[tokio::test]
async fn a_revocation_while_the_broker_answers_withholds_the_whole_page() {
    let broker = FakeBroker::new(&["c-general", "c-raw"]);
    broker.with(|state| {
        state.post("m1", "c-general", &[], "ordinary");
        state.post("m2", "c-general", &["c-raw"], "RESTRICTED-CANARY");
    });
    let mut observer = observer(&broker, "c-general");
    let cancel = CancellationToken::new();
    observer.next_frame(&cancel).await.unwrap();
    broker.with(|state| {
        state.before_resolve = Some(Box::new(|state: &mut FakeState| {
            if state.member_of.remove("c-raw") {
                state.epoch += 1;
            }
        }))
    });
    assert_eq!(
        error_code(observer.next_frame(&cancel).await),
        "scope_changed"
    );
}

#[tokio::test]
async fn a_queued_frame_is_decided_again_at_admission_under_the_new_epoch() {
    let broker = FakeBroker::new(&["c-general", "c-raw"]);
    broker.with(|state| {
        state.post("m1", "c-general", &[], "ordinary");
        state.post("m2", "c-general", &["c-raw"], "RESTRICTED-CANARY");
    });
    let mut producer = observer(&broker, "c-general");
    let cancel = CancellationToken::new();
    producer.next_frame(&cancel).await.unwrap();
    let frame = producer.next_frame(&cancel).await.unwrap();
    assert_eq!(texts(&frame), ["ordinary", "RESTRICTED-CANARY"]);
    let queued = Bytes::from(serde_json::to_vec(&frame).unwrap());

    // Queued, then the epoch moves for someone else: decided again under the new epoch, and still
    // delivered.
    broker.with(someone_else_joined);
    let resolves = broker.count("resolve:");
    let admitted = producer.admit_frame(&queued, &cancel).await.unwrap();
    assert_eq!(admitted, frame);
    assert_eq!(broker.count("resolve:") - resolves, 2);
    assert_eq!(producer.verified.as_ref().unwrap().epoch, json!(2));

    // Queued, then the source is revoked: the frame never leaves.
    broker.with(|state| {
        someone_else_joined(state);
        state.member_of.remove("c-raw");
    });
    let result = producer.admit_frame(&queued, &cancel).await;
    assert_eq!(error_code(result), "scope_changed");

    // An observer that never saw c-raw (a reconnect) is refused by the broker itself.
    let mut fresh = observer(&broker, "c-general");
    let result = fresh.admit_frame(&queued, &cancel).await;
    assert_eq!(error_code(result), "stale_cursor");
}

/// Real time: the stream waits about two seconds between state frames.
#[tokio::test]
async fn an_invitation_accepted_mid_stream_keeps_the_stream_open() {
    use futures::StreamExt;
    async fn next<S: futures::Stream<Item = Result<Bytes, Infallible>> + Unpin>(
        stream: &mut S,
    ) -> Value {
        let frame = tokio::time::timeout(Duration::from_secs(10), stream.next())
            .await
            .expect("a frame within the state cadence")
            .expect("the stream ended")
            .unwrap();
        serde_json::from_slice(&frame).unwrap()
    }
    let broker = FakeBroker::new(&["c-general"]);
    broker.with(|state| state.post("m1", "c-general", &[], "hello"));
    let mut observer = observer(&broker, "c-general");
    observer._permit = Some(SLOTS.clone().try_acquire_owned().unwrap());
    let mut stream = Box::pin(observation_stream(observer));
    assert_eq!(next(&mut stream).await["type"], "state");
    assert_eq!(texts(&next(&mut stream).await), ["hello"]);

    broker.with(|state| {
        someone_else_joined(state);
        state.post("m2", "c-general", &[], "after the invitation");
    });
    let (mut delivered, mut verified) = (false, false);
    for _ in 0..6 {
        let frame = next(&mut stream).await;
        assert_ne!(frame["type"], "error", "the view ended: {frame}");
        delivered |= texts(&frame) == ["after the invitation"];
        verified |= frame["type"] == "state" && frame["snapshot"]["workspace"]["policy_epoch"] == 2;
        if delivered && verified {
            return;
        }
    }
    panic!("the stream went on without the new message or the new epoch");
}

#[tokio::test]
async fn a_queued_frame_is_admitted_while_the_producer_waits_between_state_frames() {
    let broker = FakeBroker::new(&["c-general"]);
    broker.with(|state| state.post("m1", "c-general", &[], "hello"));
    let cancel = CancellationToken::new();
    let mut maker = observer(&broker, "c-general");
    maker.next_frame(&cancel).await.unwrap();
    let frame = maker.next_frame(&cancel).await.unwrap();
    assert_eq!(texts(&frame), ["hello"]);

    // The producer has nothing to do but wait for its next state frame.
    let mut idle = observer(&broker, "c-general");
    idle.next_frame(&cancel).await.unwrap();
    idle.pending.clear();
    idle.first = false;
    idle.state_due = true;
    idle.sleep_due = true;
    let idle = Arc::new(Mutex::new(idle));
    let (sender, queue) = mpsc::channel(1);
    sender
        .try_send(Bytes::from(serde_json::to_vec(&frame).unwrap()))
        .unwrap();
    let (terminal_sender, terminal) = oneshot::channel();
    let producer_cancel = CancellationToken::new();
    let producer = tokio::spawn(produce(
        idle.clone(),
        sender,
        terminal_sender,
        producer_cancel.child_token(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut receiver = ObservationReceiver {
        receiver: queue,
        terminal: Some(terminal),
        deferred_terminal: None,
        observer: idle,
        cancel: producer_cancel.clone(),
        finished: false,
    };
    // Held behind the producer's two-second pause, this took the whole pause.
    let admitted = tokio::time::timeout(Duration::from_millis(1000), receiver.next_frame())
        .await
        .expect("admission waited behind the pause between state frames")
        .unwrap();
    let admitted: Value = serde_json::from_slice(&admitted).unwrap();
    assert_eq!(texts(&admitted), ["hello"]);
    producer_cancel.cancel();
    producer.await.unwrap();
}

#[test]
fn frames_carry_as_many_messages_as_fit_and_always_one() {
    let small: VecDeque<Value> = (0..300).map(|i| json!({"id": format!("m{i}")})).collect();
    assert_eq!(frame_batch_len(&small), MAX_MESSAGES_PER_FRAME);
    let large: VecDeque<Value> = (0..5)
        .map(|_| json!({"body": "x".repeat(100 * 1024)}))
        .collect();
    assert_eq!(frame_batch_len(&large), 2);
    let huge: VecDeque<Value> = [json!({"body": "x".repeat(FRAME_BUDGET * 2)})].into();
    assert_eq!(frame_batch_len(&huge), 1);
}

#[test]
fn a_batched_frame_carries_every_message_s_names_and_stays_parseable() {
    let page = json!({
        "messages": [
            {"id": "m1", "sequence": "m1", "channel_id": "c-general", "actor_id": "p-alice",
             "body": "a", "source_channels": []},
            {"id": "m2", "sequence": "m2", "channel_id": "c-general", "actor_id": "p-bob",
             "body": "b", "source_channels": ["c-raw"]},
        ],
        "people": {"p-alice": {"username": "alice"}, "p-bob": {"username": "bob"}},
        "channel_names": {"c-general": "general", "c-raw": "raw"},
    });
    let mut pending = annotate_page(page["messages"].as_array().unwrap(), &page);
    let names: Vec<Value> = pending
        .iter_mut()
        .filter_map(|message| message.as_object_mut()?.remove(PAGE_NAMES))
        .collect();
    let frame = messages_frame(
        &Some("c-general".into()),
        pending.into_iter().collect(),
        &Some("m2".into()),
        true,
        0,
        200,
        names,
    );
    assert_eq!(frame["people"]["p-alice"]["username"], "alice");
    assert_eq!(frame["people"]["p-bob"]["username"], "bob");
    assert_eq!(frame["channel_names"]["c-raw"], "raw");
    assert!(!serde_json::to_string(&frame)
        .unwrap()
        .contains("page_names"));
    match serde_json::from_value::<ObserveEvent>(frame).unwrap() {
        ObserveEvent::Messages { messages, .. } => assert_eq!(messages.len(), 2),
        _ => panic!("expected a messages frame"),
    }
}

#[test]
fn error_frames_speak_plainly() {
    for code in [
        "policy_changed",
        "scope_changed",
        "channel_access_changed",
        "human_authority_required",
        "observer_capacity_reached",
        "Crew broker refused request: {\"code\":\"stale_cursor\"}",
        "Crew broker refused request: {\"code\":\"unauthorized\"}",
        "something unexpected",
    ] {
        let mut observer = plain_observer(None, None, None);
        let frame = encode_frame(&mut observer, Err(anyhow::anyhow!("{code}")));
        let value: Value = serde_json::from_slice(&frame).unwrap();
        let text = value["error"].as_str().unwrap();
        assert_eq!(value["clear"], true);
        for jargon in ["observation", "cursor", "authorized", "cached", "stale"] {
            assert!(
                !text.to_lowercase().contains(jargon),
                "{code}: {text:?} says {jargon:?}"
            );
        }
    }
}
