//! The **Claude Code** provider: drives the user's own `claude` CLI on the
//! user's own Claude subscription.
//!
//! # Why the CLI and not the Agent SDK
//!
//! This looks like the sort of thing that ought to use `@anthropic-ai/claude-agent-sdk`,
//! and it deliberately does not. Anthropic's own documentation settles it:
//!
//! * The headless page is titled *"Run Claude Code programmatically"* and states
//!   that it "covers using the Agent SDK via the CLI (`claude -p`)" — `-p` **is**
//!   the SDK's CLI form, not a lesser path around it.
//! * The SDK overview: "The SDK is available as a library for Python and
//!   TypeScript only. To drive the same agent loop from another language, run the
//!   CLI as a subprocess with the `-p` flag and `--output-format json`." Biorouter
//!   is Rust, so this is the documented route.
//! * The SDK would actively be worse here. Its overview directs third-party
//!   developers to *API key* authentication, its use is governed by Anthropic's
//!   Commercial Terms, and it is only a wrapper that spawns this same binary
//!   anyway — the npm package ships nothing but `bin/claude.exe`. Nothing about it
//!   unlocks subscription usage.
//!
//! Subscription billing is confirmed by Anthropic's own help centre, which names
//! `claude -p` as drawing from the plan's usage limits.
//!
//! # The flags that are not optional
//!
//! Four of the arguments below are load-bearing and were each established by
//! measurement, not preference:
//!
//! * `--setting-sources ""` — without it a `-p` session **executes the hooks in
//!   the working directory's `.claude/settings.json`**, because `-p` shows no
//!   workspace-trust dialog. Tested against a hostile fixture: without this flag
//!   the fixture's `SessionStart` hook ran.
//!
//!   ⚠ This used to say "Biorouter sets the child's cwd to the session's working
//!   directory". It does not, and never has — nothing on this path calls
//!   `Command::current_dir`, so the child inherits BIOROUTER's process cwd. The
//!   sentence was written from the pre-BR-54 world, where each window ran its
//!   own daemon; today one shared daemon starts at `os.homedir()`. The doc page
//!   copied this comment, so both said the same wrong thing.
//!
//!   The flag is still load-bearing, and the CLI is why: a `-p` run started from
//!   a hostile checkout executes that checkout's hooks. On the desktop path the
//!   file within range is the user's own `~/.claude/settings.json` instead —
//!   narrower than the old sentence implied, not harmless.
//! * `--strict-mcp-config` — without it the child connects the MCP servers in the
//!   user's own configuration. Measured: a bare run showed the developer's
//!   personal clinical-database server as `connected` inside a child Biorouter
//!   thought it was fully isolating. `--tools ""` does *not* cover this; it
//!   suppresses built-ins only.
//! * `--tools ""` — the child's own Read/Edit/Bash are switched off. A tool the
//!   child runs itself is invisible to Biorouter's inspectors, permission modes,
//!   `.biorouterignore` and vault. Biorouter's own tools reach it over MCP
//!   instead, where Biorouter's dispatcher executes them and every existing gate
//!   still fires.
//! * `--system-prompt` — replaces Claude Code's default prompt with Biorouter's.
//!   Besides being correct, it is a 16x saving: the default prompt measured 25,022
//!   tokens per call, and Biorouter's measured 1,527.
//!
//! `--bare` must **never** be passed. It is documented as never reading OAuth
//! credentials or the system keychain, which is precisely the credential this
//! provider exists to use. It is also documented as becoming the default for `-p`
//! in a future release, which would silently break subscription billing — hence
//! [`assert_subscription_auth`], which fails loudly rather than quietly billing an
//! API account.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{Role, Tool};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::base::{
    ConfigKey, MessageStream, ModelInfo, PendingToolCall, Provider, ProviderMetadata,
    ProviderSteerReceiver, ProviderSteerRequest, ProviderStreamItem, ProviderUsage, Usage,
};
use super::coding_agent::{
    self, bridge, claude_stream, discovery, effort, env as agent_env, mirror, transcript,
    CodingAgentKind,
};
use super::errors::ProviderError;
use super::provider_binding::{AbsoluteCommandPath, ProviderRestoreBinding};
use crate::config::search_path::SearchPaths;
use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;

const KIND: CodingAgentKind = CodingAgentKind::ClaudeCode;

/// Concrete model ids rather than the CLI's `sonnet`/`opus` aliases.
///
/// The aliases are nicer — they track the current model in each family, and a
/// pinned id rots (the previously pinned `claude-sonnet-4-20250514` was retired
/// while it sat in this file). They are still not what is advertised, for two
/// reasons that outweigh it:
///
/// 1. `tests/context_windows.rs` requires every advertised model to have its own
///    entry in `MODEL_CONTEXT_WINDOWS`, and an alias has none — so `sonnet` would
///    silently take the 128k default and the settings UI would display a wrong
///    window for a 1M model.
/// 2. Adding `"sonnet"` and `"opus"` to that table would put two very short
///    substrings into a globally-served pattern list, where any *future* model id
///    containing them but lacking its own exact entry would inherit this window.
///    That is safe only because `get_all_model_limits` happens to sort
///    longest-first, which nothing pins as an invariant.
///
/// `with_unlisted_models` still lets a user type `sonnet` by hand, and the CLI
/// accepts both spellings — verified against `claude` 2.1.235.
///
/// # Why Fable 5.1 is the default
///
/// Measured 2026-09-08: `claude --model claude-fable-5-1 -p … --output-format
/// json` answers on `claude` 2.1.260 (the binary the desktop app bundles) with
/// `modelUsage["claude-fable-5-1"].contextWindow` = 1,000,000, and both the
/// `fable` and `best` aliases resolve to it. It is Anthropic's current flagship
/// and the operator's own default.
///
/// ⚠ It needs a recent CLI. On `claude` 2.1.235 (the copy that was on this
/// machine's PATH) the API refuses it outright:
///
/// > 400 Claude Code 2.1.235 does not support this model; version **2.1.251**
/// > or newer is required. Run 'claude update', or update the Claude desktop
/// > app, then try again.
///
/// Anthropic's own model-config page says 2.1.255; the API's error is the
/// tighter, measured floor, so both numbers are recorded rather than averaged.
/// That failure is *actionable* — Biorouter surfaces the vendor sentence
/// verbatim, naming the installed version and the fix — which is why a default
/// that requires a current CLI is acceptable rather than a trap.
pub const CLAUDE_CODE_DEFAULT_MODEL: &str = "claude-fable-5-1";

pub const CLAUDE_CODE_DOC_URL: &str = "https://code.claude.com/docs/en/headless";

/// Models advertised in the picker, with each id's measured context window.
///
/// `ProviderMetadata::with_models` is used rather than `::new` because `::new`
/// hard-codes `supports_vision: None` (see `base.rs`), and this catalog carries a
/// per-model vision answer: all four of these take image input, and Codex's
/// `gpt-5.3-codex-spark` next door does not — recording a known fact as unknown is
/// its own defect.
///
/// The windows are *not* the reason. These are concrete ids rather than the CLI's
/// `sonnet`/`opus` aliases (see `CLAUDE_CODE_DEFAULT_MODEL` above on why), each
/// with its own exact `MODEL_CONTEXT_WINDOWS` entry, so `::new`'s
/// `ModelConfig::new_or_fail(name).context_limit()` would resolve every one of
/// them correctly — and `tests/context_windows.rs` pins them under either
/// constructor.
fn known_models() -> Vec<ModelInfo> {
    // Each window must equal the one `MODEL_CONTEXT_WINDOWS` declares, because
    // `tests/context_windows.rs::provider_declared_windows_match_the_registry`
    // compares the two.
    //
    // Anthropic's **current** lineup, and nothing else. Measured 2026-09-08
    // against two CLIs rather than read off a changelog: `claude` 2.1.235 (the
    // copy on this machine's PATH) for `claude-opus-5`, `claude-sonnet-5` and
    // `claude-haiku-4-5`, and `claude` 2.1.260 (the binary the desktop app
    // bundles) for `claude-fable-5-1`, which 2.1.235 cannot run at all. Each
    // window below is the number the CLI itself reported in
    // `modelUsage[<id>].contextWindow` for a real one-turn
    // completion, and it matches
    // https://platform.claude.com/docs/en/about-claude/models/overview, which
    // also states that every current model takes text *and* image input —
    // hence `with_vision()` on all four.
    //
    // ⚠ Fable 5.1 needs a recent CLI: **2.1.251 or newer** per the API's own
    // 400 on 2.1.235 (Anthropic's model-config page says 2.1.255). See
    // `CLAUDE_CODE_DEFAULT_MODEL` — the refusal is surfaced verbatim, so it
    // reads as "update the CLI", not as a Biorouter fault.
    //
    // ⚠ The probe has to be checked before it is trusted: `claude --model X -p`
    // ACCEPTS an unknown id and merely warns —
    //   "X" is not a model this version of Claude Code recognizes, so
    //   auto-compact will keep this session within 200k tokens
    // — so "it ran" proves nothing on its own. `definitely-not-a-model`,
    // `claude-opus-99` and `gpt-4` each produce that warning; the four below
    // produce a clean answer, which is what makes the list evidence.
    //
    // Two ids that used to be here are gone, and only one of them was merely
    // stale:
    //
    //   * `claude-fable-5` — legacy, superseded at the same tier by Fable 5.1.
    //     Anthropic still serves it, and `with_unlisted_models` still lets a
    //     user type it, so nothing breaks; it just is not advertised.
    //   * `claude-sonnet-4-6` — legacy, and the 1,000,000 this catalog
    //     declared for it was a live **defect** in the picker. A bare
    //     `claude-sonnet-4-6` runs at **200,000** on this plan (measured);
    //     Sonnet 4.6's 1M window needs the `[1m]` id suffix plus usage credits
    //     on Max, so the picker was offering a budget five times the one the
    //     agent actually had.
    //
    //     ⚠ Dropping it from this list fixes the **picker** and nothing else.
    //     `MODEL_CONTEXT_WINDOWS` still declares `("claude-sonnet-4-6",
    //     1_000_000)`, because the anthropic, Bedrock and Databricks providers
    //     genuinely serve that id at 1M and need the entry; and
    //     `with_unlisted_models` still lets a user type it here. So a
    //     hand-typed `claude-sonnet-4-6` on *this* provider still shows 1M
    //     against a CLI serving 200k. The window is a per-provider fact, and a
    //     registry keyed on the model name alone has no way to say so.
    vec![
        ModelInfo::new("claude-fable-5-1", 1_000_000).with_vision(),
        ModelInfo::new("claude-opus-5", 1_000_000).with_vision(),
        ModelInfo::new("claude-sonnet-5", 1_000_000).with_vision(),
        ModelInfo::new("claude-haiku-4-5", 200_000).with_vision(),
    ]
}

#[derive(Debug, serde::Serialize)]
pub struct ClaudeCodeProvider {
    command: PathBuf,
    model: ModelConfig,
    #[serde(skip)]
    name: String,
}

