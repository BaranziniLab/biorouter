//! The status `biorouter run` exits with, and the one line it prints on
//! stderr, when the chat's supervision ended the run short of its work
//! (live QA 2026-11-10, H-6 / D-4).
//!
//! Two cases count, and only these two:
//!
//! * **A monitor stopped the turn.** The monitor's notice tells a person in a
//!   chat to send a message to carry on; a headless run has ended by then, so
//!   the line names the commands that carry the chat on.
//! * **The plan ended with a phase its gate holds, or a phase left blocked.**
//!   An open phase whose gate did not pass at its last check, or a phase that
//!   is blocked (marked so, or by its gate), is work the run did not finish.
//!
//! Both exit with [`EXIT_CODE`], 3, so a script can tell "the run was stopped
//! or is waiting on a gate" from a run that finished (0), from a command that
//! needs a terminal (2, [`super::needs_terminal::EXIT_CODE`]), from a generic
//! failure (1) and from a turn that aborted (70 and up,
//! `biorouter::agents::turn_abort::exit`). A run that finishes normally,
//! including one whose plan simply has phases still pending, still exits 0.

use biorouter::agents::orchestration_plan::{OrchestrationSnapshot, PhaseSnapshot, PhaseStatus};

use super::decide::clip;

/// The exit status of a run its supervision stopped or holds.
pub const EXIT_CODE: u8 = 3;

/// A `biorouter run` whose supervision ended it short of its work. A type, so
/// `main` can print the sentence alone (no `Error:` framing: nothing failed)
/// and exit with [`EXIT_CODE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisionStopped(String);

