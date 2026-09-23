//! Terminal presentation of the profile daemon's personal conversation services.
use crate::daemon_client::{CrewClient, DaemonEventStream, DaemonRefusal};
use anyhow::{bail, ensure, Context, Result};
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers,
};
use futures::StreamExt;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use zeroize::Zeroizing;

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
    renderer: TextRenderer,
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
        renderer: TextRenderer::default(),
    };
    conversation.event(&json!({"type":"Session","session_id":conversation.session_id}))?;
    let result = conversation.execute(&options).await;
    conversation.renderer.finish()?;
    if let Err(error) = &result {
        if format != Format::Text {
            emit_json(&json!({"type":"Error","session_id":conversation.session_id,
                "error":format!("{error:#}"),"resubmit_automatically":false}))?;
        }
    }
    result.with_context(|| {
        format!(
            "Daemon session {} remains available; no automatic resubmission was attempted",
            safe_text(&conversation.session_id)
        )
    })
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
        "Approval-key stdin cannot share interactive conversation input"
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
                    "session_id":self.session_id,"load_model_and_extensions":initialize
                })),
            )
            .await?;
        ensure!(
            state["session"]["id"].as_str() == Some(&self.session_id),
            "Daemon returned a different session"
        );
        ensure!(state["pending_continuation"].is_null(), "This session has a pending Stop-and-Send continuation; resolve it in the desktop before using this terminal adapter");
        ensure!(state["initializing"].as_bool() == Some(false), "The session is still initializing; resume the exact session after initialization finishes");
        ensure!(
            state["initialization_error"].is_null(),
            "The daemon could not initialize this session: {}",
            safe_text(&state["initialization_error"].to_string())
        );
        if let Some(results) = state["extension_results"].as_array() {
            for result in results {
                ensure!(
                    result["success"].as_bool() != Some(false),
                    "A session extension failed to initialize: {}",
                    safe_text(&result.to_string())
                );
            }
        }
        Ok(state)
    }

    async fn execute(&mut self, options: &SharedConversationOptions) -> Result<()> {
        let mut state = self.resume(true).await?;
        let active = state["active_turn"]["turn_id"].as_str().map(str::to_owned);
        ensure!(active.is_none() || options.prompt.is_none(), "A turn is already running. Your prompt was not submitted; reattach to this exact session without input, then submit after it finishes");
        ensure!(
            active.is_none() || options.provider.is_none(),
            "A turn is already running. Reattach without provider/model changes"
        );
        if options.session_id.is_some() {
            if let (Some(provider), Some(model)) = (&options.provider, &options.model) {
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
        self.show_binding(&state["session"])?;
        if options.history {
            self.show_history(&state["session"])?;
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
                    let Some(text) =
                        terminal_input("You (/quit to detach): ", false, 32_768).await?
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
        let turn_id = uuid::Uuid::new_v4().to_string();
        self.event(
            &json!({"type":"TurnSubmitted","session_id":self.session_id,"turn_id":turn_id}),
        )?;
        let stream = self.client.event_stream("POST", "/reply", Some(json!({
            "session_id":self.session_id,"turn_id":turn_id,"user_message":message
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
        loop {
            let frame = tokio::select! {
                frame = stream.next_event() => frame?,
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
            match frame["type"].as_str().context("Daemon event has no type")? {
                "Message" => match self.handle_message(&frame["message"]).await? {
                    Interaction::Continue => {}
                    Interaction::Stop => return self.cancel(&active_id).await,
                },
                "Error" => bail!("{}: {}", safe_text(frame["code"].as_str().unwrap_or("daemon_error")), safe_text(frame["error"].as_str().unwrap_or("Daemon turn failed"))),
                "Finish" => {
                    self.renderer.finish()?;
                    return match frame["reason"].as_str() {
                        Some("cancelled" | "canceled" | "stopped" | "orphaned" | "error" | "failed") => bail!("Daemon turn ended: {}", safe_text(&frame["reason"].to_string())),
                        _ => Ok(TurnEnd::Finished),
                    };
                }
                "UpdateConversation" => self.notice("The daemon resynchronized the conversation; use history to read the authoritative transcript.")?,
                "PrivacyProviderPinned" | "ModelChange" => self.notice(&frame.to_string())?,
                "Ping" | "TurnStarted" | "TurnState" | "Notification" | "ToolCallPending"
                | "ToolCallsRetracted" | "SteerWaiting" | "MessagesPersisted" => {}
                kind => bail!("Unsupported daemon event {}; reattach in the desktop", safe_text(kind)),
            }
        }
    }

    async fn handle_message(&mut self, value: &Value) -> Result<Interaction> {
        let message: Message =
            serde_json::from_value(value.clone()).context("Unsupported daemon message shape")?;
        if self.format == Format::Text {
            self.renderer.message(&message, self.quiet)?;
        }
        for content in message.content {
            match content {
                MessageContent::ActionRequired(action) => {
                    self.renderer.finish()?;
                    let outcome = self.interact(action.data).await?;
                    if !matches!(outcome, Interaction::Continue) { return Ok(outcome); }
                }
                MessageContent::FrontendToolRequest(request) => bail!("Frontend tool request {} requires the desktop; this terminal did not execute it", safe_text(&request.id)),
                _ => {}
            }
        }
        Ok(Interaction::Continue)
    }

    async fn interact(&mut self, action: ActionRequiredData) -> Result<Interaction> {
        let id = match &action {
            ActionRequiredData::ToolConfirmation { id, .. }
            | ActionRequiredData::Elicitation { id, .. }
            | ActionRequiredData::SecretRequest { id, .. } => id.clone(),
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
            ActionRequiredData::ToolConfirmation { .. } => self.confirm_tool(&action, &id).await?,
            ActionRequiredData::SecretRequest { .. } => self.collect_secrets(action, &id).await?,
            ActionRequiredData::Elicitation {
                message,
                requested_schema,
                ..
            } => self.elicitation(&id, &message, &requested_schema).await?,
            ActionRequiredData::ElicitationResponse { .. } => unreachable!(),
        };
        self.answered.insert(id);
        Ok(outcome)
    }

    async fn confirm_tool(&self, action: &ActionRequiredData, id: &str) -> Result<Interaction> {
        self.notice(&serde_json::to_string_pretty(action)?)?;
        loop {
            let Some(answer) = terminal_input(
                "Type allow_once or deny (Esc cancels the turn): ",
                false,
                32,
            )
            .await?
            else {
                return Ok(Interaction::Stop);
            };
            if !matches!(answer.trim(), "allow_once" | "deny") {
                continue;
            }
            let result = self.client.request("POST", "/action-required/tool-confirmation", Some(json!({
                "sessionId":self.session_id,"id":id,"principalType":"Tool","action":answer.trim()
            }))).await?;
            ensure!(
                matches!(
                    result["status"].as_str(),
                    Some("delivered" | "unknown" | "already_resolved")
                ),
                "Unexpected tool approval response"
            );
            if matches!(
                result["status"].as_str(),
                Some("unknown" | "already_resolved")
            ) {
                self.notice("This request is no longer pending; no new tool call was approved.")?;
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
            let Some(secret) = terminal_input(&label, true, 4096).await? else {
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
        let answer =
            terminal_input("JSON object (Esc cancels this request): ", false, 16_384).await?;
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
        Ok(())
    }
    fn show_binding(&self, session: &Value) -> Result<()> {
        let binding = json!({"type":"SessionBinding","session_id":self.session_id,
            "provider":session["provider_name"],"model":session["model_config"],
            "privacy_tier":session["privacy_tier"]});
        self.event(&binding)?;
        if !self.quiet {
            self.notice(&binding.to_string())?;
        }
        Ok(())
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
    ch.is_control()
        || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}
fn safe_text(value: &str) -> String {
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
fn emit_json(value: &Value) -> Result<()> {
    let encoded = serde_json::to_string(value)?;
    let escaped: String = encoded
        .chars()
        .flat_map(|ch| {
            if terminal_control(ch) {
                format!("\\u{:04x}", ch as u32).chars().collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect();
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{escaped}")?;
    stdout.flush()?;
    Ok(())
}
struct TerminalMode;
impl Drop for TerminalMode {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stderr(), DisableBracketedPaste);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}
async fn terminal_input(
    prompt: &str,
    hidden: bool,
    limit: usize,
) -> Result<Option<Zeroizing<String>>> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "Human input requires an interactive terminal"
    );
    eprint!("{}", safe_text(prompt));
    std::io::stderr().flush()?;
    crossterm::terminal::enable_raw_mode()?;
    let _mode = TerminalMode;
    crossterm::execute!(std::io::stderr(), EnableBracketedPaste)?;
    let mut events = EventStream::new();
    let mut value = Zeroizing::new(String::new());
    while let Some(event) = events.next().await {
        let event = event?;
        if let Event::Paste(paste) = event {
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
            if !hidden {
                eprint!("{}", safe_text(&paste));
            }
            std::io::stderr().flush()?;
            continue;
        }
        let Event::Key(key) = event else { continue };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Esc => {
                eprint!("\r\n");
                return Ok(None);
            }
            KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                eprint!("\r\n");
                return Ok(None);
            }
            KeyCode::Enter => {
                eprint!("\r\n");
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

#[cfg(test)]
mod tests {
    use super::{
        safe_text, sensitive_schema, validate_id, validate_options, validate_prompt, Format,
        SharedConversationOptions,
    };
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
        let error = validate_options(&interactive).expect_err("stdin approval cannot share input");
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
}