impl ClaudeCodeProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        // Resolution only — a few stat calls. Nothing here may spawn a process:
        // `GET /config/providers` constructs every configured provider under a
        // 3-second timeout to sample tier and affiliation, so a probe here would
        // stall the whole settings page.
        let configured = discovery::configured_command(KIND);
        let command = discovery::resolve_binary(KIND, configured.as_deref()).ok_or_else(|| {
            anyhow::anyhow!(
                "could not find the `{}` command. Install it with:\n    {}\n\nIf it is installed \
                 somewhere Biorouter does not search (nvm, volta, bun, asdf), set {} to its full \
                 path.",
                KIND.default_command(),
                KIND.install_hint(),
                KIND.command_config_key(),
            )
        })?;

        Self::from_resolved(model, AbsoluteCommandPath::resolve(command)?)
    }

    pub(crate) fn from_resolved(model: ModelConfig, command: AbsoluteCommandPath) -> Result<Self> {
        let command = AbsoluteCommandPath::new(command.into_path_buf())?.into_path_buf();
        Ok(Self {
            command,
            model,
            name: KIND.provider_id().to_string(),
        })
    }

    /// Construct a provider pointed at an arbitrary binary, for tests.
    ///
    /// Exists so an integration test can drive the real `stream()` against a
    /// fake `claude` that replays recorded frames — the only way to test the
    /// whole chain (argv construction, spawn, routing, decoding, mirroring)
    /// rather than its pieces. `from_env` cannot serve that purpose: it resolves
    /// the user's actual CLI.
    #[doc(hidden)]
    pub fn for_tests(command: PathBuf, model: &str) -> Self {
        Self {
            command,
            model: ModelConfig::new(model).expect("a valid test model"),
            name: KIND.provider_id().to_string(),
        }
    }

    /// The arguments shared by every invocation.
    ///
    /// `output_format` is the only axis that varies: `json` for a single blocking
    /// result, `stream-json` for the streaming path.
    fn base_args(
        &self,
        model_config: &ModelConfig,
        system: &str,
        output_format: &str,
        mcp_config: Option<&std::path::Path>,
    ) -> Vec<String> {
        let mut args: Vec<String> = vec!["-p".into()];

        // Isolation. See the module header — both of these are security-relevant
        // and neither is a substitute for the other.
        args.push("--setting-sources".into());
        args.push(String::new());
        args.push("--strict-mcp-config".into());

        // The child's own agentic tools are off. Biorouter's tools arrive over
        // MCP, where Biorouter executes them behind its own gates.
        args.push("--tools".into());
        args.push(String::new());

        // Biorouter's own tools, when this turn has a bridge. `--strict-mcp-config`
        // above means this is the *only* MCP server the child sees, so the set is
        // exactly the session's tool surface and nothing of the user's own.
        //
        // `bypassPermissions` looks alarming and is the correct mode here: with
        // built-ins off, the only tools that exist are Biorouter's, and each one is
        // inspected and permission-checked on Biorouter's side of the bridge before
        // it runs. Leaving the child to prompt instead would stall the turn, since
        // a `-p` session has nobody to ask.
        if let Some(path) = mcp_config {
            args.push("--mcp-config".into());
            args.push(path.to_string_lossy().into_owned());
            args.push("--permission-mode".into());
            args.push("bypassPermissions".into());
        }

        // Biorouter's system prompt replaces Claude Code's, rather than being
        // appended to it.
        //
        // The tools note is appended for the same reason it is on the Codex
        // side: a coding-agent child arrives with native tools Biorouter
        // deliberately switches off, and naming the real ones is cheaper than
        // letting the model discover the disabled ones by failing.
        args.push("--system-prompt".into());
        args.push(format!(
            "{system}{}",
            coding_agent::native_tools_notice(bridge::active_bridge_url().as_deref())
        ));

        // Sessions are Biorouter's to persist. Letting the CLI also write them
        // would leave a second, divergent transcript on disk that no Biorouter
        // control governs.
        args.push("--no-session-persistence".into());

        args.push("--output-format".into());
        args.push(output_format.into());

        let model = model_config.model_name.trim();
        if !model.is_empty() {
            args.push("--model".into());
            args.push(model.to_string());
        }

        // BR-63: the turn's reasoning effort, on the CLI's own ladder rather than
        // the OpenAI-family low/high pair — `coding_agent::effort` owns the table
        // and the reasoning.
        //
        // Always emitted, including for the default: `Normal` (which arrives as
        // `None`) maps to `high`, so a turn from a user who never touched
        // `/effort` asks for more reasoning than the model would have chosen.
        args.push("--effort".into());
        args.push(effort::claude_effort(model_config.reasoning_effort).to_string());

        args
    }

    /// Build the child process. Returns it unspawned so both paths share exactly
    /// one construction site.
    fn command_for(
        &self,
        model_config: &ModelConfig,
        system: &str,
        output_format: &str,
        mcp_config: Option<&std::path::Path>,
    ) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.command);
        cmd.args(self.base_args(model_config, system, output_format, mcp_config));

        // The child shells out to git, ripgrep and node on its own account, so
        // handing it the resolved absolute path is not enough — it needs the
        // augmented PATH too. Two of the four deleted CLI providers did this and
        // two did not; the two that did not were the ones that failed under the
        // GUI's truncated PATH.
        //
        // ⚠ `node` is the one that decides whether the child runs at all where
        // the CLI is installed as a Node script rather than a native binary —
        // the Codex provider fails outright on exactly that, exiting 127 before
        // it opens stdout. `claude` is a native binary on some installs and an
        // npm shim on others, so it gets the same treatment: the CLI's own
        // directory first (an npm global install puts `node` beside it), then
        // the version managers a GUI child never inherits, because it does not
        // run a shell profile.
        let mut search = SearchPaths::builder().with_npm().with_node_runtimes();
        if let Some(dir) = self.command.parent() {
            search = search.with_leading_dir(dir);
        }
        if let Ok(path) = search.path() {
            cmd.env("PATH", path);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // LAST, after every arg and env. Both scrubs write the same env map that
        // `.env()` writes, so a later `.env()` would re-admit what they removed —
        // for the daemon scrub that means `BIOROUTER_SERVER__SECRET_KEY`, which
        // would make this child a fully authenticated client of Biorouter's own
        // REST API (issue #57).
        agent_env::configure_subscription_child(&mut cmd);
        cmd
    }

    fn completion_command(
        &self,
        model_config: &ModelConfig,
        system: &str,
        mcp_config: Option<&std::path::Path>,
        prompt: &transcript::Prompt,
    ) -> tokio::process::Command {
        let multimodal = !prompt.images.is_empty();
        let mut cmd = self.command_for(
            model_config,
            system,
            if multimodal { "stream-json" } else { "json" },
            mcp_config,
        );
        if multimodal {
            cmd.arg("--input-format");
            cmd.arg("stream-json");
            cmd.arg("--verbose");
        }
        cmd
    }

    /// Refuse to continue if the run is not actually on the subscription.
    ///
    /// `system/init` reports `apiKeySource`, which is `"none"` under subscription
    /// auth. The scrub in [`agent_env`] should already have made anything else
    /// impossible, so reaching this is a real defect — a settings-file
    /// `apiKeyHelper` (which outranks the OAuth token), or a future release
    /// flipping `-p` to `--bare`. Either way the correct response is to stop,
    /// because the alternative is billing an account the user did not choose.
    fn assert_subscription_auth(source: Option<&str>) -> Result<(), ProviderError> {
        match source {
            None | Some("none") => Ok(()),
            Some(other) => Err(ProviderError::Authentication(format!(
                "This run would have been billed to {other} rather than to your Claude \
                 subscription, so it was stopped.\n\nBiorouter removes API credentials from the \
                 environment it starts `claude` in, so something outside the environment is \
                 supplying one — most likely an `apiKeyHelper` in a Claude Code settings file, \
                 which outranks subscription sign-in."
            ))),
        }
    }

    /// Map a `system/api_retry` error category, or a nonzero exit, onto a typed
    /// provider error.
    ///
    /// Typing this matters: every one of the deleted CLI providers collapsed all
    /// failures into `RequestFailed`, so the retry layer could not tell a
    /// credential problem it must not retry from a blip it should.
    fn classify(category: Option<&str>, detail: String) -> ProviderError {
        match category {
            Some("authentication_failed") | Some("oauth_org_not_allowed") => {
                ProviderError::Authentication(detail)
            }
            Some("rate_limit") => ProviderError::RateLimitExceeded {
                details: detail,
                retry_delay: None,
            },
            Some("billing_error") => ProviderError::Authentication(detail),
            Some("overloaded") | Some("server_error") => ProviderError::ServerError(detail),
            Some("max_output_tokens") => ProviderError::ContextLengthExceeded(detail),
            Some("model_not_found") | Some("invalid_request") => {
                ProviderError::RequestFailed(detail)
            }
            _ => ProviderError::RequestFailed(detail),
        }
    }

    /// Run one turn and return every stdout line, plus stderr.
    ///
    /// stderr is drained **concurrently** with stdout. All four deleted providers
    /// piped stderr and never read it, which threw away the CLI's own diagnostic
    /// on every failure and could deadlock a child that wrote more than the pipe
    /// buffer to it.
    async fn run(
        &self,
        mut cmd: tokio::process::Command,
        prompt: &transcript::Prompt,
    ) -> Result<(Vec<String>, String, std::process::ExitStatus), ProviderError> {
        // ⚠ `kill_on_drop`, because a cancelled turn drops this future rather
        // than unwinding it. `drive_stream`'s hard-cancellation escape breaks
        // out of its `select!` while the provider call is still pending, the
        // stream is dropped, and every explicit reap below is skipped. Without
        // this the vendor CLI keeps running detached, holding the user's
        // subscription credential and burning their quota on an answer nobody
        // will read - and on the Codex path it also keeps the app-server port.
        // The default is false, which is why this has to be said out loud.
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            ProviderError::ExecutionError(format!(
                "could not start `{}`: {e}",
                self.command.display()
            ))
        })?;

        // The prompt goes on stdin, never in argv. A flattened conversation can
        // be far larger than the platform's argv limit, and the deleted provider
        // put the whole thing in a single `-p` argument.
        if let Some(mut stdin) = child.stdin.take() {
            let bytes = encode_claude_completion_input(prompt)?;
            tokio::spawn(async move {
                let _ = stdin.write_all(&bytes).await;
                let _ = stdin.shutdown().await;
            });
        }

        let stdout = child.stdout.take().ok_or_else(|| {
            ProviderError::ExecutionError("could not capture claude stdout".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ProviderError::ExecutionError("could not capture claude stderr".into())
        })?;

        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut out = Vec::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    out.push(line);
                }
            }
            out
        });
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut out = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                out.push_str(&line);
                out.push('\n');
            }
            out
        });

        let waited = coding_agent::await_turn(child.wait(), coding_agent::turn_timeout()).await;
        let status = match waited {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                return Err(ProviderError::ExecutionError(format!(
                    "waiting for `claude` failed: {e}"
                )))
            }
            Err(elapsed) => {
                let _ = child.start_kill();
                return Err(ProviderError::ExecutionError(format!(
                    "`claude` did not finish within {}s and was stopped",
                    elapsed.duration().as_secs()
                )));
            }
        };

        let lines = stdout_task.await.unwrap_or_default();
        let errors = stderr_task.await.unwrap_or_default();
        Ok((lines, errors, status))
    }

    /// Parse the result object emitted by either JSON output mode.
    fn parse_result_object(
        &self,
        model: &str,
        lines: &[String],
        stderr: &str,
        status: std::process::ExitStatus,
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // `system/init` may precede the result object when the CLI emits startup
        // frames, so scan rather than assuming a single line.
        let mut result: Option<Value> = None;
        let mut api_key_source: Option<String> = None;
        let mut retry_category: Option<String> = None;

        for line in lines {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("system") => match v.get("subtype").and_then(Value::as_str) {
                    Some("init") => {
                        api_key_source = v
                            .get("apiKeySource")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    Some("api_retry") => {
                        retry_category = v.get("error").and_then(Value::as_str).map(str::to_string);
                    }
                    _ => {}
                },
                Some("result") => result = Some(v),
                _ => {}
            }
        }

        Self::assert_subscription_auth(api_key_source.as_deref())?;

        let Some(result) = result else {
            let detail = if stderr.trim().is_empty() {
                format!("`claude` produced no result (exit {:?})", status.code())
            } else {
                format!(
                    "`claude` produced no result (exit {:?}): {}",
                    status.code(),
                    stderr.trim()
                )
            };
            return Err(Self::classify(retry_category.as_deref(), detail));
        };

        if result.get("is_error").and_then(Value::as_bool) == Some(true) {
            // Through `unwrap_json_error`, exactly as `claude_stream`'s terminal
            // frame is: `result` carries the vendor's own text, usually a
            // sentence but sometimes the upstream API's JSON body verbatim.
            // Unwrapping is a no-op on the sentence and is what keeps the
            // envelope out of the chat. Both surfaces read it the same way, so
            // a streamed turn and a blocking one cannot say different things
            // about the same failure.
            let detail = result
                .get("result")
                .and_then(Value::as_str)
                .map(super::coding_agent::unwrap_json_error)
                .or_else(|| Some(stderr.trim().to_string()))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "`claude` reported an error".into());
            let category = result
                .get("subtype")
                .and_then(Value::as_str)
                .or(retry_category.as_deref());
            return Err(Self::classify(category, detail));
        }

        let text = result
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if text.trim().is_empty() {
            return Err(ProviderError::RequestFailed(
                "`claude` returned an empty response".into(),
            ));
        }

        let usage = parse_usage(result.get("usage"));
        let message = Message::new(
            Role::Assistant,
            chrono::Utc::now().timestamp(),
            vec![MessageContent::text(text)],
        );

        // The provider field must be set, or the usage row is attributed by model
        // name and `canonical_model_pricing` invents a per-token price for a run
        // that billed a subscription — the exact failure
        // `pricing::blocks_fallback_pricing` exists to prevent, reached by a
        // different route.
        let mut provider_usage = ProviderUsage::new(model.to_string(), usage);
        provider_usage.provider = Some(KIND.provider_id().to_string());
        Ok((message, provider_usage))
    }
}

/// Write this turn's bridge configuration to a private file, if there is a bridge.
///
/// A file rather than an inline `--mcp-config` JSON string, even though the CLI
/// accepts both: the URL carries the turn's capability nonce, and argv is readable
/// by any process running as the same user. `NamedTempFile` creates it 0600.
///
/// Forward whatever the Anthropic decoder can produce **without waiting**.
///
/// Called immediately before a tool card is emitted, so the prose that preceded
/// the call is already in the transcript when the card lands. `now_or_never`
/// rather than `await` is the whole point: the decoder's stream only ends when
/// its input channel closes, so awaiting it here would deadlock the reader
/// against a child that has not finished the turn. An item the decoder is not
/// ready to yield is simply picked up at the next flush point, which delays it
/// but cannot reorder it.
///
/// Returns `false` once the consumer has dropped the stream, which is the
/// reader's signal to stop and let the child be reaped.
fn drain_ready<S>(
    decoded: &mut std::pin::Pin<Box<S>>,
    out: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
) -> bool
where
    S: futures::Stream<Item = anyhow::Result<ProviderStreamItem>>,
{
    use futures::FutureExt;
    while let Some(ready) = futures::StreamExt::next(decoded).now_or_never() {
        let Some(item) = ready else {
            // The decoder ended; nothing further will come from it.
            return true;
        };
        let item = item.map_err(|e| ProviderError::RequestFailed(e.to_string()));
        if out.send(item).is_err() {
            return false;
        }
    }
    true
}