impl SupervisionStopped {
    /// The one line `main` prints on stderr.
    pub fn sentence(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SupervisionStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SupervisionStopped {}

/// How the chat carries on, after the sentence that says what stopped. A
/// `--no-session` run kept no chat to go back to.
fn carry_on(session_id: &str, no_session: bool) -> String {
    if no_session {
        return "It ran with --no-session, so there is no chat to carry on.".to_string();
    }
    format!(
        "To carry on, send the chat a message: biorouter run --resume --session-id {session_id} \
         --text \"<message>\", or open it with biorouter session --resume --session-id \
         {session_id}."
    )
}

/// The line for a run whose turn the monitor named `monitor` stopped.
pub fn monitor_stopped(monitor: &str, session_id: &str, no_session: bool) -> SupervisionStopped {
    SupervisionStopped(format!(
        // R3-F6: a monitor's name in single quotes, as its notice gives it.
        "Monitor '{}' stopped this run (exit status {EXIT_CODE}). {}",
        clip(monitor, 60),
        carry_on(session_id, no_session)
    ))
}

fn is_open(phase: &PhaseSnapshot) -> bool {
    matches!(phase.status, PhaseStatus::Pending | PhaseStatus::InProgress)
}

/// Blocked: marked blocked, or an open phase its gate blocked.
fn is_blocked(phase: &PhaseSnapshot) -> bool {
    phase.status == PhaseStatus::Blocked
        || (is_open(phase)
            && phase
                .gate
                .as_ref()
                .is_some_and(|gate| gate.blocked || gate.needs_person))
}

/// Held by its gate: an open phase whose gate did not pass at its last check.
/// A gate never asked yet holds nothing a run can be blamed for: the run
/// simply did not get that far.
fn is_held(phase: &PhaseSnapshot) -> bool {
    is_open(phase)
        && phase
            .gate
            .as_ref()
            .is_some_and(|gate| gate.last_passed == Some(false))
}

/// `phase "report"`, `phases "a" and "b"`, `phases "a", "b" and "c"`.
fn phases_words(ids: &[&str]) -> String {
    let quoted: Vec<String> = ids
        .iter()
        .map(|id| format!("\"{}\"", clip(id, 40)))
        .collect();
    match quoted.as_slice() {
        [one] => format!("phase {one}"),
        [rest @ .., last] => format!("phases {} and {last}", rest.join(", ")),
        [] => String::new(),
    }
}

/// The line for a run whose plan ended with a phase held by its gate or left
/// blocked, or `None` when it did not.
pub fn plan_held(
    plan: &OrchestrationSnapshot,
    session_id: &str,
    no_session: bool,
) -> Option<SupervisionStopped> {
    let blocked: Vec<&str> = plan
        .phases
        .iter()
        .filter(|phase| is_blocked(phase))
        .map(|phase| phase.id.as_str())
        .collect();
    let held: Vec<&str> = plan
        .phases
        .iter()
        .filter(|phase| !is_blocked(phase) && is_held(phase))
        .map(|phase| phase.id.as_str())
        .collect();
    if blocked.is_empty() && held.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !blocked.is_empty() {
        parts.push(format!("{} blocked", phases_words(&blocked)));
    }
    if !held.is_empty() {
        parts.push(format!(
            "{} held by {} gate, which did not pass",
            phases_words(&held),
            if held.len() == 1 { "its" } else { "their" }
        ));
    }
    let see_gates = if no_session {
        String::new()
    } else {
        format!("See the gates with biorouter session gates {session_id}. ")
    };
    Some(SupervisionStopped(format!(
        "This run ended with {} (exit status {EXIT_CODE}). {see_gates}{}",
        parts.join(" and "),
        carry_on(session_id, no_session)
    )))
}

/// A headless turn's result for a caller that is not `biorouter run`: the
/// extension-failure debug chat and the bench adapter. For them a turn its
/// supervision stopped still ran to an end, so it is `Ok`; the sentence is
/// handed back for a caller that wants to show it. Only `run` turns it into
/// exit status 3. Every other error passes through unchanged.
pub fn ran_to_an_end(result: anyhow::Result<()>) -> anyhow::Result<Option<SupervisionStopped>> {
    match result {
        Ok(()) => Ok(None),
        Err(e) => match e.downcast::<SupervisionStopped>() {
            Ok(stopped) => Ok(Some(stopped)),
            Err(other) => Err(other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::agents::orchestration_plan::{
        GateOwner, PhaseCounts, PhaseGateSnapshot, PhaseKind,
    };

    fn gate() -> PhaseGateSnapshot {
        PhaseGateSnapshot {
            threshold: 0.8,
            question_count: 1,
            attempts: 0,
            max_attempts: 3,
            last_passed: None,
            last_min_p: None,
            blocked: false,
            owner: GateOwner::Workflow,
            questions: Default::default(),
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

    fn phase(id: &str, status: PhaseStatus, gate: Option<PhaseGateSnapshot>) -> PhaseSnapshot {
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
            gate,
            held_by: None,
        }
    }

    fn plan(phases: Vec<PhaseSnapshot>) -> OrchestrationSnapshot {
        OrchestrationSnapshot {
            title: "Screen".into(),
            goal: None,
            revision: 1,
            phases,
        }
    }

    /// A monitor's stop names the monitor, the exit status and the commands
    /// that carry the chat on, never the chat notice's "Send a message".
    #[test]
    fn a_run_a_monitor_stopped_says_so_and_how_to_carry_on() {
        let line = monitor_stopped("Reading notes", "20261110_5", false).to_string();
        assert!(
            line.starts_with("Monitor 'Reading notes' stopped this run (exit status 3)."),
            "{line}"
        );
        assert!(
            line.contains("biorouter run --resume --session-id 20261110_5 --text"),
            "{line}"
        );
        assert!(line.contains("biorouter session --resume --session-id 20261110_5"));
        assert!(!line.contains("Send a message"), "{line}");
        assert!(!line.contains('\n'), "one line: {line}");
        let ephemeral = monitor_stopped("x", "s", true).to_string();
        assert!(ephemeral.contains("--no-session"), "{ephemeral}");
        assert!(!ephemeral.contains("--resume"), "{ephemeral}");
    }

    /// H-6: a phase marked blocked, a phase its gate blocked, and an open
    /// phase whose gate did not pass each make the run held; finished,
    /// skipped and never-checked phases do not.
    #[test]
    fn a_plan_ending_held_or_blocked_is_named_and_a_finished_one_is_not() {
        let mut failed = gate();
        failed.last_passed = Some(false);
        failed.attempts = 1;
        let mut exhausted = gate();
        exhausted.last_passed = Some(false);
        exhausted.blocked = true;
        exhausted.needs_person = true;

        // The live case: the agent marked the gated phase blocked.
        let marked = plan(vec![
            phase("screen", PhaseStatus::Completed, None),
            phase("report", PhaseStatus::Blocked, Some(failed.clone())),
        ]);
        let line = plan_held(&marked, "s1", false).unwrap().to_string();
        assert_eq!(
            line,
            "This run ended with phase \"report\" blocked (exit status 3). See the gates with \
             biorouter session gates s1. To carry on, send the chat a message: biorouter run \
             --resume --session-id s1 --text \"<message>\", or open it with biorouter session \
             --resume --session-id s1."
        );

        let both = plan(vec![
            phase("screen", PhaseStatus::InProgress, Some(failed.clone())),
            phase("verify", PhaseStatus::InProgress, Some(exhausted)),
            phase("write", PhaseStatus::Pending, Some(failed)),
        ]);
        let line = plan_held(&both, "s1", true).unwrap().to_string();
        assert!(
            line.starts_with(
                "This run ended with phase \"verify\" blocked and phases \"screen\" and \"write\" \
                 held by their gate, which did not pass (exit status 3)."
            ),
            "{line}"
        );
        assert!(!line.contains("session gates"), "{line}");

        // A finished run, a skipped phase, a gate that passed or was never
        // asked, and a completed phase whose last check had failed: exit 0.
        let mut passed = gate();
        passed.last_passed = Some(true);
        let mut failed_then_done = gate();
        failed_then_done.last_passed = Some(false);
        failed_then_done.blocked = true;
        let done = plan(vec![
            phase("screen", PhaseStatus::Completed, Some(passed)),
            phase("check", PhaseStatus::Skipped, Some(failed_then_done)),
            phase("write", PhaseStatus::Pending, Some(gate())),
            phase("polish", PhaseStatus::Pending, None),
        ]);
        assert_eq!(plan_held(&done, "s1", false), None);
    }

    #[test]
    fn a_stopped_turn_ran_to_an_end_for_callers_other_than_run() {
        assert_eq!(ran_to_an_end(Ok(())).unwrap(), None);

        let stopped = monitor_stopped("watchdog", "s1", true);
        let got = ran_to_an_end(Err(anyhow::Error::new(stopped.clone()))).unwrap();
        assert_eq!(got, Some(stopped));

        let other = ran_to_an_end(Err(anyhow::anyhow!("provider down"))).unwrap_err();
        assert_eq!(other.to_string(), "provider down");
    }
}
