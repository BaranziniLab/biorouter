//! Session goals (`/goal`, Claude Code-style).
//!
//! `/goal <condition>` registers a session-scoped Stop hook whose LLM judge
//! evaluates the condition every time the agent tries to finish its turn. If
//! the condition is not met, the stop is blocked and the judge's feedback is
//! fed back to the model, so the agent keeps working until the condition holds.
//!
//! Robustness (see the `/Users/wgu/Downloads/hook failure.json` case that
//! motivated it — a "summarize all 400 sites in chat" goal that looped
//! forever):
//!
//! 1. **Truncation-aware judging.** The judge only ever sees a recent
//!    `transcript_tail`, never the whole conversation. A goal that emits a
//!    large volume in-chat will always look "incomplete" in the tail, so the
//!    judge rule explicitly forbids treating "output not fully visible in this
//!    excerpt" as proof the work wasn't done — it must verify the *deliverable
//!    / end state*, not require the entire output inline.
//! 2. **A real iteration cap.** The generic Stop-hook block cap resets on every
//!    tool call ([`crate::hooks::STOP_HOOK_BLOCK_CAP`]); a goal agent always
//!    runs a tool before stopping, so that cap never fires. Goals therefore
//!    keep their own iteration counter in [`GoalState`] that does NOT reset on
//!    tool calls, bounded by [`GOAL_MAX_ITERATIONS`].
//! 3. **Stall detection.** If the judge keeps returning essentially the same
//!    feedback ([`GOAL_STALL_LIMIT`] times running), the loop is not
//!    converging and is stopped early.
//! 4. **Graceful give-up.** When the cap or a stall is hit, the goal is cleared
//!    and the agent is told to deliver its best-effort answer (what it did,
//!    what's incomplete, the blocker) instead of spinning — control returns to
//!    the user rather than looping indefinitely.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::hooks::{HookDefinition, HookEvent};
use crate::session::extension_data::ExtensionState;

use super::Agent;

/// Max messages included in the transcript tail sent to the Stop-hook judge.
const TRANSCRIPT_TAIL_MESSAGES: usize = 14;
/// Per-message character cap in the transcript tail.
const TRANSCRIPT_MESSAGE_MAX_CHARS: usize = 2_000;
/// Total character cap of the transcript tail.
const TRANSCRIPT_TAIL_MAX_CHARS: usize = 12_000;

/// Hard upper bound on goal iterations (judge blocks) before the loop gives up
/// and hands a best-effort answer back to the user. Does NOT reset on tool
/// calls, unlike the generic Stop-hook block cap.
pub const GOAL_MAX_ITERATIONS: u32 = 20;
/// Consecutive near-identical judge reasons that count as "not converging".
pub const GOAL_STALL_LIMIT: u32 = 3;
/// Jaccard similarity (over word sets) at/above which two judge reasons are
/// treated as "the same demand" for stall detection. Shared with the general
/// chat stall detector (BR-32) so both loops agree on what "the same complaint"
/// means.
const GOAL_STALL_SIMILARITY: f32 = crate::agents::stall::STALL_SIMILARITY;

/// Words accepted as "clear the goal" arguments, mirroring Claude Code.
const CLEAR_WORDS: &[&str] = &["clear", "stop", "off", "reset", "none", "cancel"];

/// An active goal for one session.
///
/// Persisted into the session's `extension_data` under key `goal.v0` (see the
/// [`ExtensionState`] impl below) so an active `/goal` survives a daemon
/// restart, exactly like todos — otherwise a restart silently drops the goal
/// while its todos live on (BR-41).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalState {
    pub condition: String,
    pub set_at: DateTime<Utc>,
    /// Number of times the judge has blocked the stop for this goal.
    pub iterations: u32,
    /// The judge's most recent feedback, for stall comparison.
    pub last_reason: Option<String>,
    /// Consecutive iterations whose feedback resembled the previous one.
    pub stall_count: u32,
}

impl ExtensionState for GoalState {
    const EXTENSION_NAME: &'static str = "goal";
    const VERSION: &'static str = "v0";
}