/// Accumulated arguments for one in-flight tool call, plus the throttle that
/// decides when the growing preview is worth another frame.
///
/// ⚠ **Never emit one of these per delta.** Anthropic sends an
/// `input_json_delta` every few tokens, and each notification carries the whole
/// argument string accumulated so far — so a per-delta emit is quadratic in the
/// argument size. A bridged `text_editor` write of a 60 KB file would push tens
/// of megabytes through an unbounded channel and out over SSE, to redraw a
/// preview the card truncates anyway. The shared Anthropic decoder throttles for
/// exactly this reason (`formats/anthropic.rs`), and this mirrors its policy so
/// the two paths cost the same.
#[derive(Default)]
struct PendingArgs {
    /// The display name, remembered from `content_block_start`.
    ///
    /// Carried on EVERY preview, not just the first. The shared Anthropic
    /// decoder does the same (`formats/anthropic.rs` repeats `name.clone()` on
    /// each throttled update), and a consumer that renders each notification as
    /// a line — the CLI does, as `[tool] <name> …` — prints a blank one without
    /// it. Measured against a live turn: three `[tool]  …` lines with no name.
    name: String,
    text: String,
    /// Length at the last emit, for the size trigger.
    emitted_len: usize,
    /// When the last emit happened, for the time trigger. `None` until the
    /// first, so the first delta always produces a preview.
    emitted_at: Option<std::time::Instant>,
}

impl PendingArgs {
    /// The snapshot to send now, or `None` if it is too soon.
    fn take_due_snapshot(&mut self) -> Option<String> {
        let due_by_size = self.text.len().saturating_sub(self.emitted_len) >= PENDING_ARGS_CHARS;
        let due_by_time = self
            .emitted_at
            .map(|t| t.elapsed() >= PENDING_ARGS_INTERVAL)
            .unwrap_or(true);
        if !due_by_size && !due_by_time {
            return None;
        }
        self.emitted_len = self.text.len();
        self.emitted_at = Some(std::time::Instant::now());
        Some(self.text.clone())
    }
}

/// Minimum wall-clock gap between two partial-argument previews for one call.
const PENDING_ARGS_INTERVAL: Duration = Duration::from_millis(200);
/// …or this many newly accumulated characters, whichever comes first.
const PENDING_ARGS_CHARS: usize = 200;

/// Turn one diverted `tool_use` event into what the GUI draws.
///
/// The lifecycle mirrors an API provider's exactly, which is the point — the
/// same `ToolCallWithResponse` card, the same status progression:
///
/// * `Opened` → a `PendingToolCall`, so the skeleton card appears the moment the
///   tool's name is known, before its arguments have finished arriving;
/// * `ArgsDelta` → the same pending card with the arguments so far;
/// * `Call` → the authoritative **marked** `ToolRequest`, which replaces the
///   skeleton (the store matches on the call id) and shows the card as running;
/// * `Result` → the **marked** `ToolResponse`, which settles the card green or
///   red and fills in what the tool returned.
///
/// Every message minted here is marked [`mirror::Execution::Bridged`]: the call
/// reached Biorouter over the tool bridge and ran behind its inspectors,
/// permission mode, `.biorouterignore`, vault and privacy Gate C. The mark is
/// what stops the agent loop dispatching it a second time.
///
/// `bridge_url` is this turn's bridge, whose grant kept Biorouter's own result
/// for each call: the `Result` arm stores that rather than Claude Code's echo,
/// which has lost every annotation (QA-E F4 — see
/// [`mirror::stored_bridged_result`]).
fn emit_tool_event(
    event: claude_stream::ToolBlockEvent,
    partial_args: &mut std::collections::HashMap<String, PendingArgs>,
    out: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
    bridge_url: Option<&str>,
) -> bool {
    let send = |item: ProviderStreamItem| out.send(Ok(item)).is_ok();

    match event {
        claude_stream::ToolBlockEvent::Opened { id, name, .. } => {
            let display = mirror::display_tool_name(&name).to_string();
            partial_args.insert(
                id.clone(),
                PendingArgs {
                    name: display.clone(),
                    ..PendingArgs::default()
                },
            );
            send((
                None,
                None,
                Some(PendingToolCall {
                    id,
                    name: display,
                    partial_args: None,
                }),
            ))
        }
        claude_stream::ToolBlockEvent::ArgsDelta {
            id, partial_json, ..
        } => {
            let buffered = partial_args.entry(id.clone()).or_default();
            buffered.text.push_str(&partial_json);
            // Throttled, never per delta — see `PendingArgs`.
            let name = buffered.name.clone();
            match buffered.take_due_snapshot() {
                Some(snapshot) => send((
                    None,
                    None,
                    Some(PendingToolCall {
                        id,
                        name,
                        partial_args: Some(snapshot),
                    }),
                )),
                None => true,
            }
        }
        claude_stream::ToolBlockEvent::Closed { .. } => true,
        claude_stream::ToolBlockEvent::Call { calls, .. } => {
            for call in calls {
                partial_args.remove(&call.id);
                let message = mirror::request_message(
                    &call.id,
                    &call.name,
                    call.input,
                    mirror::Execution::Bridged,
                );
                if !send((Some(message), None, None)) {
                    return false;
                }
            }
            true
        }
        claude_stream::ToolBlockEvent::Result { results } => {
            for result in results {
                let message = match mirror::stored_bridged_result(
                    bridge_url,
                    &result.tool_use_id,
                    &result.content,
                    result.is_error,
                ) {
                    Some(recorded) => mirror::response_message_with_result(
                        &result.tool_use_id,
                        recorded,
                        mirror::Execution::Bridged,
                    ),
                    None => mirror::response_message(
                        &result.tool_use_id,
                        mirror::content_from_value(&result.content),
                        result.is_error,
                        mirror::Execution::Bridged,
                    ),
                };
                if !send((Some(message), None, None)) {
                    return false;
                }
            }
            true
        }
    }
}

/// Everything the stdout pump owns for the length of one turn.
///
/// A struct rather than six positional arguments because every one of these
/// is load-bearing for a *different* reason, and a positional list invites
/// dropping one: the child must be owned here so aborting the task reaps it,
/// and the bridge config file must be owned here because dropping the
/// `NamedTempFile` deletes the MCP configuration out from under a child that
/// is still starting.
struct PumpInputs {
    child: tokio::process::Child,
    bridge_config: Option<tempfile::NamedTempFile>,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr_task: tokio::task::JoinHandle<String>,
    initial_prompt: transcript::Prompt,
    steering: Option<ProviderSteerReceiver>,
    model_name: String,
    /// Captured when the stream is built: the task-local is gone by the time
    /// this task reads frames.
    bridge_url: Option<String>,
    out_tx: tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
}

enum ClaudePumpInput {
    Output(std::io::Result<Option<String>>),
    Steer(Option<ProviderSteerRequest>),
    Timeout(Duration),
}

async fn next_claude_pump_input(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    steering: &mut Option<ProviderSteerReceiver>,
    deadline: Option<(tokio::time::Instant, Duration)>,
) -> ClaudePumpInput {
    tokio::select! {
        line = lines.next_line() => ClaudePumpInput::Output(line),
        request = async {
            match steering.as_mut() {
                Some(steering) => steering.recv().await,
                None => std::future::pending().await,
            }
        } => ClaudePumpInput::Steer(request),
        duration = async {
            match deadline {
                Some((deadline, duration)) => {
                    tokio::time::sleep_until(deadline).await;
                    duration
                }
                None => std::future::pending().await,
            }
        } => ClaudePumpInput::Timeout(duration),
    }
}

async fn write_claude_user_message(
    stdin: &mut tokio::process::ChildStdin,
    text: &str,
) -> Result<(), ProviderError> {
    write_claude_line(stdin, encode_claude_user_message(serde_json::json!(text))?).await
}

fn encode_claude_prompt(prompt: &transcript::Prompt) -> Result<Vec<u8>, ProviderError> {
    let content = if prompt.images.is_empty() {
        serde_json::json!(prompt.text)
    } else {
        let mut content = vec![serde_json::json!({ "type": "text", "text": prompt.text })];
        content.extend(prompt.images.iter().map(|image| {
            serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": image.mime_type,
                    "data": image.data,
                }
            })
        }));
        Value::Array(content)
    };
    encode_claude_user_message(content)
}

fn encode_claude_completion_input(prompt: &transcript::Prompt) -> Result<Vec<u8>, ProviderError> {
    if prompt.images.is_empty() {
        return Ok(prompt.text.as_bytes().to_vec());
    }
    encode_claude_prompt(prompt)
}

fn encode_claude_user_message(content: Value) -> Result<Vec<u8>, ProviderError> {
    let message = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": content,
        },
        "parent_tool_use_id": null,
    });
    let mut line = serde_json::to_vec(&message)
        .map_err(|e| ProviderError::ExecutionError(format!("encoding Claude input failed: {e}")))?;
    line.push(b'\n');
    Ok(line)
}

async fn write_claude_line(
    stdin: &mut tokio::process::ChildStdin,
    line: Vec<u8>,
) -> Result<(), ProviderError> {
    stdin.write_all(&line).await.map_err(|e| {
        ProviderError::ExecutionError(format!("writing to the running `claude` turn failed: {e}"))
    })?;
    stdin.flush().await.map_err(|e| {
        ProviderError::ExecutionError(format!("flushing the running `claude` turn failed: {e}"))
    })
}

enum ClaudeLineOutcome {
    Line(String),
    Continue,
    Stop,
    SteerWritten(ProviderSteerRequest),
    Failure {
        error: ProviderError,
        kill_child: bool,
    },
}

async fn next_claude_line(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    steering: &mut Option<ProviderSteerReceiver>,
    stdin: &mut tokio::process::ChildStdin,
    deadline: Option<(tokio::time::Instant, Duration)>,
) -> ClaudeLineOutcome {
    match next_claude_pump_input(lines, steering, deadline).await {
        ClaudePumpInput::Timeout(duration) => ClaudeLineOutcome::Failure {
            error: ProviderError::ExecutionError(format!(
                "`claude` did not finish within {}s and was stopped",
                duration.as_secs()
            )),
            kill_child: true,
        },
        ClaudePumpInput::Output(Ok(Some(line))) => ClaudeLineOutcome::Line(line),
        ClaudePumpInput::Output(Ok(None)) => ClaudeLineOutcome::Stop,
        ClaudePumpInput::Output(Err(error)) => ClaudeLineOutcome::Failure {
            error: ProviderError::ExecutionError(format!(
                "reading `claude` output failed: {error}"
            )),
            kill_child: false,
        },
        ClaudePumpInput::Steer(None) => {
            *steering = None;
            ClaudeLineOutcome::Continue
        }
        ClaudePumpInput::Steer(Some(request)) => {
            match write_claude_user_message(stdin, request.text()).await {
                Ok(()) => ClaudeLineOutcome::SteerWritten(request),
                Err(error) => {
                    let detail = error.to_string();
                    request.reject(error);
                    ClaudeLineOutcome::Failure {
                        error: ProviderError::ExecutionError(detail),
                        kill_child: true,
                    }
                }
            }
        }
    }
}

enum ClaudeFrameOutcome {
    Continue,
    ConsumerClosed,
    Terminal(Result<ProviderUsage, ProviderError>),
    RejectAuthentication(ProviderError),
}

fn route_claude_frame<S>(
    frame: claude_stream::RoutedFrame,
    line_tx: &tokio::sync::mpsc::UnboundedSender<anyhow::Result<String>>,
    decoded: &mut std::pin::Pin<Box<S>>,
    partial_args: &mut std::collections::HashMap<String, PendingArgs>,
    model_name: &str,
    out_tx: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
    bridge_url: Option<&str>,
) -> ClaudeFrameOutcome
where
    S: futures::Stream<Item = anyhow::Result<ProviderStreamItem>>,
{
    match frame {
        claude_stream::RoutedFrame::AnthropicEvent(data) => {
            if line_tx.send(Ok(data)).is_err() || !drain_ready(decoded, out_tx) {
                ClaudeFrameOutcome::ConsumerClosed
            } else {
                ClaudeFrameOutcome::Continue
            }
        }
        claude_stream::RoutedFrame::Tool(event) => {
            // Everything the decoder already produced belongs before this card.
            if !drain_ready(decoded, out_tx)
                || !emit_tool_event(event, partial_args, out_tx, bridge_url)
            {
                ClaudeFrameOutcome::ConsumerClosed
            } else {
                ClaudeFrameOutcome::Continue
            }
        }
        claude_stream::RoutedFrame::Init { api_key_source } => {
            match ClaudeCodeProvider::assert_subscription_auth(api_key_source.as_deref()) {
                Ok(()) => ClaudeFrameOutcome::Continue,
                Err(error) => ClaudeFrameOutcome::RejectAuthentication(error),
            }
        }
        claude_stream::RoutedFrame::UserReplay(_) => ClaudeFrameOutcome::Continue,
        claude_stream::RoutedFrame::Terminal(frame) => {
            let terminal = match frame.error {
                Some(err) => Err(ClaudeCodeProvider::classify(
                    err.category.as_deref(),
                    err.detail,
                )),
                None => {
                    let mut usage = ProviderUsage::new(model_name.to_string(), frame.usage);
                    usage.provider = Some(KIND.provider_id().to_string());
                    // The terminal frame owns the finish reason. Inner requests
                    // may report `max_tokens`, but the child already handled
                    // their continuation; propagating one would make Biorouter
                    // launch another whole child turn after this one completed.
                    usage.finish_reason = Some("stop".to_string());
                    Ok(usage)
                }
            };
            ClaudeFrameOutcome::Terminal(terminal)
        }
        claude_stream::RoutedFrame::Ignored => ClaudeFrameOutcome::Continue,
    }
}

