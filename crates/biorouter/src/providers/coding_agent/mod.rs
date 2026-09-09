//! Shared machinery for the two providers that drive a vendor coding-agent CLI
//! on the user's **own subscription**.
//!
//! # What these providers are
//!
//! `claude_code` and `codex` are unlike every other provider in this tree: there
//! is no base URL and no API key. Each spawns a CLI the *user* installed and
//! signed in to, and that CLI resolves its own credential and bills the user's
//! own plan. BioRouter never sees, stores, brokers, proxies or transmits the
//! credential — it only starts a process.
//!
//! That is not merely a convenient design, it is the compliance boundary. Both
//! vendors permit ordinary individual use of one's own subscription and forbid a
//! third party from *offering* their login or routing requests on behalf of its
//! users. Every rule below follows from staying firmly on the permitted side:
//!
//! * BioRouter never implements a vendor login flow. It surfaces the vendor's own
//!   command ([`CodingAgentKind::login_command`]) for the user to run.
//! * BioRouter never disguises itself as the vendor's first-party harness. No
//!   entrypoint spoofing, no header rewriting, no proxy — some harnesses do this
//!   and it is exactly what the terms target.
//! * The credential-diverting environment is scrubbed ([`env`]) so a run cannot
//!   silently land on a metered API account the user did not choose.
//!
//! # The privacy tier, which is the whole security argument
//!
//! Both providers are [`crate::privacy::ProviderTier::Public`] with
//! `runs_locally: false` and no affiliation — i.e. both leave the trait defaults
//! alone, which are already correct and are documented as fail-safe.
//!
//! The tempting mistake is to reason "the subprocess is local, so this is
//! Private". That is a category error and a dangerous one. `runs_locally` is
//! defined as whether *inference* happens on the user's machine, and here it does
//! not — it happens at Anthropic or OpenAI. `Public` is, per the privacy-tier
//! design, "everything hosted by an AI company or a large cloud".
//!
//! Getting this right is what makes the feature safe for a clinical-research
//! setting. A consumer subscription carries no BAA and no zero-data-retention
//! agreement, so PHI must never reach it. Because these providers are `Public`,
//! the bind gate refuses to attach them to any session already classified
//! `Private` — one that has touched an institutional clinical extension or a
//! private knowledge base. Declaring `Private` would forge that badge and delete
//! the protection.
//!
//! # What the child agent may and may not do
//!
//! Both CLIs are full agents with their own file and shell tools. Those are
//! switched **off** (`--tools ""`, `sandbox: read-only`), because a tool the child
//! runs itself is invisible to BioRouter's inspectors, permission modes,
//! `.biorouterignore` and vault. What the child gets instead is BioRouter's own
//! tools, over MCP, executed by BioRouter's dispatcher — so every existing gate
//! still fires on them. See [`super::coding_agent::bridge`] once that lands.

pub mod appserver;
pub mod bridge;
pub mod claude_stream;
pub mod codex_stream;
pub mod discovery;
pub mod effort;
pub mod env;
pub mod mirror;
pub mod transcript;

pub use discovery::{
    codex_home, configured_command, probe, probe_all, resolve_binary, AgentAvailability, AuthState,
    CodingAgentKind,
};

use std::future::Future;
use std::time::Duration;

use super::errors::ProviderError;

/// Optional wall-clock ceiling for one Claude Code or Codex turn.
///
/// Coding-agent turns are unbounded by default: they may legitimately spend a
/// long time inside a supervised delegated task, and completion or explicit
/// cancellation is the normal terminal condition. Operators who need a hard
/// resource ceiling can set `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS` to a
/// positive number. Missing, invalid and zero values all keep the default
/// unbounded behaviour.
pub const TURN_TIMEOUT_CONFIG_KEY: &str = "BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnTimeoutElapsed {
    duration: Duration,
}

impl TurnTimeoutElapsed {
    pub fn duration(self) -> Duration {
        self.duration
    }
}

fn turn_timeout_from_seconds(seconds: u64) -> Option<Duration> {
    (seconds > 0).then_some(Duration::from_secs(seconds))
}