impl GoalState {
    fn new(condition: String) -> Self {
        Self {
            condition,
            set_at: Utc::now(),
            iterations: 0,
            last_reason: None,
            stall_count: 0,
        }
    }
}

/// What the agent loop should do after the judge blocks a goal's stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalOutcome {
    /// Keep working: feed the judge's feedback back to the model.
    Continue,
    /// Stop looping and have the agent deliver a best-effort answer.
    GiveUp { attempts: u32, stalled: bool },
}

/// Per-session goal conditions, keyed by session id.
#[derive(Default)]
pub struct GoalRegistry {
    goals: Mutex<HashMap<String, GoalState>>,
}

// BR-32: `ellipsize` and the Jaccard reason-similarity that used to live here
// are now shared with the general chat stall detector (`agents::stall`), which
// generalizes exactly this machinery to turns with no goal set. Re-exported so
// existing `goal::ellipsize` call sites keep working and the two detectors can
// never drift apart.
pub(crate) use crate::agents::stall::ellipsize;
use crate::agents::stall::reason_similarity;

/// The judge rule installed as the goal's Stop hook. `attempt` is the upcoming
/// evaluation number and `previous_feedback` is the judge's last note (if any),
/// so the judge can detect stagnation and decide to hand control back rather
/// than block forever.
fn goal_judge_rule(condition: &str, attempt: u32, previous_feedback: Option<&str>) -> String {
    let history = match previous_feedback {
        Some(prev) => format!(
            "\nThis is evaluation attempt #{attempt}. Your previous feedback was:\n\"{}\"\n\
             If the agent has NOT meaningfully progressed past that feedback, or it is \
             repeating the same kind of output without converging, the loop is stuck, \
             prefer returning control to the user (see below).",
            ellipsize(prev, 600)
        ),
        None => String::new(),
    };
    format!(
        "A goal condition is active for this session. The agent just tried to finish its \
         turn. Decide whether the goal is met.\n\n\
         Goal condition:\n{condition}\n{history}\n\n\
         IMPORTANT, how to read the evidence:\n\
         • The event JSON's `transcript_tail` is only a RECENT EXCERPT of the \
         conversation, not the whole thing. Earlier output has scrolled out of view. \
         Do NOT treat \"the full output isn't visible in this excerpt\" as proof the \
         work wasn't done.\n\
         • Verify the DELIVERABLE / end state, not whether the entire output is pasted \
         inline. Evidence of completion includes the agent reporting it finished with \
         concrete results (counts, a saved file path, a clear final answer), tool \
         results showing success, or work products written to disk.\n\
         • If the user has since relaxed or clarified the goal in the transcript, honor \
         that relaxed version.\n\n\
         Respond {{\"ok\": true}} (let the agent stop) when ANY of these hold:\n\
         • the goal is verifiably met or substantially satisfied;\n\
         • the agent is blocked on something only the user can resolve (it asked a \
         question, a permission was denied);\n\
         • the goal cannot be satisfied as literally specified (e.g. it would require \
         far more output than fits in chat), OR the agent has stopped making real \
         progress across attempts. In these cases returning control to the user with a \
         best-effort answer is the right outcome.\n\n\
         Respond {{\"ok\": false, \"reason\": \"<short, concrete next step>\"}} ONLY when \
         the goal is genuinely not met AND a specific, different action would move it \
         forward. When in doubt after several attempts, prefer {{\"ok\": true}}; do not \
         keep the agent looping."
    )
}

/// The instruction fed to the model when a goal gives up: deliver a best-effort
/// answer instead of starting another pass.
pub(crate) fn giveup_instruction(last_reason: &str) -> String {
    format!(
        "The goal has been stopped and cleared after repeated attempts without a \
         verifiable result. Do NOT start another pass or call more tools to redo the \
         work. Give the user your best final answer now, in this turn:\n\
         1. A concise summary of what you DID accomplish.\n\
         2. What remains incomplete or could not be done.\n\
         3. The specific blocker (for example: the full output is too large to place in \
         chat; some resources were unreachable; the request was ambiguous).\n\
         If the goal was too broad, suggest a narrower follow-up the user could set. \
         The evaluator's most recent note was: \"{}\"",
        ellipsize(last_reason, 400)
    )
}