enum ClaudeLoopOutcome {
    Continue,
    Stop(Option<Result<ProviderUsage, ProviderError>>),
    Kill(ProviderError),
}

struct ClaudeFrameContext<'a, S> {
    pending_steers: &'a mut std::collections::VecDeque<ProviderSteerRequest>,
    line_tx: &'a tokio::sync::mpsc::UnboundedSender<anyhow::Result<String>>,
    decoded: &'a mut std::pin::Pin<Box<S>>,
    partial_args: &'a mut std::collections::HashMap<String, PendingArgs>,
    model_name: &'a str,
    out_tx: &'a tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
    completed_usage: &'a mut Option<ProviderUsage>,
    outstanding_turns: &'a mut usize,
    bridge_url: Option<&'a str>,
}

fn apply_claude_frame<S>(
    frame: claude_stream::RoutedFrame,
    context: &mut ClaudeFrameContext<'_, S>,
) -> ClaudeLoopOutcome
where
    S: futures::Stream<Item = anyhow::Result<ProviderStreamItem>>,
{
    if let claude_stream::RoutedFrame::UserReplay(text) = &frame {
        if context
            .pending_steers
            .front()
            .is_some_and(|request| request.text() == text)
        {
            if let Some(request) = context.pending_steers.pop_front() {
                request.acknowledge();
            }
        }
        return ClaudeLoopOutcome::Continue;
    }

    match route_claude_frame(
        frame,
        context.line_tx,
        context.decoded,
        context.partial_args,
        context.model_name,
        context.out_tx,
        context.bridge_url,
    ) {
        ClaudeFrameOutcome::Continue => ClaudeLoopOutcome::Continue,
        ClaudeFrameOutcome::ConsumerClosed => ClaudeLoopOutcome::Stop(None),
        ClaudeFrameOutcome::Terminal(Ok(usage)) => {
            *context.completed_usage = Some(match context.completed_usage.take() {
                Some(previous) => previous.combine_with(&usage),
                None => usage,
            });
            *context.outstanding_turns = context.outstanding_turns.saturating_sub(1);
            if *context.outstanding_turns == 0 {
                ClaudeLoopOutcome::Stop(context.completed_usage.take().map(Ok))
            } else {
                ClaudeLoopOutcome::Continue
            }
        }
        ClaudeFrameOutcome::Terminal(Err(error)) => ClaudeLoopOutcome::Stop(Some(Err(error))),
        ClaudeFrameOutcome::RejectAuthentication(error) => ClaudeLoopOutcome::Kill(error),
    }
}

