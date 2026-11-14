//! Terminal presentation of the profile daemon's personal conversation services.
use crate::commands::session_watch::cards;
use crate::daemon_client::{CrewClient, DaemonEventStream, DaemonRefusal};
use crate::session::APPROVAL_EXPIRED;
use anyhow::{bail, ensure, Context, Result};
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers,
};
use futures::StreamExt;
use serde_json::{json, Map, Value};
use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use zeroize::{Zeroize, Zeroizing};

pub struct SharedConversationOptions {
    pub session_id: Option<String>,
    pub working_dir: PathBuf,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub interactive: bool,
    pub create_only: bool,
    pub history: bool,
    pub approval_key_stdin: bool,
    pub no_start: bool,
    pub quiet: bool,
    pub output_format: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Json,
    StreamJson,
}
impl Format {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "stream-json" => Ok(Self::StreamJson),
            _ => bail!("Shared conversations support text, json, or stream-json output"),
        }
    }
}
struct Conversation {
    client: CrewClient,
    session_id: String,
    format: Format,
    interactive: bool,
    quiet: bool,
    answered: HashSet<String>,
    /// The approvals whose prompt here ended by saying how they closed, so the daemon's own
    /// words for the same end print nothing more ([`ClosedHere`]).
    closed_here: ClosedHere,
    renderer: TextRenderer,
    continuation_owner_id: String,
    continuation_lease: Option<Zeroizing<String>>,
    /// What the chat's binding said when it was shown, so a turn's `PrivacyProviderPinned`
    /// that agrees with it says nothing.
    binding: ShownBinding,
    /// "Chat … is ready", held until the turn that follows it reaches the provider (AGT2-N4):
    /// a start the daemon refuses must not be preceded by it.
    pending_ready: Option<String>,
    dictation_state: SharedDictationState,
    #[cfg(test)]
    crew_step_test_input:
        Option<std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + Sync>>>,
    /// What the next approval prompt reads in place of the terminal.
    #[cfg(test)]
    approval_test_input: Option<ApprovalTestInput>,
    /// Every line [`Conversation::notice`] wrote, for a test to read back.
    #[cfg(test)]
    noticed: std::sync::Mutex<Vec<String>>,
}

#[cfg(feature = "dictation")]
type SharedDictationState = crate::dictation::frontend::FrontendSession;
#[cfg(not(feature = "dictation"))]
#[derive(Default)]
struct SharedDictationState;
/// A test's stand-in for the terminal an approval prompt reads.
#[cfg(test)]
type ApprovalTestInput =
    std::pin::Pin<Box<dyn Future<Output = Result<Option<Zeroizing<String>>>> + Send + Sync>>;

/// The provider, model and privacy a chat was shown with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ShownBinding {
    provider: Option<String>,
    model: Option<String>,
    private: bool,
}

impl ShownBinding {
    fn of(session: &Value) -> Self {
        Self {
            provider: session["provider_name"].as_str().map(str::to_owned),
            model: session["model_config"]["model_name"]
                .as_str()
                .map(str::to_owned),
            private: session["privacy_tier"].as_str() == Some("private"),
        }
    }
}

/// The daemon's turn error for a turn whose reply stream never opened: nothing reached the
/// provider, and nothing was submitted to it.
const START_FAILED: &str = "inference_start_failed";

/// A turn the daemon refused to start before anything reached the provider (AGT2-N4), such as a
/// Crew chat whose access was removed. Its own sentence is the whole error: "remains available;
/// no automatic resubmission was attempted" is about a turn that ran, and this one never did.
#[derive(Debug)]
struct StartRefused {
    sentence: String,
}

impl std::fmt::Display for StartRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.sentence)
    }
}

impl std::error::Error for StartRefused {}

/// A turn's `Error` frame as the error the command ends with: a start refused before the turn
/// reached the provider is its sentence alone ([`StartRefused`]); any other keeps its code.
fn turn_error(frame: &Value, reached_provider: bool) -> anyhow::Error {
    let code = frame["code"].as_str().unwrap_or("daemon_error");
    let error = frame["error"].as_str().unwrap_or("Daemon turn failed");
    if code == START_FAILED && !reached_provider {
        return StartRefused {
            sentence: safe_text(error),
        }
        .into();
    }
    anyhow::anyhow!("{}: {}", safe_text(code), safe_text(error))
}

/// What a turn's frame means for the deferred "Chat … is ready" (AGT2-N4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ready {
    /// The turn reached the provider: the chat is ready, and is said to be.
    Show,
    /// The turn ended before it reached the provider: it never was.
    Drop,
    /// Nothing yet says either.
    Wait,
}

/// [`Ready`] for `frame`, given whether an earlier frame already showed the turn reached the
/// provider (`reached`). A model's frames (a message, the provider pin sent on every turn that
/// reaches one, a model change) and a turn that finished show it; an error or a stop before any
/// of them means the start was refused. `TurnStarted` shows nothing: the daemon sends it before
/// any work that can fail.
fn ready_on(frame: &Value, reached: bool) -> Ready {
    match frame["type"].as_str() {
        Some("Message" | "PrivacyProviderPinned" | "ModelChange") => Ready::Show,
        Some("Finish")
            if !matches!(
                frame["reason"].as_str(),
                Some("cancelled" | "canceled" | "stopped" | "orphaned" | "error" | "failed")
            ) =>
        {
            Ready::Show
        }
        Some("Error" | "Finish") if !reached => Ready::Drop,
        _ => Ready::Wait,
    }
}

/// The run's error as it is printed: a start the daemon refused is its own sentence, and any
/// other failure says the session remains and that nothing was resubmitted.
fn with_session_context(result: Result<()>, session_id: &str) -> Result<()> {
    match result {
        Err(error) if error.chain().any(|cause| cause.is::<StartRefused>()) => Err(error),
        result => result.with_context(|| {
            format!(
                "Daemon session {} remains available; no automatic resubmission was attempted",
                safe_text(session_id)
            )
        }),
    }
}
enum TurnEnd {
    Finished,
    Stopped,
}
enum Interaction {
    Continue,
    Stop,
}

pub async fn run(options: SharedConversationOptions) -> Result<()> {
    let format = validate_options(&options)?;
    let client =
        CrewClient::connect_with_input(options.no_start, options.approval_key_stdin).await?;
    let session_id = open_session(&client, &options).await?;
    eprintln!("Daemon session: {}", safe_text(&session_id));
    let mut conversation = Conversation {
        client,
        session_id,
        format,
        interactive: options.interactive,
        quiet: options.quiet,
        answered: HashSet::new(),
        closed_here: ClosedHere::default(),
        renderer: TextRenderer::default(),
        continuation_owner_id: uuid::Uuid::new_v4().to_string(),
        continuation_lease: None,
        binding: ShownBinding::default(),
        pending_ready: None,
        dictation_state: Default::default(),
        #[cfg(test)]
        crew_step_test_input: None,
        #[cfg(test)]
        approval_test_input: None,
        #[cfg(test)]
        noticed: Default::default(),
    };
    conversation.event(&json!({"type":"Session","session_id":conversation.session_id}))?;
    let result = conversation.execute(&options).await;
    let cleanup = conversation.release_unused_continuation().await;
    let result = match (result, cleanup) {
        (Err(error), Err(cleanup)) => Err(error.context(cleanup.to_string())),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), cleanup) => cleanup,
    };
    conversation.renderer.finish()?;
    if let Err(error) = &result {
        if format != Format::Text {
            let mut frame = json!({"type":"Error","session_id":conversation.session_id,
                "error":format!("{error:#}"),"resubmit_automatically":false});
            if error.chain().any(|cause| cause.is::<StartRefused>()) {
                frame["code"] = json!(START_FAILED);
            }
            emit_json(&frame)?;
        }
    }
    with_session_context(result, &conversation.session_id)
}

fn validate_options(options: &SharedConversationOptions) -> Result<Format> {
    ensure!(
        options.provider.is_some() == options.model.is_some(),
        "Specify provider and model together"
    );
    ensure!(
        !options.create_only || (!options.interactive && options.prompt.is_none()),
        "Create-only cannot also submit a prompt or open an interactive conversation"
    );
    ensure!(
        !options.approval_key_stdin || !options.interactive,
        "The compatibility --approval-key-stdin flag cannot share interactive conversation input"
    );
    ensure!(
        !options.interactive || (std::io::stdin().is_terminal() && std::io::stderr().is_terminal()),
        "Interactive shared conversations need terminal input and stderr"
    );
    if let Some(id) = &options.session_id {
        validate_id(id)?;
    } else {
        ensure!(
            options.provider.is_some(),
            "New daemon conversations require an explicit provider and model"
        );
    }
    if let Some(prompt) = &options.prompt {
        validate_prompt(prompt)?;
    }
    Format::parse(&options.output_format)
}
fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')),
        "An exact daemon session or request ID is required"
    );
    Ok(())
}
fn validate_prompt(prompt: &str) -> Result<()> {
    ensure!(
        !prompt.trim().is_empty() && prompt.len() <= 32_768,
        "Conversation input must contain 1–32768 bytes"
    );
    Ok(())
}
async fn open_session(client: &CrewClient, options: &SharedConversationOptions) -> Result<String> {
    if let Some(id) = &options.session_id {
        return Ok(id.clone());
    }
    let session = client
        .request(
            "POST",
            "/agent/start",
            Some(json!({
                    "working_dir":options.working_dir,"extension_overrides":[],
            "provider":options.provider,"model":options.model
                })),
        )
        .await?;
    let id = session["id"]
        .as_str()
        .context("Daemon did not return a session ID")?;
    validate_id(id)?;
    Ok(id.to_owned())
}

impl Conversation {
    async fn resume(&self, initialize: bool) -> Result<Value> {
        let state = self
            .client
            .request(
                "POST",
                "/agent/resume",
                Some(json!({
                    "session_id":self.session_id,"load_model_and_extensions":initialize,
                    "continuation_owner_id":self.continuation_owner_id
                })),
            )
            .await?;
        ensure!(
            state["session"]["id"].as_str() == Some(&self.session_id),
            "Daemon returned a different session"
        );
        ensure!(state["initializing"].as_bool() == Some(false), "The session is still initializing; resume the exact session after initialization finishes");
        ensure!(
            state["initialization_error"].is_null(),
            "The daemon could not initialize this session: {}",
            safe_text(&state["initialization_error"].to_string())
        );
        if let Some(results) = state["extension_results"].as_array() {
            for line in extension_warnings(results) {
                self.notice(&line)?;
            }
        }
        Ok(state)
    }

    async fn resolve_pending_continuation(&mut self, state: Value) -> Result<Value> {
        let pending = &state["pending_continuation"];
        if pending.is_null() {
            return Ok(state);
        }
        let retired = pending["superseded_turn_id"]
            .as_str()
            .context("Pending continuation omitted its exact retired turn ID")?
            .to_owned();
        validate_id(&retired)?;
        ensure!(self.interactive,
            "Session {} has a pending continuation for retired turn {}. Input was not submitted. Resume this exact session interactively without input to choose takeover or abandonment",
            safe_text(&self.session_id), safe_text(&retired));
        ensure!(pending["ownership"].as_str() != Some("settling"),
            "Continuation for retired turn {} is still settling. Input was not submitted; retry this exact session interactively after settlement", safe_text(&retired));
        self.notice(&format!("Session {} has a pending Stop-and-Send continuation for retired turn {}. Taking over revokes its previous client's continuation claim. Abandoning discards that pending claim; neither action recovers another client's unsent draft.", safe_text(&self.session_id), safe_text(&retired)))?;
        let action = loop {
            let answer = terminal_input(
                "Take over / abandon / leave unchanged [takeover/abandon/leave]: ",
                false,
                32,
                None,
                None,
                None,
            )
            .await?;
            match answer.as_deref().map(|value| value.trim()) {
                Some("takeover") => break "take_over",
                Some("abandon") => break "abandon",
                None | Some("leave") => {
                    bail!("Pending continuation left unchanged; input was not submitted")
                }
                _ => self.notice("Enter takeover, abandon, or leave.")?,
            }
        };
        let response = self.client.request("POST", "/agent/continuation/recover", Some(json!({
            "session_id":self.session_id,"superseded_turn_id":retired,
            "continuation_owner_id":self.continuation_owner_id,"action":action
        }))).await.map_err(|_| anyhow::anyhow!("Continuation recovery was not confirmed for retired turn {}. Input was not submitted; resume this exact session interactively to inspect its current state. No automatic retry was attempted", safe_text(&retired)))?;
        ensure!(
            response["superseded_turn_id"].as_str() == Some(retired.as_str()),
            "Continuation recovery returned a different generation; input was not submitted"
        );
        if action == "take_over" {
            ensure!(
                response["resolution"].as_str() == Some("taken_over"),
                "Continuation takeover was not confirmed; input was not submitted"
            );
            let lease = response["continuation_lease"]
                .as_str()
                .filter(|lease| !lease.is_empty())
                .context("Continuation takeover omitted its lease; input was not submitted")?;
            self.continuation_lease = Some(Zeroizing::new(lease.to_owned()));
        } else {
            ensure!(
                response["resolution"].as_str() == Some("abandoned"),
                "Continuation abandonment was not confirmed; input was not submitted"
            );
        }
        let state = self.resume(false).await?;
        self.verify_continuation(&state)?;
        Ok(state)
    }

    fn verify_continuation(&self, state: &Value) -> Result<()> {
        let pending = &state["pending_continuation"];
        match &self.continuation_lease {
            Some(lease) => ensure!(pending["ownership"].as_str() == Some("owned")
                && pending["continuation_lease"].as_str() == Some(lease.as_str()),
                "Continuation ownership changed. Input was not submitted; resume this exact session interactively to inspect its pending continuation"),
            None => ensure!(pending.is_null(),
                "A pending continuation appeared. Input was not submitted; resume this exact session interactively without input to choose recovery"),
        }
        Ok(())
    }

    async fn release_unused_continuation(&mut self) -> Result<()> {
        let Some(lease) = self.continuation_lease.as_ref() else {
            return Ok(());
        };
        let response = self.client.request("POST", "/agent/continuation/abandon", Some(json!({
            "session_id":self.session_id,"continuation_lease":lease.as_str()
        }))).await.map_err(|_| anyhow::anyhow!("Unused owned continuation cleanup was not confirmed. Resume this exact session interactively to inspect recovery; no foreign continuation was abandoned"))?;
        ensure!(matches!(response["resolution"].as_str(), Some("abandoned" | "already_abandoned" | "already_consumed")),
            "Unused continuation cleanup was not confirmed; resume this exact session interactively to inspect recovery");
        self.continuation_lease = None;
        Ok(())
    }

    async fn execute(&mut self, options: &SharedConversationOptions) -> Result<()> {
        let state = self.resume(true).await?;
        let mut state = self.resolve_pending_continuation(state).await?;
        let active = state["active_turn"]["turn_id"].as_str().map(str::to_owned);
        ensure!(active.is_none() || options.prompt.is_none(), "A turn is already running. Your prompt was not submitted; reattach to this exact session without input, then submit after it finishes");
        ensure!(
            active.is_none() || options.provider.is_none(),
            "A turn is already running. Reattach without provider/model changes"
        );
        if options.session_id.is_some() {
            if let (Some(provider), Some(model)) = (&options.provider, &options.model) {
                // Sent even when it names the provider and model the chat already
                // records, so the daemon stays the one that decides the binding. That
                // re-bind does not move the chat under "Last activity": the daemon
                // stamps a bind only when it changes the provider or the model
                // (R-30, ruling 16; `bind_provider_if_allowed`, pinned by the route
                // test `a_same_model_rebind_keeps_a_chats_place_and_a_switch_or_a_toggle_moves_it`).
                let notice = self
                    .client
                    .request_text(
                        "POST",
                        "/agent/update_provider",
                        Some(json!({
                            "session_id":self.session_id,"provider":provider,"model":model
                        })),
                    )
                    .await?;
                if !notice.is_empty() {
                    self.notice(&notice)?;
                }
                state = self.resume(false).await?;
            }
        }
        if let (Some(provider), Some(model)) = (&options.provider, &options.model) {
            ensure!(state["session"]["provider_name"].as_str() == Some(provider.as_str())
                && state["session"]["model_config"]["model_name"].as_str() == Some(model.as_str()),
                "The daemon did not confirm the requested provider/model; no prompt was submitted. Inspect this session before continuing");
        }
        // AGT2-N4: when a turn follows at once, "is ready" waits until it reaches the
        // provider, so a start the daemon refuses is never preceded by it.
        let turn_follows = !options.create_only && (options.prompt.is_some() || active.is_some());
        self.show_binding(&state["session"], turn_follows)?;
        if options.history {
            let session = self.history_session(&state).await?;
            self.show_history(&session)?;
        }
        if options.create_only {
            return self.finish("created").await;
        }
        let mut next = options
            .prompt
            .as_ref()
            .map(|text| Message::user().with_text(text));
        if let Some(turn_id) = active {
            match self.attach(&turn_id).await? {
                TurnEnd::Stopped => return self.finish("stopped").await,
                TurnEnd::Finished => {}
            }
        }
        loop {
            let message = match next.take() {
                Some(message) => message,
                None if self.interactive => {
                    let Some(text) = terminal_input(
                        "You (/quit to detach): ",
                        false,
                        32_768,
                        None,
                        Some(self.session_id.as_str()),
                        Some(&self.dictation_state),
                    )
                    .await?
                    else {
                        break;
                    };
                    if text.as_str() == "/quit" {
                        break;
                    }
                    if text.trim().is_empty() {
                        continue;
                    }
                    Message::user().with_text(text.as_str())
                }
                None => break,
            };
            match self.submit(message).await? {
                TurnEnd::Stopped => return self.finish("stopped").await,
                TurnEnd::Finished => {}
            }
        }
        self.finish("idle").await
    }