/// A compact, role-prefixed rendering of the last few conversation messages,
/// for the Stop-hook judge. Returns `None` when there is nothing to show.
pub(crate) fn transcript_tail(conversation: &Conversation) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut budget = TRANSCRIPT_TAIL_MAX_CHARS;
    for message in conversation.messages().iter().rev() {
        if parts.len() >= TRANSCRIPT_TAIL_MESSAGES || budget == 0 {
            break;
        }
        let text = message.as_concat_text();
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let role = match message.role {
            rmcp::model::Role::User => "user",
            rmcp::model::Role::Assistant => "assistant",
        };
        let entry = format!(
            "{role}: {}",
            ellipsize(text, TRANSCRIPT_MESSAGE_MAX_CHARS.min(budget))
        );
        budget = budget.saturating_sub(entry.chars().count());
        parts.push(entry);
    }
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    Some(parts.join("\n\n"))
}

fn format_elapsed(since: DateTime<Utc>) -> String {
    let secs = (Utc::now() - since).num_seconds().max(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
    }
}

impl Agent {
    /// The goal currently active for a session, if any.
    pub async fn active_goal(&self, session_id: &str) -> Option<GoalState> {
        self.goals.goals.lock().await.get(session_id).cloned()
    }

    /// Install (or replace) the goal's Stop-hook evaluator with the given
    /// attempt context.
    async fn install_goal_hook(
        &self,
        session_id: &str,
        condition: &str,
        attempt: u32,
        previous_feedback: Option<&str>,
    ) {
        self.hooks_manager
            .set_session_hooks(
                session_id,
                HookEvent::Stop,
                vec![HookDefinition::Prompt {
                    prompt: goal_judge_rule(condition, attempt, previous_feedback),
                    model: None,
                    provider: None,
                    timeout: None,
                }],
            )
            .await;
    }

    /// Set (or replace) the session goal and install its Stop-hook evaluator.
    pub async fn set_goal(&self, session_id: &str, condition: String) {
        self.install_goal_hook(session_id, &condition, 1, None)
            .await;
        self.hooks_manager.reset_stop_blocks(session_id).await;
        let state = GoalState::new(condition);
        self.persist_goal(session_id, &state).await;
        self.goals
            .goals
            .lock()
            .await
            .insert(session_id.to_string(), state);
    }

    /// Clear the session goal and its Stop-hook evaluator. Returns the goal
    /// that was active, if any.
    pub async fn clear_goal(&self, session_id: &str) -> Option<GoalState> {
        self.hooks_manager
            .clear_session_hooks(session_id, HookEvent::Stop)
            .await;
        self.hooks_manager.reset_stop_blocks(session_id).await;
        self.clear_persisted_goal(session_id).await;
        self.goals.goals.lock().await.remove(session_id)
    }