/// Read the child's stdout to the end of the turn, emitting stream items.
///
/// Owns the child, so aborting this task drops it and `kill_on_drop(true)`
/// reaps it — which is how a cancelled turn stops a `claude` that would
/// otherwise keep spending the user's own subscription quota.
async fn pump_claude_stdout(inputs: PumpInputs) {
    let PumpInputs {
        child,
        bridge_config,
        mut stdin,
        stdout,
        stderr_task,
        initial_prompt,
        mut steering,
        model_name,
        bridge_url,
        out_tx,
    } = inputs;
    let (line_tx, line_rx) = tokio::sync::mpsc::unbounded_channel::<anyhow::Result<String>>();
    // Moved in so they live exactly as long as the read loop: the child
    // (rule 2) and the bridge config file, whose deletion on drop would
    // pull the MCP configuration out from under a child that is still
    // starting.
    let _bridge_config = bridge_config;
    let mut child = child;
    let mut router = claude_stream::ClaudeStreamRouter::new();
    let mut lines = BufReader::new(stdout).lines();
    let mut decoded = Box::pin(
        crate::providers::formats::anthropic::response_to_streaming_message(
            tokio_stream::wrappers::UnboundedReceiverStream::new(line_rx),
        ),
    );
    // Partial arguments per in-flight call, so the skeleton card can show them
    // arriving — with the throttle that makes showing them affordable. See
    // `PendingArgs`.
    let mut partial_args: std::collections::HashMap<String, PendingArgs> =
        std::collections::HashMap::new();

    let deadline = coding_agent::turn_timeout()
        .map(|duration| (tokio::time::Instant::now() + duration, duration));
    let mut terminal: Option<Result<ProviderUsage, ProviderError>> = None;
    let mut completed_usage: Option<ProviderUsage> = None;
    let mut outstanding_turns = 1usize;
    let mut pending_steers = std::collections::VecDeque::new();

    let initial_line = encode_claude_prompt(&initial_prompt);
    if let Err(error) = match initial_line {
        Ok(line) => write_claude_line(&mut stdin, line).await,
        Err(error) => Err(error),
    } {
        let _ = child.start_kill();
        let _ = out_tx.send(Err(error));
        return;
    }

    loop {
        let line = match next_claude_line(&mut lines, &mut steering, &mut stdin, deadline).await {
            ClaudeLineOutcome::Line(line) => line,
            ClaudeLineOutcome::Continue => continue,
            ClaudeLineOutcome::Stop => break,
            ClaudeLineOutcome::SteerWritten(request) => {
                outstanding_turns = outstanding_turns.saturating_add(1);
                pending_steers.push_back(request);
                continue;
            }
            ClaudeLineOutcome::Failure { error, kill_child } => {
                if kill_child {
                    let _ = child.start_kill();
                }
                terminal = Some(Err(error));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let mut context = ClaudeFrameContext {
            pending_steers: &mut pending_steers,
            line_tx: &line_tx,
            decoded: &mut decoded,
            partial_args: &mut partial_args,
            model_name: &model_name,
            out_tx: &out_tx,
            completed_usage: &mut completed_usage,
            outstanding_turns: &mut outstanding_turns,
            bridge_url: bridge_url.as_deref(),
        };
        match apply_claude_frame(router.push_line(&line), &mut context) {
            ClaudeLoopOutcome::Continue => {}
            ClaudeLoopOutcome::Stop(result) => {
                terminal = result;
                break;
            }
            ClaudeLoopOutcome::Kill(error) => {
                let _ = child.start_kill();
                terminal = Some(Err(error));
                break;
            }
        }
    }

    drop(stdin);

    // Closing the line channel ends the decoder stream; draining it to
    // completion is what flushes any text still buffered inside it.
    drop(line_tx);
    while let Some(item) = futures::StreamExt::next(&mut decoded).await {
        let item = item.map_err(|e| ProviderError::RequestFailed(e.to_string()));
        if out_tx.send(item).is_err() {
            return;
        }
    }

    // The authoritative usage (and any failure) goes last, so it is the
    // snapshot the agent keeps.
    let terminal = resolve_terminal(terminal, stderr_task).await;
    let _ = out_tx.send(terminal.map(|usage| (None, Some(usage), None)));
}

/// Settle what the turn ended as, once the child's stdout has closed.
///
/// Split out so the pump stays under the per-function line budget the repo
/// enforces (`scripts/clippy-lint.sh` runs a `too_many_lines` baseline, and the
/// fix for growing past it is to extract rather than to widen the baseline).
async fn resolve_terminal(
    terminal: Option<Result<ProviderUsage, ProviderError>>,
    stderr_task: tokio::task::JoinHandle<String>,
) -> Result<ProviderUsage, ProviderError> {
    match terminal {
        Some(terminal) => terminal,
        // stdout closed without a `result` frame. stderr is the only explanation
        // a silent child leaves behind, so it is worth waiting for here — and
        // only here, because on every other path it would just delay the answer.
        None => {
            let detail = stderr_task.await.unwrap_or_default();
            let detail = detail.trim();
            Err(ProviderError::RequestFailed(if detail.is_empty() {
                "`claude` produced no result".to_string()
            } else {
                format!("`claude` produced no result: {detail}")
            }))
        }
    }
}

/// `Ok(None)` means this turn has no bridge — a CLI process with no HTTP server, or
/// an agent that did not establish one. The child then runs with no tools at all,
/// which is the correct degradation rather than an error.
fn bridge_mcp_config() -> Result<Option<tempfile::NamedTempFile>, ProviderError> {
    let Some(url) = bridge::active_bridge_url() else {
        return Ok(None);
    };
    // #110: `timeout` is the per-server **millisecond** hard wall clock Claude
    // Code applies to one `tools/call`. Its default killed every bridged call at
    // ~60 s, which turned `workspace_watch`'s advertised 600-second wait — and
    // any other slow Biorouter tool — into "The operation timed out": a
    // transport failure the model may retry, rather than the partial answer the
    // handler was about to return. The CLI's own help is explicit that progress
    // notifications do not extend it, so raising it is the only lever.
    let body = serde_json::json!({
        "mcpServers": {
            "biorouter": {
                "type": "http",
                "url": url,
                "timeout": bridge::child_tool_call_timeout_millis(),
            }
        }
    });

    let mut file = tempfile::Builder::new()
        .prefix("biorouter-bridge-")
        .suffix(".json")
        .tempfile()
        .map_err(|e| {
            ProviderError::ExecutionError(format!("could not write the tool bridge config: {e}"))
        })?;
    use std::io::Write;
    file.write_all(body.to_string().as_bytes()).map_err(|e| {
        ProviderError::ExecutionError(format!("could not write the tool bridge config: {e}"))
    })?;
    file.flush().map_err(|e| {
        ProviderError::ExecutionError(format!("could not write the tool bridge config: {e}"))
    })?;
    Ok(Some(file))
}

/// Pull Biorouter's four disjoint token buckets out of the CLI's `usage` object.
///
/// Claude Code reports the same shape the Anthropic API does, where `input_tokens`
/// already **excludes** both cache buckets — which is exactly the invariant
/// [`Usage`] documents, so no subtraction is needed here.
fn parse_usage(usage: Option<&Value>) -> Usage {
    let Some(u) = usage else {
        return Usage::default();
    };
    let get = |k: &str| u.get(k).and_then(Value::as_i64).map(|v| v as i32);

    let input = get("input_tokens");
    let output = get("output_tokens");
    let cache_read = get("cache_read_input_tokens");
    let cache_creation = get("cache_creation_input_tokens");

    Usage {
        input_tokens: input,
        output_tokens: output,
        // Context occupancy for the live gauge, which for a cache-aware provider
        // includes the cache buckets. Deliberately not the billed total.
        total_tokens: match (input, output) {
            (None, None) => None,
            _ => Some(
                input.unwrap_or(0)
                    + output.unwrap_or(0)
                    + cache_read.unwrap_or(0)
                    + cache_creation.unwrap_or(0),
            ),
        },
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: cache_creation,
    }
}

impl ClaudeCodeProvider {
    async fn stream_inner(
        &self,
        system: &str,
        messages: &[Message],
        steering: Option<ProviderSteerReceiver>,
    ) -> Result<MessageStream, ProviderError> {
        let prompt = transcript::flatten_with_images(messages).ok_or_else(|| {
            ProviderError::RequestFailed(
                "there is no user message for `claude` to answer".to_string(),
            )
        })?;

        let bridge_config = bridge_mcp_config()?;
        let bridge_url = bridge::active_bridge_url();
        let model_config = self.model.clone();
        let model_name = model_config.model_name.clone();

        let mut cmd = self.command_for(
            &model_config,
            system,
            "stream-json",
            bridge_config.as_ref().map(|f| f.path()),
        );
        cmd.arg("--input-format");
        cmd.arg("stream-json");
        cmd.arg("--include-partial-messages");
        cmd.arg("--verbose");
        if steering.is_some() {
            cmd.arg("--replay-user-messages");
        }
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            ProviderError::ExecutionError(format!(
                "could not start `{}`: {e}",
                self.command.display()
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            ProviderError::ExecutionError("could not capture claude stdin".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ProviderError::ExecutionError("could not capture claude stdout".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ProviderError::ExecutionError("could not capture claude stderr".into())
        })?;

        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut out = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                out.push_str(&line);
                out.push('\n');
            }
            out
        });

        let (out_tx, out_rx) =
            tokio::sync::mpsc::unbounded_channel::<Result<ProviderStreamItem, ProviderError>>();
        let reader = tokio::spawn(pump_claude_stdout(PumpInputs {
            child,
            bridge_config,
            stdin,
            stdout,
            stderr_task,
            initial_prompt: prompt,
            steering,
            model_name,
            bridge_url,
            out_tx,
        }));

        let guard = coding_agent::AbortOnDrop(reader.abort_handle());
        let stream = async_stream::try_stream! {
            let _guard = guard;
            let mut out_rx = out_rx;
            while let Some(item) = out_rx.recv().await {
                yield item?;
            }
        };

        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl Provider for ClaudeCodeProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::with_models(
            KIND.provider_id(),
            KIND.display_name(),
            "Uses the Claude subscription you are already signed in to, through your own \
             installed `claude` command. No API key. Requires Anthropic's Claude Code CLI to be \
             installed and signed in.",
            CLAUDE_CODE_DEFAULT_MODEL,
            known_models(),
            CLAUDE_CODE_DOC_URL,
            vec![ConfigKey::new(
                KIND.command_config_key(),
                true,
                false,
                Some(KIND.default_command()),
            )],
        )
        .with_unlisted_models()
        // Public, and NOT `runs_locally`. The subprocess is local; the inference
        // is Anthropic's. Getting this wrong would forge a private badge and let
        // the bind gate attach this provider to a session holding clinical data —
        // which a consumer subscription has no BAA to receive. Both values are
        // the trait defaults, restated here so the decision is visible.
        .with_tier(crate::privacy::ProviderTier::Public)
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn restore_binding(&self) -> ProviderRestoreBinding {
        ProviderRestoreBinding::ClaudeCode {
            model: super::provider_binding::model_without_restore_marker(self.model.clone()),
            command: AbsoluteCommandPath::from_resolved(self.command.clone()),
        }
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    async fn complete_with_model(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // Tools are accepted and not forwarded yet. They cannot simply be dropped
        // once the MCP bridge lands, so this is the one seam that changes then.
        let prompt = transcript::flatten_with_images(messages).ok_or_else(|| {
            ProviderError::RequestFailed(
                "there is no user message for `claude` to answer".to_string(),
            )
        })?;

        // The bridge file must outlive the run, so it is bound here rather than
        // inlined: dropping a `NamedTempFile` deletes it, and a child that started
        // a moment later would find no configuration.
        let bridge_config = bridge_mcp_config()?;
        let cmd = self.completion_command(
            model_config,
            system,
            bridge_config.as_ref().map(|f| f.path()),
            &prompt,
        );
        let (lines, stderr, status) = self.run(cmd, &prompt).await?;
        self.parse_result_object(&model_config.model_name, &lines, &stderr, status)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_live_steering(&self) -> bool {
        true
    }

    /// Stream one turn: the answer appears as the model writes it.
    ///
    /// # The three rules this obeys, each learned the hard way
    ///
    /// 1. **The bridge URL is read HERE, at construction — never from a poll.**
    ///    `Agent::reply` runs the call that builds this stream inside
    ///    `ACTIVE_BRIDGE_URL.scope(...)`, and that scope is gone by the time the
    ///    returned stream is polled. `bridge_mcp_config()` reads the URL, so it
    ///    must be called before the stream is returned — as must the spawn, since
    ///    the child needs the file. See `coding_agent::bridge`'s module header.
    /// 2. **The child is owned by a task the stream aborts on drop.** Cancelling
    ///    a turn drops the provider stream rather than unwinding it, and a
    ///    detached `claude` would keep burning the user's own subscription quota
    ///    on an answer nobody will read. `AbortOnDrop` aborts the reader, which
    ///    drops the child, which `kill_on_drop(true)` then reaps.
    /// 3. **The turn ceiling lives inside the stream.** The blocking path's
    ///    ceiling wraps `child.wait()` in `run`, which this path never calls, and
    ///    the agent loop's cancellation check only fires *between* stream items —
    ///    so without a deadline here a wedged child would hang the session
    ///    forever with user cancel as the only escape.
    ///
    /// Text and thinking are decoded by the Anthropic decoder the API provider
    /// already uses; `claude_stream` diverts every `tool_use` event away from it
    /// first, because that decoder would otherwise mint unmarked `ToolRequest`s
    /// the agent loop would dispatch — a second execution of a call the child
    /// already ran. See `claude_stream`'s module header.
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        self.stream_inner(system, messages, None).await
    }

    async fn stream_with_steering(
        &self,
        system: &str,
        messages: &[Message],
        _tools: &[Tool],
        steering: ProviderSteerReceiver,
    ) -> Result<MessageStream, ProviderError> {
        self.stream_inner(system, messages, Some(steering)).await
    }

    /// Ask the CLI's credential store whether this provider can run at all.
    ///
    /// Spawns, so it must never be called from `from_env` — see
    /// [`discovery`]'s module header.
    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let availability = discovery::probe(KIND).await;
        if !availability.auth.is_subscription() {
            return Err(coding_agent::unavailable_error(KIND, &availability));
        }
        Ok(Some(known_models().into_iter().map(|m| m.name).collect()))
    }
}

#[cfg(test)]
mod bridged_result_tests {
    use super::*;
    use rmcp::model::{CallToolResult, Content};

    const DATE: &str = "Thu Sep 11 01:00:00 PDT 2026";

    fn shell_shaped(output: &str) -> CallToolResult {
        CallToolResult::success(vec![
            Content::text(output).with_audience(vec![Role::Assistant]),
            Content::text(output)
                .with_audience(vec![Role::User])
                .with_priority(0.0),
        ])
    }

    /// Feed one `tool_result` frame through the real handler and return the
    /// `ToolResponse` it stored.
    fn store(
        tool_use_id: &str,
        echo: Value,
        is_error: bool,
        bridge_url: Option<&str>,
    ) -> crate::conversation::message::ToolResponse {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut partial_args = std::collections::HashMap::new();
        assert!(emit_tool_event(
            claude_stream::ToolBlockEvent::Result {
                results: vec![claude_stream::ToolResultBlock {
                    tool_use_id: tool_use_id.to_string(),
                    content: echo,
                    is_error,
                    detail: None,
                }],
            },
            &mut partial_args,
            &tx,
            bridge_url,
        ));
        let Ok(Ok((Some(message), _, _))) = rx.try_recv() else {
            panic!("the handler must emit the response message");
        };
        let MessageContent::ToolResponse(response) = &message.content[0] else {
            panic!("expected a tool response");
        };
        assert_eq!(
            mirror::response_execution(response),
            Some(mirror::Execution::Bridged)
        );
        response.clone()
    }

    /// QA-E F4 through the real handler. Claude Code echoes a shell result with
    /// every annotation gone, so storing the echo made "2 results" of one shell
    /// call and put the output twice into the next turn's prompt. What is stored
    /// is the result the bridge kept for that call — both blocks, audiences
    /// intact: one for the model, one for the card.
    #[test]
    fn a_bridged_result_is_stored_as_the_bridge_recorded_it() {
        let recorded = shell_shaped(DATE);
        let lease = bridge::lease_holding_for_test("toolu_F4", recorded.clone());
        // Claude Code 2.1.266's echo of the view the bridge handed it.
        let echo = serde_json::json!([{ "type": "text", "text": DATE }]);

        let stored = store("toolu_F4", echo, false, Some(lease.url()));

        assert_eq!(stored.tool_result.as_ref().ok(), Some(&recorded));
    }

    /// A child that timed out waiting never saw Biorouter's result, and its
    /// card says what it did see.
    #[test]
    fn a_child_that_timed_out_is_stored_as_it_saw_it() {
        let lease = bridge::lease_holding_for_test("toolu_late", shell_shaped(DATE));
        let echo = serde_json::json!("The operation timed out");

        let stored = store("toolu_late", echo, true, Some(lease.url()));

        let result = stored.tool_result.as_ref().expect("a successful transport");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(
            result.content[0].as_text().map(|t| t.text.as_str()),
            Some("The operation timed out")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::effort::ReasoningEffort;

    #[test]
    fn restore_binding_pins_the_resolved_claude_command() {
        let command = std::env::current_exe().unwrap();
        let provider = ClaudeCodeProvider::from_resolved(
            ModelConfig::new_or_fail("claude-sonnet-4-6"),
            AbsoluteCommandPath::new(command.clone()).unwrap(),
        )
        .unwrap();
        let encoded = serde_json::to_value(provider.restore_binding()).unwrap();
        assert_eq!(encoded["kind"], "claude_code");
        assert_eq!(encoded["command"], serde_json::json!(command));
    }

    /// #110: the bridge config must carry the per-call deadline.
    ///
    /// `timeout` is a **millisecond** field that Claude Code's own help calls a
    /// "hard wall-clock limit per call", and its default abandoned every bridged
    /// call at ~60 s — turning `workspace_watch`'s advertised 600-second wait,
    /// and any other slow Biorouter tool, into "The operation timed out" instead
    /// of the partial result the handler was about to return.
    ///
    /// Asserted here as well as in the live test, because a live test needs a
    /// signed-in CLI and two minutes of a plan's quota: a dropped field should
    /// fail in `cargo test`, not in a user's session.
    #[tokio::test]
    async fn the_bridge_config_carries_the_per_call_deadline() {
        let written = bridge::ACTIVE_BRIDGE_URL
            .scope(
                Some("http://127.0.0.1:9/tool_bridge/deadbeef".to_string()),
                async {
                    let file = bridge_mcp_config()
                        .expect("the config is written")
                        .expect("a bridge URL is in scope");
                    std::fs::read_to_string(file.path()).expect("the config is readable")
                },
            )
            .await;
        let body: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
        let server = &body["mcpServers"]["biorouter"];
        assert_eq!(server["url"], "http://127.0.0.1:9/tool_bridge/deadbeef");
        assert_eq!(
            server["timeout"],
            bridge::child_tool_call_timeout().as_millis() as u64,
            "milliseconds — Claude Code's unit, not Codex's seconds"
        );
        // The unit is easy to get wrong in the direction that looks fine: 660
        // would be read as two-thirds of a second and abandon every call
        // instantly, which reads as a broken bridge rather than a wrong number.
        assert!(
            server["timeout"].as_u64().unwrap_or(0) > 1000,
            "a value under a second means the unit was confused: {server}"
        );
    }

    /// No bridge means no config file at all — the child then runs tool-less,
    /// which is the correct degradation for a chat turn.
    #[tokio::test]
    async fn no_bridge_url_writes_no_config() {
        let none = bridge::ACTIVE_BRIDGE_URL
            .scope(None, async { bridge_mcp_config().expect("no error") })
            .await;
        assert!(none.is_none());
    }

    fn provider() -> ClaudeCodeProvider {
        ClaudeCodeProvider {
            command: PathBuf::from("/usr/bin/claude"),
            model: ModelConfig::new("claude-sonnet-4-6").unwrap(),
            name: KIND.provider_id().to_string(),
        }
    }

    /// Issue #109. The provider does not forward `_tools`, but the
    /// provider-driven tool-turn seam supplies the macro dispatcher over its MCP
    /// bridge, so coding-agent-backed knowledge loops are supported.
    #[test]
    fn the_provider_declares_that_it_can_drive_a_biorouter_run_tool_loop() {
        assert!(
            provider().supports_tool_calls(),
            "the provider-driven bridge makes macro tools reachable"
        );
    }

    /// The isolation flags are the security boundary, so their presence is pinned
    /// rather than left to review. Without `--setting-sources ""` a `-p` run
    /// executes the working directory's `.claude/settings.json` hooks; without
    /// `--strict-mcp-config` it connects the user's own MCP servers.
    #[test]
    fn the_isolation_flags_are_always_present() {
        let args = provider().base_args(
            &ModelConfig::new("claude-sonnet-4-6").unwrap(),
            "SYS",
            "json",
            None,
        );

        let i = args
            .iter()
            .position(|a| a == "--setting-sources")
            .expect("--setting-sources is required: without it a -p run executes the cwd's hooks");
        assert_eq!(
            args[i + 1],
            "",
            "--setting-sources must be given an empty value to load no sources"
        );
        assert!(
            args.iter().any(|a| a == "--strict-mcp-config"),
            "--strict-mcp-config is required: without it the child loads the user's own MCP servers"
        );

        let t = args
            .iter()
            .position(|a| a == "--tools")
            .expect("--tools is required so the child's own Read/Edit/Bash stay off");
        assert_eq!(
            args[t + 1],
            "",
            "--tools must be empty to disable all built-ins"
        );
    }

    /// `--mcp-config` is **variadic**, so nothing that follows it may be a bare
    /// positional: the CLI would swallow it as a second config path and die with
    /// "MCP config file not found: <that argument>". This is not hypothetical — it
    /// is exactly how the live bridge test failed first time.
    ///
    /// Two things keep the provider safe, and both are asserted: the prompt is
    /// never in argv at all (it goes on stdin), and whatever follows `--mcp-config`
    /// is either its own value or another flag.
    #[test]
    fn nothing_positional_can_follow_the_variadic_mcp_config_flag() {
        let p = provider();
        let m = ModelConfig::new("claude-sonnet-4-6").unwrap();
        let path = std::path::Path::new("/tmp/bridge.json");
        let args = p.base_args(&m, "SYS", "json", Some(path));

        let i = args
            .iter()
            .position(|a| a == "--mcp-config")
            .expect("the bridge config should be passed");
        assert_eq!(args[i + 1], path.to_string_lossy(), "its value comes first");
        // Everything after the value must be a flag, never a positional.
        for later in &args[i + 2..] {
            assert!(
                later.starts_with("--") || is_flag_value(&args, later),
                "`{later}` follows the variadic --mcp-config and would be eaten as a \
                 second config path"
            );
        }

        // And the prompt is never an argv element in the first place.
        assert!(
            !args.iter().any(|a| a.contains("SYS") && a != "SYS"),
            "the prompt must travel on stdin, not argv"
        );
    }

    /// True when `value` occupies the slot immediately after a `--flag`.
    fn is_flag_value(args: &[String], value: &String) -> bool {
        args.windows(2)
            .any(|w| w[0].starts_with("--") && &w[1] == value)
    }

    /// `--bare` never reads OAuth credentials or the keychain, so passing it would
    /// silently defeat the entire provider.
    #[test]
    fn bare_mode_is_never_requested() {
        let args = provider().base_args(
            &ModelConfig::new("claude-sonnet-4-6").unwrap(),
            "SYS",
            "json",
            None,
        );
        assert!(
            !args.iter().any(|a| a == "--bare"),
            "--bare never reads OAuth credentials and must not be passed"
        );
    }

    /// Biorouter's prompt replaces Claude Code's rather than being appended, which
    /// is both correct and a 16x token saving.
    #[test]
    fn the_system_prompt_replaces_rather_than_appends() {
        let args = provider().base_args(
            &ModelConfig::new("claude-sonnet-4-6").unwrap(),
            "SYS",
            "json",
            None,
        );
        let i = args.iter().position(|a| a == "--system-prompt").unwrap();
        assert_eq!(args[i + 1], "SYS");
        assert!(!args.iter().any(|a| a == "--append-system-prompt"));
    }

    /// The ladder, as it reaches the CLI. `Deep` climbing to `max` is the point:
    /// the Claude CLI's scale runs `low, medium, high, xhigh, max` (verified
    /// against 2.1.235), so stopping at `high` — where the OpenAI-family formats
    /// stop, because that is the top of *their* scale — would leave the two
    /// strongest rungs unreachable and make "deep" mean less here than it says.
    #[test]
    fn the_effort_ladder_reaches_the_cli() {
        for (effort, expected) in [
            (Some(ReasoningEffort::Quick), "low"),
            (Some(ReasoningEffort::Normal), "high"),
            (Some(ReasoningEffort::Deep), "max"),
            (None, "high"),
        ] {
            let m = ModelConfig::new("claude-sonnet-4-6")
                .unwrap()
                .with_reasoning_effort(effort);
            let args = provider().base_args(&m, "SYS", "json", None);
            let i = args
                .iter()
                .position(|a| a == "--effort")
                .unwrap_or_else(|| panic!("{effort:?} must reach the CLI as --effort"));
            assert_eq!(
                args[i + 1],
                expected,
                "{effort:?} maps to --effort {expected}"
            );
        }
    }

    /// An effort level is sent on **every** turn, including the default one — a
    /// deliberate departure from every other provider, where `Normal` is silence.
    ///
    /// Two reasons to assert it rather than leave it implicit. It costs the user
    /// thinking tokens on their own subscription on every turn, so it must not be
    /// able to change by accident. And `Normal` never actually reaches a provider:
    /// `Agent::effort_stamped_provider` returns early when the effort is default,
    /// so the config is not re-stamped and `None` arrives instead — a mapping that
    /// handled only `Some(Normal)` would be dead code and the middle rung would
    /// silently never apply.
    #[test]
    fn the_default_effort_is_high_rather_than_silence() {
        for effort in [None, Some(ReasoningEffort::Normal)] {
            let m = ModelConfig::new("claude-sonnet-4-6")
                .unwrap()
                .with_reasoning_effort(effort);
            let args = provider().base_args(&m, "SYS", "json", None);
            let i = args
                .iter()
                .position(|a| a == "--effort")
                .unwrap_or_else(|| panic!("{effort:?} must still send an effort"));
            assert_eq!(args[i + 1], "high", "{effort:?} is Biorouter's normal rung");
        }
    }

    /// `ultra` is not on the Claude CLI's scale at all: it warns and falls back to
    /// the default, which is a silent DOWNGRADE rather than an error. So every
    /// rung this provider can emit must be one the CLI actually knows.
    #[test]
    fn every_rung_we_emit_is_one_the_cli_accepts() {
        const ACCEPTED: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
        for effort in [
            None,
            Some(ReasoningEffort::Quick),
            Some(ReasoningEffort::Normal),
            Some(ReasoningEffort::Deep),
        ] {
            let m = ModelConfig::new("claude-sonnet-4-6")
                .unwrap()
                .with_reasoning_effort(effort);
            let args = provider().base_args(&m, "SYS", "json", None);
            let i = args.iter().position(|a| a == "--effort").unwrap();
            assert!(
                ACCEPTED.contains(&args[i + 1].as_str()),
                "{effort:?} emitted `{}`, which the CLI would warn about and ignore",
                args[i + 1]
            );
        }
    }

    /// `--effort <level>` sits after the variadic `--mcp-config`, so it is
    /// subject to the same trap as everything else there: a bare `low` would be
    /// eaten as a second config path. Asserted separately because the invariant
    /// test above builds its args without an effort and so cannot see this.
    #[test]
    fn the_effort_flag_survives_the_variadic_mcp_config_flag() {
        let m = ModelConfig::new("claude-sonnet-4-6")
            .unwrap()
            .with_reasoning_effort(Some(ReasoningEffort::Deep));
        let path = std::path::Path::new("/tmp/bridge.json");
        let args = provider().base_args(&m, "SYS", "json", Some(path));

        let i = args.iter().position(|a| a == "--mcp-config").unwrap();
        assert_eq!(args[i + 1], path.to_string_lossy());
        for later in &args[i + 2..] {
            assert!(
                later.starts_with("--") || is_flag_value(&args, later),
                "`{later}` follows the variadic --mcp-config and would be eaten as a \
                 second config path"
            );
        }
    }

    #[test]
    fn the_output_format_is_the_only_axis_that_varies() {
        let p = provider();
        let m = ModelConfig::new("claude-sonnet-4-6").unwrap();
        let json = p.base_args(&m, "SYS", "json", None);
        let stream = p.base_args(&m, "SYS", "stream-json", None);
        assert_eq!(json.len(), stream.len());
        let differences: Vec<_> = json
            .iter()
            .zip(stream.iter())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(
            differences.len(),
            1,
            "only --output-format's value may differ"
        );
    }

    #[test]
    fn blocking_completion_pairs_compatible_input_and_output_formats() {
        fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
            let index = args
                .iter()
                .position(|arg| arg == flag)
                .unwrap_or_else(|| panic!("missing {flag}"));
            args.get(index + 1).map(String::as_str)
        }

        let provider = provider();
        let model = ModelConfig::new("claude-sonnet-4-6").unwrap();

        let text_prompt = transcript::Prompt::text_only("Name this chat");
        let text_command = provider.completion_command(&model, "SYS", None, &text_prompt);
        let text_args: Vec<String> = text_command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(value_after(&text_args, "--output-format"), Some("json"));
        assert!(!text_args.iter().any(|arg| arg == "--input-format"));
        assert_eq!(
            encode_claude_completion_input(&text_prompt).unwrap(),
            b"Name this chat"
        );

        let image_prompt = transcript::Prompt {
            text: "Describe this".to_string(),
            images: vec![transcript::ImageInput {
                data: "cGl4ZWxz".to_string(),
                mime_type: "image/png",
            }],
        };
        let image_command = provider.completion_command(&model, "SYS", None, &image_prompt);
        let image_args: Vec<String> = image_command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            value_after(&image_args, "--output-format"),
            Some("stream-json")
        );
        assert_eq!(
            value_after(&image_args, "--input-format"),
            Some("stream-json")
        );
        assert!(image_args.iter().any(|arg| arg == "--verbose"));
        let frame: Value =
            serde_json::from_slice(&encode_claude_completion_input(&image_prompt).unwrap())
                .unwrap();
        assert!(frame["message"]["content"].is_array());
    }

    /// An `apiKeySource` other than "none" means the run would be billed to a
    /// metered account, which is the one outcome this provider exists to prevent.
    #[test]
    fn a_non_subscription_auth_source_is_refused() {
        assert!(ClaudeCodeProvider::assert_subscription_auth(Some("none")).is_ok());
        assert!(ClaudeCodeProvider::assert_subscription_auth(None).is_ok());

        let err = ClaudeCodeProvider::assert_subscription_auth(Some("ANTHROPIC_API_KEY"))
            .expect_err("a key-sourced run must be refused");
        assert!(matches!(err, ProviderError::Authentication(_)));
        assert!(err.to_string().contains("apiKeyHelper"));
    }

    /// Failures must be typed, so the retry layer does not retry a credential
    /// problem or give up on a blip.
    #[test]
    fn error_categories_map_to_typed_errors() {
        for (category, matches_auth) in [
            ("authentication_failed", true),
            ("oauth_org_not_allowed", true),
            ("billing_error", true),
        ] {
            let e = ClaudeCodeProvider::classify(Some(category), "d".into());
            assert_eq!(
                matches!(e, ProviderError::Authentication(_)),
                matches_auth,
                "{category} should be an authentication error"
            );
        }
        assert!(matches!(
            ClaudeCodeProvider::classify(Some("rate_limit"), "d".into()),
            ProviderError::RateLimitExceeded { .. }
        ));
        assert!(matches!(
            ClaudeCodeProvider::classify(Some("overloaded"), "d".into()),
            ProviderError::ServerError(_)
        ));
        assert!(matches!(
            ClaudeCodeProvider::classify(Some("max_output_tokens"), "d".into()),
            ProviderError::ContextLengthExceeded(_)
        ));
        // An unknown category must not be silently treated as success.
        assert!(matches!(
            ClaudeCodeProvider::classify(Some("something_new"), "d".into()),
            ProviderError::RequestFailed(_)
        ));
    }

    /// The four token buckets are disjoint, which is the invariant `Usage`
    /// documents and what makes `billed_total` reconcile with a vendor bill.
    #[test]
    fn usage_buckets_stay_disjoint() {
        let v = serde_json::json!({
            "input_tokens": 10,
            "output_tokens": 45,
            "cache_read_input_tokens": 5232,
            "cache_creation_input_tokens": 907
        });
        let u = parse_usage(Some(&v));
        assert_eq!(u.input_tokens, Some(10));
        assert_eq!(u.output_tokens, Some(45));
        assert_eq!(u.cache_read_input_tokens, Some(5232));
        assert_eq!(u.cache_creation_input_tokens, Some(907));
        assert_eq!(u.total_tokens, Some(10 + 45 + 5232 + 907));
    }

    #[test]
    fn absent_usage_is_not_invented() {
        assert_eq!(parse_usage(None).input_tokens, None);
        assert_eq!(parse_usage(None).total_tokens, None);
    }

    /// A real captured `result` frame parses, and the usage row is attributed to
    /// this provider rather than left for the model name to decide.
    #[test]
    fn a_captured_result_frame_parses_and_is_attributed() {
        let lines = vec![
            r#"{"type":"system","subtype":"init","apiKeySource":"none","model":"claude-haiku-4-5-20251001"}"#.to_string(),
            r#"{"type":"result","subtype":"success","is_error":false,"result":"PROOF_OK","usage":{"input_tokens":10,"output_tokens":45,"cache_read_input_tokens":0,"cache_creation_input_tokens":25022}}"#.to_string(),
        ];
        let (message, usage) = provider()
            .parse_result_object("claude-sonnet-4-6", &lines, "", exit_ok())
            .expect("a success frame must parse");

        assert_eq!(message.as_concat_text(), "PROOF_OK");
        assert_eq!(message.role, Role::Assistant);
        assert_eq!(
            usage.provider.as_deref(),
            Some("claude_code"),
            "an unattributed usage row gets a fabricated per-token price"
        );
        assert_eq!(usage.usage.cache_creation_input_tokens, Some(25022));
    }

    /// A run that reports a key source is refused even though the frame says
    /// success — the billing path is wrong, and that is not something to recover
    /// from silently.
    #[test]
    fn a_successful_frame_with_a_key_source_is_still_refused() {
        let lines = vec![
            r#"{"type":"system","subtype":"init","apiKeySource":"ANTHROPIC_API_KEY"}"#.to_string(),
            r#"{"type":"result","subtype":"success","is_error":false,"result":"hi","usage":{}}"#
                .to_string(),
        ];
        let err = provider()
            .parse_result_object("claude-sonnet-4-6", &lines, "", exit_ok())
            .expect_err("a key-sourced run must be refused even on success");
        assert!(matches!(err, ProviderError::Authentication(_)));
    }

    /// When the CLI produces nothing usable, its stderr is what tells the user
    /// why — all four deleted providers discarded it.
    #[test]
    fn stderr_reaches_the_error_when_there_is_no_result() {
        let err = provider()
            .parse_result_object(
                "claude-sonnet-4-6",
                &[],
                "claude: command failed spectacularly",
                exit_ok(),
            )
            .expect_err("no result frame is a failure");
        assert!(
            err.to_string().contains("command failed spectacularly"),
            "the CLI's own diagnostic must survive into the error: {err}"
        );
    }

    /// The label is "Claude Code" and the description names the CLI the user has
    /// to install. Both are pinned because both are decisions.
    ///
    /// The label deviates from Anthropic's branding guidelines knowingly — see
    /// `CodingAgentKind::display_name` for the reasoning. What is asserted here is
    /// only that the two strings stay deliberate: the label is the product name the
    /// maintainer chose, and the description still attributes the dependency to
    /// Anthropic so a reader can tell what to install and whose it is.
    #[test]
    fn the_label_and_the_named_dependency_are_both_deliberate() {
        let m = ClaudeCodeProvider::metadata();
        assert_eq!(m.display_name, "Claude Code");
        assert!(
            m.description.contains("Claude Code CLI"),
            "the description must name the tool the user installs: {}",
            m.description
        );
        assert!(
            m.description.contains("Anthropic"),
            "…and attribute it, so it reads as a dependency rather than as our own product"
        );
    }

    #[test]
    fn metadata_is_public_and_not_locally_computed() {
        let m = ClaudeCodeProvider::metadata();
        assert_eq!(
            m.name, "claude_code",
            "the underscore is what pricing keys on"
        );
        assert_eq!(m.tier, crate::privacy::ProviderTier::Public);
        assert!(
            !m.runs_locally,
            "the subprocess is local but the inference is not"
        );
        // One required key with a default, which is what makes a keyless provider
        // report as configured at all.
        assert_eq!(m.config_keys.len(), 1);
        assert_eq!(m.config_keys[0].name, "CLAUDE_CODE_COMMAND");
        assert!(m.config_keys[0].required);
        assert!(!m.config_keys[0].secret);
        assert_eq!(m.config_keys[0].default.as_deref(), Some("claude"));
        assert!(
            m.known_models
                .iter()
                .all(|model| model.supports_vision == Some(true)),
            "every current Claude model accepts image inputs"
        );
    }

    /// The advertised catalog is Anthropic's *current* lineup and nothing else.
    ///
    /// Pinned as an exact list rather than as "contains X", because the failure
    /// this guards against is an id staying behind after it stops being
    /// current: a legacy model in the picker is not neutral, and one of the two
    /// removed here (`claude-sonnet-4-6`) was showing a 1,000,000 window for a
    /// model the CLI runs at 200,000 on this plan.
    #[test]
    fn the_catalog_is_anthropics_current_lineup_and_defaults_to_fable_5_1() {
        let m = ClaudeCodeProvider::metadata();
        let advertised: Vec<&str> = m.known_models.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            advertised,
            [
                "claude-fable-5-1",
                "claude-opus-5",
                "claude-sonnet-5",
                "claude-haiku-4-5",
            ],
            "measured 2026-09-08 against `claude` 2.1.235 (and 2.1.260 for \
             Fable 5.1) plus Anthropic's model overview"
        );
        assert_eq!(
            m.default_model, CLAUDE_CODE_DEFAULT_MODEL,
            "the picker's default is the constant, not a second opinion"
        );
        assert_eq!(m.default_model, "claude-fable-5-1");

        // Legacy ids a user may still type by hand, and which must not be
        // advertised. `with_unlisted_models` is what keeps them reachable.
        for legacy in ["claude-fable-5", "claude-sonnet-4-6", "claude-opus-4-8"] {
            assert!(
                !advertised.contains(&legacy),
                "{legacy} is a legacy model and must not be in the picker"
            );
        }
        assert!(
            m.allows_unlisted_models,
            "…which is only acceptable because an unlisted id can still be typed"
        );

        // The window each entry declares is the one the CLI itself reported.
        // Haiku differing from the rest is the point: a blanket 1M would be a
        // 5x overstatement for it.
        let window = |id: &str| {
            m.known_models
                .iter()
                .find(|i| i.name == id)
                .unwrap_or_else(|| panic!("{id} is advertised"))
                .context_limit
        };
        assert_eq!(window("claude-fable-5-1"), 1_000_000);
        assert_eq!(window("claude-opus-5"), 1_000_000);
        assert_eq!(window("claude-sonnet-5"), 1_000_000);
        assert_eq!(window("claude-haiku-4-5"), 200_000);
    }

    #[test]
    fn multimodal_prompt_uses_claude_stream_json_content_blocks() {
        let prompt = transcript::Prompt {
            text: "describe".to_string(),
            images: vec![transcript::ImageInput {
                data: "cGl4ZWxz".to_string(),
                mime_type: "image/webp",
            }],
        };

        let line = encode_claude_prompt(&prompt).expect("the input frame encodes");
        let frame: Value = serde_json::from_slice(&line).expect("the frame is JSON");
        assert_eq!(frame["type"], "user");
        assert_eq!(frame["message"]["role"], "user");
        assert_eq!(
            frame["message"]["content"][0],
            serde_json::json!({
                "type": "text",
                "text": "describe",
            })
        );
        assert_eq!(
            frame["message"]["content"][1],
            serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/webp",
                    "data": "cGl4ZWxz",
                }
            })
        );
    }

    fn exit_ok() -> std::process::ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(0)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(0)
        }
    }
}

