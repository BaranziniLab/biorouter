use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use biorouter::dictation::service::{self, TranscribeRequest};
use biorouter::dictation::{bias_set, cli_bias, BiasChannels, CliTerminal, DictionaryRequest};
use biorouter::security::dictionary_store;
use biorouter_dictation::recognize::EligibleEntries;
use crossterm::event::{Event as TerminalEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::capture::{CaptureChild, CaptureEvent};
use super::consent::CaptureConsent;
use super::controller::{self, Ended, FixedText, LiveDecode, Notice, Recorder, ServiceEngine};
use super::messages;

const STREAM: u64 = 1 << 58;

#[cfg(debug_assertions)]
fn record_first_audio_timing(dispatch_started: Instant) {
    if std::env::var_os("BIOROUTER_TEST_DICTATION_TIMINGS").as_deref()
        != Some(std::ffi::OsStr::new("1"))
    {
        return;
    }
    let elapsed_ms = dispatch_started.elapsed().as_millis();
    let path = biorouter::config::paths::Paths::in_local_data_dir("dictation/test-first-audio-ms");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{elapsed_ms}");
    }
}

struct TerminalTitle;

impl TerminalTitle {
    fn push() -> Self {
        let _ = std::io::stderr().write_all(b"\x1b[22;0t\x1b]0;\xe2\x97\x8f Biorouter\x07");
        Self
    }
}

impl Drop for TerminalTitle {
    fn drop(&mut self) {
        let _ = std::io::stderr().write_all(b"\x1b[23;0t");
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Starting,
    Recording(String),
    Transcribing(String),
    Text(String),
    Failed(String),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Stop,
    Cancel,
}

pub struct Frontend {
    pub events: mpsc::UnboundedReceiver<Event>,
    commands: mpsc::UnboundedSender<Command>,
    state: FrontendSession,
    operation: u64,
}

#[derive(Clone, Default)]
pub struct FrontendSession(Arc<Mutex<SessionState>>);

#[derive(Default)]
struct SessionState {
    failures: controller::Failures,
    retry: controller::RetryHold,
    next_operation: u64,
    active: Option<ActiveOperation>,
}

#[derive(Clone, Copy)]
struct ActiveOperation {
    id: u64,
    cancelled: bool,
}

impl FrontendSession {
    pub fn paused(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failures
            .paused()
    }

    pub fn clear(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.as_mut() {
            active.cancelled = true;
        }
        state.retry.clear();
    }

    fn begin_capture(&self) -> Option<u64> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.failures.paused() {
            return None;
        }
        state.retry.clear();
        state.next_operation = state.next_operation.wrapping_add(1);
        let id = state.next_operation;
        state.active = Some(ActiveOperation {
            id,
            cancelled: false,
        });
        Some(id)
    }

    fn begin_retry(&self) -> Result<(u64, Vec<i16>), &'static str> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.failures.paused() {
            return Err(messages::MSG_PAUSED);
        }
        let audio = state
            .retry
            .take(Instant::now())
            .ok_or("There is no recording available to retry.")?;
        state.next_operation = state.next_operation.wrapping_add(1);
        let id = state.next_operation;
        state.active = Some(ActiveOperation {
            id,
            cancelled: false,
        });
        Ok((id, audio))
    }

    fn cancel_operation(&self, operation: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state
            .active
            .as_mut()
            .filter(|active| active.id == operation)
        {
            active.cancelled = true;
            state.retry.clear();
        }
    }

    fn record_failure(&self, operation: u64, mut failure: Failure) -> Option<String> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !state
            .active
            .is_some_and(|active| active.id == operation && !active.cancelled)
        {
            return None;
        }
        if let Some(audio) = failure.audio.take() {
            state.retry.keep(audio, Instant::now());
        }
        let paused = state.failures.record(Instant::now());
        Some(if paused {
            messages::MSG_PAUSED.into()
        } else {
            failure.message.clone()
        })
    }

    fn send_text_if_live(
        &self,
        operation: u64,
        events: &mpsc::UnboundedSender<Event>,
        text: String,
    ) {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .active
            .is_some_and(|active| active.id == operation && !active.cancelled)
        {
            let _ = events.send(Event::Text(text));
        } else {
            let _ = events.send(Event::Cancelled);
        }
    }

    fn report_failure(
        &self,
        operation: u64,
        failure: Failure,
        events: &mpsc::UnboundedSender<Event>,
    ) {
        let event = match self.record_failure(operation, failure) {
            Some(message) => Event::Failed(message),
            None => Event::Cancelled,
        };
        let _ = events.send(event);
    }
}

struct Failure {
    message: String,
    audio: Option<Vec<i16>>,
}