pub fn turn_timeout() -> Option<Duration> {
    crate::config::Config::global()
        .get_param::<u64>(TURN_TIMEOUT_CONFIG_KEY)
        .ok()
        .and_then(turn_timeout_from_seconds)
}

pub async fn await_turn<F>(
    future: F,
    limit: Option<Duration>,
) -> Result<F::Output, TurnTimeoutElapsed>
where
    F: Future,
{
    match limit {
        Some(duration) => tokio::time::timeout(duration, future)
            .await
            .map_err(|_| TurnTimeoutElapsed { duration }),
        None => Ok(future.await),
    }
}

/// Aborts a spawned task when dropped — the streaming path's equivalent of
/// `kill_on_drop`.
///
/// On the blocking path the child process is owned by the provider's own future,
/// so cancelling a turn (which **drops** that future rather than unwinding it)
/// drops the child, and `kill_on_drop(true)` reaps it. The streaming path breaks
/// that chain: the child has to be owned by a spawned reader task, and a spawned
/// task outlives the stream that was feeding from it. Without this guard a
/// cancelled turn would leave `claude` or `codex app-server` running detached —
/// holding the user's own subscription credential and burning their quota on an
/// answer nobody will read.
///
/// Held inside the returned stream, so the abort fires exactly when the consumer
/// lets go of it.
/// Tell a coding-agent child which tools are real before it reaches for one
/// that is not.
///
/// ⚠ **The child is a whole agent, and Biorouter switches its NATIVE tools
/// off** — they run outside Biorouter's inspectors, permission mode, vault and
/// privacy gates, which is precisely what the tool bridge exists to prevent.
/// The vendor CLI does not stop ADVERTISING all of them when the feature behind
/// them is disabled, so the model can still pick one. Measured with Codex
/// 0.147.0: asked to delegate, it called its own `spawn_agent` even though
/// `multi_agent` was passed to `--disable`, and got the vendor's internal
/// `no thread with id` back — an error that describes nothing the user can act
/// on and makes a working bridge look broken.
///
/// The cheap fix is to say so up front. Re-enabling the vendor's own multi-agent
/// tools would silence it too, and would hand the child exactly the unmediated
/// surface the bridge is there to deny.
///
/// Returns an empty string when there is no bridge, so a child with no Biorouter
/// tools is not told about tools it does not have.
pub fn native_tools_notice(bridge_url: Option<&str>) -> String {
    let Some(url) = bridge_url else {
        return String::new();
    };
    let names = bridge::advertised_tool_names(url);
    if names.is_empty() {
        return String::new();
    }
    let listed = names
        .iter()
        .map(|n| format!("`mcp__biorouter__{n}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\n\n# Tools in this environment\n\n\
         Your own built-in tools are DISABLED here and calling one fails with an \
         internal error from your CLI, not from Biorouter. In particular you have \
         no native sub-agent, delegation or multi-agent tool: do not call \
         `spawn_agent` or anything like it.\n\n\
         The tools you actually have are provided over MCP by Biorouter, and they \
         are: {listed}. Use those, and if none of them fits, say so plainly \
         instead of reaching for a tool of your own. Tool names elsewhere in \
         Biorouter's instructions are internal MCP tool IDs; invoke their \
         matching MCP-qualified names listed here, not the bare internal IDs."
    )
}

pub struct AbortOnDrop(pub tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A vendor payload that arrived as a JSON envelope, reduced to the sentence
/// inside it.
///
/// ⚠ **This is not hypothetical, and it is what the user sees.** Both CLIs relay
/// the upstream API's response body verbatim, sometimes as a *string* rather
/// than as prose, and the transcript then reads:
///
/// ```text
/// Ran into this error: Request failed: {"type":"error","status":400,"error":
/// {"type":"invalid_request_error","message":"The 'gpt-6-astra' model requires a
/// newer version of Codex. Please upgrade to the latest app or CLI and try
/// again."}}.
/// ```
///
/// The one actionable sentence there is buried in JSON the reader cannot act on.
///
/// ⚠ **Best-effort, and it must stay that way.** A payload that is not JSON, an
/// object with no message, or an empty message is returned UNCHANGED: printing
/// an ugly error is a much smaller failure than losing an unrecognised one.
/// `error.message` is read before a top-level `message` because that is the
/// shape both vendors send; neither is a guess this function is entitled to
/// improve on.
///
/// Lives here rather than in either decoder because it is about the upstream
/// API's envelope, which is the one thing the two vendors have in common.
pub fn unwrap_json_error(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') {
        return raw.to_string();
    }
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return raw.to_string();
    };
    parsed
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| parsed.get("message").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

/// Turn a missing-or-unusable CLI into the error the user should act on.
///
/// Separate from the generic error mapper because these are *setup* failures, and
/// the only useful response is a specific instruction. A coding-agent provider
/// that fails this way must not look like a transient server error, or the retry
/// layer will hide the one message that would fix it.
pub fn unavailable_error(kind: CodingAgentKind, availability: &AgentAvailability) -> ProviderError {
    match &availability.auth {
        AuthState::NotInstalled => ProviderError::ExecutionError(format!(
            "{} is not installed, or is not on a path Biorouter searches.\n\n\
             Install it with:\n    {}\n\n\
             If it is already installed somewhere unusual (nvm, volta, bun, asdf), set {} to its \
             full path in Settings instead.",
            kind.display_name(),
            kind.install_hint(),
            kind.command_config_key(),
        )),
        AuthState::SignedOut => ProviderError::Authentication(format!(
            "{} is installed but not signed in.\n\n\
             Run this yourself, in a terminal:\n    {}\n\n\
             Biorouter deliberately does not perform the sign-in: the credential stays between \
             you and the vendor, and never passes through Biorouter.",
            kind.display_name(),
            kind.login_command(),
        )),
        AuthState::SignedInWithApiKey => ProviderError::Authentication(format!(
            "{} is signed in with an API key rather than a subscription, so this provider cannot \
             run: it exists specifically to use your own plan, and it removes API credentials from \
             the environment it starts.\n\n\
             Either sign in with your subscription:\n    {}\n\n\
             or use the metered provider for this vendor instead.",
            kind.display_name(),
            kind.login_command(),
        )),
        AuthState::Indeterminate { detail } => ProviderError::ExecutionError(format!(
            "Could not determine whether {} is usable: {detail}",
            kind.display_name(),
        )),
        AuthState::SignedInSubscription { .. } => ProviderError::ExecutionError(format!(
            "{} reported a usable subscription but the run could not start.",
            kind.display_name(),
        )),
    }
}

#[cfg(test)]
mod tests {
    /// A child with no bridge has none of Biorouter's tools, so telling it which
    /// ones it has would be a lie. Say nothing.
    #[test]
    fn no_bridge_means_no_notice() {
        assert!(super::native_tools_notice(None).is_empty());
        assert!(
            super::native_tools_notice(Some("http://127.0.0.1:1/tool_bridge/unknown")).is_empty()
        );
    }

    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// The real payload, from a Codex turn on `gpt-6-astra` against codex-cli
    /// 0.147.0 (PR #180's live verification). Before this, the whole envelope
    /// was rendered in the chat and the only actionable sentence in it was
    /// buried in the middle.
    const CODEX_400: &str = r#"{"type":"error","status":400,"error":{"type":"invalid_request_error","message":"The 'gpt-6-astra' model requires a newer version of Codex. Please upgrade to the latest app or CLI and try again."}}"#;

    #[test]
    fn a_json_envelope_is_reduced_to_the_sentence_inside_it() {
        assert_eq!(
            unwrap_json_error(CODEX_400),
            "The 'gpt-6-astra' model requires a newer version of Codex. Please upgrade to the \
             latest app or CLI and try again."
        );
    }

    #[test]
    fn a_top_level_message_is_read_when_there_is_no_error_object() {
        assert_eq!(
            unwrap_json_error(r#"{"message":"You've hit your usage limit."}"#),
            "You've hit your usage limit."
        );
    }

    /// The other vendor's text is already a sentence, so unwrapping it must be a
    /// no-op. Verbatim from the same PR's Claude Code run on claude 2.1.235.
    #[test]
    fn prose_is_returned_unchanged() {
        const CLAUDE_400: &str = "API Error: 400 Claude Code 2.1.235 does not support this model; \
                                  version 2.1.251 or newer is required. Run 'claude update', or \
                                  update the Claude desktop app, then try again.";
        assert_eq!(unwrap_json_error(CLAUDE_400), CLAUDE_400);
    }

    /// Losing an unrecognised failure would be far worse than printing an ugly
    /// one, so every shape this function cannot improve on is passed through.
    #[test]
    fn anything_it_cannot_improve_on_survives_verbatim() {
        for raw in [
            // Not JSON at all, but starts with a brace.
            "{not json",
            // JSON, but nothing to pull out.
            r#"{"status":500}"#,
            // A message that is present and empty is not an improvement.
            r#"{"error":{"message":"   "}}"#,
            // A JSON array is not an envelope.
            r#"["a"]"#,
            "",
        ] {
            assert_eq!(unwrap_json_error(raw), raw, "mangled: {raw}");
        }
    }

    fn availability(kind: CodingAgentKind, auth: AuthState) -> AgentAvailability {
        AgentAvailability {
            kind,
            provider_id: kind.provider_id().to_string(),
            display_name: kind.display_name().to_string(),
            path: None,
            version: None,
            auth,
            login_command: kind.login_command().to_string(),
            install_hint: kind.install_hint().to_string(),
        }
    }

    /// A setup failure must be classified so the retry layer does not bury it.
    /// "Not signed in" is an auth error; "not installed" is not something a retry
    /// or a credential change fixes, so it stays an execution error.
    #[test]
    fn setup_failures_are_classified_for_the_retry_layer() {
        let signed_out = unavailable_error(
            CodingAgentKind::ClaudeCode,
            &availability(CodingAgentKind::ClaudeCode, AuthState::SignedOut),
        );
        assert!(matches!(signed_out, ProviderError::Authentication(_)));

        let missing = unavailable_error(
            CodingAgentKind::Codex,
            &availability(CodingAgentKind::Codex, AuthState::NotInstalled),
        );
        assert!(matches!(missing, ProviderError::ExecutionError(_)));
    }

    /// Every setup error names the exact command the user should run. The whole
    /// reason this function exists instead of a generic string is that the user
    /// cannot fix any of these states without being told the command.
    #[test]
    fn every_actionable_error_names_a_command() {
        for kind in CodingAgentKind::all() {
            for (auth, expected) in [
                (AuthState::NotInstalled, kind.install_hint()),
                (AuthState::SignedOut, kind.login_command()),
                (AuthState::SignedInWithApiKey, kind.login_command()),
            ] {
                let msg = unavailable_error(kind, &availability(kind, auth)).to_string();
                assert!(
                    msg.contains(expected),
                    "{kind:?} error should tell the user to run `{expected}`, got: {msg}"
                );
            }
        }
    }

    /// The signed-out message must state that Biorouter is not doing the login.
    /// That sentence is the user-visible half of the compliance posture, so it is
    /// pinned rather than left to future editing.
    #[test]
    fn signed_out_message_states_that_biorouter_does_not_broker_the_login() {
        let msg = unavailable_error(
            CodingAgentKind::ClaudeCode,
            &availability(CodingAgentKind::ClaudeCode, AuthState::SignedOut),
        )
        .to_string();
        assert!(msg.contains("never passes through Biorouter"));
    }

    /// ⚠ **Both vendor children must be spawned with `kill_on_drop(true)`.**
    ///
    /// A cancelled turn does not unwind these providers, it DROPS them.
    /// `drive_stream`'s hard-cancellation escape breaks out of its `select!`
    /// while the provider call is still pending, the stream is dropped, and the
    /// in-flight future goes with it — so `AppServer::shutdown()` and
    /// `claude_code`'s `start_kill()` never run. `tokio::process::Child`
    /// defaults to `kill_on_drop(false)` and DETACHES, so the vendor CLI kept
    /// running with the user's subscription credential, burning quota on an
    /// answer nobody would read, and on the Codex path holding its app-server
    /// port too.
    ///
    /// Structural rather than behavioural, deliberately. Proving the fix by
    /// observation means spawning a real `claude`/`codex`, cancelling
    /// mid-answer, and asserting the pid is gone — which needs both vendor CLIs
    /// installed and signed in, so it cannot run in CI and would therefore
    /// never be the thing that catches a regression. What CAN be checked
    /// everywhere is that the flag is set before the spawn it governs.
    #[test]
    fn every_vendor_child_is_reaped_when_its_turn_is_dropped() {
        for (label, src) in [
            ("claude_code.rs", include_str!("../claude_code.rs")),
            ("coding_agent/appserver.rs", include_str!("appserver.rs")),
        ] {
            // Comments mention both spellings; only executable lines count.
            let code: String = src
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            let kill = code.find("kill_on_drop(true)").unwrap_or_else(|| {
                panic!(
                    "{label} spawns a vendor CLI without kill_on_drop(true); a cancelled turn \
                     drops the future rather than unwinding it, so the explicit reap never runs \
                     and the child detaches with the user's credential"
                )
            });
            let spawn = code
                .find(".spawn()")
                .unwrap_or_else(|| panic!("{label} no longer spawns anything"));
            assert!(
                kill < spawn,
                "{label} sets kill_on_drop AFTER the spawn it is meant to govern"
            );
        }
    }

    #[test]
    fn turn_timeout_is_opt_in_and_zero_disables_it() {
        assert_eq!(turn_timeout_from_seconds(0), None);
        assert_eq!(turn_timeout_from_seconds(90), Some(Duration::from_secs(90)));
    }

    #[test]
    fn every_vendor_turn_uses_the_shared_opt_in_timeout_policy() {
        for (label, src) in [
            ("claude_code.rs", include_str!("../claude_code.rs")),
            ("codex.rs", include_str!("../codex.rs")),
        ] {
            assert!(
                src.contains("coding_agent::turn_timeout()"),
                "{label} bypasses the shared default-unbounded turn policy"
            );
            assert!(
                !src.contains("const TURN_TIMEOUT"),
                "{label} reintroduced a hard-coded provider turn ceiling"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_default_turn_has_no_hidden_wall_clock_deadline() {
        let turn = tokio::spawn(await_turn(std::future::pending::<()>(), None));

        tokio::time::advance(Duration::from_secs(24 * 60 * 60)).await;
        tokio::task::yield_now().await;

        assert!(
            !turn.is_finished(),
            "an unconfigured coding-agent turn must end only on completion or cancellation"
        );
        turn.abort();
        assert!(turn.await.unwrap_err().is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn an_explicit_turn_timeout_is_enforced() {
        let limit = Duration::from_secs(90);
        let turn = tokio::spawn(await_turn(std::future::pending::<()>(), Some(limit)));
        tokio::task::yield_now().await;

        tokio::time::advance(limit - Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert!(!turn.is_finished());

        tokio::time::advance(Duration::from_secs(1)).await;
        let elapsed = turn.await.unwrap().unwrap_err();
        assert_eq!(elapsed.duration(), limit);
    }

    #[tokio::test(start_paused = true)]
    async fn supervision_activity_does_not_create_a_turn_deadline() {
        let (supervisor, mut observed) = tokio::sync::watch::channel(0_u8);
        let turn = tokio::spawn(await_turn(
            async move { while observed.changed().await.is_ok() {} },
            None,
        ));

        for tick in 1..=12 {
            supervisor.send_replace(tick);
            tokio::time::advance(Duration::from_secs(10 * 60)).await;
            tokio::task::yield_now().await;
            assert!(
                !turn.is_finished(),
                "watching a supervised turn must not impose a hidden lifetime"
            );
        }

        drop(supervisor);
        assert!(turn.await.unwrap().is_ok());
    }

    struct PendingUntilDropped(Arc<AtomicBool>);

    impl Future for PendingUntilDropped {
        type Output = ();

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    impl Drop for PendingUntilDropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_still_drops_an_unbounded_turn_immediately() {
        let dropped = Arc::new(AtomicBool::new(false));
        let turn = tokio::spawn(await_turn(PendingUntilDropped(Arc::clone(&dropped)), None));
        tokio::task::yield_now().await;

        turn.abort();
        assert!(turn.await.unwrap_err().is_cancelled());
        assert!(
            dropped.load(Ordering::SeqCst),
            "cancelling the provider future must drop and reap its child process"
        );
    }
}