    async fn submit(&mut self, message: Message) -> Result<TurnEnd> {
        let state = self.resume(false).await?;
        ensure!(
            state["active_turn"].is_null(),
            "Another turn is running; the new input was not submitted. Reattach without input"
        );
        self.verify_continuation(&state)?;
        let turn_id = uuid::Uuid::new_v4().to_string();
        self.event(
            &json!({"type":"TurnSubmitted","session_id":self.session_id,"turn_id":turn_id}),
        )?;
        // Once admission starts, even a transport failure may mean the lease was consumed.
        // Never automatically abandon that uncertain successor's continuation claim.
        let lease = self.continuation_lease.take();
        let stream = self.client.event_stream("POST", "/reply", Some(json!({
            "session_id":self.session_id,"turn_id":turn_id,"user_message":message,
            "continuation_lease":lease.as_ref().map(|value| value.as_str())
        }))).await.with_context(|| format!("Turn admission outcome is unknown for {turn_id}; inspect the session before submitting any new input"))?;
        self.read_turn(stream, turn_id).await
    }

    async fn attach(&mut self, turn_id: &str) -> Result<TurnEnd> {
        validate_id(turn_id)?;
        let placeholder = json!({"role":"user","created":0,"content":[],
            "metadata":{"userVisible":false,"agentVisible":false}});
        let stream = self
            .client
            .event_stream(
                "POST",
                "/reply",
                Some(json!({
                    "session_id":self.session_id,"turn_id":turn_id,"from_seq":0,
                    "user_message":placeholder
                })),
            )
            .await?;
        self.read_turn(stream, turn_id.to_owned()).await
    }

    async fn read_turn(
        &mut self,
        mut stream: DaemonEventStream,
        submitted_id: String,
    ) -> Result<TurnEnd> {
        let mut active_id = submitted_id;
        let mut sequence = None;
        let mut bound = false;
        // Whether the turn reached the provider: a model's frame arrived.
        let mut reached = false;
        // Frames read while a card's prompt was open (`observe_crew_step`,
        // `ask_until_expired`), handled here in the order they arrived before
        // the stream is read again.
        let mut deferred = VecDeque::new();
        loop {
            let frame = tokio::select! {
                frame = async {
                    match deferred.pop_front() {
                        Some(frame) => Ok(Some(frame)),
                        None => stream.next_event().await,
                    }
                } => frame?,
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    return self.cancel(&active_id).await;
                }
            }.with_context(|| format!("Turn stream ended without completion; reattach to session {} without new input (turn {}, next sequence {})", self.session_id, active_id, sequence.map_or(0, |n: u64| n.saturating_add(1))))?;
            if let Some(turn) = frame["turn_id"].as_str() {
                if bound {
                    ensure!(turn == active_id, "Turn stream changed its generation");
                } else {
                    active_id = turn.to_owned();
                    bound = true;
                }
            }
            if let Some(seq) = frame["seq"].as_u64() {
                if sequence.is_some_and(|previous| seq <= previous) {
                    continue;
                }
                sequence = Some(seq);
            }
            self.event(&frame)?;
            match ready_on(&frame, reached) {
                Ready::Show => {
                    reached = true;
                    self.show_ready()?;
                }
                Ready::Drop => self.pending_ready = None,
                Ready::Wait => {}
            }
            match frame["type"].as_str().context("Daemon event has no type")? {
                "Message" => match self
                    .handle_message(
                        &frame["message"],
                        &mut stream,
                        &mut deferred,
                        &active_id,
                        sequence,
                    )
                    .await?
                {
                    Interaction::Continue => {}
                    Interaction::Stop => return self.cancel(&active_id).await,
                },
                "ActionRequiredEnded" => {
                    if let Some(id) = frame["id"].as_str() {
                        validate_id(id)?;
                        ensure!(
                            self.answered.len() < 1024 || self.answered.contains(id),
                            "Too many interaction prompts; reconnect to the session"
                        );
                        self.answered.insert(id.to_owned());
                    }
                    if let Some(line) = self.closed_here.ended_line(&frame) {
                        self.notice(&line)?;
                    }
                }
                "Error" => return Err(turn_error(&frame, reached)),
                "Finish" => {
                    self.renderer.finish()?;
                    return match frame["reason"].as_str() {
                        Some("cancelled" | "canceled" | "stopped" | "orphaned" | "error" | "failed") => bail!("Daemon turn ended: {}", safe_text(&frame["reason"].to_string())),
                        _ => Ok(TurnEnd::Finished),
                    };
                }
                "UpdateConversation" => self.notice("The daemon resynchronized the conversation; use history to read the authoritative transcript.")?,
                // AGT2-N4, CLIDOCS-F12: sentences in text, never the raw frame.
                "PrivacyProviderPinned" | "ModelChange" => {
                    if let Some(notice) = self.binding_change(&frame) {
                        self.notice(&notice)?;
                    }
                }
                "Ping" | "TurnStarted" | "TurnState" | "Notification" | "ToolCallPending"
                | "ToolCallsRetracted" | "SteerWaiting" | "MessagesPersisted" => {}
                kind => bail!("Unsupported daemon event {}; use a compatible client to resume this exact session without new input", safe_text(kind)),
            }
        }
    }

    async fn handle_message(
        &mut self,
        value: &Value,
        stream: &mut DaemonEventStream,
        deferred: &mut VecDeque<Value>,
        turn_id: &str,
        sequence: Option<u64>,
    ) -> Result<Interaction> {
        let message: Message =
            serde_json::from_value(value.clone()).context("Unsupported daemon message shape")?;
        // An expiry notice for an approval whose prompt already said it expired is that same
        // end a second time.
        if self.format == Format::Text && !self.closed_here.said(&message) {
            self.renderer.message(&message, self.quiet)?;
        }
        for content in message.content {
            match content {
                MessageContent::ActionRequired(action) => {
                    self.renderer.finish()?;
                    let outcome = self
                        .interact_while_observing(action.data, stream, deferred, turn_id, sequence)
                        .await?;
                    if !matches!(outcome, Interaction::Continue) {
                        return Ok(outcome);
                    }
                }
                MessageContent::FrontendToolRequest(request) => {
                    let name = request
                        .tool_call
                        .as_ref()
                        .map(|call| call.name.as_ref())
                        .unwrap_or("unknown");
                    bail!("Client-executed tool {} (request {}) is unsupported in this terminal; resume with a client providing that tool, or stop the exact turn. No tool was executed", safe_text(name), safe_text(&request.id));
                }
                _ => {}
            }
        }
        Ok(Interaction::Continue)
    }

    async fn interact_while_observing(
        &mut self,
        action: ActionRequiredData,
        stream: &mut DaemonEventStream,
        deferred: &mut VecDeque<Value>,
        turn_id: &str,
        sequence: Option<u64>,
    ) -> Result<Interaction> {
        let ActionRequiredData::CrewStep { id, step } = &action else {
            // Any other card is asked with the turn's stream lent to it, so an approval
            // stops asking the moment its card expires (`ask_until_expired`).
            let watch = TurnWatch {
                stream,
                deferred,
                turn_id,
                sequence,
            };
            return self.interact(action, Some(watch)).await;
        };
        if biorouter::pending_user_action::crew_step_is_a_notice(step) {
            return self.interact(action, None).await;
        }
        let id = id.clone();
        validate_id(&id)?;
        if self.answered.contains(&id) {
            return Ok(Interaction::Continue);
        }
        ensure!(
            self.answered.len() < 1024,
            "Too many interaction prompts; reconnect to the session"
        );
        let interactive = self.interactive;
        // Drop the prompt's reader and zeroizing buffers before clearing queued input or
        // rendering deferred chat output. A detached reader could consume the next prompt.
        let observed = {
            let interaction = self.interact(action, None);
            observe_crew_step(
                interaction,
                stream,
                deferred,
                &id,
                turn_id,
                sequence,
                interactive,
            )
            .await?
        };
        match observed {
            ObservedCrewStep::Completed(outcome) => {
                discard_terminal_input()?;
                Ok(outcome)
            }
            ObservedCrewStep::Ended => {
                discard_terminal_input()?;
                self.answered.insert(id);
                Ok(Interaction::Continue)
            }
            ObservedCrewStep::Stop => {
                discard_terminal_input()?;
                Ok(Interaction::Stop)
            }
        }
    }

    /// Ask the person for card `action`. `watch` lends the turn's stream to an approval's
    /// prompt (`ask_until_expired`); a Crew step is observed from outside instead
    /// (`observe_crew_step`), so it gets none.
    async fn interact(
        &mut self,
        action: ActionRequiredData,
        watch: Option<TurnWatch<'_>>,
    ) -> Result<Interaction> {
        let id = match &action {
            ActionRequiredData::ToolConfirmation { id, .. }
            | ActionRequiredData::Elicitation { id, .. }
            | ActionRequiredData::SecretRequest { id, .. }
            | ActionRequiredData::CrewStep { id, .. } => id.clone(),
            ActionRequiredData::ElicitationResponse { .. } => return Ok(Interaction::Continue),
        };
        validate_id(&id)?;
        if self.answered.contains(&id) {
            return Ok(Interaction::Continue);
        }
        self.event(
            &json!({"type":"PendingInteraction","session_id":self.session_id,"request_id":id}),
        )?;
        ensure!(self.interactive, "Human input is pending in session {}, request {}. Detached without answering; resume interactively or open this session in the desktop", self.session_id, safe_text(&id));
        ensure!(
            self.answered.len() < 1024,
            "Too many interaction prompts; reconnect to the session"
        );
        let outcome = match action {
            ActionRequiredData::ToolConfirmation { .. } => {
                self.confirm_tool(&action, &id, watch).await?
            }
            ActionRequiredData::SecretRequest { .. } => self.collect_secrets(action, &id).await?,
            ActionRequiredData::Elicitation {
                message,
                requested_schema,
                ..
            } => self.elicitation(&id, &message, &requested_schema).await?,
            // D1: the person's own Sign in window, vault prompt or host-key help, in this
            // terminal. Nothing typed comes back through the conversation.
            ActionRequiredData::CrewStep { step, .. } => {
                self.take_crew_step(&id, &step).await?;
                Interaction::Continue
            }
            ActionRequiredData::ElicitationResponse { .. } => unreachable!(),
        };
        self.answered.insert(id);
        Ok(outcome)
    }

    async fn take_crew_step(
        &mut self,
        id: &str,
        step: &biorouter::conversation::message::CrewStepRequest,
    ) -> Result<()> {
        #[cfg(test)]
        if let Some(input) = self.crew_step_test_input.take() {
            return input.await;
        }
        let surface = crate::commands::crew_step::StepSurface {
            client: &self.client,
            session_id: &self.session_id,
            card_id: id,
        };
        crate::commands::crew_step::take(&surface, step).await
    }

    async fn confirm_tool(
        &mut self,
        action: &ActionRequiredData,
        id: &str,
        mut watch: Option<TurnWatch<'_>>,
    ) -> Result<Interaction> {
        // The card whole, but never an argument whose name looks like a credential, one line
        // of it per line on the terminal.
        self.notice_lines(&card_lines(action))?;
        let folder_access = matches!(
            action,
            ActionRequiredData::ToolConfirmation {
                preview: Some(
                    biorouter::conversation::tool_preview::ToolPreview::FolderAccess { .. }
                ),
                ..
            }
        );
        // AO-12's read consent lasts until the person revokes it, so it is never offered as
        // "once": the choice is `allow` or `deny`, and the prompt says how to take it back.
        let read_consent = cards::is_read_consent(action);
        let question = approval_question(read_consent, folder_access, &self.session_id);
        loop {
            // The daemon stops waiting for a card when its window closes, and
            // says so on the turn's stream. Keep reading it while the person
            // types, so the prompt ends the moment the card expires instead of
            // taking an answer nothing waits on.
            #[cfg(test)]
            let input = match self.approval_test_input.take() {
                Some(injected) => futures::future::Either::Left(injected),
                None => futures::future::Either::Right(terminal_input(
                    &question, false, 32, None, None, None,
                )),
            };
            #[cfg(not(test))]
            let input = terminal_input(&question, false, 32, None, None, None);
            let asked = match watch.as_mut() {
                Some(watch) => {
                    ask_until_expired(
                        input,
                        &mut *watch.stream,
                        &mut *watch.deferred,
                        id,
                        watch.turn_id,
                        watch.sequence,
                    )
                    .await?
                }
                None => WhileAsking::Answered(input.await?),
            };
            let answer = match asked {
                WhileAsking::Answered(None) => return Ok(Interaction::Stop),
                WhileAsking::Answered(Some(answer)) => answer,
                // The turn ended under the prompt (stopped from another
                // window, say): nothing can use an answer now. The turn loop
                // reads the ending (from `deferred`, or the ended stream) and
                // reports it.
                WhileAsking::TurnEnded => {
                    eprint!("\r\n");
                    return Ok(Interaction::Continue);
                }
                // The card expired, or ended in another window: one line says
                // so, and the daemon's own words for that end, read after the
                // prompt, print nothing more.
                closed @ (WhileAsking::Expired | WhileAsking::Closed(_)) => {
                    eprint!("\r\n");
                    if let Some(line) = self.closed_here.close(id, &closed) {
                        self.notice(&line)?;
                    }
                    return Ok(Interaction::Continue);
                }
            };
            let decision = if folder_access && answer.trim() == "allow_session" {
                Some("allow_session")
            } else {
                cards::approval_answer(&answer, read_consent)
            };
            let Some(decision) = decision else {
                continue;
            };
            let result = self
                .client
                .request(
                    "POST",
                    "/action-required/tool-confirmation",
                    Some(json!({
                        "sessionId":self.session_id,"id":id,"principalType":"Tool","action":decision
                    })),
                )
                .await?;
            ensure!(
                matches!(
                    result["status"].as_str(),
                    Some("delivered" | "unknown" | "already_resolved")
                ),
                "Unexpected tool approval response"
            );
            if let Some(notice) = late_answer_notice(result["status"].as_str()) {
                // The daemon's own words for that end (its end frame, its
                // expiry notice) may still be on their way: this line said it.
                self.notice(notice)?;
                self.closed_here.note(id);
            } else if read_consent && decision == "allow_once" {
                self.notice(&cards::read_consent_allowed(&self.session_id))?;
            }
            return Ok(Interaction::Continue);
        }
    }

    async fn collect_secrets(&self, action: ActionRequiredData, id: &str) -> Result<Interaction> {
        let ActionRequiredData::SecretRequest {
            prompt,
            keys,
            destination,
            ..
        } = action
        else {
            unreachable!()
        };
        ensure!(
            !keys.is_empty() && keys.len() <= 32,
            "Unsupported secret request {}: expected 1–32 fields",
            safe_text(id)
        );
        self.notice(&format!("{}\nCredential destination: {}\nValues are hidden and sent only to the daemon credential endpoint.", prompt, serde_json::to_string(&destination)?))?;
        let mut secrets = SecretValues(Map::new());
        let values = &mut secrets.0;
        for key in keys {
            ensure!(
                !key.key.is_empty() && key.key.len() <= 256 && !values.contains_key(&key.key),
                "Unsupported credential field in request {}",
                safe_text(id)
            );
            let label = format!(
                "{}{}: ",
                key.label,
                if key.required {
                    " (required)"
                } else {
                    " (optional)"
                }
            );
            let Some(secret) = terminal_input(&label, true, 4096, None, None, None).await? else {
                erase_values(values);
                self.client
                    .request(
                        "POST",
                        "/action-required/secrets",
                        Some(json!({"id":id,"cancelled":true})),
                    )
                    .await?;
                return Ok(Interaction::Continue);
            };
            ensure!(
                !key.required || !secret.is_empty(),
                "Required credential was empty; request {} remains pending",
                safe_text(id)
            );
            if !secret.is_empty() {
                values.insert(key.key, Value::String(secret.to_string()));
            }
        }
        let mut body = Map::new();
        body.insert("id".into(), json!(id));
        body.insert("values".into(), Value::Object(std::mem::take(values)));
        body.insert("cancelled".into(), json!(false));
        // The transport owns ordinary JSON/byte copies; complete memory erasure is not promised.
        let result = self
            .client
            .request(
                "POST",
                "/action-required/secrets",
                Some(Value::Object(body)),
            )
            .await;
        let result = result.map_err(|_| anyhow::anyhow!("Credential submission was not confirmed; inspect the request in the desktop. No values were added to the conversation"))?;
        match result["status"].as_str() {
            Some("configured") => self.notice("The daemon confirmed credential configuration; no values were added to the conversation.")?,
            Some("unknown") => self.notice("The credential request is no longer pending; the daemon did not confirm configuration.")?,
            _ => bail!("Credential configuration was not confirmed; inspect the request in the desktop"),
        }
        Ok(Interaction::Continue)
    }

    async fn elicitation(&self, id: &str, prompt: &str, schema: &Value) -> Result<Interaction> {
        ensure!(!sensitive_schema(schema), "Elicitation {} requests credential-like input; use a trusted secret request instead. No answer was collected", safe_text(id));
        self.notice(&format!("{prompt}\nRequested JSON schema: {}\nOnly ordinary non-secret data belongs in this response.", serde_json::to_string_pretty(schema)?))?;
        let answer = terminal_input(
            "JSON object (Esc cancels this request): ",
            false,
            16_384,
            None,
            None,
            None,
        )
        .await?;
        let body = match answer {
            Some(answer) => {
                let data: Value = serde_json::from_str(&answer)
                    .context("Elicitation answer must be a JSON object; no answer was submitted")?;
                ensure!(
                    data.is_object(),
                    "Elicitation answer must be a JSON object; no answer was submitted"
                );
                json!({"session_id":self.session_id,"id":id,"data":data})
            }
            None => json!({"session_id":self.session_id,"id":id,"cancelled":true}),
        };
        let response = self
            .client
            .request("POST", "/action-required/elicitation", Some(body))
            .await;
        let response = match response {
            Err(error)
                if error
                    .downcast_ref::<DaemonRefusal>()
                    .is_some_and(|refusal| {
                        refusal.status == 409 && refusal.kind.as_deref() == Some("unknown")
                    }) =>
            {
                self.notice("This information request is no longer waiting; no answer was delivered or resubmitted.")?;
                return Ok(Interaction::Continue);
            }
            result => result?,
        };
        ensure!(response["status"] == "delivered", "The elicitation answer was not delivered; inspect request {} in this session before retrying", safe_text(id));
        Ok(Interaction::Continue)
    }

    async fn cancel(&self, turn_id: &str) -> Result<TurnEnd> {
        let result = self
            .client
            .request(
                "POST",
                "/agent/cancel",
                Some(json!({
                    "session_id":self.session_id,"expected_turn_id":turn_id,"wait_for_idle":true,
                    "continuation_pending":false
                })),
            )
            .await?;
        ensure!(
            result["settled"].as_bool() == Some(true),
            "Cancellation is not yet settled; inspect this exact session before submitting again"
        );
        self.event(&json!({"type":"Stopped","session_id":self.session_id,"result":result}))?;
        self.notice("The daemon confirmed the addressed turn is settled.")?;
        Ok(TurnEnd::Stopped)
    }

    fn event(&self, value: &Value) -> Result<()> {
        if self.format == Format::StreamJson {
            emit_json(value)?;
        }
        Ok(())
    }
    fn notice(&self, value: &str) -> Result<()> {
        let mut stderr = std::io::stderr().lock();
        writeln!(stderr, "{}", safe_text(value))?;
        stderr.flush()?;
        #[cfg(test)]
        self.noticed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(value.to_owned());
        Ok(())
    }
    /// [`Self::notice`] for text already split into `lines`, each printed on its own line and
    /// passed through [`safe_text`] (which leaves an escaped line as it is).
    fn notice_lines(&self, lines: &[String]) -> Result<()> {
        let mut stderr = std::io::stderr().lock();
        for line in lines {
            writeln!(stderr, "{}", safe_text(line))?;
        }
        stderr.flush()?;
        Ok(())
    }
    /// The chat's binding: the `SessionBinding` event, and on stderr "Chat … is ready" (or the
    /// binding itself in the JSON formats). With `defer`, the notice waits for
    /// [`Self::show_ready`], once the turn that follows reaches the provider.
    fn show_binding(&mut self, session: &Value, defer: bool) -> Result<()> {
        let binding = json!({"type":"SessionBinding","session_id":self.session_id,
            "provider":session["provider_name"],"model":session["model_config"],
            "privacy_tier":session["privacy_tier"]});
        self.event(&binding)?;
        self.binding = ShownBinding::of(session);
        if !self.quiet {
            let notice = binding_notice(self.format, &self.session_id, session, &binding);
            if defer {
                self.pending_ready = Some(notice);
            } else {
                self.notice(&notice)?;
            }
        }
        Ok(())
    }

    /// The deferred binding notice, once, now that the turn reached the provider.
    fn show_ready(&mut self) -> Result<()> {
        match self.pending_ready.take() {
            Some(notice) => self.notice(&notice),
            None => Ok(()),
        }
    }

    fn binding_change(&mut self, frame: &Value) -> Option<String> {
        binding_change(&mut self.binding, self.format, frame)
    }
    /// The session whose transcript `--history` prints. A resume that loads the
    /// model and extensions answers with the chat's row and no transcript (the
    /// agent load reads none, ruling 28), so when `state` came from one, the
    /// transcript comes from the resume that loads nothing, which carries it.
    /// An older daemon still sends it with the agent load, and then no second
    /// request is made.
    async fn history_session(&self, state: &Value) -> Result<Value> {
        if state["session"]["conversation"].is_array() {
            return Ok(state["session"].clone());
        }
        let mut painted = self.resume(false).await?;
        Ok(painted["session"].take())
    }

    fn show_history(&self, session: &Value) -> Result<()> {
        if self.format == Format::StreamJson {
            return self.event(&json!({"type":"History","session_id":self.session_id,"conversation":session["conversation"]}));
        }
        if self.format == Format::Text {
            if let Some(messages) = session["conversation"].as_array() {
                for value in messages {
                    print_message(&serde_json::from_value(value.clone())?, self.quiet)?;
                }
            }
        }
        Ok(())
    }
    async fn finish(&self, status: &str) -> Result<()> {
        let session = self
            .client
            .request(
                "GET",
                &format!("/sessions/{}", urlencoding::encode(&self.session_id)),
                None,
            )
            .await?;
        if self.format == Format::Json {
            emit_json(&json!({"status":status,"session":session}))?;
        } else {
            self.event(
                &json!({"type":"ConversationStatus","session_id":self.session_id,"status":status}),
            )?;
        }
        Ok(())
    }
}