impl Drop for Failure {
    fn drop(&mut self) {
        if let Some(audio) = self.audio.as_mut() {
            controller::wipe(audio);
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            audio: None,
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

impl Frontend {
    #[cfg(test)]
    pub fn for_test() -> Self {
        let (_events_tx, events) = mpsc::unbounded_channel();
        let (commands, _commands_rx) = mpsc::unbounded_channel();
        Self {
            events,
            commands,
            state: FrontendSession::default(),
            operation: 0,
        }
    }
    pub fn start(
        session_id: Option<String>,
        model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
        state: FrontendSession,
    ) -> Self {
        let dispatch_started = Instant::now();
        let (events_tx, events) = mpsc::unbounded_channel();
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let operation = state.begin_capture();
        if let Some(operation) = operation {
            tokio::spawn(run(
                session_id,
                model_affiliation,
                state.clone(),
                operation,
                dispatch_started,
                commands_rx,
                events_tx,
            ));
        } else {
            let _ = events_tx.send(Event::Failed(messages::MSG_PAUSED.into()));
        }
        Self {
            events,
            commands,
            state,
            operation: operation.unwrap_or(0),
        }
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
    }

    pub fn retry(
        session_id: Option<String>,
        model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
        state: FrontendSession,
    ) -> Self {
        let (events_tx, events) = mpsc::unbounded_channel();
        let (commands, commands_rx) = mpsc::unbounded_channel();
        let operation = match state.begin_retry() {
            Ok((operation, audio)) => {
                tokio::spawn(retry_run(
                    session_id,
                    model_affiliation,
                    state.clone(),
                    operation,
                    commands_rx,
                    events_tx,
                    audio,
                ));
                operation
            }
            Err(reason) => {
                let _ = events_tx.send(Event::Failed(reason.into()));
                0
            }
        };
        Self {
            events,
            commands,
            state,
            operation,
        }
    }

    pub fn cancel(&self) {
        self.state.cancel_operation(self.operation);
        let _ = self.commands.send(Command::Cancel);
    }
}

async fn retry_run(
    session_id: Option<String>,
    model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
    state: FrontendSession,
    operation: u64,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
    mut audio: Vec<i16>,
) {
    let _ = events.send(Event::Transcribing(messages::line_transcribing(
        audio.len() as u64 / 16,
    )));
    let result = async {
        let svc = service::service();
        svc.spawn_ahead().map_err(|e| e.to_string())?;
        let language = service::configured_language();
        let spec = svc.resolve_for(None, Some(&language)).await.map_err(|e| e.to_string())?;
        svc.prepare_with(spec.clone()).await.map_err(|e| e.to_string())?;
        let dictionary = dictionary(session_id, model_affiliation).await;
        let cancel = CancellationToken::new();
        let request = TranscribeRequest { language, cancel: cancel.clone(), dictionary: dictionary.clone(), decode: None };
        let engine: Arc<dyn controller::Engine> = match controller::test_text() {
            Some(text) => Arc::new(FixedText { text }),
            None => Arc::new(ServiceEngine { spec, stream: STREAM + 1 }),
        };
        let mut decoder = LiveDecode::new(engine, async move { request });
        decoder.poll(&audio);
        let decoded = tokio::select! {
            result = decoder.finish(&audio) => result.map_err(|_| messages::MSG_ENGINE_FAILED_CLI.to_string())?,
            _ = commands.recv() => {
                cancel.cancel();
                return Ok(None);
            }
        };
        let raw = decoded.raw_text();
        let text = dictionary.as_ref().map_or_else(|| raw.clone(), |dictionary| dictionary.correct(&raw, &dictionary.prompt_terms()).text);
        Ok::<Option<String>, String>(Some(text))
    }.await;
    match result {
        Ok(Some(text)) if !text.trim().is_empty() => {
            state.send_text_if_live(operation, &events, text)
        }
        Ok(Some(_)) => {
            let _ = events.send(Event::Failed(messages::MSG_NO_SPEECH.into()));
        }
        Ok(None) => {
            let _ = events.send(Event::Cancelled);
        }
        Err(reason) => state.report_failure(
            operation,
            Failure {
                message: reason,
                audio: Some(std::mem::take(&mut audio)),
            },
            &events,
        ),
    }
    controller::wipe(&mut audio);
}

pub async fn capture_in_terminal(
    session_id: Option<String>,
    model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
    state: FrontendSession,
) -> Result<Option<String>, String> {
    interact(Frontend::start(session_id, model_affiliation, state)).await
}

pub async fn retry_in_terminal(
    session_id: Option<String>,
    model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
    state: FrontendSession,
) -> Result<Option<String>, String> {
    interact(Frontend::retry(session_id, model_affiliation, state)).await
}

async fn interact(mut frontend: Frontend) -> Result<Option<String>, String> {
    struct RawMode;
    impl Drop for RawMode {
        fn drop(&mut self) {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
    crossterm::terminal::enable_raw_mode().map_err(|e| e.to_string())?;
    let _raw = RawMode;
    let mut terminal = EventStream::new();
    let mut last_status = String::new();
    loop {
        tokio::select! {
            event = frontend.events.recv() => match event {
                Some(Event::Starting) => eprint!("\r{}", messages::LINE_STARTING),
                Some(Event::Recording(status) | Event::Transcribing(status)) => {
                    if status != last_status {
                        eprint!("\r{status}  ");
                        last_status = status;
                    }
                }
                Some(Event::Text(text)) => {
                    eprintln!();
                    return Ok(Some(text));
                }
                Some(Event::Failed(reason)) => {
                    eprintln!();
                    return Err(reason);
                }
                Some(Event::Cancelled) | None => {
                    eprintln!();
                    return Ok(None);
                }
            },
            key = terminal.next() => match key {
                Some(Ok(TerminalEvent::Key(key))) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Enter => frontend.stop(),
                    KeyCode::Esc => frontend.cancel(),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => frontend.cancel(),
                    KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && Some(c.to_ascii_lowercase()) == dictation_key() => frontend.stop(),
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

fn dictation_key() -> Option<char> {
    let key = std::env::var("BIOROUTER_CLI_DICTATION_KEY").unwrap_or_else(|_| "t".into());
    let key = key.chars().next()?.to_ascii_lowercase();
    key.is_ascii_alphabetic().then_some(key)
}

fn hint_admitted_by_rules(
    enabled: &biorouter::managed::Rule,
    capture: &biorouter::managed::Rule,
    terminal: bool,
    engine: bool,
) -> bool {
    enabled.is_allowed() && capture.is_allowed() && terminal && engine
}

pub fn hint_admitted_without_device() -> bool {
    let policy = biorouter::managed::ManagedPolicy::load();
    hint_admitted_by_rules(
        &policy.dictation().enabled,
        &policy.dictation().capture,
        CaptureConsent::at_this_terminal().is_ok(),
        service::spawn_refusal().is_none(),
    )
}

#[derive(Clone, Debug)]
pub struct DictatedSpan {
    pub before: String,
    pub original: String,
    pub after: String,
    pub record_id: String,
    pub range: Option<(usize, usize)>,
    fully_deleted: bool,
    tail_retyping: bool,
}

impl DictatedSpan {
    pub fn new(before: String, original: String, after: String) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            before,
            original,
            after,
            record_id: format!("terminal-{at}-{id}"),
            range: None,
            fully_deleted: false,
            tail_retyping: false,
        }
    }

    pub fn at_range(mut self, start: usize, end: usize) -> Self {
        self.range = Some((start, end));
        self
    }

    pub fn map_edit(&mut self, start: usize, end: usize, inserted_len: usize) {
        let Some((span_start, span_end)) = self.range else {
            return;
        };
        if start == end && start == span_start {
            self.tail_retyping = false;
            self.range = Some((span_start, span_end + inserted_len));
        } else if end <= span_start {
            self.tail_retyping = false;
            let delta = inserted_len as isize - (end - start) as isize;
            self.range = Some((
                span_start.saturating_add_signed(delta),
                span_end.saturating_add_signed(delta),
            ));
        } else if start < span_end && start >= span_start && end <= span_end {
            let delta = inserted_len as isize - (end - start) as isize;
            let next_end = span_end.saturating_add_signed(delta);
            if next_end == span_start && inserted_len == 0 {
                self.range = None;
                self.fully_deleted = true;
                self.tail_retyping = false;
            } else {
                self.range = Some((span_start, next_end));
                self.tail_retyping = inserted_len == 0 && end == span_end;
            }
        } else if start < span_end {
            self.range = None;
            self.tail_retyping = false;
        } else {
            self.tail_retyping = false;
        }
    }

    pub fn map_edit_text(&mut self, start: usize, end: usize, inserted: &str) {
        if let Some((span_start, span_end)) = self.range {
            if start == end && start == span_end && span_end > span_start && self.tail_retyping {
                let mut kept = inserted.len();
                for (offset, c) in inserted.char_indices() {
                    if c.is_whitespace() {
                        kept = offset;
                        break;
                    }
                    if matches!(c, '.' | '!' | '?' | '。' | '！' | '？') {
                        kept = offset + c.len_utf8();
                        break;
                    }
                }
                self.range = Some((span_start, span_end + kept));
                self.tail_retyping = kept == inserted.len()
                    && !inserted
                        .chars()
                        .next_back()
                        .is_some_and(|c| matches!(c, '.' | '!' | '?' | '。' | '！' | '？'));
                return;
            }
        }
        self.map_edit(start, end, inserted.len());
    }

    pub fn correction(&self, submitted: &str) -> Option<biorouter_dictation::store::Observation> {
        use biorouter_dictation::recognize::learn;
        use biorouter_dictation::store::{Observation, ObservationKind};
        let changed = match self.range {
            Some((start, end)) => submitted.get(start..end)?,
            None => submitted
                .strip_prefix(&self.before)?
                .strip_suffix(&self.after)?,
        };
        let changed = if self.fully_deleted {
            changed
                .split_whitespace()
                .next()?
                .trim_matches(|c: char| !c.is_alphanumeric())
        } else {
            changed
        };
        let old: Vec<&str> = self.original.split_whitespace().collect();
        let new: Vec<&str> = changed.split_whitespace().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let mut from = old[prefix..old.len().saturating_sub(suffix)].join(" ");
        let mut to = new[prefix..new.len().saturating_sub(suffix)].join(" ");
        trim_shared_sentence_ending(&mut from, &mut to);
        if from.is_empty()
            || to.is_empty()
            || old.len().max(new.len()) >= 4
                && old.len().saturating_sub(prefix + suffix)
                    + new.len().saturating_sub(prefix + suffix)
                    > old.len().max(new.len())
        {
            return None;
        }
        if !learn::accepts_correction(&from, &to) {
            return None;
        }
        if [from.as_str(), to.as_str()].into_iter().any(|text| {
            biorouter::guardrails::secret_output::redact_text(text).is_some()
                || biorouter::guardrails::pii::PiiDetector::new().has_pii(text)
        }) {
            return None;
        }
        Some(Observation {
            record_id: self.record_id.clone(),
            kind: ObservationKind::Correction,
            from,
            to,
            entry_id: None,
        })
    }
}

fn trim_shared_sentence_ending(from: &mut String, to: &mut String) {
    while let Some(last) = from.chars().next_back() {
        if !matches!(last, '.' | '!' | '?' | '。' | '！' | '？') || !to.ends_with(last) {
            break;
        }
        from.pop();
        to.pop();
    }
}

pub fn map_replacement(spans: &mut [DictatedSpan], before: &str, after: &str) {
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let suffix = before
        .split_at(prefix)
        .1
        .chars()
        .rev()
        .zip(after.split_at(prefix).1.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum::<usize>();
    let replacement = after
        .get(prefix..after.len() - suffix)
        .expect("common prefix and suffix end at UTF-8 boundaries");
    for span in spans {
        span.map_edit_text(prefix, before.len() - suffix, replacement);
    }
}

pub async fn observe_correction(
    session_id: &str,
    model: Option<(
        biorouter::privacy::ProviderTier,
        Option<biorouter::privacy::ModelAffiliation>,
    )>,
    span: &DictatedSpan,
    submitted: &str,
) {
    if crate::commands::dictionary::authorize_session_action(
        &crate::commands::dictionary::Action::Add {
            terms: Vec::new(),
            spoken: Vec::new(),
            private: false,
        },
    )
    .is_err()
    {
        return;
    }
    use biorouter::dictation::chat_reach;
    use biorouter::privacy::affiliation::{dictionary_row_institutions, WriterModel};
    use biorouter::security::dictionary_store::store::{
        EntryTier, ObservationStamp, RowAffiliation,
    };
    let Some(observation) = span.correction(submitted) else {
        return;
    };
    let manager = biorouter::session::SessionManager::instance();
    let stored = manager
        .get_session(session_id, false)
        .await
        .ok()
        .map(|session| session.privacy_tier);
    let private = chat_reach(stored, model.map(|(tier, _)| tier)).stamp_private();
    let affiliation = if private {
        let writer = model.map_or(WriterModel::Unknown, |(_, affiliation)| {
            WriterModel::Known(affiliation)
        });
        dictionary_row_institutions(writer, manager.session_affiliations(session_id).await)
            .stored_ids()
            .map(|institutions| RowAffiliation { institutions })
    } else {
        None
    };
    let stamp = ObservationStamp {
        tier: if private {
            EntryTier::Private
        } else {
            EntryTier::Public
        },
        affiliation,
        session_id: Some(session_id.to_string()),
    };
    let store = dictionary_store::open_store();
    let view = match store.view() {
        Ok(view) => view,
        Err(_) => return,
    };
    let (mut target, standing) = cli_bias(CliTerminal {
        session: stored,
        interactive: true,
        caller_private: true,
        agent_shell: false,
    });
    target.affiliation = model.and_then(|(_, affiliation)| affiliation);
    let eligible = bias_set(
        &view,
        &target,
        standing,
        biorouter::privacy::privacy_tiers_enabled(),
    );
    let allowed: std::collections::HashSet<String> = eligible
        .entries()
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let _ = store.write(|file| {
        Ok(file.observe(
            &observation,
            &stamp,
            &|entry| allowed.contains(entry.id.as_str()),
            biorouter::security::dictionary_store::store::schema::now(),
        ))
    });
}

pub async fn add_term(
    session_id: &str,
    model: Option<(
        biorouter::privacy::ProviderTier,
        Option<biorouter::privacy::ModelAffiliation>,
    )>,
    written: &str,
) -> Result<String, String> {
    use biorouter::dictation::chat_reach;
    use biorouter::privacy::affiliation::{dictionary_row_institutions, WriterModel};
    use biorouter::security::dictionary_store::store::{EntryTier, NewTerm, Reach, RowAffiliation};

    if written.trim().is_empty() {
        return Err("Type a term after /dictionary add.".into());
    }
    crate::commands::dictionary::authorize_session_action(
        &crate::commands::dictionary::Action::Add {
            terms: vec![written.trim().to_string()],
            spoken: Vec::new(),
            private: false,
        },
    )?;
    if biorouter::guardrails::secret_output::redact_text(written).is_some()
        || biorouter::guardrails::pii::PiiDetector::new().has_pii(written)
    {
        return Err("That term contains private information and was not added.".into());
    }
    let manager = biorouter::session::SessionManager::instance();
    let stored = manager
        .get_session(session_id, false)
        .await
        .ok()
        .map(|session| session.privacy_tier);
    let private = chat_reach(stored, model.map(|(tier, _)| tier)).stamp_private();
    let tier = if private {
        EntryTier::Private
    } else {
        EntryTier::Public
    };
    let affiliation = if private {
        let writer = model.map_or(WriterModel::Unknown, |(_, affiliation)| {
            WriterModel::Known(affiliation)
        });
        dictionary_row_institutions(writer, manager.session_affiliations(session_id).await)
            .stored_ids()
            .map(|institutions| RowAffiliation { institutions })
    } else {
        None
    };
    let mut term = NewTerm::person(written.trim(), tier);
    term.affiliation = affiliation;
    term.suggested_in = Some(session_id.to_string());
    dictionary_store::open_store()
        .write(|file| {
            file.add(
                term,
                Reach::All,
                biorouter::security::dictionary_store::store::schema::now(),
            )
        })
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Added {} to the {} dictionary.",
        written.trim(),
        tier.as_str()
    ))
}

async fn dictionary(
    session_id: Option<String>,
    model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
) -> Option<Arc<DictionaryRequest>> {
    if !biorouter::managed::ManagedPolicy::load()
        .dictation()
        .dictionary
        .is_allowed()
    {
        return None;
    }
    let tier = match session_id {
        Some(id) => biorouter::session::SessionManager::instance()
            .get_session(&id, false)
            .await
            .ok()
            .map(|session| session.privacy_tier),
        None => None,
    };
    let (mut target, standing) = cli_bias(CliTerminal {
        session: tier,
        interactive: true,
        caller_private: true,
        agent_shell: false,
    });
    target.affiliation = model_affiliation;
    let eligible = match dictionary_store::open_store().view() {
        Ok(view) => bias_set(
            &view,
            &target,
            standing,
            biorouter::privacy::privacy_tiers_enabled(),
        ),
        Err(_) => EligibleEntries::empty(),
    };
    let request = Arc::new(DictionaryRequest::new(
        eligible,
        "",
        BiasChannels::for_dictation(),
    ));
    let _ = service::service().install_bias(&request);
    Some(request)
}

async fn run(
    session_id: Option<String>,
    model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
    state: FrontendSession,
    operation: u64,
    dispatch_started: Instant,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
) {
    let _ = events.send(Event::Starting);
    let result = run_inner(
        session_id,
        model_affiliation,
        &state,
        operation,
        dispatch_started,
        &mut commands,
        &events,
    )
    .await;
    if let Err(failure) = result {
        state.report_failure(operation, failure, &events);
    }
}

async fn run_inner(
    session_id: Option<String>,
    model_affiliation: Option<biorouter::privacy::ModelAffiliation>,
    state: &FrontendSession,
    operation: u64,
    _dispatch_started: Instant,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    events: &mpsc::UnboundedSender<Event>,
) -> Result<(), Failure> {
    let policy = biorouter::managed::ManagedPolicy::load();
    for rule in [&policy.dictation().enabled, &policy.dictation().capture] {
        if let Some(reason) = rule.reason() {
            return Err(reason.to_string().into());
        }
    }
    let consent =
        CaptureConsent::at_this_terminal().map_err(|_| messages::MSG_NO_TTY.to_string())?;
    if let Some(refusal) = service::spawn_refusal() {
        return Err(refusal.to_string().into());
    }
    let language = service::configured_language();
    let svc = service::service();
    svc.spawn_ahead().map_err(|e| e.to_string())?;
    if commands.try_recv().is_ok() {
        let _ = events.send(Event::Cancelled);
        return Ok(());
    }
    let model = svc.resolve_for(None, Some(&language));
    let mut child = CaptureChild::spawn().map_err(|_| messages::MSG_NO_DEVICE.to_string())?;
    let probe = child.probe().await.ok_or(messages::MSG_NO_DEVICE)?;
    if probe.error.is_some() || probe.devices.is_empty() && !probe.test_source {
        return Err(messages::MSG_NO_DEVICE.into());
    }
    let spec = tokio::select! {
        spec = model => spec.map_err(|e| e.to_string())?,
        _ = commands.recv() => {
            let _ = events.send(Event::Cancelled);
            return Ok(());
        }
    };
    if commands.try_recv().is_ok() {
        let _ = events.send(Event::Cancelled);
        return Ok(());
    }
    let prepared = tokio::spawn({
        let spec = spec.clone();
        async move { svc.prepare_with(spec).await }
    });
    let dictionary = tokio::spawn(dictionary(session_id, model_affiliation));
    child
        .start(
            &consent,
            std::env::var("BIOROUTER_CLI_DICTATION_DEVICE").ok(),
        )
        .await
        .map_err(|_| messages::MSG_MIC_BUSY.to_string())?;
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match child.next().await {
                CaptureEvent::Ready(_) => return true,
                CaptureEvent::Error(_) | CaptureEvent::Exited => return false,
                _ => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    if !ready {
        return Err(messages::MSG_MIC_BUSY.into());
    }
    let title = TerminalTitle::push();
    let cancel = CancellationToken::new();
    let (dictionary_tx, dictionary_rx) = tokio::sync::oneshot::channel();
    let request = {
        let cancel = cancel.clone();
        async move {
            let dictionary = dictionary.await.ok().flatten();
            let _ = dictionary_tx.send(dictionary.clone());
            TranscribeRequest {
                language,
                cancel,
                dictionary,
                decode: None,
            }
        }
    };
    let engine: Arc<dyn controller::Engine> = match controller::test_text() {
        Some(text) => Arc::new(FixedText { text }),
        None => Arc::new(ServiceEngine {
            spec,
            stream: STREAM,
        }),
    };
    let mut decoder = LiveDecode::new(engine, request);
    let mut recorder = Recorder::new(None);
    let mut peaks = VecDeque::from(vec![0.0; 5]);
    let started = Instant::now();
    #[cfg(debug_assertions)]
    let mut first_audio_seen = false;
    let mut cancelled = false;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Stop) => break,
                Some(Command::Cancel) | None => { cancelled = true; break; }
            },
            event = child.next() => match event {
                CaptureEvent::Audio(mut samples) => {
                    #[cfg(debug_assertions)]
                    if !first_audio_seen {
                        first_audio_seen = true;
                        record_first_audio_timing(_dispatch_started);
                    }
                    let notices = recorder.push(&samples);
                    controller::wipe(&mut samples);
                    decoder.poll(recorder.pcm());
                    if notices.iter().any(|notice| matches!(notice, Notice::Stop(_) | Notice::Denied)) {
                        break;
                    }
                }
                CaptureEvent::Level(level) => {
                    peaks.pop_front();
                    peaks.push_back(level.peak);
                    let _ = events.send(Event::Recording(messages::line_recording(recorder.duration_ms(), peaks.make_contiguous())));
                }
                CaptureEvent::Error(_) | CaptureEvent::Exited => return Err(messages::MSG_NO_DEVICE.into()),
                _ => {}
            },
        }
    }
    if cancelled {
        cancel.cancel();
        tokio::spawn(child.close());
        drop(title);
        let _ = events.send(Event::Cancelled);
        return Ok(());
    }
    child.stop().await;
    drop(title);
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match child.next().await {
                CaptureEvent::Audio(mut samples) => {
                    recorder.push(&samples);
                    controller::wipe(&mut samples);
                }
                CaptureEvent::Stopped(_) | CaptureEvent::Exited | CaptureEvent::Error(_) => break,
                _ => {}
            }
        }
    })
    .await;
    tokio::spawn(child.close());
    let pcm = recorder.finish().map_err(|ended| match ended {
        Ended::TooShort => messages::MSG_TOO_SHORT.to_string(),
        Ended::Silent => messages::MSG_NO_SPEECH.to_string(),
    })?;
    let _ = events.send(Event::Transcribing(messages::line_transcribing(
        started.elapsed().as_millis() as u64,
    )));
    let decoded = tokio::select! {
        decoded = decoder.finish(&pcm) => decoded.map_err(|_| Failure { message: messages::MSG_ENGINE_FAILED_CLI.into(), audio: Some(pcm.to_vec()) })?,
        command = commands.recv() => {
            if matches!(command, Some(Command::Cancel) | None) {
                cancel.cancel();
                let _ = events.send(Event::Cancelled);
                return Ok(());
            }
            return Err(messages::MSG_ENGINE_FAILED_CLI.into());
        }
    };
    drop(pcm);
    drop(prepared);
    let raw = decoded.raw_text();
    let dictionary = dictionary_rx.await.ok().flatten();
    let text = dictionary.as_ref().map_or_else(
        || raw.clone(),
        |dictionary| dictionary.correct(&raw, &dictionary.prompt_terms()).text,
    );
    if text.trim().is_empty() {
        return Err(messages::MSG_NO_SPEECH.into());
    }
    state.send_text_if_live(operation, events, text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_edited_dictated_word_yields_one_pair() {
        let span = DictatedSpan::new(
            "Please check ".into(),
            "Epas one in the report".into(),
            String::new(),
        );
        let observed = span
            .correction("Please check EPAS1 in the report")
            .expect("one short correction");
        assert_eq!(observed.from, "Epas one");
        assert_eq!(observed.to, "EPAS1");
    }

    #[test]
    fn correction_of_final_word_keeps_sentence_period_out_of_the_dictionary_pair() {
        let dictated = "Query the SPOKE knowledge graph for drugs that target EPADS1.";
        let submitted = "prefix Query the SPOKE knowledge graph for drugs that target EPAS1.";
        let span = DictatedSpan::new("prefix ".into(), dictated.into(), String::new());
        let observed = span
            .correction(submitted)
            .expect("one corrected final word");
        assert_eq!(
            (observed.from.as_str(), observed.to.as_str()),
            ("EPADS1", "EPAS1")
        );
        assert!(submitted.ends_with("EPAS1."));
    }

    #[test]
    fn classic_seeded_line_replacement_keeps_final_word_and_period_in_span() {
        let original = "prefix Query the SPOKE knowledge graph for drugs that target EPADS1.";
        let submitted = "prefix Query the SPOKE knowledge graph for drugs that target EPAS1.";
        let mut spans = vec![DictatedSpan::new(
            "prefix ".into(),
            original.strip_prefix("prefix ").unwrap().into(),
            String::new(),
        )
        .at_range("prefix ".len(), original.len())];
        map_replacement(&mut spans, original, submitted);
        let observed = spans[0].correction(submitted).expect("one classic edit");
        assert_eq!(
            (observed.from.as_str(), observed.to.as_str()),
            ("EPADS1", "EPAS1")
        );
    }

    #[test]
    fn sentence_ending_normalization_preserves_internal_scientific_punctuation() {
        for (from, to, expected_from, expected_to) in [
            ("CD4+.", "CD4++.", "CD4+", "CD4++"),
            ("HIF-2α!", "HIF-2β!", "HIF-2α", "HIF-2β"),
            ("C++?", "C+?", "C++", "C+"),
            ("β。", "γ。", "β", "γ"),
        ] {
            let mut from = from.to_string();
            let mut to = to.to_string();
            trim_shared_sentence_ending(&mut from, &mut to);
            assert_eq!((from.as_str(), to.as_str()), (expected_from, expected_to));
        }
    }

    #[test]
    fn sequential_backspace_and_retype_keeps_one_correction_without_adjacent_words() {
        let mut span = DictatedSpan::new(String::new(), "Epas one".into(), String::new())
            .at_range(0, "Epas one".len());
        let mut input = "Epas one".to_string();
        while !input.is_empty() {
            let end = input.len();
            span.map_edit(end - 1, end, 0);
            input.pop();
        }
        for c in "EPAS1 later".chars() {
            span.map_edit(input.len(), input.len(), c.len_utf8());
            input.push(c);
        }
        let observed = span.correction(&input).expect("one corrected term");
        assert_eq!(
            (observed.from.as_str(), observed.to.as_str()),
            ("Epas one", "EPAS1")
        );
        assert_eq!(span.correction("unrelated rewrite"), None);
    }

    #[test]
    fn unrelated_edits_and_wholesale_rewrites_are_not_learned() {
        let span = DictatedSpan::new(
            "Please check ".into(),
            "Epas one in the report".into(),
            " today".into(),
        );
        assert!(span
            .correction("Please check Epas one in the report tomorrow")
            .is_none());
        assert!(span
            .correction("Please check EPAS1 unrelated replacement today")
            .is_none());
    }

    #[test]
    fn three_failures_in_ten_seconds_pause_the_session_and_expired_failures_do_not() {
        let session = FrontendSession::default();
        let start = Instant::now();
        let mut state = session.0.lock().unwrap();
        assert!(!state.failures.record(start));
        assert!(!state.failures.record(start + Duration::from_secs(4)));
        assert!(state.failures.record(start + Duration::from_secs(9)));
        drop(state);
        assert!(session.paused());

        let mut new_window = controller::Failures::default();
        assert!(!new_window.record(start));
        assert!(!new_window.record(start + Duration::from_secs(11)));
        assert!(!new_window.record(start + Duration::from_secs(20)));
    }

    #[test]
    fn third_frontend_failure_emits_msg_paused_and_refuses_another_capture() {
        let session = FrontendSession::default();
        let mut messages_seen = Vec::new();
        for _ in 0..3 {
            let operation = session.begin_capture().unwrap();
            messages_seen.push(
                session
                    .record_failure(
                        operation,
                        Failure {
                            message: messages::MSG_ENGINE_FAILED_CLI.into(),
                            audio: None,
                        },
                    )
                    .unwrap(),
            );
        }
        assert_eq!(messages_seen[2], messages::MSG_PAUSED);
        let mut refused = Frontend::start(None, None, session);
        assert_eq!(
            refused.events.try_recv().unwrap(),
            Event::Failed(messages::MSG_PAUSED.into())
        );
    }

    #[test]
    fn retry_holds_audio_for_five_minutes_then_drops_it() {
        let start = Instant::now();
        let mut hold = controller::RetryHold::default();
        hold.keep(vec![1, 2, 3], start);
        assert_eq!(
            hold.take(start + Duration::from_secs(299)),
            Some(vec![1, 2, 3])
        );
        hold.keep(vec![4, 5], start);
        assert_eq!(hold.take(start + Duration::from_secs(301)), None);
    }

    #[test]
    fn seeded_line_diff_tracks_unicode_edits_across_more_than_one_capture() {
        let mut spans = vec![
            DictatedSpan::new("β ".into(), "Epas one".into(), String::new())
                .at_range("β ".len(), "β Epas one".len()),
        ];
        map_replacement(&mut spans, "β Epas one", "β EPAS1");
        let observed = spans[0].correction("β EPAS1").unwrap();
        assert_eq!(
            (observed.from.as_str(), observed.to.as_str()),
            ("Epas one", "EPAS1")
        );
    }

    #[test]
    fn cancelling_a_fresh_decode_error_cannot_resurrect_retry_audio_in_either_order() {
        for cancel_first in [true, false] {
            let session = FrontendSession::default();
            let operation = session.begin_capture().unwrap();
            if cancel_first {
                session.cancel_operation(operation);
            }
            let result = session.record_failure(
                operation,
                Failure {
                    message: messages::MSG_ENGINE_FAILED_CLI.into(),
                    audio: Some(vec![11, 22]),
                },
            );
            if !cancel_first {
                session.cancel_operation(operation);
            }
            assert_eq!(result.is_none(), cancel_first);
            assert!(session
                .0
                .lock()
                .unwrap()
                .retry
                .take(Instant::now())
                .is_none());
        }
    }

    #[test]
    fn cancelling_a_retry_decode_error_cannot_restore_the_consumed_audio() {
        for cancel_first in [true, false] {
            let session = FrontendSession::default();
            let first = session.begin_capture().unwrap();
            assert!(session
                .record_failure(
                    first,
                    Failure {
                        message: messages::MSG_ENGINE_FAILED_CLI.into(),
                        audio: Some(vec![33, 44]),
                    }
                )
                .is_some());
            let (retry, audio) = session.begin_retry().unwrap();
            if cancel_first {
                session.cancel_operation(retry);
            }
            let result = session.record_failure(
                retry,
                Failure {
                    message: messages::MSG_ENGINE_FAILED_CLI.into(),
                    audio: Some(audio),
                },
            );
            if !cancel_first {
                session.cancel_operation(retry);
            }
            assert_eq!(result.is_none(), cancel_first);
            assert!(session
                .0
                .lock()
                .unwrap()
                .retry
                .take(Instant::now())
                .is_none());
        }
    }

    #[test]
    fn a_late_decode_result_does_not_insert_text_after_cancel() {
        let session = FrontendSession::default();
        let operation = session.begin_capture().unwrap();
        session.cancel_operation(operation);
        let (events, mut receiver) = mpsc::unbounded_channel();
        session.send_text_if_live(operation, &events, "private transcript".into());
        assert_eq!(receiver.try_recv().unwrap(), Event::Cancelled);
    }

    #[test]
    fn hint_requires_enabled_capture_person_and_engine_before_model_resolution() {
        use biorouter::managed::Rule;
        let allowed = Rule::Allowed;
        let denied = Rule::Forbidden("managed deny".into());
        assert!(hint_admitted_by_rules(&allowed, &allowed, true, true));
        assert!(
            !hint_admitted_by_rules(&denied, &allowed, true, true),
            "enabled false"
        );
        assert!(
            !hint_admitted_by_rules(&allowed, &denied, true, true),
            "capture false"
        );
        assert!(
            !hint_admitted_by_rules(&allowed, &allowed, false, true),
            "agent shell or no terminal"
        );
        assert!(
            !hint_admitted_by_rules(&allowed, &allowed, true, false),
            "engine unavailable"
        );
    }

    #[tokio::test]
    async fn failed_engine_surfaces_retry_sentence_and_retry_consumes_the_same_audio() {
        use biorouter::dictation::service::SegmentDecode;
        use biorouter::dictation::{DictationError, PcmSlice, Span};

        struct FailingEngine;
        #[async_trait::async_trait]
        impl controller::Engine for FailingEngine {
            async fn vad(&self, _pcm: PcmSlice) -> Result<Vec<Span>, DictationError> {
                Ok(Vec::new())
            }
            async fn decode(
                &self,
                _pcm: PcmSlice,
                _speech_ms: Option<u64>,
                _request: &TranscribeRequest,
            ) -> Result<SegmentDecode, DictationError> {
                Err(DictationError::Cancelled)
            }
        }

        let state = FrontendSession::default();
        let first = state.begin_capture().unwrap();
        let audio = vec![1_000i16; 16_000];
        let failed = LiveDecode::new(Arc::new(FailingEngine), async {
            TranscribeRequest::new("en")
        })
        .finish(&audio)
        .await;
        assert!(
            failed.is_err(),
            "the engine itself failed to decode the recording"
        );
        let (events, mut received) = mpsc::unbounded_channel();
        state.report_failure(
            first,
            Failure {
                message: messages::MSG_ENGINE_FAILED_CLI.into(),
                audio: Some(audio.clone()),
            },
            &events,
        );
        assert_eq!(
            received.try_recv().unwrap(),
            Event::Failed(messages::MSG_ENGINE_FAILED_CLI.into())
        );
        let (retry, held) = state
            .begin_retry()
            .expect("/dictate retry consumes the kept recording");
        assert_eq!(held, audio);
        assert!(
            state.0.lock().unwrap().retry.take(Instant::now()).is_none(),
            "a second retry has no retained audio"
        );
        let decoded = LiveDecode::new(
            Arc::new(FixedText {
                text: "EPAS1 levels rose".into(),
            }),
            async { TranscribeRequest::new("en") },
        )
        .finish(&held)
        .await
        .unwrap();
        state.send_text_if_live(retry, &events, decoded.raw_text());
        assert_eq!(
            received.try_recv().unwrap(),
            Event::Text("EPAS1 levels rose".into())
        );
    }
}
