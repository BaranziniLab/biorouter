//! `biorouter session gates <SESSION_ID> [list|set|remove|pass|skip|retry]`:
//! the person's controls over the decision gates of one chat's phase plan.
//!
//! A gate asks a decision model yes/no questions about a phase's hand-off
//! before the phase may complete, and holds every phase listed after it until
//! it passes. Whoever set it (the agent, the person or the workflow the chat
//! was started from), a stored gate binds the agent: it cannot change, remove
//! or skip one. Only the person can, in the app or here.
//!
//! * **With a reachable daemon** the plan is read with
//!   `GET /sessions/{id}/subagents` (the route the side panel reads) and
//!   written with `PUT` / `DELETE /sessions/{id}/phase-gates/{phase}` and
//!   `POST /sessions/{id}/phase-gates/{phase}/decision`. Those writes need the
//!   user-action proof on a daemon that holds a key, so they go through
//!   `session_watch::protected_write_with_key_if_wanted`, as the monitors'
//!   writes do. A refusal that is not about the key (400, 404, 409, 422) is
//!   printed as the daemon's sentence alone.
//! * **With none** the plan is read and written in this process, under the
//!   tier rule the route applies (`check_local_reach`), with the model check
//!   the route makes before it stores a gate
//!   (`DecisionService::check_model_use`, which sends nothing of the chat) and
//!   the same person transitions the route calls
//!   (`orchestration_plan::person_set_gate`, `person_remove_gate`,
//!   `person_gate_decision`).

use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use biorouter::agents::orchestration_plan::{
    self as orchestration_plan, GateOwner, OrchestrationSnapshot, PersonGateAction,
    PersonGateChoice, PersonGateOutcome, PhaseGateSnapshot, PhaseGateSpec, PhaseSnapshot,
    PhaseStatus, MAX_GATE_QUESTIONS, MAX_PHASE_ID_CHARS,
};
use biorouter::agents::subagent_roster::SessionSubagents;
use biorouter::decision::types::MAX_QUESTION_NAME_CHARS;
use biorouter::decision::{DecisionModelRef, DecisionService, DecisionSession, ModelSource};
use biorouter::privacy::ProviderTier;
use biorouter::session::SessionManager;

use super::apps::configured_port;
use super::decide::{clip, format_p, parse_probability, terminal_text};
use super::session_decisions::{check_local_reach, daemon_error, parse_json, reach, Reach};
use super::session_subagents::{parse_roster, phase_glyph, printable};
use super::session_watch::{
    daemon_get_json, protected_write_with_key_if_wanted, refusal_sentence, DaemonAuth,
};

const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// What a terminal on a public model is told about a private chat on the local
/// path.
pub const LOCAL_OUT_OF_REACH: &str =
    "This chat is private, and this terminal is configured for a model hosted outside your \
     institution, so its phase gates are not shown or changed here. Run this from a terminal \
     configured for a private model, or open the chat in the desktop app.";

/// The refusal of a phase id that is not one, word for word the daemon's.
pub const NOT_A_PHASE_ID: &str =
    "That is not a phase id: it must be 1 to 40 lowercase letters, digits or hyphens.";

/// `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GatesFormat {
    Text,
    Json,
}

/// `session gates <id> set`.
#[derive(clap::Args, Debug, Clone)]
pub struct SetGate {
    /// The phase to gate, by its id (as `list` shows it).
    pub phase: String,
    /// One yes/no question the decision model answers from the phase's
    /// hand-off, as NAME=QUESTION (for example facts="Does the hand-off list
    /// two facts?"). Repeat it, up to 8 times; every question must pass.
    #[arg(
        long = "ask",
        value_name = "NAME=QUESTION",
        value_parser = parse_ask,
        required = true
    )]
    pub asks: Vec<(String, String)>,
    /// The P(yes) every question must reach, from 0 to 1. Default 0.80.
    #[arg(long, value_name = "P", value_parser = parse_probability)]
    pub threshold: Option<f64>,
    /// Failed evaluations allowed before the phase is blocked, 1 to 5.
    /// Default 3.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=5))]
    pub attempts: Option<u32>,
    /// The decision model, `provider/model`. Without it the chat's default
    /// decision model is used.
    #[arg(long, value_name = "PROVIDER/MODEL")]
    pub model: Option<String>,
    /// Accept probabilities a model states in words (uncalibrated) rather
    /// than refusing them.
    #[arg(long)]
    pub allow_uncalibrated: bool,
}

/// What `session gates` does.
#[derive(clap::Subcommand, Debug, Clone)]
pub enum GatesCommand {
    /// List the plan's phases and their gates (the default).
    List,
    /// Set or replace a phase's gate. The gate is yours: the agent cannot
    /// change, remove or skip it, and its attempts start over.
    Set(SetGate),
    /// Remove a phase's gate, which releases the phases after it.
    Remove {
        /// The phase's id.
        phase: String,
    },
    /// Pass a gated phase without asking its gate: it completes at once, or
    /// once its working subagents finish and the agent completes it.
    Pass {
        /// The phase's id.
        phase: String,
    },
    /// Skip a gated phase without asking its gate: it is skipped at once, or
    /// once its working subagents finish and the agent completes it.
    Skip {
        /// The phase's id.
        phase: String,
    },
    /// Give a gate its attempts again, so the agent can complete the phase
    /// once more to ask it.
    Retry {
        /// The phase's id.
        phase: String,
    },
}

/// What a write changes.
#[derive(Debug, Clone, PartialEq)]
pub enum GateEdit {
    Set {
        phase: String,
        spec: PhaseGateSpec,
    },
    Remove {
        phase: String,
    },
    Decide {
        phase: String,
        choice: PersonGateChoice,
    },
}

impl GateEdit {
    fn phase(&self) -> &str {
        match self {
            Self::Set { phase, .. } | Self::Remove { phase } | Self::Decide { phase, .. } => phase,
        }
    }
}