/// What a turn's `PrivacyProviderPinned` or `ModelChange` frame says for a person (AGT2-N4),
/// given what the chat was shown with (`shown`, which it updates). The daemon sends the pin on
/// every turn that reaches a provider, so it is said only when it differs from what was shown:
/// the chat became private, or runs on another model. The JSON formats keep the frame as it is.
fn binding_change(shown: &mut ShownBinding, format: Format, frame: &Value) -> Option<String> {
    if format != Format::Text {
        return Some(frame.to_string());
    }
    let model = frame["model"].as_str().filter(|model| !model.is_empty())?;
    if frame["type"] == "ModelChange" {
        let mode = frame["mode"].as_str().filter(|mode| !mode.is_empty());
        return Some(match mode {
            Some(mode) => format!("Model changed to {model} in {mode} mode."),
            None => format!("Model changed to {model}."),
        });
    }
    let pinned = ShownBinding {
        provider: frame["provider"].as_str().map(str::to_owned),
        model: Some(model.to_owned()),
        private: frame["privacy_tier"].as_str() == Some("private"),
    };
    let before = std::mem::replace(shown, pinned.clone());
    let on = match &pinned.provider {
        Some(provider) => format!("{provider}/{model}"),
        None => model.to_owned(),
    };
    if pinned.private && !before.private {
        Some(format!("This chat is private now and stays on {on}."))
    } else if (pinned.provider != before.provider) || (pinned.model != before.model) {
        Some(format!("This chat now uses {on}."))
    } else {
        None
    }
}

#[derive(Default)]
struct TextRenderer {
    message_id: Option<String>,
    open: bool,
    ends_with_newline: bool,
}
impl TextRenderer {
    fn message(&mut self, message: &Message, quiet: bool) -> Result<()> {
        if !message.metadata.user_visible || (quiet && message.role != rmcp::model::Role::Assistant)
        {
            return Ok(());
        }
        if self.open && self.message_id != message.id {
            self.finish()?;
        }
        self.message_id = message.id.clone();
        for content in &message.content {
            match content {
                MessageContent::Text(text) => {
                    let mut stdout = std::io::stdout().lock();
                    write!(stdout, "{}", safe_content(&text.text))?;
                    stdout.flush()?;
                    self.open = true;
                    if !text.text.is_empty() {
                        self.ends_with_newline = text.text.ends_with('\n');
                    }
                }
                MessageContent::ActionRequired(_) => {}
                other if !quiet => {
                    self.finish()?;
                    writeln!(std::io::stdout(), "{}", safe_text(&format!("{other}")))?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        if self.open && !self.ends_with_newline {
            writeln!(std::io::stdout())?;
        }
        self.open = false;
        self.ends_with_newline = false;
        std::io::stdout().flush()?;
        Ok(())
    }
}
fn print_message(message: &Message, quiet: bool) -> Result<()> {
    let mut renderer = TextRenderer::default();
    renderer.message(message, quiet)?;
    renderer.finish()
}
fn safe_content(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| {
            if terminal_control(ch) && !matches!(ch, '\n' | '\t') {
                ch.escape_default().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect()
}
/// The question an approval card is answered with in this terminal. A read consent offers
/// `allow`, which lasts until the person revokes it, and says how; a folder grant can also be
/// allowed for the session; any other approval is allowed once.
fn approval_question(read_consent: bool, folder_access: bool, session_id: &str) -> String {
    if read_consent {
        format!(
            "Type {} (Esc cancels the turn): ",
            cards::read_consent_choices(session_id)
        )
    } else if folder_access {
        "Type allow_once, allow_session, or deny (Esc cancels the turn): ".to_string()
    } else {
        "Type allow_once or deny (Esc cancels the turn): ".to_string()
    }
}
fn sensitive_schema(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.get("writeOnly").and_then(Value::as_bool) == Some(true)
                || fields
                    .get("format")
                    .and_then(Value::as_str)
                    .is_some_and(|v| matches!(v, "password" | "secret"))
                || fields.iter().any(|(key, value)| {
                    key.to_ascii_lowercase().contains("password")
                        || key.to_ascii_lowercase().contains("secret")
                        || key.to_ascii_lowercase().contains("credential")
                        || matches!(
                            key.to_ascii_lowercase().as_str(),
                            "api_key"
                                | "apikey"
                                | "access_token"
                                | "refresh_token"
                                | "private_key"
                                | "authorization"
                        )
                        || sensitive_schema(value)
                })
        }
        Value::Array(items) => items.iter().any(sensitive_schema),
        _ => false,
    }
}
struct SecretValues(Map<String, Value>);
impl Drop for SecretValues {
    fn drop(&mut self) {
        erase_values(&mut self.0);
    }
}
fn erase_values(values: &mut Map<String, Value>) {
    use zeroize::Zeroize;
    for value in values.values_mut() {
        if let Value::String(secret) = value {
            secret.zeroize();
        }
    }
    values.clear();
}
fn terminal_control(ch: char) -> bool {
    ch.is_control() || biorouter::utils::is_invisible_formatting(ch)
}
/// The secret-output redactor's word for a private key it withheld, which it writes on a line of
/// its own (`secret_output::redact_text`), so inside a JSON string of a card.
const WITHHELD_PRIVATE_KEY: &str = "\n[REDACTED:private-key]\n";

/// An approval card as this terminal prints it before asking: its JSON
/// ([`cards::shown_card_json`]), one line per line of the pretty-printed document, each with every
/// control and invisible formatting character escaped. Splitting at its line breaks lets no
/// untrusted text start a line of its own, because JSON writes a line break inside a string as
/// `\n`. The one other line break is the redactor's around a withheld private key, which is
/// written back as `\n` first, so every line break left is the printer's.
fn card_lines(action: &ActionRequiredData) -> Vec<String> {
    cards::shown_card_json(action)
        .replace(
            WITHHELD_PRIVATE_KEY,
            &WITHHELD_PRIVATE_KEY.replace('\n', "\\n"),
        )
        .split('\n')
        .map(safe_text)
        .collect()
}

/// One warning for each session extension the daemon could not start, worded as an in-process
/// chat words it (`session::builder`): the chat goes on without it. Printed through
/// [`Conversation::notice`], so the daemon's words pass [`safe_text`].
fn extension_warnings(results: &[Value]) -> Vec<String> {
    results
        .iter()
        .filter(|result| result["success"].as_bool() == Some(false))
        .map(|result| {
            let name = result["name"].as_str().unwrap_or("unknown");
            let reason = result["error"].as_str().unwrap_or("no reason given");
            format!("Warning: Failed to start extension '{name}' ({reason}), continuing without it")
        })
        .collect()
}

pub(crate) fn safe_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| {
            if terminal_control(ch) {
                ch.escape_default().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect()
}
fn json_terminal_safe(encoded: &str) -> String {
    let mut escaped = String::with_capacity(encoded.len());
    for ch in encoded.chars() {
        if terminal_control(ch) {
            use std::fmt::Write;
            let mut units = [0; 2];
            for unit in ch.encode_utf16(&mut units) {
                let _ = write!(escaped, "\\u{unit:04x}");
            }
        } else {
            escaped.push(ch);
        }
    }
    escaped
}
fn emit_json(value: &Value) -> Result<()> {
    let escaped = json_terminal_safe(&serde_json::to_string(value)?);
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{escaped}")?;
    stdout.flush()?;
    Ok(())
}
const MAX_DEFERRED_CARD_FRAMES: usize = 256;
const MAX_DEFERRED_CARD_BYTES: usize = 32 * 1024 * 1024;

enum ObservedCrewStep {
    Completed(Interaction),
    Ended,
    Stop,
}

fn frame_ends_crew_step(frame: &Value, id: &str) -> bool {
    cards::card_events(frame)
        .into_iter()
        .any(|event| match event {
            cards::CardEvent::Ended { id: ended, .. } | cards::CardEvent::Responded(ended) => {
                ended == id
            }
            cards::CardEvent::TurnEnded => true,
            _ => false,
        })
}

fn observe_card_sequence(frame: &Value, turn_id: &str, sequence: &mut Option<u64>) -> Result<bool> {
    if let Some(turn) = frame["turn_id"].as_str() {
        ensure!(turn == turn_id, "Turn stream changed its generation");
    }
    if let Some(seq) = frame["seq"].as_u64() {
        if sequence.is_some_and(|previous| seq <= previous) {
            return Ok(false);
        }
        *sequence = Some(seq);
    }
    Ok(true)
}

async fn observe_crew_step<F, S: FrameSource>(
    interaction: F,
    stream: &mut S,
    deferred: &mut VecDeque<Value>,
    id: &str,
    turn_id: &str,
    sequence: Option<u64>,
    interactive: bool,
) -> Result<ObservedCrewStep>
where
    F: Future<Output = Result<Interaction>>,
{
    let mut observed_sequence = sequence;
    let mut deferred_bytes = 0usize;
    for frame in deferred.iter() {
        deferred_bytes = deferred_bytes.saturating_add(serde_json::to_vec(frame)?.len());
        if observe_card_sequence(frame, turn_id, &mut observed_sequence)?
            && frame_ends_crew_step(frame, id)
        {
            return Ok(ObservedCrewStep::Ended);
        }
    }
    if interactive {
        discard_terminal_input()?;
    }
    tokio::pin!(interaction);
    loop {
        tokio::select! {
            biased;
            signal = tokio::signal::ctrl_c() => {
                signal?;
                return Ok(ObservedCrewStep::Stop);
            }
            frame = stream.next_frame() => {
                let frame = frame?.context("Turn stream ended while a request waited; reattach to this session without new input")?;
                // The heartbeat (every 500 ms) is liveness, not content: deferring it would
                // fill `deferred` within about two minutes of a step left open (a sign-in the
                // person is still finishing) and end the turn with the error below.
                if frame["type"].as_str() == Some("Ping")
                    || !observe_card_sequence(&frame, turn_id, &mut observed_sequence)?
                {
                    continue;
                }
                deferred_bytes = deferred_bytes.saturating_add(serde_json::to_vec(&frame)?.len());
                ensure!(
                    deferred.len() < MAX_DEFERRED_CARD_FRAMES
                        && deferred_bytes <= MAX_DEFERRED_CARD_BYTES,
                    "Too many daemon events arrived while a request waited; reattach to this session without new input"
                );
                let ended = frame_ends_crew_step(&frame, id);
                deferred.push_back(frame);
                if ended {
                    return Ok(ObservedCrewStep::Ended);
                }
            }
            outcome = &mut interaction => {
                return outcome.map(ObservedCrewStep::Completed);
            }
        }
    }
}

/// The line for a decision the daemon did not deliver. `unknown` is a card
/// nothing waits on any more, which for a card still on screen is a window
/// that closed: said as the same expiry, never as a vague "no longer pending".
fn late_answer_notice(status: Option<&str>) -> Option<&'static str> {
    match status {
        Some("unknown") => Some(APPROVAL_EXPIRED),
        Some("already_resolved") => {
            Some("This request was already answered elsewhere; no new tool call was approved.")
        }
        _ => None,
    }
}

/// The turn's stream, lent to an approval's prompt so it keeps reading while the person types
/// ([`ask_until_expired`]). What it reads waits in `deferred` for the turn loop, which has
/// handled `turn_id`'s frames up to `sequence`.
struct TurnWatch<'a> {
    stream: &'a mut DaemonEventStream,
    deferred: &'a mut VecDeque<Value>,
    turn_id: &'a str,
    sequence: Option<u64>,
}

/// The turn stream, as a prompt reads it while it waits.
trait FrameSource {
    async fn next_frame(&mut self) -> Result<Option<Value>>;
}

impl FrameSource for DaemonEventStream {
    async fn next_frame(&mut self) -> Result<Option<Value>> {
        self.next_event().await
    }
}

/// How a prompt raised by approval card `id` ended.
#[derive(Debug)]
enum WhileAsking<T> {
    Answered(T),
    /// The daemon reported the card's window closed before an answer came: a
    /// parked card's `timed_out` end frame, or the notice that names the card
    /// (the only word of the agent's own prompt expiring).
    Expired,
    /// The daemon said the card ended another way (`ActionRequiredEnded`), with
    /// its word for how: answered or dismissed in another window, or withdrawn.
    Closed(String),
    /// The turn's stream ended (a terminal frame, or the stream's end), so no
    /// call is left to take an answer.
    TurnEnded,
}

/// Whether `frame` ends the prompt for approval card `id`, and how.
fn ends_approval_prompt<T>(frame: &Value, id: &str) -> Option<WhileAsking<T>> {
    if cards::expiry_reported_by(frame) == Some(id) {
        return Some(WhileAsking::Expired);
    }
    match frame["type"].as_str() {
        Some("ActionRequiredEnded") if frame["id"].as_str() == Some(id) => Some(
            WhileAsking::Closed(frame["ending"].as_str().unwrap_or_default().to_owned()),
        ),
        Some("Finish" | "Error") => Some(WhileAsking::TurnEnded),
        _ => None,
    }
}

/// The approvals whose prompt in this terminal ended by saying how they closed (expired, or
/// ended in another window), by card ID. The daemon says such an end in its own words too, an
/// end frame and, for an expiry, a notice naming the card, and the turn loop reads them after
/// the prompt: printed as well, one end would be two or three lines. Every ID here is also in
/// [`Conversation::answered`], whose bound holds for both.
#[derive(Debug, Default)]
struct ClosedHere(HashSet<String>);

impl ClosedHere {
    /// The one line a prompt for approval `id` ends with when `how` closed it, noting `id` so
    /// the daemon's words for that end print nothing more. `None` for an answer or the turn's
    /// end, which say their own.
    fn close<T>(&mut self, id: &str, how: &WhileAsking<T>) -> Option<String> {
        let line = match how {
            WhileAsking::Expired => APPROVAL_EXPIRED.to_string(),
            WhileAsking::Closed(ending) => cards::approval_closed_line(ending),
            WhileAsking::Answered(_) | WhileAsking::TurnEnded => return None,
        };
        self.note(id);
        Some(line)
    }

    /// Note that a line here said how approval `id` ended: an answer the daemon took for no
    /// waiting call ([`late_answer_notice`]) is said that way.
    fn note(&mut self, id: &str) {
        self.0.insert(id.to_owned());
    }

    /// The line for a card's end frame (`ActionRequiredEnded`), or `None` when the prompt for
    /// that card already said how it ended.
    fn ended_line(&self, frame: &Value) -> Option<String> {
        if frame["id"].as_str().is_some_and(|id| self.0.contains(id)) {
            return None;
        }
        Some(cards::ended_frame_line(frame))
    }

    /// Whether `message` is the expiry notice of an approval whose prompt here already said
    /// how it ended.
    fn said(&self, message: &Message) -> bool {
        biorouter::pending_user_action::expired_card_id(message)
            .is_some_and(|id| self.0.contains(id))
    }
}

/// Wait for the person's `input` while reading the turn's stream, and stop
/// as soon as the daemon reports that card `id` expired or ended another way
/// (answered in another window, say), or the turn ends.
///
/// Every frame read here (the expiry included) waits in `deferred`, in order and
/// within the same bounds as [`observe_crew_step`]'s, for the turn loop to handle
/// after the prompt, so nothing the stream said while the person was typing is
/// lost or reordered. Frames already waiting there are read first: an expiry
/// that arrived while another prompt was open ends this one before it is asked.
/// The stream's end ends the prompt, and the turn loop meets it again after the
/// frames before it (the stream stays ended); a stream error is the turn's error.
/// An answer that is ready wins over a frame that is ready at the same moment:
/// the person spoke, and the daemon's reply to it says whether it landed.
async fn ask_until_expired<T, S: FrameSource>(
    input: impl Future<Output = Result<T>>,
    stream: &mut S,
    deferred: &mut VecDeque<Value>,
    id: &str,
    turn_id: &str,
    sequence: Option<u64>,
) -> Result<WhileAsking<T>> {
    let mut observed_sequence = sequence;
    let mut deferred_bytes = 0usize;
    for frame in deferred.iter() {
        deferred_bytes = deferred_bytes.saturating_add(serde_json::to_vec(frame)?.len());
        if observe_card_sequence(frame, turn_id, &mut observed_sequence)? {
            if let Some(ending) = ends_approval_prompt(frame, id) {
                return Ok(ending);
            }
        }
    }
    tokio::pin!(input);
    loop {
        tokio::select! {
            biased;
            answer = &mut input => return Ok(WhileAsking::Answered(answer?)),
            frame = stream.next_frame() => {
                let Some(frame) = frame? else {
                    return Ok(WhileAsking::TurnEnded);
                };
                // The heartbeat (every 500 ms) is liveness, not content: deferring
                // it would fill `deferred` for as long as the person reads the card.
                if frame["type"].as_str() == Some("Ping")
                    || !observe_card_sequence(&frame, turn_id, &mut observed_sequence)?
                {
                    continue;
                }
                deferred_bytes = deferred_bytes.saturating_add(serde_json::to_vec(&frame)?.len());
                ensure!(
                    deferred.len() < MAX_DEFERRED_CARD_FRAMES
                        && deferred_bytes <= MAX_DEFERRED_CARD_BYTES,
                    "Too many daemon events arrived while a request waited; reattach to this session without new input"
                );
                let ending = ends_approval_prompt(&frame, id);
                deferred.push_back(frame);
                if let Some(ending) = ending {
                    return Ok(ending);
                }
            }
        }
    }
}

/// A cancelled prompt must not leave a passphrase or approval in the next prompt's input.
/// Call after the cancelled EventStream drops, or before a new one is created.
pub(crate) fn discard_terminal_input() -> Result<()> {
    if !std::io::stdin().is_terminal() {
        return Ok(());
    }
    let was_raw = crossterm::terminal::is_raw_mode_enabled()?;
    if !was_raw {
        crossterm::terminal::enable_raw_mode()?;
    }
    let discarded = (|| -> Result<()> {
        #[cfg(unix)]
        // SAFETY: stdin is a terminal; tcflush discards pending input only.
        ensure!(
            unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) } == 0,
            "Couldn't discard terminal input: {}",
            std::io::Error::last_os_error()
        );
        for _ in 0..1024 {
            if !crossterm::event::poll(std::time::Duration::ZERO)? {
                return Ok(());
            }
            if let Event::Paste(mut paste) = crossterm::event::read()? {
                paste.zeroize();
            }
        }
        bail!("Terminal input did not settle; no new request was answered")
    })();
    let restored = if was_raw {
        Ok(())
    } else {
        crossterm::terminal::disable_raw_mode().map_err(Into::into)
    };
    discarded.and(restored)
}