/// Phase 1 end-to-end: the streaming path driven by a fake `claude` that replays
/// **recorded vendor frames**.
///
/// This is the only test that exercises the whole chain the user actually sees —
/// argv construction through `command_for`, the spawn, the line router, the
/// reused Anthropic decoder, and the terminal usage — so it is the one that
/// would catch a regression anywhere along it. The frames are the real ones
/// captured in `tests/fixtures/coding_agent/claude/`, not idealised.
/// The pending-argument throttle, tested directly.
///
/// Worth its own test because the failure it prevents is invisible in a small
/// fixture: every recorded tool call has tiny arguments, so an unthrottled
/// implementation looks perfectly fine against the corpus and only misbehaves in
/// production, on the large `text_editor` writes a coding agent actually makes.
#[cfg(test)]
mod pending_args_tests {
    use super::PendingArgs;

    /// A long argument stream must not produce a frame per delta.
    ///
    /// The arithmetic is the point: each notification carries the WHOLE string
    /// accumulated so far, so N frames over an N-chunk argument is quadratic in
    /// bytes. 2000 one-character deltas unthrottled would be 2000 frames and
    /// ~2 MB of snapshots; throttled by size it is ~10 frames.
    #[test]
    fn a_long_argument_stream_is_not_one_frame_per_delta() {
        let mut args = PendingArgs::default();
        let mut frames = 0usize;
        let mut bytes = 0usize;

        for _ in 0..2000 {
            args.text.push('x');
            if let Some(snapshot) = args.take_due_snapshot() {
                frames += 1;
                bytes += snapshot.len();
            }
        }

        assert!(
            frames <= 25,
            "2000 deltas produced {frames} preview frames; the throttle is not \
             working and one tool call becomes hundreds of SSE frames"
        );
        assert!(
            bytes < 100_000,
            "the accumulated snapshots totalled {bytes} bytes for a 2000-byte \
             argument — that is the quadratic blow-up the throttle exists to stop"
        );
    }