/// `facts` from `Two facts`, a question's name as the decision service takes
/// it: letters, digits, `_` and `-`, at most 64 characters. Anything else
/// becomes a `-`, and runs of them one.
pub fn question_name(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.contains('?') || raw.chars().count() > MAX_QUESTION_NAME_CHARS {
        return Err(format!(
            "--ask takes NAME=QUESTION with a short name before the `=`, for example \
             facts=\"Does the hand-off list two facts?\"; `{}` is not a name",
            clip(raw, 40)
        ));
    }
    let mut out = String::new();
    for c in raw.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return Err(
            "a question's name needs a letter or digit, as in facts=\"Does the hand-off list two \
             facts?\""
                .to_string(),
        );
    }
    Ok(out)
}

/// `--ask NAME=QUESTION`: the name (made into a valid one) and the question.
pub fn parse_ask(raw: &str) -> Result<(String, String), String> {
    let (name, question) = raw.split_once('=').ok_or_else(|| {
        "--ask takes NAME=QUESTION, for example facts=\"Does the hand-off list two facts?\""
            .to_string()
    })?;
    let name = question_name(name)?;
    let question = question.trim();
    if question.is_empty() {
        return Err(format!("--ask {name}= needs a question after the `=`"));
    }
    Ok((name, question.to_string()))
}

/// A phase id as the daemon takes it: 1 to 40 lowercase letters, digits or
/// hyphens. Checked here too, because the id is a segment of the request's
/// path.
pub fn phase_id(raw: &str) -> Result<String> {
    let id = raw.trim();
    let valid = !id.is_empty()
        && id.chars().count() <= MAX_PHASE_ID_CHARS
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        bail!("{NOT_A_PHASE_ID}");
    }
    Ok(id.to_string())
}

/// The gate `set` describes, as the daemon's route takes it. Checked as the
/// route checks it ([`orchestration_plan::validate_person_gate`]), so a
/// mistake is reported before anything is sent.
pub fn build_spec(set: &SetGate) -> Result<(String, PhaseGateSpec)> {
    let phase = phase_id(&set.phase)?;
    if set.asks.len() > MAX_GATE_QUESTIONS {
        bail!(
            "a gate takes 1 to {MAX_GATE_QUESTIONS} questions, not {}",
            set.asks.len()
        );
    }
    let mut seen = std::collections::HashSet::new();
    for (name, _) in &set.asks {
        if !seen.insert(name.as_str()) {
            bail!("two --ask questions are named `{name}`; give each its own name");
        }
    }
    let model = match set.model.as_deref() {
        Some(raw) => Some(DecisionModelRef::parse(raw).ok_or_else(|| {
            anyhow!("--model takes `provider/model`, for example `ollama_decision/tev1:0.8b`")
        })?),
        None => None,
    };
    let spec = PhaseGateSpec {
        questions: set.asks.iter().cloned().collect(),
        threshold: set.threshold,
        max_attempts: set.attempts,
        model,
        allow_uncalibrated: set.allow_uncalibrated.then_some(true),
    };
    orchestration_plan::validate_person_gate(&phase, &spec).map_err(|e| anyhow!("{e}"))?;
    Ok((phase, spec))
}

/// The write `command` asks for, or `None` for `list`.
pub fn edit_of(command: GatesCommand) -> Result<Option<GateEdit>> {
    Ok(Some(match command {
        GatesCommand::List => return Ok(None),
        GatesCommand::Set(set) => {
            let (phase, spec) = build_spec(&set)?;
            GateEdit::Set { phase, spec }
        }
        GatesCommand::Remove { phase } => GateEdit::Remove {
            phase: phase_id(&phase)?,
        },
        GatesCommand::Pass { phase } => GateEdit::Decide {
            phase: phase_id(&phase)?,
            choice: PersonGateChoice::Pass,
        },
        GatesCommand::Skip { phase } => GateEdit::Decide {
            phase: phase_id(&phase)?,
            choice: PersonGateChoice::Skip,
        },
        GatesCommand::Retry { phase } => GateEdit::Decide {
            phase: phase_id(&phase)?,
            choice: PersonGateChoice::Retry,
        },
    }))
}

// ── rendering ───────────────────────────────────────────────────────────────

/// Who set a gate, said to the person reading: "set by you", "set by the
/// agent", "set by the workflow".
pub fn owner_words(owner: GateOwner) -> &'static str {
    match owner {
        GateOwner::Person => "set by you",
        GateOwner::Agent => "set by the agent",
        GateOwner::Workflow => "set by the workflow",
    }
}

/// The person's own decision on a gate: "passed by you", "skipped by you".
pub fn decision_words(decision: PersonGateAction) -> &'static str {
    match decision {
        PersonGateAction::Pass => "passed by you",
        PersonGateAction::Skip => "skipped by you",
    }
}

/// "waits for research's gate", the line a held phase carries. `holder` is
/// what the surface names phases by.
pub fn held_words(holder: &str) -> String {
    format!("waits for {}'s gate", printable(holder))
}

fn status_words(status: PhaseStatus) -> &'static str {
    match status {
        PhaseStatus::Pending => "pending",
        PhaseStatus::InProgress => "in progress",
        PhaseStatus::Completed => "completed",
        PhaseStatus::Blocked => "blocked",
        PhaseStatus::Skipped => "skipped",
    }
}