struct TerminalMode {
    accepted: bool,
}
impl Drop for TerminalMode {
    fn drop(&mut self) {
        if !self.accepted {
            let _ = discard_terminal_input();
        }
        let _ = crossterm::execute!(std::io::stderr(), DisableBracketedPaste);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
#[cfg(test)]
tokio::task_local! {
    static HIDDEN_INPUT_ACCEPTED: std::cell::RefCell<Option<tokio::sync::oneshot::Sender<()>>>;
    static HIDDEN_INPUT_READY: ();
}

#[cfg(test)]
fn observe_hidden_input(hidden: bool, has_input: bool) {
    if hidden && has_input {
        let _ = HIDDEN_INPUT_ACCEPTED.try_with(|sender| {
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(());
            }
        });
    }
}

/// One line typed at this terminal after `prompt`, echoed unless `hidden`, at most `limit`
/// bytes. `Ok(None)` when the person pressed Esc, Ctrl-C or Ctrl-D.
pub(crate) async fn terminal_input(
    prompt: &str,
    hidden: bool,
    limit: usize,
    initial: Option<&str>,
    dictation_session: Option<&str>,
    dictation_state: Option<&SharedDictationState>,
) -> Result<Option<Zeroizing<String>>> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "Human input requires an interactive terminal"
    );
    ensure!(
        !hidden || initial.is_none(),
        "Credential input cannot use an initial seed"
    );
    #[cfg(feature = "dictation")]
    let dictation = TerminalDictation::new(dictation_session, dictation_state);
    crossterm::terminal::enable_raw_mode()?;
    let mut mode = TerminalMode { accepted: false };
    crossterm::execute!(std::io::stderr(), EnableBracketedPaste)?;
    if hidden {
        discard_terminal_input()?;
    }
    let mut events = EventStream::new();
    eprint!("{}", safe_text(prompt));
    std::io::stderr().flush()?;
    #[cfg(test)]
    if hidden {
        let _ = HIDDEN_INPUT_READY.try_with(|_| eprintln!("BIOROUTER_B01_HIDDEN_INPUT_READY"));
    }
    let mut value = seeded_value(initial, hidden, limit)?;
    while let Some(event) = events.next().await {
        let event = event?;
        if let Event::Paste(paste) = event {
            accept_paste(&mut value, paste, hidden, limit)?;
            continue;
        }
        let Event::Key(key) = event else { continue };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            #[cfg(feature = "dictation")]
            KeyCode::Char(c) if dictation.is_chord(c, key.modifiers) => {
                events = DictateLine::Chord
                    .run(events, prompt, limit, &mut value, &dictation)
                    .await?;
            }
            KeyCode::Esc => {
                eprint!("\r\n");
                return Ok(None);
            }
            KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                eprint!("\r\n");
                return Ok(None);
            }
            KeyCode::Enter => {
                #[cfg(feature = "dictation")]
                if let Some(command) = dictation.command(&value) {
                    value.clear();
                    events = command
                        .run(events, prompt, limit, &mut value, &dictation)
                        .await?;
                    continue;
                }
                eprint!("\r\n");
                mode.accepted = true;
                return Ok(Some(value));
            }
            KeyCode::Backspace => {
                if value.pop().is_some() && !hidden {
                    eprint!("\x08 \x08");
                }
            }
            KeyCode::Char(ch)
                if !ch.is_control() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                ensure!(
                    value.len() + ch.len_utf8() <= limit,
                    "Input exceeds the permitted byte limit; nothing was submitted"
                );
                value.push(ch);
                #[cfg(test)]
                observe_hidden_input(hidden, true);
                if !hidden {
                    eprint!("{}", safe_text(&ch.to_string()));
                }
            }
            _ => {}
        }
        std::io::stderr().flush()?;
    }
    Ok(None)
}

/// A bracketed paste into the prompt: refused whole above `limit`, refused for a hidden
/// (credential) prompt when it holds a control character, and echoed when it may be shown.
fn accept_paste(
    value: &mut Zeroizing<String>,
    paste: String,
    hidden: bool,
    limit: usize,
) -> Result<()> {
    let paste = Zeroizing::new(paste);
    ensure!(
        value.len() + paste.len() <= limit,
        "Input exceeds the permitted byte limit; nothing was submitted"
    );
    ensure!(
        !hidden || !paste.chars().any(char::is_control),
        "Credential input must be a single line; nothing was submitted"
    );
    value.push_str(&paste);
    #[cfg(test)]
    observe_hidden_input(hidden, !value.is_empty());
    if !hidden {
        eprint!("{}", safe_text(&paste));
    }
    std::io::stderr().flush()?;
    Ok(())
}

/// The prompt's text before the person types: `initial`, refused for a hidden (credential)
/// prompt and above `limit`, and echoed when it may be shown.
fn seeded_value(initial: Option<&str>, hidden: bool, limit: usize) -> Result<Zeroizing<String>> {
    let value = Zeroizing::new(initial.unwrap_or("").to_string());
    ensure!(
        value.len() <= limit,
        "Input exceeds the permitted byte limit; nothing was submitted"
    );
    if !hidden && !value.is_empty() {
        eprint!("{}", safe_text(value.as_str()));
    }
    Ok(value)
}

/// Dictation at a terminal prompt (S9): the chat it dictates for, if any, and the
/// session's dictation state. A prompt with no chat offers no dictation at all.
#[cfg(feature = "dictation")]
struct TerminalDictation<'a> {
    session: Option<&'a str>,
    state: Option<&'a SharedDictationState>,
}

#[cfg(feature = "dictation")]
impl<'a> TerminalDictation<'a> {
    /// Says, once per prompt, when the dictation key is also the newline key, so the chord
    /// cannot start a recording here.
    fn new(session: Option<&'a str>, state: Option<&'a SharedDictationState>) -> Self {
        if session.is_some() {
            if let Some(key) = crate::session::dictation_key()
                .filter(|key| *key == crate::session::get_newline_key())
            {
                eprintln!("{}", crate::dictation::messages::msg_key_clash(key));
            }
        }
        Self { session, state }
    }

    fn is_chord(&self, c: char, modifiers: KeyModifiers) -> bool {
        let key = Some(c.to_ascii_lowercase());
        self.session.is_some()
            && modifiers.contains(KeyModifiers::CONTROL)
            && key == crate::session::dictation_key()
            && key != Some(crate::session::get_newline_key())
    }

    fn command(&self, line: &str) -> Option<DictateLine> {
        self.session.and(DictateLine::parse(line))
    }
}

/// What dictation does at a terminal prompt (S9): the chord, and the `/dictate` and
/// `/dictionary` commands the prompt answers itself, read from the whole line as the person
/// typed it before Enter. The grammar is the in-process chat's (`session/input.rs`), so a chat
/// on the shared daemon never hands one of these lines to the model as a message.
#[cfg(feature = "dictation")]
enum DictateLine {
    Chord,
    Capture,
    Retry,
    Status,
    Install(Option<String>),
    DictionaryAdd(String),
    DictionaryReview,
    DictionaryScan,
}