    /// Every preview carries the tool's name, not just the first.
    ///
    /// The GUI keys its card on the call id and ignores the name on updates, so
    /// this looks free to omit — and the first implementation did omit it. The
    /// CLI renders each notification as its own line (`[tool] <name> …`), so a
    /// live turn printed three `[tool]  …` lines with a blank where the tool
    /// name belongs. The shared Anthropic decoder repeats the name on every
    /// throttled update; this keeps the two paths rendering identically.
    #[test]
    fn every_preview_carries_the_tool_name() {
        let mut args = PendingArgs {
            name: "developer__shell".to_string(),
            ..PendingArgs::default()
        };
        args.text.push_str("{\"command\":");
        assert!(
            args.take_due_snapshot().is_some(),
            "the first preview fires"
        );
        assert_eq!(
            args.name, "developer__shell",
            "the name must survive to every later preview, or a line-oriented \
             consumer prints a blank where the tool name belongs"
        );
    }

    /// The first delta always previews, so the card fills in immediately rather
    /// than staying empty for the throttle interval.
    #[test]
    fn the_first_delta_always_previews() {
        let mut args = PendingArgs::default();
        args.text.push_str("{\"command\":");
        assert!(
            args.take_due_snapshot().is_some(),
            "the first preview must not wait"
        );
    }

    /// A snapshot is the whole argument so far, not just the new part — the card
    /// renders it as a preview of the arguments, not as a delta to append.
    #[test]
    fn a_snapshot_is_cumulative() {
        let mut args = PendingArgs::default();
        args.text.push_str("abc");
        assert_eq!(args.take_due_snapshot().as_deref(), Some("abc"));

        args.text.push_str(&"d".repeat(super::PENDING_ARGS_CHARS));
        let second = args.take_due_snapshot().expect("size trigger");
        assert!(
            second.starts_with("abc"),
            "the preview must carry everything so far (got {second:.20}…)"
        );
    }
}

#[cfg(all(test, unix))]
mod streaming_tests {
    use super::*;
    use futures::StreamExt;
    use std::os::unix::fs::PermissionsExt;

    /// A written, executable stand-in for a vendor CLI.
    ///
    /// ⚠ **Written with `fs::write` into a directory, never a `NamedTempFile`.**
    /// A `NamedTempFile` holds the file open read-write for as long as it lives,
    /// and Linux refuses to `exec` a file that any process has open for
    /// writing — `ETXTBSY`, surfaced as `Text file busy (os error 26)`. macOS
    /// permits it, so the bug is invisible locally and fails the whole Linux
    /// test job. `fs::write` closes the handle before it returns; the `TempDir`
    /// is kept only so the directory outlives the child.
    struct FakeCli {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }

    impl FakeCli {
        fn new(body: &str) -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("fake-cli");
            std::fs::write(&path, body).expect("write the fake CLI");
            let mut perms = std::fs::metadata(&path).expect("stat").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("chmod");
            Self { _dir: dir, path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    /// A stand-in for the `claude` binary that prints one fixture cell and exits.
    ///
    /// It ignores its arguments and its stdin, which is exactly what makes it a
    /// replay: the frames are fixed, so any difference in what the provider
    /// produces is the provider's doing.
    fn fake_claude(cell: &str) -> FakeCli {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/coding_agent/claude")
            .join(format!("{cell}.ndjson"));
        assert!(
            fixture.exists(),
            "missing fixture cell: {}",
            fixture.display()
        );

        FakeCli::new(&format!(
            // `cat` the fixture, then drain stdin so the prompt writer never
            // sees a broken pipe before it finishes.
            "#!/bin/sh\ncat {}\ncat > /dev/null\n",
            fixture.display()
        ))
    }

    fn steerable_claude() -> FakeCli {
        FakeCli::new(
            r#"#!/usr/bin/env python3
import sys, json

def send(obj):
    print(json.dumps(obj), flush=True)

args = sys.argv[1:]
if "--input-format" not in args or args[args.index("--input-format") + 1] != "stream-json":
    sys.exit("stream-json input was not enabled")

initial = json.loads(sys.stdin.readline())
if initial.get("type") != "user" or initial.get("message", {}).get("role") != "user":
    sys.exit("initial input was not a user message")

send({"type":"system","subtype":"init","apiKeySource":"none","session_id":"s"})
send(initial)
send({"type":"stream_event","event":{"type":"message_start",
      "message":{"id":"msg_1","role":"assistant","content":[],
                 "usage":{"input_tokens":1,"output_tokens":1}}}})
send({"type":"stream_event","event":{"type":"content_block_start","index":0,
      "content_block":{"type":"text","text":""}}})
send({"type":"stream_event","event":{"type":"content_block_delta","index":0,
      "delta":{"type":"text_delta","text":"before steering"}}})
send({"type":"stream_event","event":{"type":"content_block_stop","index":0}})
send({"type":"stream_event","event":{"type":"message_delta",
      "delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}})
send({"type":"stream_event","event":{"type":"message_stop"}})

steer = json.loads(sys.stdin.readline())
if steer.get("message", {}).get("content") != "change course":
    sys.exit("steering input was not delivered")

send({"type":"result","subtype":"success","is_error":False,
      "result":"before steering","usage":{"input_tokens":1,"output_tokens":2}})
send({"type":"system","subtype":"init","apiKeySource":"none","session_id":"s"})
send(steer)
send({"type":"stream_event","event":{"type":"message_start",
      "message":{"id":"msg_2","role":"assistant","content":[],
                 "usage":{"input_tokens":3,"output_tokens":1}}}})
send({"type":"stream_event","event":{"type":"content_block_start","index":0,
      "content_block":{"type":"text","text":""}}})
send({"type":"stream_event","event":{"type":"content_block_delta","index":0,
      "delta":{"type":"text_delta","text":"changed course"}}})
send({"type":"stream_event","event":{"type":"content_block_stop","index":0}})
send({"type":"stream_event","event":{"type":"message_delta",
      "delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}})
send({"type":"stream_event","event":{"type":"message_stop"}})
send({"type":"result","subtype":"success","is_error":False,
      "result":"changed course","usage":{"input_tokens":3,"output_tokens":2}})
"#,
        )
    }

    fn provider_running(script: &FakeCli) -> ClaudeCodeProvider {
        ClaudeCodeProvider {
            command: script.path().to_path_buf(),
            model: ModelConfig::new("claude-sonnet-4-6").unwrap(),
            name: KIND.provider_id().to_string(),
        }
    }

    /// Everything the stream produced, flattened for assertions.
    struct Streamed {
        texts: Vec<String>,
        usages: Vec<ProviderUsage>,
        tool_requests: usize,
        /// Requests carrying the provider-executed marker. Anything counted in
        /// `tool_requests` but missing here would be dispatched by the loop.
        marked_requests: Vec<String>,
        /// `(call id, is_error)` for each mirrored response.
        responses: Vec<(String, bool)>,
        pendings: usize,
        pending_names: Vec<String>,
    }

    async fn drive(cell: &str) -> Result<Streamed, ProviderError> {
        let script = fake_claude(cell);
        let provider = provider_running(&script);
        let messages = vec![Message::user().with_text("hello")];

        let stream = provider.stream("SYS", &messages, &[]).await?;
        futures::pin_mut!(stream);

        let mut out = Streamed {
            texts: Vec::new(),
            usages: Vec::new(),
            tool_requests: 0,
            marked_requests: Vec::new(),
            responses: Vec::new(),
            pendings: 0,
            pending_names: Vec::new(),
        };
        while let Some(item) = stream.next().await {
            let (message, usage, pending) = item?;
            if let Some(message) = message {
                for content in &message.content {
                    match content {
                        MessageContent::Text(t) => out.texts.push(t.text.clone()),
                        MessageContent::ToolRequest(r) => {
                            out.tool_requests += 1;
                            if mirror::request_execution(r).is_some() {
                                out.marked_requests.push(r.id.clone());
                            }
                        }
                        MessageContent::ToolResponse(r) => {
                            let is_error = r
                                .tool_result
                                .as_ref()
                                .ok()
                                .and_then(|v| v.is_error)
                                .unwrap_or(false);
                            out.responses.push((r.id.clone(), is_error));
                        }
                        _ => {}
                    }
                }
            }
            if let Some(usage) = usage {
                out.usages.push(usage);
            }
            if let Some(pending) = pending {
                out.pendings += 1;
                if !pending.name.is_empty() {
                    out.pending_names.push(pending.name);
                }
            }
        }
        Ok(out)
    }

    /// Text arrives in pieces, and the pieces are the answer.
    ///
    /// More than one text item is the whole point: one item would mean the
    /// answer still appears all at once, which is the behaviour this phase
    /// exists to remove.
    #[tokio::test]
    async fn a_recorded_text_turn_streams_its_answer_in_parts() {
        let streamed = drive("turn-text").await.expect("stream");

        assert!(
            streamed.texts.len() > 1,
            "the answer must arrive in parts, not as one final blob (got {:?})",
            streamed.texts
        );
        let joined: String = streamed.texts.concat();
        assert!(
            !joined.trim().is_empty(),
            "the streamed parts must reconstruct the answer"
        );
        assert_eq!(
            streamed.tool_requests, 0,
            "a text turn mints no tool requests"
        );
    }

    /// The terminal `result` frame is the authoritative usage, and it must be
    /// attributed to this provider — without the provider field the usage row is
    /// priced by model name and a subscription turn is billed as if it were an
    /// API call.
    #[tokio::test]
    async fn the_terminal_usage_is_attributed_to_the_provider() {
        let streamed = drive("turn-text").await.expect("stream");

        let last = streamed.usages.last().expect("a terminal usage item");
        assert_eq!(
            last.provider.as_deref(),
            Some(KIND.provider_id()),
            "the last usage must carry the provider id, or pricing invents a \
             per-token cost for a run that billed a subscription"
        );
        assert!(
            last.usage.input_tokens.unwrap_or(0) > 0 || last.usage.output_tokens.unwrap_or(0) > 0,
            "the terminal frame carries real token counts"
        );
    }

    /// **The parity contract.** Every tool call the child made appears as a
    /// card, and every one of them is marked as already executed.
    ///
    /// Both halves matter and they pull in opposite directions. Zero cards would
    /// mean the user still cannot see what the agent did — the thing this work
    /// exists to fix. An *unmarked* card would mean the agent loop dispatches the
    /// call a second time, actually re-running it. So the assertion is not
    /// "requests exist" but "requests exist AND every one carries the marker".
    #[tokio::test]
    async fn a_recorded_tool_turn_mirrors_every_call_as_a_marked_card() {
        let streamed = drive("turn-tools").await.expect("stream");

        assert!(
            streamed.tool_requests > 0,
            "the child's tool calls must be visible as cards"
        );
        assert_eq!(
            streamed.marked_requests.len(),
            streamed.tool_requests,
            "every mirrored request must carry the provider-executed marker, or \
             the loop will run the call again (marked: {:?}, total: {})",
            streamed.marked_requests,
            streamed.tool_requests
        );

        // The skeleton card: the name is known at content_block_start, before the
        // arguments have finished arriving.
        assert!(
            streamed.pendings > 0,
            "a pending card must appear as soon as the tool's name is known"
        );
        assert!(
            streamed
                .pending_names
                .iter()
                .all(|n| !n.starts_with("mcp__biorouter__")),
            "the card shows the tool name the user knows, not the child's \
             MCP-namespaced spelling (got {:?})",
            streamed.pending_names
        );

        // Every card resolves: an unpaired request would leave the card spinning
        // for the rest of the session.
        for id in &streamed.marked_requests {
            assert!(
                streamed.responses.iter().any(|(rid, _)| rid == id),
                "request {id} has no matching response; its card would never settle \
                 (responses: {:?})",
                streamed.responses
            );
        }

        assert!(
            !streamed.texts.concat().trim().is_empty(),
            "the turn's prose still streams alongside the cards"
        );
    }

    /// A tool the child ran that FAILED must reach the card as a failure.
    ///
    /// This is the half that a happy-path fixture cannot prove: `is_error` lives
    /// on the `tool_result` block, and losing it would paint a failed call green.
    #[tokio::test]
    async fn a_failed_tool_call_is_mirrored_as_a_failure() {
        let streamed = drive("turn-tool-error").await.expect("stream");

        assert!(
            streamed.tool_requests > 0,
            "the failing call must still be shown"
        );
        assert!(
            streamed.responses.iter().any(|(_, is_error)| *is_error),
            "the failure must survive as is_error on the result, which is what \
             turns the card red (responses: {:?})",
            streamed.responses
        );
    }

    /// A failed turn must surface as an error rather than as an empty success.
    #[tokio::test]
    async fn a_recorded_auth_failure_becomes_an_error() {
        let result = drive("auth-failure").await;
        let err = result.err().expect(
            "an auth failure must fail the stream — the recorded frame carries \
             is_error:true with subtype:\"success\", so classifying on subtype \
             would report this turn as fine",
        );
        let rendered = format!("{err}");
        assert!(
            !rendered.is_empty(),
            "the failure must carry a message the user can act on"
        );
    }

    #[tokio::test]
    async fn a_user_instruction_steers_the_running_claude_turn() {
        let script = steerable_claude();
        let provider = provider_running(&script);
        let messages = vec![Message::user().with_text("hello")];
        let (steering_tx, steering_rx) = crate::providers::base::provider_steer_channel();
        let stream = provider
            .stream_with_steering("SYS", &messages, &[], steering_rx)
            .await
            .expect("the stream should open");

        let (request, acknowledged) = ProviderSteerRequest::new("change course");
        assert!(steering_tx.send(request).is_ok(), "the turn is running");
        acknowledged
            .await
            .expect("the provider kept the acknowledgement")
            .expect("the stream-json pipe accepted the instruction");

        futures::pin_mut!(stream);
        let mut text = String::new();
        let mut usage = None;
        while let Some(item) = stream.next().await {
            let (message, update, _) = item.expect("the steered turn should finish");
            if let Some(message) = message {
                for content in message.content {
                    if let MessageContent::Text(content) = content {
                        text.push_str(&content.text);
                    }
                }
            }
            if update.is_some() {
                usage = update;
            }
        }
        assert_eq!(text, "before steeringchanged course");
        let usage = usage.expect("the steered run should report combined usage");
        assert_eq!(usage.usage.input_tokens, Some(4));
        assert_eq!(usage.usage.output_tokens, Some(4));
    }
}

/// Phase 5: cancellation on the streaming path.
///
/// On the blocking path the child is owned by the provider's own future, so a
/// cancelled turn — which **drops** that future rather than unwinding it — drops
/// the child and `kill_on_drop(true)` reaps it. Streaming breaks that chain: the
/// child must be owned by a spawned reader task, and a spawned task outlives the
/// stream feeding from it. `coding_agent::AbortOnDrop` is what restores it.
///
/// A leaked `claude` is not a tidiness problem. It holds the user's own
/// subscription credential and keeps spending their quota on an answer nobody
/// will ever read.
#[cfg(all(test, unix))]
mod cancellation_tests {
    use super::*;
    use futures::StreamExt;
    use std::os::unix::fs::PermissionsExt;

    /// A written, executable stand-in for a vendor CLI.
    ///
    /// ⚠ **Written with `fs::write` into a directory, never a `NamedTempFile`.**
    /// A `NamedTempFile` holds the file open read-write for as long as it lives,
    /// and Linux refuses to `exec` a file that any process has open for
    /// writing — `ETXTBSY`, surfaced as `Text file busy (os error 26)`. macOS
    /// permits it, so the bug is invisible locally and fails the whole Linux
    /// test job. `fs::write` closes the handle before it returns; the `TempDir`
    /// is kept only so the directory outlives the child.
    struct FakeCli {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
    }

    impl FakeCli {
        fn new(body: &str) -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let path = dir.path().join("fake-cli");
            std::fs::write(&path, body).expect("write the fake CLI");
            let mut perms = std::fs::metadata(&path).expect("stat").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("chmod");
            Self { _dir: dir, path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    /// A fake `claude` that streams one text delta and then hangs forever,
    /// writing its own pid where the test can find it.
    ///
    /// Hanging is the point: it stands in for a child still working when the
    /// user hits stop.
    fn hanging_claude(pid_file: &std::path::Path) -> FakeCli {
        let frames = [
            r#"{"type":"system","subtype":"init","apiKeySource":"none","session_id":"s"}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":1}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"working"}}}"#,
        ];

        let mut body = String::from("#!/bin/sh\n");
        body.push_str(&format!("echo $$ > {}\n", pid_file.display()));
        for frame in frames {
            body.push_str(&format!("echo '{frame}'\n"));
        }
        // Never exits on its own.
        body.push_str("while true; do sleep 1; done\n");
        FakeCli::new(&body)
    }

    fn alive(pid: i32) -> bool {
        // Signal 0 tests for existence without delivering anything.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// How long these tests wait for a child process to spawn or to exit.
    ///
    /// ⚠ Deliberately generous, and it used to be `100` — five seconds. Five
    /// seconds is not a process-spawn budget on a CI runner. `test
    /// (macos-latest)` compiles the whole workspace cold on three cores, and
    /// this loop was measured failing on a developer machine at load ~20 with
    /// three worktrees building:
    ///
    /// ```text
    /// dropping_an_unread_stream_reaps_the_child
    ///   panicked at claude_code.rs: "the child should have started and wrote its pid"
    /// ```
    ///
    /// The failure reads as a defect in child reaping and is really the
    /// scheduler not having got round to the child yet. It is also
    /// self-perpetuating: CI saves its Rust cache only on a green run, so one
    /// such red job keeps the next run cold, which makes the next timeout
    /// MORE likely.
    ///
    /// Raising it costs nothing when the child behaves — every loop here exits
    /// the moment it sees what it is waiting for, so the ceiling is only ever
    /// paid by a genuine failure.
    const CHILD_WAIT_TICKS: usize = 1_200; // 60 s at 50 ms per tick

    async fn wait_for_exit(pid: i32) -> bool {
        for _ in 0..CHILD_WAIT_TICKS {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        false
    }

    /// Dropping the stream mid-turn kills the child.
    #[tokio::test]
    async fn dropping_a_live_stream_reaps_the_child() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pid_file = dir.path().join("pid");
        let script = hanging_claude(&pid_file);

        let provider = ClaudeCodeProvider {
            command: script.path().to_path_buf(),
            model: ModelConfig::new("claude-sonnet-4-6").unwrap(),
            name: KIND.provider_id().to_string(),
        };
        let messages = vec![Message::user().with_text("hello")];

        // NOT `pin_mut!`: that shadows the stream with a `Pin<&mut _>`, so a
        // later `drop` would drop the *reference* and leave the stream itself
        // alive until the end of the function — the test would then report a
        // leak that is entirely its own doing. `stream()` already returns a
        // `Pin<Box<_>>`, which is `Unpin`, so it can be polled and dropped
        // directly.
        let mut stream = provider
            .stream("SYS", &messages, &[])
            .await
            .expect("stream");

        // Read until the first text arrives, so the child is definitely running
        // and the turn is definitely mid-flight.
        let mut saw_text = false;
        while let Some(item) = stream.next().await {
            let (message, _, _) = item.expect("no error before the drop");
            if message.is_some_and(|m| {
                m.content
                    .iter()
                    .any(|c| matches!(c, MessageContent::Text(_)))
            }) {
                saw_text = true;
                break;
            }
        }
        assert!(saw_text, "the fake child should have streamed some text");

        let pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("the child wrote its pid")
            .trim()
            .parse()
            .expect("a numeric pid");
        assert!(alive(pid), "the child is running while the turn is live");

        // The cancellation itself: the consumer lets go of the stream.
        drop(stream);

        assert!(
            wait_for_exit(pid).await,
            "the child survived the stream being dropped — a cancelled turn has \
             leaked a `claude` process that still holds the user's credential and \
             is still spending their quota"
        );
    }

    /// The same guarantee when the stream is never read at all.
    ///
    /// A turn can be cancelled before the first frame arrives, and the reader
    /// task is already running by then — it is spawned inside `stream()`, before
    /// the stream is returned.
    #[tokio::test]
    async fn dropping_an_unread_stream_reaps_the_child() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pid_file = dir.path().join("pid");
        let script = hanging_claude(&pid_file);

        let provider = ClaudeCodeProvider {
            command: script.path().to_path_buf(),
            model: ModelConfig::new("claude-sonnet-4-6").unwrap(),
            name: KIND.provider_id().to_string(),
        };
        let messages = vec![Message::user().with_text("hello")];

        let stream = provider
            .stream("SYS", &messages, &[])
            .await
            .expect("stream");

        // Give the child long enough to start and record its pid.
        let mut pid = None;
        for _ in 0..CHILD_WAIT_TICKS {
            if let Ok(text) = std::fs::read_to_string(&pid_file) {
                if let Ok(parsed) = text.trim().parse::<i32>() {
                    pid = Some(parsed);
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let pid = pid.expect("the child should have started and written its pid");

        drop(stream);

        assert!(
            wait_for_exit(pid).await,
            "a stream dropped before it was ever polled still has to reap its child"
        );
    }
}