/// A gate in a line: "1 question at P(yes) >= 0.30, 0 of 3 attempts used, set
/// by you".
pub fn gate_line(gate: &PhaseGateSnapshot) -> String {
    let mut words = format!(
        "{} question{} at P(yes) >= {}, {} of {} attempts used",
        gate.question_count,
        if gate.question_count == 1 { "" } else { "s" },
        format_p(gate.threshold),
        gate.attempts,
        gate.max_attempts
    );
    // A gate passes or does not; "failed" reads as an error to a person.
    match (gate.last_passed, gate.last_min_p) {
        (Some(true), _) => words.push_str(", passed"),
        (Some(false), Some(p)) => words.push_str(&format!(
            ", not passed last time (lowest P(yes) {})",
            format_p(p)
        )),
        (Some(false), None) => words.push_str(", not passed last time"),
        (None, _) => {}
    }
    if gate.blocked {
        words.push_str(", blocked");
    }
    words.push_str(", ");
    words.push_str(owner_words(gate.owner));
    if let Some(decision) = gate.person_decision {
        words.push_str(", ");
        words.push_str(decision_words(decision));
    }
    words
}

/// The phase's lines after its head: the gate, the model it asks, its
/// questions, what it holds, and what the person can do now.
fn gate_lines(session_id: &str, phase: &PhaseSnapshot, gate: &PhaseGateSnapshot) -> Vec<String> {
    let mut out = vec![format!("     gate: {}", gate_line(gate))];
    let mut details = Vec::new();
    match &gate.model {
        Some(model) => details.push(format!(
            "asked of {}",
            super::decide::person_model_names(&terminal_text(model))
        )),
        None => details.push("asked of the chat's default decision model".to_string()),
    }
    if gate.allow_uncalibrated {
        details.push("accepts uncalibrated answers".to_string());
    }
    if gate.retries_granted > 0 {
        details.push(format!(
            "given its attempts again {} time{}",
            gate.retries_granted,
            if gate.retries_granted == 1 { "" } else { "s" }
        ));
    }
    out.push(format!("     {}", details.join(" · ")));
    for (name, question) in &gate.questions {
        out.push(format!(
            "       - {}: {}",
            clip(name, 64),
            clip(question, 100)
        ));
    }
    if let Some(error) = gate.last_error.as_deref() {
        out.push(format!(
            "     its last check decided nothing: {}",
            clip(&super::decide::terminal_advice(error), 160)
        ));
    }
    if !gate.holds.is_empty() {
        out.push(format!(
            "     holds {}",
            gate.holds
                .iter()
                .map(|id| printable(id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if phase.status.is_closed() {
        return out;
    }
    let id = printable(&phase.id);
    match gate.person_decision {
        Some(decision) => out.push(format!(
            "     your {} stands: it applies once the phase's subagents finish and the agent \
             completes it",
            match decision {
                PersonGateAction::Pass => "pass",
                PersonGateAction::Skip => "skip",
            }
        )),
        None if gate.needs_person => out.push(format!(
            "     needs you: biorouter session gates {session_id} retry|pass|skip|remove {id}"
        )),
        None => {}
    }
    out
}

/// The text `session gates <id> list` prints: one entry per phase, its gate
/// and the gate that holds it.
pub fn render_gates(session_id: &str, plan: Option<&OrchestrationSnapshot>) -> Vec<String> {
    let Some(plan) = plan else {
        return vec![
            format!("Session {session_id} has no phase plan, so it has no gates."),
            "A plan is declared by the chat's agent; its phases can then be gated here or in the \
             app."
                .to_string(),
        ];
    };
    let gated = plan.phases.iter().filter(|p| p.gate.is_some()).count();
    let mut out = vec![format!(
        "Phase gates of session {session_id}, plan \"{}\" (revision {}): {gated} of {} phase{} \
         gated.",
        printable(&plan.title),
        plan.revision,
        plan.phases.len(),
        if plan.phases.len() == 1 { "" } else { "s" }
    )];
    for (index, phase) in plan.phases.iter().enumerate() {
        let mut head = format!(
            "  {}. {} {}  {}  {}",
            index + 1,
            phase_glyph(phase.status),
            printable(&phase.id),
            printable(&phase.title),
            status_words(phase.status)
        );
        if let Some(holder) = phase.held_by.as_deref() {
            head.push_str(" · ");
            head.push_str(&held_words(holder));
        }
        out.push(head);
        match &phase.gate {
            Some(gate) => out.extend(gate_lines(session_id, phase, gate)),
            None if !phase.status.is_closed() => out.push("     no gate".to_string()),
            None => {}
        }
    }
    out.push(String::new());
    out.push(format!(
        "Set one: biorouter session gates {session_id} set <phase> --ask NAME=QUESTION \
         [--threshold P]. Or: biorouter session gates {session_id} retry|pass|skip|remove <phase>."
    ));
    out
}

// ── reading and writing ─────────────────────────────────────────────────────

/// The request a write makes of the daemon: method, path and body.
pub fn request_for(session_id: &str, edit: &GateEdit) -> Result<(&'static str, String, String)> {
    let base = format!("/sessions/{session_id}/phase-gates/{}", edit.phase());
    Ok(match edit {
        GateEdit::Set { spec, .. } => ("PUT", base, serde_json::to_string(spec)?),
        GateEdit::Remove { .. } => ("DELETE", base, String::new()),
        GateEdit::Decide { choice, .. } => (
            "POST",
            format!("{base}/decision"),
            serde_json::json!({ "decision": choice }).to_string(),
        ),
    })
}

/// What a gate write did, as the daemon answers it or as it ran here.
#[derive(Debug, Clone, PartialEq)]
pub struct GateAnswer {
    pub message: String,
    pub applied: bool,
    /// The chat's plan after the write, as this caller may see it.
    pub plan: Option<serde_json::Value>,
}

impl GateAnswer {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "message": self.message,
            "applied": self.applied,
            "plan": self.plan,
        })
    }
}

/// What the daemon's answer to a gate write means: what it did, or the
/// refusal as the sentence a person reads.
///
/// A gate that is not valid (400), a plan or phase that is not there (404), a
/// closed phase or a phase without a gate (409) and a gate whose model cannot
/// answer it in this chat (422) are the daemon's sentence alone: none is
/// about the user-action key. A 403 keeps the key advice.
pub fn write_answer(session_id: &str, request: &str, code: u16, body: &str) -> Result<GateAnswer> {
    match code {
        200 => {
            let value: serde_json::Value = parse_json(body, "a phase-gate answer")?;
            let message = value
                .get("message")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow!("the daemon's answer to {request} said nothing"))?
                .to_string();
            Ok(GateAnswer {
                message,
                applied: value
                    .get("applied")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
                plan: value.get("plan").filter(|plan| !plan.is_null()).cloned(),
            })
        }
        400 | 404 | 409 | 422 => match refusal_sentence(body) {
            Some(sentence) => Err(anyhow!("{sentence}")),
            None if code == 404 => Err(anyhow!("Session '{session_id}' not found.")),
            None => Err(anyhow!(
                "the daemon refused the change ({code}) without saying why. Nothing was changed."
            )),
        },
        403 => {
            let sentence = refusal_sentence(body).unwrap_or_else(|| "no reason given".to_string());
            let sentence = sentence.trim_end().trim_end_matches('.');
            let nothing = if sentence.contains("Nothing was changed") {
                ""
            } else {
                " Nothing was changed."
            };
            Err(anyhow!(
                "the daemon refused the change: {sentence}.{nothing} A daemon started with a \
                 user-action key wants it: pass --user-action-key-stdin and pipe the key as the \
                 first line of stdin, or run this from a terminal."
            ))
        }
        other => Err(daemon_error(session_id, request, other, body)),
    }
}

async fn daemon_roster(session_id: &str, auth: &DaemonAuth) -> Result<SessionSubagents> {
    let path = format!("/sessions/{session_id}/subagents");
    let (code, body) = daemon_get_json(&path, auth, configured_port(), REQUEST_DEADLINE).await?;
    if code != 200 {
        return Err(daemon_error(
            session_id,
            &format!("GET {path}"),
            code,
            &body,
        ));
    }
    parse_roster(&body)
}

async fn daemon_write(session_id: &str, edit: &GateEdit, auth: DaemonAuth) -> Result<GateAnswer> {
    let (method, path, body) = request_for(session_id, edit)?;
    let ((code, answer), _) = protected_write_with_key_if_wanted(
        method,
        &path,
        &body,
        auth,
        configured_port(),
        REQUEST_DEADLINE,
    )
    .await?;
    write_answer(session_id, &format!("{method} {path}"), code, &answer)
}

/// The local session store, once the chat is within this terminal's reach,
/// with the privacy switch and the capability as this command sampled them
/// (once, for the reach rule, the listing and the model check alike).
pub struct LocalStore {
    pub manager: Arc<SessionManager>,
    pub enforced: bool,
    pub capability: ProviderTier,
}

/// [`LocalStore`] for `session_id`: refused with [`LOCAL_OUT_OF_REACH`] when
/// the chat is private and `capability` may not read it.
pub async fn local_store_at(
    manager: Arc<SessionManager>,
    session_id: &str,
    capability: ProviderTier,
    enforced: bool,
) -> Result<LocalStore> {
    check_local_reach(
        &manager,
        session_id,
        capability,
        enforced,
        LOCAL_OUT_OF_REACH,
    )
    .await?;
    Ok(LocalStore {
        manager,
        enforced,
        capability,
    })
}

async fn local_store(session_id: &str) -> Result<LocalStore> {
    let manager = Arc::new(SessionManager::instance());
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let capability = crate::session::privacy::caller_capability().await;
    local_store_at(manager, session_id, capability, enforced).await
}

/// The chat's plan from the local store, as the side panel's route would
/// show it to this terminal.
pub async fn local_plan(
    store: &LocalStore,
    session_id: &str,
) -> Result<Option<OrchestrationSnapshot>> {
    Ok(super::session_subagents::from_store(
        &store.manager,
        session_id,
        store.capability,
        store.enforced,
    )
    .await?
    .plan)
}

/// `text` without the full stop it ends with, so it can sit inside a
/// sentence.
fn without_full_stop(text: &str) -> &str {
    let text = text.trim_end();
    text.strip_suffix('.').unwrap_or(text)
}

/// A gate write in the local store, as the daemon's routes make it: a new
/// gate is checked as the route checks it, and its model must be one this
/// chat may use (asked of `service`, which sends nothing of the chat), before
/// the person transition stores it. `Err` is the sentence a person reads,
/// and then nothing was changed.
pub async fn write_locally(
    service: &DecisionService,
    store: &LocalStore,
    session_id: &str,
    edit: GateEdit,
) -> Result<PersonGateOutcome> {
    let manager = &store.manager;
    let written = match edit {
        GateEdit::Set { phase, spec } => {
            let gate = orchestration_plan::validate_person_gate(&phase, &spec)
                .map_err(|e| anyhow!("{e}"))?;
            let session =
                DecisionSession::load(Arc::clone(manager), session_id, store.enforced).await?;
            if let Err(error) = service
                .check_model_use(
                    &session,
                    gate.model.as_ref(),
                    &gate.questions,
                    ModelSource::Setting,
                )
                .await
            {
                bail!(
                    "The gate on phase \"{phase}\" cannot be answered in this chat. {}. Nothing \
                     was changed.",
                    without_full_stop(&super::decide::terminal_advice(&error.to_string()))
                );
            }
            orchestration_plan::person_set_gate(manager, session_id, &phase, gate).await
        }
        GateEdit::Remove { phase } => {
            orchestration_plan::person_remove_gate(manager, session_id, &phase).await
        }
        GateEdit::Decide { phase, choice } => {
            orchestration_plan::person_gate_decision(manager, session_id, &phase, choice).await
        }
    };
    written.map_err(|refusal| anyhow!("{}", refusal.message()))
}

fn print_plan(session_id: &str, plan: Option<&OrchestrationSnapshot>, format: GatesFormat) {
    match format {
        GatesFormat::Json => super::print_line(
            serde_json::to_string_pretty(&plan).unwrap_or_else(|_| "null".to_string()),
        ),
        GatesFormat::Text => {
            for line in render_gates(session_id, plan) {
                super::print_line(line);
            }
        }
    }
}

fn print_answer(answer: &GateAnswer, format: GatesFormat) -> Result<()> {
    match format {
        GatesFormat::Json => super::print_line(serde_json::to_string_pretty(&answer.json())?),
        GatesFormat::Text => super::print_line(&answer.message),
    }
    Ok(())
}

/// `biorouter session gates <SESSION_ID> ...`.
pub async fn handle_session_gates(
    session_id: &str,
    command: Option<GatesCommand>,
    format: GatesFormat,
    user_action_key_stdin: bool,
) -> Result<()> {
    // Read before anything is asked, so a mistake costs no request.
    let edit = edit_of(command.unwrap_or(GatesCommand::List))?;
    let source = reach(user_action_key_stdin).await?;
    match source {
        Reach::Daemon(auth) => match edit {
            None => {
                let roster = daemon_roster(session_id, &auth).await?;
                print_plan(session_id, roster.plan.as_ref(), format);
            }
            Some(edit) => {
                let answer = daemon_write(session_id, &edit, auth)
                    .await
                    .map_err(super::decide::advise)?;
                print_answer(&answer, format)?;
            }
        },
        Reach::Local(why) => {
            eprintln!("note: read from the local session store ({why}).");
            let store = local_store(session_id).await?;
            match edit {
                None => {
                    let plan = local_plan(&store, session_id).await?;
                    print_plan(session_id, plan.as_ref(), format);
                }
                Some(edit) => {
                    let outcome =
                        write_locally(biorouter::decision::global(), &store, session_id, edit)
                            .await?;
                    let plan = match format {
                        GatesFormat::Json => local_plan(&store, session_id)
                            .await
                            .ok()
                            .flatten()
                            .and_then(|plan| serde_json::to_value(plan).ok()),
                        GatesFormat::Text => None,
                    };
                    print_answer(
                        &GateAnswer {
                            message: outcome.message,
                            applied: outcome.applied,
                            plan,
                        },
                        format,
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::agents::orchestration_plan::{
        OrchestratePhaseInput, PhaseCounts, PhaseGateInput, PhaseKind, PlanPhaseInput, PLAN_STATE,
        PLAN_VERSION,
    };
    use biorouter::decision::types::{DecisionKindTag, ModelVerdicts};
    use biorouter::decision::{
        DecisionAdapter, DecisionCaps, DecisionError, DecisionModel, DecisionModelEntry,
        DecisionModelSource, DecisionRequest, DecisionResult, DecisionUseGate,
    };
    use biorouter::privacy::{ModelAffiliation, SessionClassification};
    use biorouter::session::SessionType;
    use clap::Parser;

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        command: GatesCommand,
    }

    fn parse(argv: &[&str]) -> Result<GatesCommand, clap::Error> {
        let mut full = vec!["gates"];
        full.extend_from_slice(argv);
        Harness::try_parse_from(full).map(|h| h.command)
    }

    fn set(argv: &[&str]) -> SetGate {
        let mut full = vec!["set"];
        full.extend_from_slice(argv);
        match parse(&full).unwrap() {
            GatesCommand::Set(set) => set,
            _ => panic!("not set"),
        }
    }

    #[test]
    fn every_subcommand_parses() {
        assert!(matches!(parse(&["list"]).unwrap(), GatesCommand::List));
        let set = set(&[
            "research",
            "--ask",
            "facts=Does the hand-off list two facts about the moon?",
            "--ask",
            "Sources Cited = Are the sources named?",
            "--threshold",
            "0.3",
            "--attempts",
            "2",
            "--model",
            "ollama_decision/tev1:0.8b",
            "--allow-uncalibrated",
        ]);
        assert_eq!(set.phase, "research");
        assert_eq!(
            set.asks,
            vec![
                (
                    "facts".to_string(),
                    "Does the hand-off list two facts about the moon?".to_string()
                ),
                (
                    "sources-cited".to_string(),
                    "Are the sources named?".to_string()
                ),
            ]
        );
        assert_eq!(set.threshold, Some(0.3));
        assert_eq!(set.attempts, Some(2));
        assert_eq!(set.model.as_deref(), Some("ollama_decision/tev1:0.8b"));
        assert!(set.allow_uncalibrated);
        for (word, choice) in [
            ("pass", Some(PersonGateChoice::Pass)),
            ("skip", Some(PersonGateChoice::Skip)),
            ("retry", Some(PersonGateChoice::Retry)),
            ("remove", None),
        ] {
            let edit = edit_of(parse(&[word, "write"]).unwrap()).unwrap().unwrap();
            match (edit, choice) {
                (GateEdit::Decide { phase, choice: got }, Some(want)) => {
                    assert_eq!((phase.as_str(), got), ("write", want), "{word}")
                }
                (GateEdit::Remove { phase }, None) => assert_eq!(phase, "write"),
                (other, _) => panic!("{word} read as {other:?}"),
            }
        }
        // A gate needs a question, a phase and sane terms.
        assert!(parse(&["set", "research"]).is_err(), "no --ask");
        assert!(parse(&["set", "--ask", "a=b"]).is_err(), "no phase");
        assert!(parse(&["pass"]).is_err(), "no phase");
        assert!(parse(&["set", "r", "--ask", "a=b", "--attempts", "6"]).is_err());
        assert!(parse(&["set", "r", "--ask", "a=b", "--threshold", "1.5"]).is_err());
        assert!(parse(&["teleport", "r"]).is_err());
        assert!(edit_of(GatesCommand::List).unwrap().is_none());
    }

    #[test]
    fn ask_takes_a_name_and_a_question_and_slugs_the_name() {
        assert_eq!(
            parse_ask("facts=Does it list two facts?").unwrap(),
            ("facts".into(), "Does it list two facts?".into())
        );
        assert_eq!(
            parse_ask("  Two Facts!  =  Does it?  ").unwrap(),
            ("two-facts".into(), "Does it?".into())
        );
        assert_eq!(
            parse_ask("tests_pass=Is 1 = 1 shown?").unwrap(),
            ("tests_pass".into(), "Is 1 = 1 shown?".into()),
            "only the first `=` divides"
        );
        assert!(parse_ask("Does it list two facts?")
            .unwrap_err()
            .contains("NAME=QUESTION"));
        assert!(
            parse_ask("Is x=1 true?").is_ok(),
            "a short name before `=` is a name"
        );
        assert!(
            parse_ask("Is x?=1 true").is_err(),
            "a question is not a name"
        );
        assert!(parse_ask("!!=Does it?").is_err(), "a name needs a letter");
        assert!(parse_ask("facts=  ").is_err(), "a question is needed");
        let long = format!("{}=q", "n".repeat(MAX_QUESTION_NAME_CHARS + 1));
        assert!(parse_ask(&long).is_err());
    }

    #[test]
    fn a_gate_is_built_and_checked_before_anything_is_sent() {
        let (phase, spec) = build_spec(&set(&[
            "research",
            "--ask",
            "facts=Two facts?",
            "--threshold",
            "0.3",
            "--model",
            "ollama_decision/tev1:0.8b",
        ]))
        .unwrap();
        assert_eq!(phase, "research");
        assert_eq!(
            spec.questions.get("facts").map(String::as_str),
            Some("Two facts?")
        );
        assert_eq!(spec.threshold, Some(0.3), "the person has no 0.50 floor");
        assert_eq!(spec.max_attempts, None);
        assert_eq!(spec.allow_uncalibrated, None);
        assert_eq!(
            spec.model,
            Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"))
        );
        // The body is the route's own shape, camelCase.
        let (method, path, body) = request_for("s1", &GateEdit::Set { phase, spec }).unwrap();
        assert_eq!(
            (method, path.as_str()),
            ("PUT", "/sessions/s1/phase-gates/research")
        );
        let body: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["questions"]["facts"], "Two facts?");
        assert_eq!(body["threshold"], 0.3);
        assert_eq!(body["model"]["provider"], "ollama_decision");
        assert!(body.get("maxAttempts").is_none());

        let duplicate = build_spec(&set(&["r", "--ask", "a=One?", "--ask", "A=Two?"]));
        assert!(duplicate
            .unwrap_err()
            .to_string()
            .contains("two --ask questions"));
        let model = build_spec(&set(&["r", "--ask", "a=One?", "--model", "tev1"]));
        assert!(model.unwrap_err().to_string().contains("provider/model"));
        let phase = build_spec(&set(&["Research", "--ask", "a=One?"]));
        assert_eq!(phase.unwrap_err().to_string(), NOT_A_PHASE_ID);
        let many: Vec<String> = (0..=MAX_GATE_QUESTIONS)
            .flat_map(|i| ["--ask".to_string(), format!("q{i}=Question {i}?")])
            .collect();
        let mut argv = vec!["r"];
        argv.extend(many.iter().map(String::as_str));
        assert!(build_spec(&set(&argv)).is_err());

        let (method, path, body) = request_for(
            "s1",
            &GateEdit::Decide {
                phase: "write".into(),
                choice: PersonGateChoice::Retry,
            },
        )
        .unwrap();
        assert_eq!(
            (method, path.as_str(), body.as_str()),
            (
                "POST",
                "/sessions/s1/phase-gates/write/decision",
                r#"{"decision":"retry"}"#
            )
        );
        let (method, path, body) = request_for(
            "s1",
            &GateEdit::Remove {
                phase: "write".into(),
            },
        )
        .unwrap();
        assert_eq!(
            (method, path.as_str(), body.as_str()),
            ("DELETE", "/sessions/s1/phase-gates/write", "")
        );
    }

    #[test]
    fn the_daemons_answers_read_as_the_person_should_read_them() {
        let ok = write_answer(
            "s1",
            "POST x",
            200,
            r#"{"message":"Your pass stands: phase \"research\" completes.","applied":false,"plan":null}"#,
        )
        .unwrap();
        assert_eq!(
            ok,
            GateAnswer {
                message: "Your pass stands: phase \"research\" completes.".into(),
                applied: false,
                plan: None,
            }
        );
        // Not about the key: the sentence alone.
        for code in [400, 404, 409, 422] {
            let sentence = "Phase \"x\" is already completed. Nothing was changed.";
            let error = write_answer("s1", "PUT x", code, sentence).unwrap_err();
            assert_eq!(error.to_string(), sentence, "{code}");
        }
        assert_eq!(
            write_answer("s1", "PUT x", 404, "")
                .unwrap_err()
                .to_string(),
            "Session 's1' not found."
        );
        // About the key: the advice, and "Nothing was changed" once.
        let refused = write_answer(
            "s1",
            "PUT x",
            403,
            "Changing a phase's gate is the user's decision. Nothing was changed.",
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("--user-action-key-stdin"), "{refused}");
        assert_eq!(
            refused.matches("Nothing was changed").count(),
            1,
            "{refused}"
        );
        assert!(write_answer("s1", "PUT x", 401, "")
            .unwrap_err()
            .to_string()
            .contains("secret key"));
    }

    fn snapshot_gate(owner: GateOwner) -> PhaseGateSnapshot {
        PhaseGateSnapshot {
            threshold: 0.3,
            question_count: 1,
            attempts: 0,
            max_attempts: 3,
            last_passed: None,
            last_min_p: None,
            blocked: false,
            owner,
            questions: [(
                "facts".to_string(),
                "Does the hand-off list two facts?".to_string(),
            )]
            .into_iter()
            .collect(),
            model: None,
            model_adapter: None,
            allow_uncalibrated: false,
            last_error: None,
            person_decision: None,
            retries_granted: 0,
            needs_person: false,
            holds: Vec::new(),
        }
    }

    fn snapshot_phase(id: &str, status: PhaseStatus) -> PhaseSnapshot {
        PhaseSnapshot {
            id: id.to_string(),
            title: format!("{id} title"),
            goal: None,
            kind: PhaseKind::Research,
            status,
            depends_on: Vec::new(),
            agent_ids: Vec::new(),
            counts: PhaseCounts::default(),
            handoff: None,
            started_at: None,
            finished_at: None,
            gate: None,
            held_by: None,
        }
    }

    #[test]
    fn the_list_says_who_set_each_gate_and_what_waits() {
        let mut research = snapshot_phase("research", PhaseStatus::InProgress);
        let mut gate = snapshot_gate(GateOwner::Person);
        gate.holds = vec!["write".into()];
        research.gate = Some(gate);
        let mut write = snapshot_phase("write", PhaseStatus::Pending);
        write.held_by = Some("research".into());
        let mut review = snapshot_phase("review", PhaseStatus::Blocked);
        let mut agent = snapshot_gate(GateOwner::Agent);
        agent.attempts = 3;
        agent.last_passed = Some(false);
        agent.last_min_p = Some(0.42);
        agent.blocked = true;
        agent.needs_person = true;
        agent.model = Some("ollama_decision/tev1:0.8b".into());
        agent.retries_granted = 1;
        agent.last_error = Some("Ollama is not answering. (decision_models lists them)".into());
        review.gate = Some(agent);
        let mut done = snapshot_phase("done", PhaseStatus::Skipped);
        let mut workflow = snapshot_gate(GateOwner::Workflow);
        workflow.person_decision = Some(PersonGateAction::Skip);
        done.gate = Some(workflow);
        let plan = OrchestrationSnapshot {
            title: "Moon".into(),
            goal: None,
            revision: 4,
            phases: vec![research, write, review, done],
        };
        let text = render_gates("s1", Some(&plan)).join("\n");
        assert!(
            text.starts_with(
                "Phase gates of session s1, plan \"Moon\" (revision 4): 3 of 4 phases gated."
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "     gate: 1 question at P(yes) >= 0.30, 0 of 3 attempts used, set by you\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("       - facts: Does the hand-off list two facts?"),
            "{text}"
        );
        assert!(text.contains("     holds write"), "{text}");
        assert!(
            text.contains("  2. ○ write  write title  pending · waits for research's gate\n"),
            "{text}"
        );
        assert!(text.contains("     no gate"), "{text}");
        assert!(
            text.contains(&format!(
                "not passed last time (lowest P(yes) 0.42), blocked, set by the agent\n     \
                 asked of tev1:0.8b · {} · given its attempts again 1 time\n",
                super::super::decide::model_place("ollama_decision", "tev1:0.8b").unwrap()
            )),
            "{text}"
        );
        assert!(
            text.contains(
                "     its last check decided nothing: Ollama is not answering. \
                 (`biorouter models decision list` shows them)"
            ),
            "{text}"
        );
        assert!(
            text.contains("needs you: biorouter session gates s1 retry|pass|skip|remove review"),
            "{text}"
        );
        assert!(
            text.contains("set by the workflow, skipped by you"),
            "{text}"
        );
        assert_eq!(
            render_gates("s1", None)[0],
            "Session s1 has no phase plan, so it has no gates."
        );
    }

    // ── the local store ─────────────────────────────────────────────────────

    struct Allow;

    #[async_trait::async_trait]
    impl DecisionUseGate for Allow {
        async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
            Ok(())
        }
    }

    fn local_entry() -> DecisionModelEntry {
        DecisionModelEntry {
            provider: "test_decision".into(),
            model: "local".into(),
            display_name: "local".into(),
            context_tokens: 8_192,
            kinds: vec![DecisionKindTag::Binary],
            tier: ProviderTier::Private,
            affiliation: Some(ModelAffiliation::Local),
            adapter: DecisionAdapter::SystemOne,
            local: true,
            load_refusal: None,
        }
    }

    /// A model that is listed and opens, and is never asked anything: the
    /// save-time check sends nothing.
    struct Listed;

    #[async_trait::async_trait]
    impl DecisionModel for Listed {
        fn model_ref(&self) -> DecisionModelRef {
            local_entry().model_ref()
        }
        fn display_name(&self) -> String {
            "local".into()
        }
        fn tier(&self) -> ProviderTier {
            ProviderTier::Private
        }
        fn affiliation(&self) -> Option<ModelAffiliation> {
            Some(ModelAffiliation::Local)
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: vec![DecisionKindTag::Binary],
                max_questions: 64,
                context_tokens: 8_192,
                max_concurrency: 2,
                max_request_bytes: None,
                local: true,
            }
        }
        async fn decide(&self, _: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            panic!("a save-time check sent a decision");
        }
    }

    /// One local model, `test_decision/local`, or none at all.
    struct Source {
        listed: bool,
    }

    #[async_trait::async_trait]
    impl DecisionModelSource for Source {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            if self.listed {
                vec![local_entry()]
            } else {
                Vec::new()
            }
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            if self.listed && local_entry().answers_to(wanted) {
                Ok(Arc::new(Listed))
            } else {
                Err(DecisionError::NoModel(format!(
                    "No decision model `{}` is listed.",
                    wanted.label()
                )))
            }
        }
    }

    fn service(listed: bool) -> DecisionService {
        DecisionService::new(Arc::new(Source { listed }))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None)
    }

    /// A chat whose plan is research, write and review, none gated.
    async fn chat_with_plan(manager: &SessionManager, dir: &std::path::Path) -> String {
        let chat = manager
            .create_session(dir.to_path_buf(), "moon".into(), SessionType::User)
            .await
            .unwrap();
        let phase = |id: &str, gate: Option<PhaseGateInput>| OrchestratePhaseInput {
            phase: PlanPhaseInput {
                id: id.into(),
                title: format!("{id} title"),
                kind: Some("research".into()),
                goal: None,
                depends_on: Vec::new(),
            },
            gate,
            monitors: None,
        };
        let plan = orchestration_plan::apply_workflow_plan(
            "Moon",
            None,
            vec![
                phase("research", None),
                phase("write", None),
                phase("review", None),
            ],
            chrono::Utc::now(),
        )
        .unwrap();
        manager
            .update_extension_state(&chat.id, PLAN_STATE, PLAN_VERSION, |_| {
                Ok(orchestration_plan::plan_record(&plan))
            })
            .await
            .unwrap();
        chat.id
    }

    /// The local path end to end: set a gate (checked against the chat's
    /// models), list it as yours and holding what follows, pass it, list it
    /// passed by you with nothing held.
    #[tokio::test]
    async fn a_local_set_pass_and_list_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let chat = chat_with_plan(&manager, dir.path()).await;
        let store = local_store_at(manager.clone(), &chat, ProviderTier::Public, true)
            .await
            .unwrap();
        let (phase, spec) = build_spec(&set(&[
            "research",
            "--ask",
            "facts=Does the hand-off list two facts about the moon?",
            "--threshold",
            "0.3",
            "--model",
            "test_decision/local",
        ]))
        .unwrap();

        let said = write_locally(&service(true), &store, &chat, GateEdit::Set { phase, spec })
            .await
            .unwrap();
        assert!(said.applied);
        assert!(
            said.message
                .starts_with("Phase \"research\" now has your gate"),
            "{}",
            said.message
        );
        let plan = local_plan(&store, &chat).await.unwrap().unwrap();
        let gate = plan.phases[0].gate.as_ref().expect("research is gated");
        assert_eq!(gate.owner, GateOwner::Person);
        assert_eq!(gate.threshold, 0.3);
        assert_eq!(plan.phases[1].held_by.as_deref(), Some("research"));
        let text = render_gates(&chat, Some(&plan)).join("\n");
        assert!(text.contains("set by you"), "{text}");
        assert!(text.contains("waits for research's gate"), "{text}");

        let passed = write_locally(
            &service(true),
            &store,
            &chat,
            GateEdit::Decide {
                phase: "research".into(),
                choice: PersonGateChoice::Pass,
            },
        )
        .await
        .unwrap();
        assert!(
            passed.applied,
            "no subagent works in it: {}",
            passed.message
        );
        assert!(
            passed
                .message
                .starts_with("Phase \"research\" is completed: you passed its gate."),
            "{}",
            passed.message
        );
        let plan = local_plan(&store, &chat).await.unwrap().unwrap();
        assert_eq!(plan.phases[0].status, PhaseStatus::Completed);
        assert_eq!(
            plan.phases[0].gate.as_ref().unwrap().person_decision,
            Some(PersonGateAction::Pass)
        );
        assert_eq!(plan.phases[1].held_by, None, "write is released");
        let text = render_gates(&chat, Some(&plan)).join("\n");
        assert!(text.contains("set by you, passed by you"), "{text}");
        assert!(!text.contains("waits for"), "{text}");

        // The person's words for a phase with no gate.
        let refused = write_locally(
            &service(true),
            &store,
            &chat,
            GateEdit::Remove {
                phase: "write".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            refused.to_string(),
            "Phase \"write\" has no gate. Nothing was changed."
        );
    }

    /// A gate whose model this chat cannot use is refused as the route
    /// refuses it, and nothing is stored.
    #[tokio::test]
    async fn a_local_set_checks_the_gates_model_first() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let chat = chat_with_plan(&manager, dir.path()).await;
        let store = local_store_at(manager.clone(), &chat, ProviderTier::Public, true)
            .await
            .unwrap();
        let (phase, spec) = build_spec(&set(&[
            "research",
            "--ask",
            "facts=Two facts?",
            "--model",
            "test_decision/local",
        ]))
        .unwrap();
        let error = write_locally(
            &service(false),
            &store,
            &chat,
            GateEdit::Set { phase, spec },
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("The gate on phase \"research\" cannot be answered in this chat. "),
            "{error}"
        );
        assert!(
            error.to_string().ends_with(". Nothing was changed."),
            "{error}"
        );
        let plan = local_plan(&store, &chat).await.unwrap().unwrap();
        assert!(plan.phases[0].gate.is_none(), "nothing was stored");
    }

    /// The local path applies the route's tier rule: a terminal on a public
    /// model is refused a private chat's gates, in this command's words.
    #[tokio::test]
    async fn a_public_terminal_is_refused_a_private_chats_gates() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let chat = chat_with_plan(&manager, dir.path()).await;
        manager
            .update(&chat)
            .provider_name("versa_azure")
            .model_config(biorouter::model::ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
        let refused = local_store_at(manager.clone(), &chat, ProviderTier::Public, true)
            .await
            .err()
            .expect("a public terminal may not read a private chat");
        assert_eq!(refused.to_string(), LOCAL_OUT_OF_REACH);
        assert!(
            local_store_at(manager.clone(), &chat, ProviderTier::Private, true)
                .await
                .is_ok()
        );
        assert!(
            local_store_at(manager.clone(), &chat, ProviderTier::Public, false)
                .await
                .is_ok(),
            "with the master switch off nothing is refused, as on the route"
        );
    }
}