#[cfg(feature = "dictation")]
impl DictateLine {
    fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "/dictate" => Some(Self::Capture),
            "/dictate retry" => Some(Self::Retry),
            "/dictate status" => Some(Self::Status),
            "/dictionary review" => Some(Self::DictionaryReview),
            "/dictionary scan" => Some(Self::DictionaryScan),
            other => match other.strip_prefix("/dictionary add ") {
                Some(term) => Some(Self::DictionaryAdd(term.trim().to_string())),
                None => other
                    .strip_prefix("/dictate install")
                    .filter(|rest| rest.is_empty() || rest.starts_with(' '))
                    .map(|rest| {
                        Self::Install(Some(rest.trim().to_string()).filter(|m| !m.is_empty()))
                    }),
            },
        }
    }

    /// Runs it and hands back the terminal's event reader (a new one when it left raw
    /// mode). The chord adds its transcript after what is typed; a command, whose line the
    /// caller emptied, writes the prompt again and puts its transcript there.
    async fn run(
        self,
        events: EventStream,
        prompt: &str,
        limit: usize,
        value: &mut Zeroizing<String>,
        dictation: &TerminalDictation<'_>,
    ) -> Result<EventStream> {
        let session = dictation.session.map(str::to_string);
        let state = dictation.state.cloned().unwrap_or_default();
        match self {
            Self::Chord => {
                let capture = crate::dictation::frontend::capture_in_terminal(session, None, state);
                let (events, transcript) = outside_raw_mode(events, capture).await?;
                insert_dictated(value, transcript, limit, true);
                Ok(events)
            }
            Self::Capture | Self::Retry => {
                let (events, transcript) = if matches!(self, Self::Capture) {
                    let capture =
                        crate::dictation::frontend::capture_in_terminal(session, None, state);
                    outside_raw_mode(events, capture).await?
                } else {
                    let retry = crate::dictation::frontend::retry_in_terminal(session, None, state);
                    outside_raw_mode(events, retry).await?
                };
                eprint!("\r{}", safe_text(prompt));
                insert_dictated(value, transcript, limit, false);
                Ok(events)
            }
            Self::Status => {
                let paused = dictation.state.is_some_and(|state| state.paused());
                eprintln!(
                    "\r\n{}",
                    if paused {
                        crate::dictation::messages::MSG_PAUSED
                    } else {
                        "Dictation is ready."
                    }
                );
                eprint!("{}", safe_text(prompt));
                Ok(events)
            }
            Self::Install(model) => {
                let install = crate::commands::dictation::handle_install(
                    crate::commands::dictation::InstallArgs {
                        model,
                        from: None,
                        system: false,
                        yes: false,
                    },
                );
                let (events, result) = outside_raw_mode(events, install).await?;
                if let Err(error) = result {
                    eprintln!("{error}");
                }
                eprint!("\r{}", safe_text(prompt));
                Ok(events)
            }
            // The three run as they do in an in-process chat, outside raw mode so the
            // review's questions and the scan's Ctrl+C read the terminal as they do there.
            // With no model in this process, `add_term` takes the chat's tier from its
            // stored row, which the daemon keeps.
            Self::DictionaryAdd(term) => {
                let chat = session.unwrap_or_default();
                let add = crate::dictation::frontend::add_term(&chat, None, &term);
                let (events, added) = outside_raw_mode(events, add).await?;
                match added {
                    Ok(note) => eprintln!("{note}"),
                    Err(reason) => eprintln!("{reason}"),
                }
                eprint!("\r{}", safe_text(prompt));
                Ok(events)
            }
            Self::DictionaryReview => {
                let review = async { crate::session::review_dictionary_in_session() };
                let (events, ()) = outside_raw_mode(events, review).await?;
                eprint!("\r{}", safe_text(prompt));
                Ok(events)
            }
            Self::DictionaryScan => {
                let scan = crate::commands::dictionary::scan_in_classic_session();
                let (events, ()) = outside_raw_mode(events, scan).await?;
                eprint!("\r{}", safe_text(prompt));
                Ok(events)
            }
        }
    }
}

/// Runs `work` with the terminal out of raw mode: a capture reads the terminal itself
/// (Enter stops, Esc cancels), so the prompt's event reader is dropped before it starts
/// and a new one is made after.
#[cfg(feature = "dictation")]
async fn outside_raw_mode<T>(
    events: EventStream,
    work: impl std::future::Future<Output = T>,
) -> Result<(EventStream, T)> {
    drop(events);
    crossterm::terminal::disable_raw_mode()?;
    let out = work.await;
    crossterm::terminal::enable_raw_mode()?;
    Ok((EventStream::new(), out))
}

/// Puts a dictated transcript into the line being typed and echoes it, or says why it did
/// not. `after_text` separates it from text already typed with one space.
#[cfg(feature = "dictation")]
fn insert_dictated(
    value: &mut Zeroizing<String>,
    transcript: Result<Option<String>, String>,
    limit: usize,
    after_text: bool,
) {
    match transcript {
        Ok(Some(text)) => {
            let text = text.trim();
            let separator = if after_text
                && value
                    .chars()
                    .next_back()
                    .is_some_and(|c| !c.is_whitespace())
            {
                " "
            } else {
                ""
            };
            if value.len() + separator.len() + text.len() <= limit {
                value.push_str(separator);
                value.push_str(text);
                eprint!("{separator}{}", safe_text(text));
            } else {
                eprintln!("Input exceeds the permitted byte limit; nothing was inserted");
            }
        }
        Ok(None) => {}
        Err(reason) => eprintln!("{reason}"),
    }
}