    /// Persist `state` into the session's `extension_data` (key `goal.v0`), so
    /// an active goal survives a daemon restart — mirroring how todos persist.
    /// Best-effort: a persistence failure only forfeits the restore-after-restart
    /// property, never the live in-memory goal.
    ///
    /// Only `goal.v0` is written, in one per-key transaction: a whole-blob
    /// rewrite would put back every other key as it was read, erasing what a
    /// concurrent writer (a spawn recording its child into the chat's
    /// orchestration plan, a todo call) committed in between.
    async fn persist_goal(&self, session_id: &str, state: &GoalState) {
        let value = match state.to_value() {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!("Failed to serialize the goal for session {session_id}: {e}");
                return;
            }
        };
        self.write_persisted_goal(session_id, value, "persist")
            .await;
    }

    /// Clear any persisted goal for `session_id` (null out the `goal.v0` key),
    /// so a resolved or manually cleared goal does not resurrect on resume.
    async fn clear_persisted_goal(&self, session_id: &str) {
        self.write_persisted_goal(session_id, serde_json::Value::Null, "clear")
            .await;
    }

    /// Store `value` under `goal.v0` and nothing else. Best-effort, like both
    /// callers.
    async fn write_persisted_goal(&self, session_id: &str, value: serde_json::Value, what: &str) {
        let written = self
            .config
            .session_manager
            .update_extension_state(
                session_id,
                GoalState::EXTENSION_NAME,
                GoalState::VERSION,
                move |_| Ok(value),
            )
            .await;
        match written {
            Ok(Some(_)) => {}
            Ok(None) => {
                tracing::warn!("Failed to {what} the goal: session {session_id} does not exist");
            }
            Err(e) => {
                tracing::warn!("Failed to {what} the goal for session {session_id}: {e}");
            }
        }
    }

    /// Restore a persisted goal into the in-memory registry and re-install its
    /// Stop-hook judge, when a daemon restart dropped the live state but the
    /// goal is still recorded in the session's `extension_data`. A no-op when
    /// the goal is already live in this process or none was stored. Called at
    /// the start of a turn (see `Agent::reply`) so a resumed `/goal` keeps
    /// working instead of silently vanishing.
    pub(crate) async fn restore_goal(&self, session_id: &str) {
        if self.goals.goals.lock().await.contains_key(session_id) {
            return;
        }
        let Ok(session) = self
            .config
            .session_manager
            .get_session(session_id, false)
            .await
        else {
            return;
        };
        let Some(state) = GoalState::from_extension_data(&session.extension_data) else {
            return;
        };
        // The Stop-hook judge was in-memory too, so re-install it with the same
        // progress context the loop would have had (next attempt #, last note).
        self.install_goal_hook(
            session_id,
            &state.condition,
            state.iterations + 1,
            state.last_reason.as_deref(),
        )
        .await;
        self.goals
            .goals
            .lock()
            .await
            .insert(session_id.to_string(), state);
    }

    /// Record a judge block against the active goal and decide whether to keep
    /// looping or give up. Updates the iteration / stall counters and, when
    /// continuing, refreshes the judge hook with progress context so the next
    /// evaluation is stagnation-aware. Returns `None` if no goal is active.
    pub(crate) async fn record_goal_block(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Option<GoalOutcome> {
        let (condition, attempt, outcome, snapshot) = {
            let mut goals = self.goals.goals.lock().await;
            let state = goals.get_mut(session_id)?;
            state.iterations += 1;

            let similar = state
                .last_reason
                .as_deref()
                .map(|prev| reason_similarity(prev, reason) >= GOAL_STALL_SIMILARITY)
                .unwrap_or(false);
            state.stall_count = if similar { state.stall_count + 1 } else { 0 };
            state.last_reason = Some(reason.to_string());

            let stalled = state.stall_count >= GOAL_STALL_LIMIT;
            let capped = state.iterations >= GOAL_MAX_ITERATIONS;
            let outcome = if capped || stalled {
                GoalOutcome::GiveUp {
                    attempts: state.iterations,
                    stalled,
                }
            } else {
                GoalOutcome::Continue
            };
            (
                state.condition.clone(),
                state.iterations,
                outcome,
                state.clone(),
            )
        };

        // Persist the bumped iteration/stall counters so a restart resumes the
        // goal with the right remaining budget instead of an infinite one. On a
        // give-up the caller clears the goal right after, which nulls this out.
        self.persist_goal(session_id, &snapshot).await;

        if outcome == GoalOutcome::Continue {
            // Refresh the judge with the new attempt # and latest feedback so it
            // can recognize stagnation on the next stop.
            self.install_goal_hook(session_id, &condition, attempt + 1, Some(reason))
                .await;
        }
        Some(outcome)
    }

    /// `/goal` slash command: no args shows status, a clear word clears, and
    /// anything else sets the condition and immediately starts working toward
    /// it (the returned user-role message continues into the agent loop).
    pub(crate) async fn handle_goal_command(
        &self,
        params_str: &str,
        session_id: &str,
    ) -> Result<Option<Message>> {
        let arg = params_str.trim();

        if arg.is_empty() {
            let text = match self.active_goal(session_id).await {
                Some(goal) => format!(
                    "🎯 Active goal (set {} ago, {} evaluation(s) so far):\n{}\n\nThe goal \
                     clears automatically once met, or after {} attempts if it can't \
                     converge; `/goal clear` stops it early.",
                    format_elapsed(goal.set_at),
                    goal.iterations,
                    goal.condition,
                    GOAL_MAX_ITERATIONS,
                ),
                None => "No active goal. Set one with `/goal <condition>`, a verifiable end \
                         state, e.g. `/goal cargo test exits 0 and git status is clean`. \
                         Biorouter keeps working until an automatic evaluator confirms the \
                         condition is met, then hands back a best-effort answer if it can't."
                    .to_string(),
            };
            return Ok(Some(Message::assistant().with_text(text)));
        }

        if CLEAR_WORDS.contains(&arg.to_ascii_lowercase().as_str()) {
            let text = match self.clear_goal(session_id).await {
                Some(goal) => format!("Goal cleared: {}", ellipsize(&goal.condition, 200)),
                None => "No active goal to clear.".to_string(),
            };
            return Ok(Some(Message::assistant().with_text(text)));
        }

        self.set_goal(session_id, arg.to_string()).await;

        // A user-role message continues into the agent loop, so work toward
        // the goal starts immediately (mirrors workflow slash commands).
        Ok(Some(Message::user().with_text(format!(
            "A goal has been set for this session. Work toward it now and keep working \
             until it is verifiably met. When you believe it is met, finish your turn; \
             an automatic evaluator checks the condition and sends feedback if it is not \
             met yet, in which case you must keep going. Prefer durable deliverables \
             (e.g. saving large output to a file) over pasting huge volumes into chat. \
             Stop early if you are blocked on input only the user can provide.\n\n\
             Goal condition:\n{arg}"
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_respects_char_boundaries() {
        assert_eq!(ellipsize("short", 10), "short");
        let long = "éééééééééééé"; // 13 two-byte chars
        let cut = ellipsize(long, 5);
        assert_eq!(cut.chars().count(), 5);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn transcript_tail_formats_roles_and_caps_messages() {
        let mut conversation = Conversation::default();
        conversation.push(Message::user().with_text("please fix the tests"));
        conversation.push(Message::assistant().with_text("done, all 5 tests pass"));
        let tail = transcript_tail(&conversation).unwrap();
        assert!(tail.starts_with("user: please fix the tests"));
        assert!(tail.contains("assistant: done, all 5 tests pass"));
    }

    #[test]
    fn transcript_tail_empty_conversation_is_none() {
        assert!(transcript_tail(&Conversation::default()).is_none());
    }

    #[test]
    fn judge_rule_embeds_condition_and_truncation_guard() {
        let rule = goal_judge_rule("npm test exits 0", 1, None);
        assert!(rule.contains("npm test exits 0"));
        assert!(rule.contains("transcript_tail"));
        // The truncation guard is the core fix.
        assert!(rule.contains("RECENT EXCERPT"));
    }

    #[test]
    fn judge_rule_includes_attempt_and_prior_feedback() {
        let rule = goal_judge_rule("do x", 4, Some("you only showed part of the list"));
        assert!(rule.contains("attempt #4"));
        assert!(rule.contains("only showed part of the list"));
    }

    #[test]
    fn similar_reasons_score_high() {
        let a = "The transcript only shows the start of the 407-entry list, continue outputting";
        let b = "The transcript still only shows the beginning of the 407-entry list, continue";
        assert!(reason_similarity(a, b) >= GOAL_STALL_SIMILARITY);
    }

    #[test]
    fn different_reasons_score_low() {
        let a = "Run cargo test and make it pass";
        let b = "The website is unreachable, document the failure reason";
        assert!(reason_similarity(a, b) < GOAL_STALL_SIMILARITY);
    }

    #[test]
    fn giveup_instruction_mentions_blocker_and_reason() {
        let text = giveup_instruction("output too large for chat");
        assert!(text.contains("best final answer"));
        assert!(text.contains("output too large for chat"));
    }

    #[test]
    fn goal_state_persists_under_versioned_key() {
        use crate::session::extension_data::ExtensionData;

        let mut state = GoalState::new("cargo test exits 0".to_string());
        state.iterations = 3;
        state.stall_count = 1;
        state.last_reason = Some("still two tests failing".to_string());

        let mut data = ExtensionData::new();
        state.to_extension_data(&mut data).unwrap();

        // Stored under the todo-style versioned key "goal.v0".
        assert!(data.get_extension_state("goal", "v0").is_some());

        let back = GoalState::from_extension_data(&data).expect("goal round-trips");
        assert_eq!(back.condition, "cargo test exits 0");
        assert_eq!(back.iterations, 3);
        assert_eq!(back.stall_count, 1);
        assert_eq!(back.last_reason.as_deref(), Some("still two tests failing"));
    }

    #[test]
    fn cleared_goal_does_not_load() {
        use crate::session::extension_data::ExtensionData;

        let mut data = ExtensionData::new();
        // Absent → None.
        assert!(GoalState::from_extension_data(&data).is_none());

        GoalState::new("do x".to_string())
            .to_extension_data(&mut data)
            .unwrap();
        assert!(GoalState::from_extension_data(&data).is_some());

        // Nulled out (how `clear_persisted_goal` clears it) → None, so a
        // resolved goal does not resurrect on the next resume.
        data.set_extension_state(
            GoalState::EXTENSION_NAME,
            GoalState::VERSION,
            serde_json::Value::Null,
        );
        assert!(GoalState::from_extension_data(&data).is_none());
    }
}

/// End-to-end persistence/restore across a *simulated daemon restart*: a fresh
/// `Agent` + `SessionManager` over the same on-disk session DB models the new
/// process, so its in-memory goal registry starts empty and must rehydrate from
/// `extension_data` (BR-41).
#[cfg(test)]
mod persistence_tests {
    use super::*;
    use crate::agents::AgentConfig;
    use crate::config::permission::PermissionManager;
    use crate::config::BioRouterMode;
    use crate::hooks::HookEvent;
    use crate::session::session_manager::SessionType;
    use crate::session::SessionManager;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// A fresh `Agent` bound to an isolated `SessionManager` over `dir`.
    fn agent_over(dir: &std::path::Path) -> Agent {
        let session_manager = Arc::new(SessionManager::new(dir.to_path_buf()));
        let permission_manager = Arc::new(PermissionManager::new(dir.to_path_buf()));
        Agent::with_config(AgentConfig::new(
            session_manager,
            permission_manager,
            None,
            BioRouterMode::Auto,
        ))
    }

    async fn new_session(agent: &Agent, name: &str) -> String {
        agent
            .config
            .session_manager
            .create_session(PathBuf::from("."), name.to_string(), SessionType::User)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn goal_survives_a_simulated_daemon_restart() {
        let dir = TempDir::new().unwrap();

        // Process 1: set a goal on a session.
        let session_id = {
            let agent = agent_over(dir.path());
            let session_id = new_session(&agent, "goal-persist").await;
            agent
                .set_goal(&session_id, "cargo test exits 0".to_string())
                .await;
            assert!(agent.active_goal(&session_id).await.is_some());
            session_id
        };

        // Process 2: a fresh Agent + SessionManager over the same on-disk DB
        // starts with an empty in-memory registry (the restart). Restore
        // rehydrates the goal and its Stop-hook judge.
        let agent2 = agent_over(dir.path());
        assert!(
            agent2.active_goal(&session_id).await.is_none(),
            "a fresh process starts with no live goal"
        );
        assert!(
            !agent2
                .hooks_manager()
                .has_session_hooks(&session_id, HookEvent::Stop)
                .await,
            "and no live Stop-hook judge"
        );

        agent2.restore_goal(&session_id).await;

        let restored = agent2
            .active_goal(&session_id)
            .await
            .expect("goal restored after restart");
        assert_eq!(restored.condition, "cargo test exits 0");
        assert!(
            agent2
                .hooks_manager()
                .has_session_hooks(&session_id, HookEvent::Stop)
                .await,
            "the Stop-hook judge is re-installed on restore"
        );
    }

    #[tokio::test]
    async fn record_goal_block_persists_the_iteration_budget() {
        let dir = TempDir::new().unwrap();
        let session_id;
        {
            let agent = agent_over(dir.path());
            session_id = new_session(&agent, "goal-budget").await;
            agent
                .set_goal(&session_id, "finish the report".to_string())
                .await;
            // Two judge blocks bump the (non-resetting) iteration counter to 2
            // and persist it, so the give-up budget can't reset across restarts.
            agent
                .record_goal_block(&session_id, "still missing section 3")
                .await;
            agent
                .record_goal_block(&session_id, "conclusion not written yet")
                .await;
        }

        let agent2 = agent_over(dir.path());
        agent2.restore_goal(&session_id).await;
        let restored = agent2
            .active_goal(&session_id)
            .await
            .expect("goal restored after restart");
        assert_eq!(
            restored.iterations, 2,
            "the iteration budget survives the restart"
        );
    }

    #[tokio::test]
    async fn cleared_goal_does_not_resurrect_on_restart() {
        let dir = TempDir::new().unwrap();
        let session_id;
        {
            let agent = agent_over(dir.path());
            session_id = new_session(&agent, "goal-clear").await;
            agent
                .set_goal(&session_id, "do the thing".to_string())
                .await;
            agent.clear_goal(&session_id).await;
        }

        let agent2 = agent_over(dir.path());
        agent2.restore_goal(&session_id).await;
        assert!(
            agent2.active_goal(&session_id).await.is_none(),
            "a cleared goal must not come back after a restart"
        );
    }

    /// Persisting or clearing a goal touches `goal.v0` only. It used to read
    /// the whole `extension_data`, change the goal in memory and write the
    /// whole blob back, which erased any other key committed in between, such
    /// as a child a parallel spawn recorded into the chat's orchestration plan.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn goal_writes_never_erase_a_concurrent_write_to_another_key() {
        const ROUNDS: usize = 24;
        let dir = TempDir::new().unwrap();
        let agent = Arc::new(agent_over(dir.path()));
        let session_id = new_session(&agent, "goal races another key").await;
        let manager = Arc::clone(&agent.config.session_manager);

        let mut tasks = Vec::new();
        for i in 0..ROUNDS {
            let goals = Arc::clone(&agent);
            let session = session_id.clone();
            tasks.push(tokio::spawn(async move {
                if i % 3 == 2 {
                    goals.clear_goal(&session).await;
                } else {
                    goals.set_goal(&session, format!("goal {i}")).await;
                }
            }));
            let other = Arc::clone(&manager);
            let session = session_id.clone();
            tasks.push(tokio::spawn(async move {
                other
                    .update_extension_state(&session, "race_probe", "v1", move |current| {
                        let mut seen: Vec<usize> = current
                            .and_then(|value| serde_json::from_value(value.clone()).ok())
                            .unwrap_or_default();
                        seen.push(i);
                        Ok(serde_json::to_value(seen)?)
                    })
                    .await
                    .unwrap()
                    .expect("the session exists");
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        let stored = manager
            .get_extension_state(&session_id, "race_probe", "v1")
            .await
            .unwrap()
            .expect("a goal write erased the other key");
        let seen: Vec<usize> = serde_json::from_value(stored).unwrap();
        assert_eq!(
            seen.len(),
            ROUNDS,
            "a goal write erased {} concurrent write(s)",
            ROUNDS - seen.len()
        );
    }
}