/// What the conversation says on stderr once it knows its chat (AG-F7): in text, a sentence
/// naming the chat, its provider and model; in JSON formats, the binding itself, as before.
/// Text mode printed the raw `SessionBinding` JSON.
fn binding_notice(format: Format, session_id: &str, session: &Value, binding: &Value) -> String {
    if format != Format::Text {
        return binding.to_string();
    }
    let provider = session["provider_name"]
        .as_str()
        .filter(|provider| !provider.is_empty());
    let model = session["model_config"]["model_name"]
        .as_str()
        .filter(|model| !model.is_empty());
    match (provider, model) {
        (Some(provider), Some(model)) => {
            format!("Chat {session_id} is ready ({provider}/{model}).")
        }
        (Some(provider), None) => format!("Chat {session_id} is ready ({provider})."),
        _ => format!("Chat {session_id} is ready."),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        approval_question, binding_change, binding_notice, card_lines, cards, extension_warnings,
        json_terminal_safe, ready_on, safe_text, sensitive_schema, terminal_control, turn_error,
        validate_id, validate_options, validate_prompt, with_session_context, Format, Ready,
        SharedConversationOptions, ShownBinding, StartRefused,
    };
    use biorouter::conversation::message::ActionRequiredData;

    /// Each extension that failed to start gets one warning naming it and the daemon's reason,
    /// in an in-process chat's words; one that started gets none.
    #[test]
    fn a_failed_extension_s_warning_names_it_and_its_reason() {
        let results = [
            serde_json::json!({"name": "lab-mcp", "success": false, "error": "spawn failed"}),
            serde_json::json!({"name": "developer", "success": true}),
            serde_json::json!({"success": false}),
        ];
        assert_eq!(
            extension_warnings(&results),
            [
                "Warning: Failed to start extension 'lab-mcp' (spawn failed), continuing without it",
                "Warning: Failed to start extension 'unknown' (no reason given), continuing without it",
            ]
        );
    }

    fn approval(arguments: serde_json::Value) -> ActionRequiredData {
        ActionRequiredData::ToolConfirmation {
            id: "card-1".into(),
            tool_name: "crew__change".into(),
            arguments: arguments.as_object().cloned().unwrap_or_default(),
            prompt: Some("Post as you in #general\nIt is public to the workspace.".into()),
            risk: None,
            preview: None,
        }
    }

    /// An approval card prints as the readable JSON it is, one line per line, rather than one
    /// line full of `\n`; and no line carries a control or invisible formatting character, nor
    /// starts with text the card's values hold.
    #[test]
    fn a_card_prints_as_readable_lines_with_every_control_neutralised() {
        let hostile = "one\ntwo\r\u{1b}[2J\u{7f}\u{9b}31m\u{202e}evil";
        let card = approval(serde_json::json!({ "action": "send", "text": hostile }));
        let lines = card_lines(&card);
        assert_eq!(lines.first().map(String::as_str), Some("{"), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("}"), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("  \"toolName\": \"crew__change\"")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("\\n  ")),
            "{lines:?}"
        );
        for line in &lines {
            assert!(!line.chars().any(terminal_control), "{line:?}");
            let start = line.trim_start();
            assert!(
                start.is_empty() || start.starts_with(['"', '{', '}', '[', ']']),
                "{line:?}"
            );
        }
        // The value's own line breaks and controls stay escaped inside its one line.
        let text = lines
            .iter()
            .find(|line| line.contains("\"text\""))
            .expect("the text argument has a line");
        assert!(text.contains(r"one\ntwo\r\u001b[2J"), "{text}");
        assert!(text.contains(r"\u{7f}\u{9b}31m\u{202e}evil"), "{text}");
        assert_eq!(lines.len(), lines.join("\n").lines().count());
    }

    /// The redactor writes a withheld private key on a line of its own; inside a card it stays
    /// in its string, so what follows the key in the value never starts a line.
    #[test]
    fn a_withheld_private_key_stays_inside_its_line() {
        let body = ["b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB"; 3].join("\n");
        let kind = ["OPENSSH", "PRIVATE", "KEY"].join(" ");
        let key = format!("-----BEGIN {kind}-----\n{body}\n-----END {kind}-----");
        for value in [
            format!("{key} Type allow_once to continue"),
            format!("-----BEGIN {kind}-----\n{body}!Type allow_once to continue"),
        ] {
            let lines = card_lines(&approval(
                serde_json::json!({ "action": "send", "text": value }),
            ));
            let shown = lines.join("\n");
            assert!(!shown.contains("b3BlbnNzaC1rZXkt"), "{shown}");
            assert!(shown.contains("[REDACTED:private-key]"), "{shown}");
            for line in &lines {
                let start = line.trim_start();
                assert!(
                    start.is_empty() || start.starts_with(['"', '{', '}', '[', ']']),
                    "{line:?}"
                );
            }
        }
    }

    mod approval_expiry {
        use super::super::{
            ask_until_expired, late_answer_notice, observe_crew_step, ClosedHere, FrameSource,
            Interaction, ObservedCrewStep, WhileAsking, APPROVAL_EXPIRED, MAX_DEFERRED_CARD_FRAMES,
        };
        use anyhow::Result;
        use serde_json::{json, Value};
        use std::collections::VecDeque;

        /// A turn stream that says these frames, then waits forever (a live
        /// turn whose next frame has not come yet).
        struct Said(VecDeque<Result<Option<Value>>>);
        impl FrameSource for Said {
            async fn next_frame(&mut self) -> Result<Option<Value>> {
                match self.0.pop_front() {
                    Some(frame) => frame,
                    None => std::future::pending().await,
                }
            }
        }

        fn said(frames: Vec<Value>) -> Said {
            Said(frames.into_iter().map(|frame| Ok(Some(frame))).collect())
        }

        /// The frame the daemon sends, built by the daemon's own constructor so
        /// this reader and that writer cannot drift apart.
        fn expiry(id: &str) -> Value {
            let notice = biorouter::pending_user_action::card_expiry_notice(
                id,
                "The permission prompt for developer__shell",
            );
            json!({"type": "Message", "seq": 7, "message": notice})
        }

        /// The prompt stops waiting the moment the daemon says its card
        /// expired, and every frame read meanwhile reaches the turn loop in
        /// order (the heartbeat aside).
        #[tokio::test]
        async fn the_prompt_stops_when_its_card_expires() {
            let mut stream = said(vec![
                json!({"type": "Ping"}),
                json!({"type": "Notification", "seq": 6}),
                expiry("card-1"),
            ]);
            let mut deferred = VecDeque::new();
            let asked = ask_until_expired(
                std::future::pending::<Result<Option<String>>>(),
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                Some(5),
            )
            .await
            .unwrap();
            assert!(matches!(asked, WhileAsking::Expired));
            assert_eq!(
                Vec::from(deferred),
                vec![json!({"type": "Notification", "seq": 6}), expiry("card-1")]
            );
        }

        /// Another card's expiry leaves the prompt open: the person still
        /// answers it, and the frame is deferred for the loop.
        #[tokio::test]
        async fn another_cards_expiry_does_not_end_the_prompt() {
            let mut stream = said(vec![expiry("card-2")]);
            let mut deferred = VecDeque::new();
            let (answer, typed) = tokio::sync::oneshot::channel::<()>();
            let input = async move {
                typed.await.unwrap();
                Ok(Some("deny".to_string()))
            };
            let asked = tokio::join!(
                ask_until_expired(input, &mut stream, &mut deferred, "card-1", "turn-1", None),
                async move {
                    tokio::task::yield_now().await;
                    answer.send(()).unwrap();
                }
            )
            .0
            .unwrap();
            assert!(matches!(asked, WhileAsking::Answered(Some(ref text)) if text == "deny"));
            assert_eq!(Vec::from(deferred), vec![expiry("card-2")]);
        }

        /// The heartbeats of a step held open longer than the deferred-frame cap
        /// (twice the cap is about four minutes at one Ping every 500 ms).
        fn heartbeats_past_the_cap() -> Vec<Value> {
            vec![json!({"type": "Ping"}); MAX_DEFERRED_CARD_FRAMES * 2 + 1]
        }

        /// A Crew step left open (a sign-in the person is still finishing) keeps
        /// waiting through the heartbeat however long it takes: Pings are not
        /// deferred, so they never reach the cap, and the frames that matter still
        /// reach the turn loop in order.
        #[tokio::test]
        async fn a_crew_step_held_open_past_the_cap_still_completes() {
            let mut frames = heartbeats_past_the_cap();
            frames.insert(3, json!({"type": "Notification", "seq": 6}));
            let mut stream = said(frames);
            let mut deferred = VecDeque::new();
            let observed = observe_crew_step(
                async { Ok(Interaction::Continue) },
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                Some(5),
                false,
            )
            .await
            .expect("the heartbeat does not end a step held open");
            assert!(matches!(
                observed,
                ObservedCrewStep::Completed(Interaction::Continue)
            ));
            assert_eq!(
                Vec::from(deferred),
                vec![json!({"type": "Notification", "seq": 6})]
            );
        }

        /// The same for an approval's prompt the person reads for minutes.
        #[tokio::test]
        async fn an_approval_held_open_past_the_cap_still_expires_cleanly() {
            let mut frames = heartbeats_past_the_cap();
            frames.push(expiry("card-1"));
            let mut stream = said(frames);
            let mut deferred = VecDeque::new();
            let asked = ask_until_expired(
                std::future::pending::<Result<Option<String>>>(),
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                Some(5),
            )
            .await
            .expect("the heartbeat does not end a prompt held open");
            assert!(matches!(asked, WhileAsking::Expired));
            assert_eq!(Vec::from(deferred), vec![expiry("card-1")]);
        }

        /// A turn that ends under the prompt (stopped from another window)
        /// ends the prompt too, and the loop still reads the ending.
        #[tokio::test]
        async fn the_prompt_stops_when_the_turn_ends() {
            let finish = json!({"type": "Finish", "seq": 9, "reason": "cancelled"});
            let mut stream = said(vec![finish.clone()]);
            let mut deferred = VecDeque::new();
            let asked = ask_until_expired(
                std::future::pending::<Result<Option<String>>>(),
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                None,
            )
            .await
            .unwrap();
            assert!(matches!(asked, WhileAsking::TurnEnded));
            assert_eq!(deferred.pop_front(), Some(finish));
        }

        /// A stream that ends under the prompt ends it too; what came before
        /// the end still reaches the loop, which then meets the end itself.
        #[tokio::test]
        async fn the_prompt_stops_when_the_stream_ends() {
            let notification = json!({"type": "Notification", "seq": 6});
            let mut stream = Said(VecDeque::from([Ok(Some(notification.clone())), Ok(None)]));
            let mut deferred = VecDeque::new();
            let asked = ask_until_expired(
                std::future::pending::<Result<Option<String>>>(),
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                None,
            )
            .await
            .unwrap();
            assert!(matches!(asked, WhileAsking::TurnEnded));
            assert_eq!(Vec::from(deferred), vec![notification]);
        }

        /// An expiry read while another card's prompt was open ends this
        /// card's prompt before it is asked, and stays for the loop.
        #[tokio::test]
        async fn an_expiry_already_read_ends_the_prompt_before_it_is_asked() {
            let mut stream = said(Vec::new());
            let mut deferred = VecDeque::from([expiry("card-1")]);
            let asked = ask_until_expired(
                async { Err::<Option<String>, _>(anyhow::anyhow!("the prompt was asked")) },
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                Some(5),
            )
            .await
            .unwrap();
            assert!(matches!(asked, WhileAsking::Expired));
            assert_eq!(Vec::from(deferred), vec![expiry("card-1")]);
        }

        /// A late click the daemon answers `unknown` is said as the same expiry,
        /// never as a vague "no longer pending".
        #[test]
        fn a_late_answer_is_said_as_expired() {
            assert_eq!(late_answer_notice(Some("unknown")), Some(APPROVAL_EXPIRED));
            assert!(
                APPROVAL_EXPIRED.starts_with("Expired") && APPROVAL_EXPIRED.contains("Ask again")
            );
            assert!(late_answer_notice(Some("already_resolved"))
                .is_some_and(|line| line.contains("already answered")));
            assert_eq!(late_answer_notice(Some("delivered")), None);
        }

        fn ended(id: &str, ending: &str, seq: u64) -> Value {
            json!({"type": "ActionRequiredEnded", "id": id, "ending": ending, "seq": seq})
        }

        /// A parked card's expiry comes first as its end frame (`timed_out`), and the notice
        /// follows: the prompt stops at the end frame. One that ends any other way in another
        /// window (answered on the desktop, say) stops the prompt too, with the daemon's word
        /// for how. Either way the frame stays for the turn loop.
        #[tokio::test]
        async fn the_prompt_stops_when_its_card_ends_elsewhere() {
            for (ending, expected) in [("timed_out", None), ("answered", Some("answered"))] {
                let mut stream = said(vec![
                    ended("card-2", "answered", 6),
                    ended("card-1", ending, 7),
                ]);
                let mut deferred = VecDeque::new();
                // Bounded, so a prompt the end frame leaves open fails rather than hangs.
                let asked = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    ask_until_expired(
                        std::future::pending::<Result<Option<String>>>(),
                        &mut stream,
                        &mut deferred,
                        "card-1",
                        "turn-1",
                        Some(5),
                    ),
                )
                .await
                .unwrap_or_else(|_| panic!("{ending}: the card's end frame left its prompt open"))
                .unwrap();
                match (asked, expected) {
                    (WhileAsking::Expired, None) => {}
                    (WhileAsking::Closed(how), Some(word)) if how == word => {}
                    (asked, _) => panic!("{ending}: the prompt ended as {asked:?}"),
                }
                assert_eq!(
                    Vec::from(deferred),
                    vec![ended("card-2", "answered", 6), ended("card-1", ending, 7)],
                    "another card's end leaves the prompt open, and both reach the loop"
                );
            }
        }

        /// An end already read while another card's prompt was open ends this card's prompt
        /// before it is asked.
        #[tokio::test]
        async fn an_end_already_read_ends_the_prompt_before_it_is_asked() {
            let mut stream = said(Vec::new());
            let mut deferred = VecDeque::from([ended("card-1", "dismissed", 6)]);
            let asked = ask_until_expired(
                async { Err::<Option<String>, _>(anyhow::anyhow!("the prompt was asked")) },
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                Some(5),
            )
            .await
            .unwrap();
            assert!(matches!(asked, WhileAsking::Closed(ref how) if how == "dismissed"));
        }

        /// One expiry is one line at the prompt. A parked card's expiry reaches the turn as its
        /// end frame and then the notice naming it; the agent's own prompt's, as the notice
        /// alone. The prompt says `APPROVAL_EXPIRED`, and neither of the daemon's words for that
        /// expiry, read after it, prints again. A card that ended in another window says the
        /// daemon's word for how, once. Another card's end and notice still print, and an
        /// answer or the turn's end notes nothing.
        #[test]
        fn a_closed_prompt_is_the_one_line_for_its_end() {
            let notice = |id: &str| {
                biorouter::pending_user_action::card_expiry_notice(
                    id,
                    "The folder access request for /synthetic/results",
                )
            };
            let mut closed = ClosedHere::default();
            assert_eq!(
                closed.close("card-1", &WhileAsking::<()>::Expired),
                Some(APPROVAL_EXPIRED.to_string())
            );
            assert_eq!(closed.ended_line(&ended("card-1", "timed_out", 8)), None);
            assert!(closed.said(&notice("card-1")));

            assert_eq!(
                closed.close("card-2", &WhileAsking::<()>::Closed("answered".into())),
                Some("[approval closed] This request was answered somewhere else.".to_string())
            );
            assert_eq!(closed.ended_line(&ended("card-2", "answered", 9)), None);

            assert_eq!(
                closed.ended_line(&ended("card-3", "timed_out", 10)),
                Some("[closed] A request in this chat is over: timed out.".to_string())
            );
            assert!(!closed.said(&notice("card-3")));
            assert_eq!(closed.close("card-4", &WhileAsking::Answered(())), None);
            assert_eq!(closed.close("card-4", &WhileAsking::<()>::TurnEnded), None);
            assert!(closed
                .ended_line(&ended("card-4", "answered", 11))
                .is_some());

            // A late answer the daemon took for no waiting call is said as the expiry, and
            // that card's end frame and notice then print nothing more.
            closed.note("card-5");
            assert_eq!(closed.ended_line(&ended("card-5", "timed_out", 12)), None);
            assert!(closed.said(&notice("card-5")));
        }
    }

    /// AGT2-N4: "Chat … is ready" waits for the turn that follows it to reach the provider,
    /// and is dropped when the daemon refuses the start first; `TurnStarted`, sent before any
    /// work that can fail, says neither.
    #[test]
    fn the_ready_line_waits_for_the_turn_to_reach_the_provider() {
        let frame = |kind: &str| serde_json::json!({"type": kind});
        assert_eq!(ready_on(&frame("TurnStarted"), false), Ready::Wait);
        assert_eq!(ready_on(&frame("Ping"), false), Ready::Wait);
        for kind in ["Message", "PrivacyProviderPinned", "ModelChange"] {
            assert_eq!(ready_on(&frame(kind), false), Ready::Show, "{kind}");
        }
        let refused = serde_json::json!({"type": "Error", "code": "inference_start_failed"});
        assert_eq!(ready_on(&refused, false), Ready::Drop);
        assert_eq!(ready_on(&refused, true), Ready::Wait, "already shown");
        let finished = serde_json::json!({"type": "Finish", "reason": "stop"});
        assert_eq!(ready_on(&finished, false), Ready::Show);
        let stopped = serde_json::json!({"type": "Finish", "reason": "cancelled"});
        assert_eq!(ready_on(&stopped, false), Ready::Drop);
    }

    /// AGT2-N4, CLIDOCS-F12: a pin that agrees with what the chat was shown with says nothing
    /// (the daemon sends one on every turn); one that makes the chat private, or moves it,
    /// says so in a sentence. Text never gets the raw frame; the JSON formats keep it.
    #[test]
    fn a_privacy_pin_is_a_sentence_only_when_it_changes_something() {
        let frame = serde_json::json!({"type": "PrivacyProviderPinned", "provider": "versa_azure",
            "model": "gpt-5.5-2026-04-24", "privacy_tier": "private", "privacy_reason": "mcp:crew",
            "seq": 2, "turn_id": "turn-3"});
        let mut shown = ShownBinding {
            provider: Some("versa_azure".into()),
            model: Some("gpt-5.5-2026-04-24".into()),
            private: false,
        };
        assert_eq!(
            binding_change(&mut shown, Format::Text, &frame).as_deref(),
            Some("This chat is private now and stays on versa_azure/gpt-5.5-2026-04-24.")
        );
        // The next turn's pin agrees with what was said.
        assert_eq!(binding_change(&mut shown, Format::Text, &frame), None);
        let moved = serde_json::json!({"type": "PrivacyProviderPinned", "provider": "versa_azure",
            "model": "gpt-5.5-mini", "privacy_tier": "private"});
        assert_eq!(
            binding_change(&mut shown, Format::Text, &moved).as_deref(),
            Some("This chat now uses versa_azure/gpt-5.5-mini.")
        );
        for text in [
            binding_change(&mut ShownBinding::default(), Format::Text, &frame),
            binding_change(&mut shown, Format::Text, &moved),
        ]
        .into_iter()
        .flatten()
        {
            assert!(!text.contains('{'), "{text}");
        }
        let change =
            serde_json::json!({"type": "ModelChange", "model": "gpt-5.5-mini", "mode": "worker"});
        assert_eq!(
            binding_change(&mut shown, Format::Text, &change).as_deref(),
            Some("Model changed to gpt-5.5-mini in worker mode.")
        );
        for format in [Format::Json, Format::StreamJson] {
            assert_eq!(
                binding_change(&mut shown, format, &frame),
                Some(frame.to_string())
            );
        }
    }

    /// AGT2-N4: a start the daemon refused before anything reached the provider is its own
    /// sentence, without the code or the "remains available" wrapper; a turn that failed after
    /// it reached the provider keeps both.
    #[test]
    fn a_refused_start_is_the_daemons_sentence_alone() {
        const REMOVED: &str =
            "This chat's Crew access was removed. Grant access again to use Crew from this chat.";
        let frame = serde_json::json!({"type": "Error", "code": "inference_start_failed", "error": REMOVED});
        let refused = turn_error(&frame, false);
        assert!(refused.is::<StartRefused>());
        let shown = with_session_context(Err(refused), "20260928_2").unwrap_err();
        assert_eq!(format!("{shown:#}"), REMOVED);
        assert_eq!(format!("{shown:?}"), REMOVED);

        let later = turn_error(&frame, true);
        assert!(!later.is::<StartRefused>());
        let failed = serde_json::json!({"type": "Error", "code": "provider_failure", "error": "Authentication failed."});
        for error in [later, turn_error(&failed, false)] {
            let shown = format!(
                "{:#}",
                with_session_context(Err(error), "20260928_2").unwrap_err()
            );
            assert!(
                shown.starts_with("Daemon session 20260928_2 remains available; no automatic resubmission was attempted: "),
                "{shown}"
            );
        }
        assert!(with_session_context(Ok(()), "s").is_ok());
    }

    /// AG-F7: text mode says what was created in words; the JSON formats keep the binding.
    #[test]
    fn the_binding_is_a_sentence_in_text_and_json_otherwise() {
        let session = serde_json::json!({"provider_name": "versa_azure",
            "model_config": {"model_name": "gpt-5.5"}, "privacy_tier": "private"});
        let binding = serde_json::json!({"type": "SessionBinding", "session_id": "20260927_1"});
        assert_eq!(
            binding_notice(Format::Text, "20260927_1", &session, &binding),
            "Chat 20260927_1 is ready (versa_azure/gpt-5.5)."
        );
        for format in [Format::Json, Format::StreamJson] {
            assert_eq!(
                binding_notice(format, "20260927_1", &session, &binding),
                binding.to_string()
            );
        }
        assert_eq!(
            binding_notice(Format::Text, "s", &serde_json::json!({}), &binding),
            "Chat s is ready."
        );
    }
    use std::path::PathBuf;

    fn options() -> SharedConversationOptions {
        SharedConversationOptions {
            session_id: Some("session-123".into()),
            working_dir: PathBuf::from("/synthetic/workdir"),
            provider: None,
            model: None,
            prompt: None,
            interactive: false,
            create_only: false,
            history: false,
            approval_key_stdin: false,
            no_start: true,
            quiet: false,
            output_format: "text".into(),
        }
    }

    #[cfg(unix)]
    mod continuation_tests {
        use super::super::{Conversation, Format, TextRenderer, TurnEnd};
        use crate::daemon_client::CrewClient;
        use biorouter::daemon_runtime::{self, Descriptor, Endpoint};
        use bytes::Bytes;
        use http_body_util::{BodyExt, Full, StreamBody};
        use hyper::server::conn::http1 as server_http1;
        use hyper::{
            body::{Frame, Incoming},
            service::service_fn,
            Request, Response, StatusCode,
        };
        use hyper_util::rt::TokioIo;
        use serde_json::{json, Value};
        use serial_test::serial;
        use std::fs::OpenOptions;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};
        use zeroize::Zeroizing;

        fn conversation(client: CrewClient, interactive: bool) -> Conversation {
            Conversation {
                client,
                session_id: "synthetic-session".into(),
                format: Format::Json,
                interactive,
                quiet: true,
                answered: Default::default(),
                closed_here: Default::default(),
                renderer: TextRenderer::default(),
                continuation_owner_id: "owner-a".into(),
                continuation_lease: None,
                binding: Default::default(),
                pending_ready: None,
                dictation_state: Default::default(),
                crew_step_test_input: None,
                approval_test_input: None,
                noticed: Default::default(),
            }
        }

        fn descriptor(socket_path: PathBuf) -> Descriptor {
            Descriptor {
                version: daemon_runtime::VERSION,
                profile_id: daemon_runtime::profile_identity()
                    .expect("synthetic test profile identity"),
                instance_id: "22222222-2222-4222-8222-222222222222".into(),
                pid: std::process::id(),
                endpoint: Endpoint::Unix { path: socket_path },
                api_secret: "synthetic-daemon-secret-0123456789012345".into(),
                user_action_installed: true,
            }
        }

        fn pending(ownership: &str, lease: Option<&str>) -> Value {
            let mut value = json!({
                "session": {"id": "synthetic-session"},
                "initializing": false,
                "initialization_error": null,
                "extension_results": [],
                "active_turn": null,
                "pending_continuation": {
                    "ownership": ownership,
                    "superseded_turn_id": "retired-turn"
                }
            });
            if let Some(lease) = lease {
                value["pending_continuation"]["continuation_lease"] = json!(lease);
            }
            value
        }

        struct DropCrewStepInput(Arc<std::sync::atomic::AtomicBool>);

        impl Drop for DropCrewStepInput {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        type CapturedRequestHeaders = (String, Option<String>, Option<String>);

        struct FakeDaemon {
            _directory: tempfile::TempDir,
            accept_task: Option<tokio::task::JoinHandle<()>>,
            requests: Arc<Mutex<Vec<(String, String, Value)>>>,
            request_notify: Arc<tokio::sync::Notify>,
            event_sender: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<Bytes>>>>,
            headers: Arc<Mutex<Vec<CapturedRequestHeaders>>>,
            descriptor: Descriptor,
        }

        impl FakeDaemon {
            async fn start(status: StatusCode, abandon_body: Value) -> Self {
                Self::start_with_resume(status, abandon_body, json!({})).await
            }

            async fn start_with_resume(
                status: StatusCode,
                abandon_body: Value,
                resume_body: Value,
            ) -> Self {
                let directory = tempfile::tempdir().expect("synthetic daemon directory");
                let runtime_directory = daemon_runtime::runtime_directory();
                daemon_runtime::private_directory(&runtime_directory)
                    .expect("synthetic daemon runtime directory");
                assert!(
                    daemon_runtime::read_descriptor().is_err(),
                    "fresh child must not have a live daemon descriptor"
                );
                let socket_path = runtime_directory.join("daemon.sock");
                let listener =
                    tokio::net::UnixListener::bind(&socket_path).expect("synthetic daemon socket");
                std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
                    .expect("synthetic daemon socket permissions");
                let requests = Arc::new(Mutex::new(Vec::new()));
                let request_notify = Arc::new(tokio::sync::Notify::new());
                let headers = Arc::new(Mutex::new(Vec::new()));
                let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
                let event_sender = Arc::new(Mutex::new(Some(event_tx)));
                let event_receiver = Arc::new(Mutex::new(Some(event_rx)));
                let synthetic_descriptor = descriptor(socket_path.clone());
                let captured = Arc::clone(&requests);
                let captured_request_notify = Arc::clone(&request_notify);
                let captured_headers = Arc::clone(&headers);
                let captured_event_receiver = Arc::clone(&event_receiver);
                let identity = synthetic_descriptor.identity();
                let accept_task = tokio::spawn(async move {
                    loop {
                        let Ok((stream, _)) = listener.accept().await else {
                            break;
                        };
                        let captured = Arc::clone(&captured);
                        let captured_request_notify = Arc::clone(&captured_request_notify);
                        let captured_headers = Arc::clone(&captured_headers);
                        let captured_event_receiver = Arc::clone(&captured_event_receiver);
                        let abandon_body = abandon_body.clone();
                        let resume_body = resume_body.clone();
                        let identity = identity.clone();
                        tokio::spawn(async move {
                            let service = service_fn(move |request: Request<Incoming>| {
                                let captured = Arc::clone(&captured);
                                let captured_request_notify = Arc::clone(&captured_request_notify);
                                let captured_headers = Arc::clone(&captured_headers);
                                let captured_event_receiver = Arc::clone(&captured_event_receiver);
                                let abandon_body = abandon_body.clone();
                                let resume_body = resume_body.clone();
                                let identity = identity.clone();
                                async move {
                                    let path = request.uri().path().to_owned();
                                    let method = request.method().to_string();
                                    let accept = request
                                        .headers()
                                        .get("accept")
                                        .and_then(|value| value.to_str().ok())
                                        .map(str::to_owned);
                                    let card_endings = request
                                        .headers()
                                        .get("x-card-endings")
                                        .and_then(|value| value.to_str().ok())
                                        .map(str::to_owned);
                                    captured_headers.lock().unwrap().push((
                                        path.clone(),
                                        accept.clone(),
                                        card_endings,
                                    ));
                                    let body = BodyExt::collect(request.into_body())
                                        .await
                                        .map(|body| {
                                            serde_json::from_slice(&body.to_bytes())
                                                .unwrap_or(Value::Null)
                                        })
                                        .unwrap_or(Value::Null);
                                    captured.lock().unwrap().push((method, path.clone(), body));
                                    captured_request_notify.notify_one();
                                    let is_event_stream = accept.as_deref()
                                        == Some("text/event-stream")
                                        && status.is_success();
                                    let (response_status, response_bytes, content_type) = if path
                                        == "/daemon/identity"
                                    {
                                        (
                                            StatusCode::OK,
                                            Bytes::from(
                                                serde_json::to_vec(&json!(identity)).unwrap(),
                                            ),
                                            "application/json",
                                        )
                                    } else if path == "/agent/resume" {
                                        (
                                            StatusCode::OK,
                                            Bytes::from(serde_json::to_vec(&resume_body).unwrap()),
                                            "application/json",
                                        )
                                    } else if is_event_stream {
                                        (StatusCode::OK, Bytes::new(), "text/event-stream")
                                    } else {
                                        (
                                            status,
                                            Bytes::from(serde_json::to_vec(&abandon_body).unwrap()),
                                            "application/json",
                                        )
                                    };
                                    let response_body = if is_event_stream {
                                        let receiver = captured_event_receiver
                                            .lock()
                                            .unwrap()
                                            .take()
                                            .expect("one synthetic event stream");
                                        let chunks = futures::stream::unfold(
                                            receiver,
                                            |mut receiver| async move {
                                                receiver.recv().await.map(|bytes| {
                                                    (
                                                        Ok::<_, std::convert::Infallible>(
                                                            Frame::data(bytes),
                                                        ),
                                                        receiver,
                                                    )
                                                })
                                            },
                                        );
                                        StreamBody::new(chunks).boxed_unsync()
                                    } else {
                                        Full::new(response_bytes).boxed_unsync()
                                    };
                                    Ok::<_, std::convert::Infallible>(
                                        Response::builder()
                                            .status(response_status)
                                            .header("content-type", content_type)
                                            .body(response_body)
                                            .unwrap(),
                                    )
                                }
                            });
                            let _ = server_http1::Builder::new()
                                .serve_connection(TokioIo::new(stream), service)
                                .await;
                        });
                    }
                });
                let mut descriptor_file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(daemon_runtime::descriptor_path())
                    .expect("synthetic daemon descriptor must be new");
                serde_json::to_writer(&mut descriptor_file, &synthetic_descriptor)
                    .expect("synthetic daemon descriptor serializes");
                descriptor_file
                    .sync_all()
                    .expect("synthetic daemon descriptor syncs");
                daemon_runtime::write_user_action_key(
                    &synthetic_descriptor,
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                )
                .expect("synthetic launcher saves its user-action key");
                Self {
                    _directory: directory,
                    accept_task: Some(accept_task),
                    requests,
                    request_notify,
                    event_sender,
                    headers,
                    descriptor: synthetic_descriptor,
                }
            }

            fn client(&self) -> CrewClient {
                CrewClient::for_test(
                    self.descriptor.clone(),
                    daemon_runtime::read_user_action_key(&self.descriptor)
                        .expect("synthetic key is private")
                        .expect("synthetic key matches the daemon")
                        .as_str(),
                )
            }

            fn requests(&self) -> Vec<(String, String, Value)> {
                self.requests.lock().unwrap().clone()
            }

            async fn wait_for_request(&self, path: &str) -> Value {
                loop {
                    let notified = self.request_notify.notified();
                    if let Some((_, _, body)) = self
                        .requests()
                        .into_iter()
                        .find(|(_, request_path, _)| request_path == path)
                    {
                        return body;
                    }
                    notified.await;
                }
            }

            fn send_event(&self, frame: &Value) {
                let bytes = Bytes::from(format!(
                    "data: {}\n\n",
                    serde_json::to_string(frame).expect("frame serializes")
                ));
                self.event_sender
                    .lock()
                    .unwrap()
                    .as_ref()
                    .expect("stream sender remains open")
                    .send(bytes)
                    .expect("synthetic event receiver is alive");
            }

            fn finish_events(&self) {
                self.event_sender.lock().unwrap().take();
            }

            fn headers(&self) -> Vec<(String, Option<String>, Option<String>)> {
                self.headers.lock().unwrap().clone()
            }
        }

        impl Drop for FakeDaemon {
            fn drop(&mut self) {
                if let Some(task) = self.accept_task.take() {
                    task.abort();
                }
            }
        }

        /// D1: a Crew step card in a chat with nobody at the terminal is the same refusal every
        /// card gets, and nothing is asked of the daemon. The client points at no daemon at all,
        /// so any request would fail the test differently.
        #[tokio::test]
        async fn a_noninteractive_chat_refuses_a_crew_step_like_any_card() {
            use biorouter::conversation::message::{
                ActionRequiredData, CrewSignInTarget, CrewStepRequest,
            };
            let directory = tempfile::tempdir().expect("a directory for no daemon");
            let client = CrewClient::for_test(
                Descriptor {
                    version: daemon_runtime::VERSION,
                    profile_id: "synthetic-profile".into(),
                    instance_id: "33333333-3333-4333-8333-333333333333".into(),
                    pid: std::process::id(),
                    endpoint: Endpoint::Unix {
                        path: directory.path().join("no-daemon.sock"),
                    },
                    api_secret: "synthetic-daemon-secret-0123456789012345".into(),
                    user_action_installed: true,
                },
                "synthetic-proof",
            );
            let mut conversation = conversation(client, false);
            let Err(error) = conversation
                .interact(
                    ActionRequiredData::CrewStep {
                        id: "card-1".into(),
                        step: CrewStepRequest::SignIn {
                            target: CrewSignInTarget::Setup {
                                job_id: "job-1".into(),
                            },
                            server: "hpc.example.edu".into(),
                            login: "you@hpc.example.edu".into(),
                        },
                    },
                    None,
                )
                .await
            else {
                panic!("nobody is at this terminal, so the card must be refused");
            };
            let text = error.to_string();
            assert!(
                text.starts_with(
                    "Human input is pending in session synthetic-session, request card-1."
                ),
                "{text}"
            );
            assert!(!conversation.answered.contains("card-1"));
        }

        #[tokio::test]
        #[serial]
        async fn noninteractive_pending_continuation_performs_no_mutation() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            let mut conversation = conversation(daemon.client(), false);
            let error = conversation
                .resolve_pending_continuation(pending("owned", Some("lease-a")))
                .await
                .expect_err("noninteractive pending state must refuse input");
            assert!(error.to_string().contains("Input was not submitted"));
            assert!(conversation.continuation_lease.is_none());
            assert!(
                daemon.requests().is_empty(),
                "refusal must not mutate the daemon"
            );
        }

        #[tokio::test]
        #[serial]
        async fn settling_pending_continuation_performs_no_recovery() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            let mut conversation = conversation(daemon.client(), true);
            let error = conversation
                .resolve_pending_continuation(pending("settling", None))
                .await
                .expect_err("settling state must wait for a later explicit recovery");
            assert!(error.to_string().contains("still settling"));
            assert!(daemon.requests().is_empty());
        }

        #[tokio::test]
        #[serial]
        async fn changed_pending_ownership_rejects_the_old_lease() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            let mut conversation = conversation(daemon.client(), false);
            conversation.continuation_lease = Some(Zeroizing::new("lease-a".into()));
            let error = conversation
                .verify_continuation(&pending("foreign", None))
                .expect_err("a foreign pending state cannot use the old lease");
            assert!(error.to_string().contains("ownership changed"));
            assert_eq!(
                conversation
                    .continuation_lease
                    .as_ref()
                    .map(|lease| lease.as_str()),
                Some("lease-a")
            );
            assert!(daemon.requests().is_empty());
        }

        #[tokio::test]
        #[serial]
        async fn known_unused_owned_lease_is_abandoned_once() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon =
                FakeDaemon::start(StatusCode::OK, json!({"resolution": "abandoned"})).await;
            let mut conversation = conversation(daemon.client(), false);
            conversation.continuation_lease = Some(Zeroizing::new("lease-a".into()));
            conversation
                .release_unused_continuation()
                .await
                .unwrap_or_else(|error| panic!("known unused lease cleanup: {error:#}"));
            assert!(conversation.continuation_lease.is_none());
            let requests = daemon.requests();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[1].1, "/agent/continuation/abandon");
            assert_eq!(requests[1].2["session_id"], "synthetic-session");
            assert_eq!(requests[1].2["continuation_lease"], "lease-a");
            conversation
                .release_unused_continuation()
                .await
                .expect("a consumed cleanup is a no-op");
            assert_eq!(daemon.requests().len(), 2, "cleanup must happen once");
        }

        #[tokio::test]
        #[serial]
        async fn failed_unused_lease_cleanup_preserves_lease_without_disclosure() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "lease-a"}),
            )
            .await;
            let mut conversation = conversation(daemon.client(), false);
            conversation.continuation_lease = Some(Zeroizing::new("lease-a".into()));
            let error = conversation
                .release_unused_continuation()
                .await
                .expect_err("cleanup failure must remain visible");
            assert!(error.to_string().contains("cleanup was not confirmed"));
            assert!(!error.to_string().contains("lease-a"));
            assert_eq!(
                conversation
                    .continuation_lease
                    .as_ref()
                    .map(|lease| lease.as_str()),
                Some("lease-a")
            );
        }

        #[tokio::test]
        #[serial]
        async fn uncertain_reply_admission_consumes_lease_without_auto_cleanup_or_retry() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start_with_resume(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error": "synthetic admission unavailable"}),
                pending("owned", Some("lease-a")),
            )
            .await;
            let mut conversation = conversation(daemon.client(), true);
            conversation.continuation_lease = Some(Zeroizing::new("lease-a".into()));
            let admission = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                conversation.submit(
                    biorouter::conversation::message::Message::user().with_text("successor"),
                ),
            )
            .await
            .expect("the synthetic admission response completes promptly");
            let error = match admission {
                Ok(_) => panic!("the synthetic admission must be uncertain"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains("outcome is unknown"),
                "unexpected uncertain admission error: {error:#}"
            );
            assert!(conversation.continuation_lease.is_none());
            conversation
                .release_unused_continuation()
                .await
                .expect("uncertain admission must not trigger cleanup");

            let requests = daemon.requests();
            let paths: Vec<&str> = requests.iter().map(|(_, path, _)| path.as_str()).collect();
            assert_eq!(
                paths,
                vec![
                    "/daemon/identity",
                    "/agent/resume",
                    "/daemon/identity",
                    "/reply"
                ]
            );
            assert_eq!(requests[3].2["continuation_lease"], "lease-a");
            assert!(!paths.contains(&"/agent/continuation/abandon"));
        }

        /// An extension the daemon could not start is a warning, as in an in-process chat, and
        /// the chat goes on: it used to end the whole chat.
        #[tokio::test]
        #[serial]
        async fn a_failed_extension_is_a_warning_and_the_chat_goes_on() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let mut state = pending("owned", None);
            state["pending_continuation"] = Value::Null;
            state["extension_results"] = json!([
                {"name": "lab-mcp", "success": false, "error": "spawn failed"},
                {"name": "developer", "success": true},
            ]);
            let daemon = FakeDaemon::start_with_resume(StatusCode::OK, json!({}), state).await;
            let conversation = conversation(daemon.client(), false);
            let resumed = conversation
                .resume(true)
                .await
                .unwrap_or_else(|error| panic!("a failed extension ended the chat: {error:#}"));
            assert_eq!(resumed["session"]["id"], "synthetic-session");
        }

        /// Ruling 28: the daemon's agent-loading resume answers with the row and
        /// no transcript, so `--history` reads it from the resume that loads
        /// nothing. A state that already carries one (an older daemon) costs no
        /// second request.
        #[tokio::test]
        #[serial]
        async fn history_reads_the_transcript_the_agent_load_no_longer_sends() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let transcript = json!([{"role": "user", "content": []}]);
            let mut painted = pending("owned", None);
            painted["pending_continuation"] = Value::Null;
            painted["session"]["conversation"] = transcript.clone();
            let daemon =
                FakeDaemon::start_with_resume(StatusCode::OK, json!({}), painted.clone()).await;
            let conversation = conversation(daemon.client(), false);

            let mut loaded = painted.clone();
            loaded["session"]["conversation"] = Value::Null;
            let session = conversation
                .history_session(&loaded)
                .await
                .unwrap_or_else(|error| panic!("history after an agent load: {error:#}"));
            assert_eq!(session["conversation"], transcript);
            let requests = daemon.requests();
            let resumes: Vec<&Value> = requests
                .iter()
                .filter(|(_, path, _)| path == "/agent/resume")
                .map(|(_, _, body)| body)
                .collect();
            assert_eq!(resumes.len(), 1, "{requests:?}");
            assert_eq!(resumes[0]["load_model_and_extensions"], false);

            let before = daemon.requests().len();
            let session = conversation
                .history_session(&painted)
                .await
                .expect("history from a state that carries the transcript");
            assert_eq!(session["conversation"], transcript);
            assert_eq!(
                daemon.requests().len(),
                before,
                "a state with its transcript must not be read again"
            );
        }

        #[tokio::test]
        #[serial]
        async fn attached_read_turn_observes_the_matching_crew_step_end() {
            use biorouter::conversation::message::{CrewStepRequest, Message, MessageContent};
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::time::Duration;

            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            let mut conversation = conversation(daemon.client(), true);
            let dropped = Arc::new(AtomicBool::new(false));
            let input_dropped = Arc::clone(&dropped);
            let (input_started_tx, input_started_rx) = tokio::sync::oneshot::channel();
            let drop_probe = DropCrewStepInput(input_dropped);
            conversation.crew_step_test_input = Some(Box::pin(async move {
                let _drop = drop_probe;
                let _ = input_started_tx.send(());
                std::future::pending::<anyhow::Result<()>>().await
            }));
            let mut task = tokio::spawn(async move {
                let result = conversation.attach("turn-1").await;
                (conversation, result)
            });

            let message =
                Message::assistant().with_content(MessageContent::action_required_crew_step(
                    "card-1",
                    CrewStepRequest::HostKey {
                        host: "host.invalid".into(),
                        port: 22,
                        login: "test-user".into(),
                        offered: None,
                        known: None,
                        changed: false,
                    },
                ));
            daemon.send_event(&json!({
                "type":"Message", "turn_id":"turn-1", "seq":1, "message":message
            }));
            tokio::time::timeout(Duration::from_secs(2), input_started_rx)
                .await
                .expect("Crew step input starts before its end is published")
                .expect("input-start signal arrives");
            daemon.send_event(&json!({
                "type":"ActionRequiredEnded", "turn_id":"turn-1", "id":"card-1",
                "ending":"dismissed", "seq":2
            }));
            daemon.send_event(&json!({
                "type":"Message", "turn_id":"turn-1", "seq":3,
                "message":Message::assistant().with_text("after card")
            }));
            daemon.send_event(&json!({
                "type":"Finish", "turn_id":"turn-1", "seq":4, "reason":"completed"
            }));
            daemon.finish_events();

            let completed = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
            if completed.is_err() {
                task.abort();
                let _ = task.await;
            }
            assert!(
                completed.is_ok(),
                "the matching ActionRequiredEnded frame must end the waiting Crew step"
            );
            let (conversation, result) = completed.unwrap().expect("attach task joins");
            assert!(matches!(&result, Ok(TurnEnd::Finished)));
            assert!(
                dropped.load(Ordering::SeqCst),
                "the started input future is dropped"
            );
            assert!(conversation.answered.contains("card-1"));

            let requests = daemon.requests();
            let paths: Vec<&str> = requests.iter().map(|(_, path, _)| path.as_str()).collect();
            assert_eq!(paths.iter().filter(|path| **path == "/reply").count(), 1);
            assert!(
                !paths.iter().any(|path| path.starts_with("/action-required/") || path.starts_with("/crew/")),
                "ending the card cannot approve it or submit a Crew step: {paths:?}"
            );
            let reply = requests
                .iter()
                .find(|(_, path, _)| path == "/reply")
                .expect("attach sends one reply request");
            assert_eq!(reply.2["turn_id"], "turn-1");
            assert_eq!(reply.2["from_seq"], 0);
            assert_eq!(reply.2["user_message"]["metadata"]["userVisible"], false);
            assert_eq!(reply.2["user_message"]["content"], json!([]));
            let header = daemon
                .headers()
                .into_iter()
                .find(|(path, _, _)| path == "/reply")
                .expect("reply request headers captured");
            assert_eq!(header.1.as_deref(), Some("text/event-stream"));
            assert_eq!(header.2.as_deref(), Some("1"));
        }

        /// A parked approval's expiry reaches the turn first as its end frame (`timed_out`),
        /// then as the notice naming it. The prompt stops at the end frame, answers nothing,
        /// and notes the card as closed here, so the loop prints neither of the daemon's words
        /// for that end after the prompt's own line.
        #[tokio::test]
        #[serial]
        async fn an_approval_prompt_stops_at_its_cards_end_frame() {
            use biorouter::conversation::message::{Message, MessageContent};
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::time::Duration;

            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            let mut conversation = conversation(daemon.client(), true);
            let dropped = Arc::new(AtomicBool::new(false));
            let (asked_tx, asked_rx) = tokio::sync::oneshot::channel();
            let drop_probe = DropCrewStepInput(Arc::clone(&dropped));
            conversation.approval_test_input = Some(Box::pin(async move {
                let _drop = drop_probe;
                let _ = asked_tx.send(());
                std::future::pending::<anyhow::Result<Option<Zeroizing<String>>>>().await
            }));
            let mut task = tokio::spawn(async move {
                let result = conversation.attach("turn-1").await;
                (conversation, result)
            });

            let card =
                Message::assistant().with_content(MessageContent::action_required_with_context(
                    "card-1",
                    "filesystem__folder_access".into(),
                    serde_json::Map::new(),
                    Some("Allow access to /synthetic/results?".into()),
                    None,
                    None,
                ));
            daemon.send_event(&json!({
                "type":"Message", "turn_id":"turn-1", "seq":1, "message":card
            }));
            tokio::time::timeout(Duration::from_secs(2), asked_rx)
                .await
                .expect("the approval prompt starts before its card ends")
                .expect("prompt-start signal arrives");
            daemon.send_event(&json!({
                "type":"ActionRequiredEnded", "turn_id":"turn-1", "id":"card-1",
                "ending":"timed_out", "seq":2
            }));
            let stopped = tokio::time::timeout(Duration::from_secs(2), async {
                while !dropped.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            if stopped.is_err() {
                task.abort();
                let _ = task.await;
                panic!("the card's timed_out end frame must stop its prompt");
            }

            let notice = biorouter::pending_user_action::card_expiry_notice(
                "card-1",
                "The folder access request for /synthetic/results",
            );
            daemon.send_event(&json!({
                "type":"Message", "turn_id":"turn-1", "seq":3, "message":notice
            }));
            daemon.send_event(&json!({
                "type":"Finish", "turn_id":"turn-1", "seq":4, "reason":"completed"
            }));
            daemon.finish_events();

            let completed = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
            if completed.is_err() {
                task.abort();
                let _ = task.await;
            }
            let (conversation, result) = completed
                .expect("the turn finishes after its expired card")
                .expect("attach task joins");
            assert!(
                matches!(&result, Ok(TurnEnd::Finished)),
                "{:?}",
                result.as_ref().err()
            );
            assert!(conversation.answered.contains("card-1"));
            // One line for the expiry: the prompt's own. The end frame and the notice the
            // loop read after it print nothing more.
            let noticed = conversation.noticed.lock().unwrap().clone();
            assert_eq!(
                noticed
                    .iter()
                    .filter(|line| line.as_str() == super::super::APPROVAL_EXPIRED)
                    .count(),
                1,
                "{noticed:?}"
            );
            assert!(
                !noticed.iter().any(|line| line.starts_with("[closed]")),
                "{noticed:?}"
            );
            let end = json!({"type":"ActionRequiredEnded", "id":"card-1", "ending":"timed_out"});
            assert_eq!(
                conversation.closed_here.ended_line(&end),
                None,
                "the prompt said the expiry, so its end frame prints nothing more"
            );
            assert!(conversation.closed_here.said(&notice));
            let paths: Vec<String> = daemon
                .requests()
                .into_iter()
                .map(|(_, path, _)| path)
                .collect();
            assert!(
                !paths
                    .iter()
                    .any(|path| path.starts_with("/action-required/")),
                "an expired card is never answered: {paths:?}"
            );
        }

        #[tokio::test]
        #[serial]
        async fn ordinary_json_requests_do_not_opt_into_card_endings() {
            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            daemon
                .client()
                .request("GET", "/snapshot", None)
                .await
                .expect("ordinary JSON request succeeds");
            let header = daemon
                .headers()
                .into_iter()
                .find(|(path, _, _)| path == "/snapshot")
                .expect("snapshot request headers captured");
            assert_ne!(header.1.as_deref(), Some("text/event-stream"));
            assert_eq!(header.2, None);
        }

        #[tokio::test]
        #[serial]
        #[ignore = "requires the bounded private synthetic PTY runner"]
        async fn a_cancelled_vault_prompt_cannot_feed_the_next_card() {
            use biorouter::conversation::message::{
                CrewStepRequest, CrewVaultAction, Message, MessageContent,
            };
            use std::time::Duration;

            assert_eq!(
                std::env::var_os("BIOROUTER_B01_PTY").as_deref(),
                Some(std::ffi::OsStr::new("1")),
                "this ignored test requires the private PTY runner"
            );
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
            let client = daemon.client();
            let mut turn = tokio::spawn(async move {
                let mut conversation = conversation(client, true);
                super::super::HIDDEN_INPUT_READY
                    .scope((), async move {
                        super::super::HIDDEN_INPUT_ACCEPTED
                            .scope(std::cell::RefCell::new(Some(accepted_tx)), async move {
                                conversation.attach("turn-pty").await
                            })
                            .await
                    })
                    .await
            });

            let vault_card = |id: &str| {
                Message::assistant().with_content(MessageContent::action_required_crew_step(
                    id,
                    CrewStepRequest::Vault {
                        action: CrewVaultAction::Unlock,
                    },
                ))
            };
            daemon.send_event(&json!({
                "type":"Message", "turn_id":"turn-pty", "seq":1,
                "message":vault_card("vault-card-1")
            }));
            tokio::time::timeout(Duration::from_secs(20), accepted_rx)
                .await
                .expect("PTY runner supplied the public canary without Enter")
                .expect("hidden-input acceptance signal arrives");

            daemon.send_event(&json!({
                "type":"ActionRequiredEnded", "turn_id":"turn-pty",
                "id":"vault-card-1", "ending":"dismissed", "seq":2
            }));
            daemon.send_event(&json!({
                "type":"Message", "turn_id":"turn-pty", "seq":3,
                "message":vault_card("vault-card-2")
            }));

            let body = tokio::time::timeout(
                Duration::from_secs(30),
                async {
                    tokio::select! {
                        body = daemon.wait_for_request("/crew/credentials/unlock") => body,
                        outcome = &mut turn => match outcome {
                            Err(error) => panic!("the synthetic turn task failed before fresh input: {error}"),
                            Ok(Err(error)) => panic!("the synthetic turn failed before fresh input: {error:#}"),
                            Ok(Ok(_)) => panic!("the synthetic turn finished before fresh input"),
                        },
                    }
                },
            )
            .await
            .expect("the second PTY prompt submitted its fresh synthetic input");
            assert!(
                body["passphrase"] == "CANARY-7f3a",
                "fresh input was forwarded exactly"
            );
            let credential_posts = daemon
                .requests()
                .into_iter()
                .filter(|(_, path, _)| path == "/crew/credentials/unlock")
                .count();
            assert_eq!(credential_posts, 1, "cancelled card was never posted");
            assert!(
                !daemon.requests().iter().any(|(_, path, value)| {
                    path != "/crew/credentials/unlock" && value.to_string().contains("CANARY-7f3a")
                }),
                "the canary is absent from non-credential requests"
            );
            daemon.send_event(&json!({
                "type":"Finish", "turn_id":"turn-pty", "reason":"stop", "seq":4
            }));
            let result = tokio::time::timeout(Duration::from_secs(10), turn)
                .await
                .expect("synthetic turn finishes")
                .expect("turn task joins");
            assert!(result.is_ok(), "synthetic turn returns successfully");
        }

        #[tokio::test]
        #[serial]
        async fn observer_ignores_foreign_and_replayed_card_endings() {
            use std::collections::VecDeque;

            if !crate::test_sandbox::in_a_process_of_its_own() {
                return;
            }
            let daemon = FakeDaemon::start(StatusCode::OK, json!({})).await;
            daemon.send_event(&json!({
                "type":"ActionRequiredEnded", "turn_id":"turn-1", "id":"card-other",
                "ending":"dismissed", "seq":2
            }));
            daemon.send_event(&json!({
                "type":"ActionRequiredEnded", "turn_id":"turn-1", "id":"card-1",
                "ending":"dismissed", "seq":1
            }));
            daemon.send_event(&json!({
                "type":"ActionRequiredEnded", "turn_id":"turn-1", "id":"card-1",
                "ending":"dismissed", "seq":3
            }));
            daemon.finish_events();

            let mut stream = daemon
                .client()
                .event_stream("POST", "/reply", None)
                .await
                .expect("the synthetic SSE stream opens");
            let mut deferred = VecDeque::new();
            let outcome = super::super::observe_crew_step(
                std::future::pending::<anyhow::Result<super::super::Interaction>>(),
                &mut stream,
                &mut deferred,
                "card-1",
                "turn-1",
                Some(1),
                false,
            )
            .await
            .expect("matching end is observed");
            assert!(matches!(outcome, super::super::ObservedCrewStep::Ended));
            let deferred_ends: Vec<(String, u64)> = deferred
                .iter()
                .map(|frame| {
                    (
                        frame["id"].as_str().expect("ended id").to_owned(),
                        frame["seq"].as_u64().expect("event sequence"),
                    )
                })
                .collect();
            assert_eq!(
                deferred_ends,
                vec![("card-other".into(), 2), ("card-1".into(), 3)],
                "a foreign end stays deferred and the stale matching replay is ignored"
            );
        }
    }

    #[test]
    fn exact_session_ids_and_output_formats_are_validated_without_local_lookup() {
        let mut resumed = options();
        assert!(matches!(
            validate_options(&resumed).expect("resume options validate"),
            Format::Text
        ));
        resumed.output_format = "json".into();
        assert!(matches!(
            validate_options(&resumed).expect("json output validates"),
            Format::Json
        ));
        resumed.output_format = "stream-json".into();
        assert!(matches!(
            validate_options(&resumed).expect("stream JSON output validates"),
            Format::StreamJson
        ));

        resumed.output_format = "yaml".into();
        let error = validate_options(&resumed).expect_err("unsupported output must refuse");
        assert!(error.to_string().contains("text, json, or stream-json"));
    }

    #[test]
    fn new_conversations_require_provider_and_model_together() {
        let mut new = options();
        new.session_id = None;
        let error = validate_options(&new).expect_err("new conversation needs a provider");
        assert!(error.to_string().contains("explicit provider and model"));

        new.provider = Some("synthetic-provider".into());
        let error = validate_options(&new).expect_err("provider without model must refuse");
        assert!(error.to_string().contains("provider and model together"));

        new.model = Some("synthetic-model".into());
        assert!(validate_options(&new).is_ok());
    }

    #[test]
    fn create_only_and_stdin_modes_cannot_consume_conversation_input() {
        let mut create = options();
        create.create_only = true;
        create.prompt = Some("must not submit".into());
        let error = validate_options(&create).expect_err("create-only prompt must refuse");
        assert!(error.to_string().contains("Create-only"));

        let mut interactive = options();
        interactive.interactive = true;
        interactive.approval_key_stdin = true;
        let error =
            validate_options(&interactive).expect_err("compatibility stdin cannot share input");
        assert!(error.to_string().contains("cannot share interactive"));
    }

    #[test]
    fn ids_and_prompts_are_bounded_and_reject_controls() {
        assert!(validate_id("session-123:turn_1.v2").is_ok());
        assert!(validate_id("session/with/slash").is_err());
        assert!(validate_id(&"x".repeat(129)).is_err());
        assert!(validate_prompt("ordinary prompt").is_ok());
        assert!(validate_prompt(" \t\n").is_err());
        assert!(validate_prompt(&"x".repeat(32_769)).is_err());
    }

    #[test]
    fn json_terminal_safe_round_trips_invisible_non_bmp_tags_and_unicode() {
        let value = serde_json::json!({
            "text": "emoji🙂 café\u{202e}bidi\u{200b}zero\u{feff}bom\u{e0041}tag\n",
            "plain": "東京"
        });
        let encoded = serde_json::to_string(&value).unwrap();
        let safe = json_terminal_safe(&encoded);
        let reparsed: serde_json::Value = serde_json::from_str(&safe).unwrap();
        assert_eq!(reparsed, value);
        assert!(safe.contains("emoji🙂 café"));
        assert!(safe.contains("東京"));
        assert!(safe.chars().all(|ch| !terminal_control(ch)));
        for ch in ['\u{202e}', '\u{200b}', '\u{feff}', '\u{e0041}'] {
            assert!(
                !safe.contains(ch),
                "raw invisible character {ch:?} survived"
            );
        }
    }

    #[test]
    fn terminal_text_escapes_controls_and_sensitive_schemas_are_rejected() {
        assert_eq!(safe_text("ok\u{202e}secret\n"), "ok\\u{202e}secret\\n");
        assert!(sensitive_schema(
            &serde_json::json!({"properties":{"password":{"type":"string"}}})
        ));
        assert!(sensitive_schema(&serde_json::json!({"writeOnly":true})));
        assert!(sensitive_schema(&serde_json::json!({"format":"secret"})));
        assert!(!sensitive_schema(
            &serde_json::json!({"properties":{"cohort":{"type":"string"}}})
        ));
    }

    // The `/dictate` lines a shared conversation's prompt answers itself, as the four
    // string conditions inside `terminal_input` read them before they became one parser.
    #[cfg(feature = "dictation")]
    #[test]
    fn the_prompt_reads_the_dictate_lines_it_always_did() {
        use super::DictateLine;
        let parse = |line: &str| DictateLine::parse(line);
        assert!(matches!(parse("/dictate"), Some(DictateLine::Capture)));
        assert!(matches!(parse("  /dictate  "), Some(DictateLine::Capture)));
        assert!(matches!(parse("/dictate retry"), Some(DictateLine::Retry)));
        assert!(matches!(
            parse("/dictate status"),
            Some(DictateLine::Status)
        ));
        assert!(matches!(
            parse("/dictate install"),
            Some(DictateLine::Install(None))
        ));
        assert!(matches!(
            parse("/dictate install  small.en "),
            Some(DictateLine::Install(Some(model))) if model == "small.en"
        ));
        for other in [
            "/dictateX",
            "/dictate installer",
            "/dictate stat",
            "dictate",
            "hello",
        ] {
            assert!(parse(other).is_none(), "{other}");
        }
    }

    // The `/dictionary` lines an in-process chat answers (`session/input.rs`), answered by a
    // chat on the shared daemon too, so none reaches the model as a message (Phase C F-4:
    // `/dictionary scan` was sent to the agent, which read files with its shell instead).
    #[cfg(feature = "dictation")]
    #[test]
    fn the_prompt_answers_the_dictionary_lines_an_in_process_chat_does() {
        use super::DictateLine;
        let parse = |line: &str| DictateLine::parse(line);
        assert!(matches!(
            parse("/dictionary scan"),
            Some(DictateLine::DictionaryScan)
        ));
        assert!(matches!(
            parse(" /dictionary review "),
            Some(DictateLine::DictionaryReview)
        ));
        assert!(matches!(
            parse("/dictionary add  EPAS1 "),
            Some(DictateLine::DictionaryAdd(term)) if term == "EPAS1"
        ));
        for other in [
            "/dictionary",
            "/dictionary add",
            "/dictionary scanner",
            "/dictionary reviews",
            "/dictionaryscan",
        ] {
            assert!(parse(other).is_none(), "{other}");
        }
    }

    // Where a transcript goes in the line being typed: after a space when it follows text
    // (the chord), never past the prompt's byte limit, and an error said instead of typed.
    #[cfg(feature = "dictation")]
    #[test]
    fn a_dictated_transcript_joins_the_line_within_its_limit() {
        use super::insert_dictated;
        use zeroize::Zeroizing;
        let mut value = Zeroizing::new("check".to_string());
        insert_dictated(&mut value, Ok(Some(" EPAS1 levels ".into())), 64, true);
        assert_eq!(value.as_str(), "check EPAS1 levels");
        let mut value = Zeroizing::new("check ".to_string());
        insert_dictated(&mut value, Ok(Some("EPAS1".into())), 64, true);
        assert_eq!(value.as_str(), "check EPAS1");
        let mut value = Zeroizing::new(String::new());
        insert_dictated(&mut value, Ok(Some("EPAS1".into())), 64, false);
        assert_eq!(value.as_str(), "EPAS1");
        let mut value = Zeroizing::new("abc".to_string());
        insert_dictated(&mut value, Ok(Some("defg".into())), 7, true);
        assert_eq!(
            value.as_str(),
            "abc",
            "a transcript past the limit is not inserted"
        );
        insert_dictated(&mut value, Err("no model".into()), 64, true);
        insert_dictated(&mut value, Ok(None), 64, true);
        assert_eq!(value.as_str(), "abc");
    }

    #[test]
    fn a_prompt_seed_stays_within_its_limit() {
        use super::seeded_value;
        assert_eq!(
            seeded_value(Some("draft"), false, 8).unwrap().as_str(),
            "draft"
        );
        assert_eq!(seeded_value(None, true, 8).unwrap().as_str(), "");
        assert!(seeded_value(Some("too long"), false, 4).is_err());
    }

    /// WI-7: the read consent lasts until the person revokes it, so this terminal never
    /// offers it as "once": it asks for allow or deny and names the command that takes it
    /// back. Every other approval keeps its words.
    #[test]
    fn the_read_consent_is_asked_as_lasting_until_revoked() {
        let consent = approval_question(true, false, "20261106_4");
        assert_eq!(
            consent,
            "Type allow (until you revoke it with biorouter crew grants revoke 20261106_4 \
             --reads) or deny (Esc cancels the turn): "
        );
        assert!(!consent.contains("once"), "{consent}");
        assert!(!consent.contains(['\u{2014}', '\u{2013}']), "{consent}");
        assert_eq!(
            approval_question(false, false, "20261106_4"),
            "Type allow_once or deny (Esc cancels the turn): "
        );
        assert_eq!(
            approval_question(false, true, "20261106_4"),
            "Type allow_once, allow_session, or deny (Esc cancels the turn): "
        );
        assert_eq!(cards::approval_answer("allow", true), Some("allow_once"));
        assert_eq!(cards::approval_answer("allow_once", true), None);
        assert_eq!(
            cards::read_consent_allowed("20261106_4"),
            "Allowed. This chat may read the workspace as you until you revoke it with biorouter \
             crew grants revoke 20261106_4 --reads."
        );
    }
}
