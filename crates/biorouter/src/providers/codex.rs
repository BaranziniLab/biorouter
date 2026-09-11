//! The **Codex** provider: drives the user's own `codex` CLI on the user's own
//! ChatGPT subscription.
//!
//! # Why `codex app-server` and not `codex exec`
//!
//! `codex exec --json` is the obvious choice and it is the wrong one, for a
//! reason that only shows up once tools are involved. `exec` has no channel for
//! answering an approval, so the moment the agent wants to call a tool the call
//! fails with "user cancelled MCP tool call" — verified — and the only ways
//! around it are `--approve-for-me` (which forces a workspace-write sandbox) or
//! `--dangerously-bypass-approvals-and-sandbox`. Both hand the child more
//! authority than Biorouter wants it to have.
//!
//! `codex app-server` speaks JSON-RPC over stdio and routes every approval back
//! to the host as a **server-originated request**, blocking the turn until it is
//! answered. That is precisely the shape Biorouter needs: the decision stays here.
//! It also exposes `account/read`, `account/rateLimits/read` and `model/list`,
//! and its whole protocol can be regenerated with
//! `codex app-server generate-json-schema --out DIR` rather than reverse-engineered.
//!
//! # `thread/start` carries Biorouter's instructions
//!
//! `baseInstructions` replaces Codex's own system prompt with Biorouter's, which
//! is both correct and a large token saving — Codex's default preamble measured
//! ~15k input tokens on a trivial prompt.
//!
//! # What the child may do
//!
//! `sandbox: "read-only"` and process-level feature disables remove the child's
//! local model-controlled tools. Biorouter's own
//! tools reach it over the one MCP bridge, and execute in Biorouter's dispatcher
//! where every existing gate still fires. An unexpected approval request for a
//! command or file change is refused rather than rubber-stamped; the thread's
//! approval policy refuses those inside the CLI and lets only the MCP tool-call
//! approval through, which Biorouter accepts for its own bridge alone.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{Role, Tool};
use serde_json::{json, Value};

use super::base::{
    ConfigKey, MessageStream, ModelInfo, PendingToolCall, Provider, ProviderMetadata,
    ProviderSteerReceiver, ProviderSteerRequest, ProviderStreamItem, ProviderUsage, Usage,
};
use super::coding_agent::appserver::{AppServer, Inbound};
use super::coding_agent::{
    self, bridge, codex_stream, discovery, env as agent_env, mirror, transcript, CodingAgentKind,
};
use super::errors::ProviderError;
use super::provider_binding::{AbsoluteCommandPath, ProviderRestoreBinding};
use crate::agents::effort::ReasoningEffort;
use crate::config::search_path::SearchPaths;
use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;

const KIND: CodingAgentKind = CodingAgentKind::Codex;

/// The model a Codex session starts on.
///
/// Astra is the vendor's own bundled default: codex-cli 0.153.4's changelog
/// entry is "Fixed Astra's visibility in the bundled model picker and made it
/// the bundled default", and `model/list` on 0.153.4 returns it first with
/// `isDefault: true` (measured 2026-09-08). It is also what the operator's own
/// `~/.codex/config.toml` selects.
///
/// ⚠ It needs **codex-cli 0.153.4 or newer**. On 0.147.0 the request is
/// refused with `400 The 'gpt-6-astra' model requires a newer version of Codex.
/// Please upgrade to the latest app or CLI and try again.` — which Biorouter
/// surfaces verbatim, so a stale CLI fails actionably rather than silently.
pub const CODEX_DEFAULT_MODEL: &str = "gpt-6-astra";
pub const CODEX_DOC_URL: &str = "https://developers.openai.com/codex/cli";

/// How long the turn will wait for the app server to say which account it bills.
///
/// `AppServer::request` has no timeout of its own — it awaits its oneshot until
/// the child's stdout closes — which is right for `thread/start`, whose answer
/// legitimately takes as long as the model does, and wrong for this. The check
/// is documented as fail-open ("a failed request is not a failed check"), and
/// that only holds for an app server that *answers* an unknown method with a
/// JSON-RPC error. One that silently ignores it never resolves, and the turn
/// hangs on its very first round trip with no error, no output and nothing to
/// time it out — the same shape as the Versa/Bedrock freeze already in this
/// repo's history, which is why the mitigation is here rather than filed.
///
/// Ten seconds because the answer is local: the app server reads its own auth
/// state and replies. Anything approaching this is already a broken install, and
/// exceeding it lands in the same fail-open branch a rejected method does.
const ACCOUNT_READ_TIMEOUT: Duration = Duration::from_secs(10);

enum StreamPumpEvent {
    Continue,
    ConsumerClosed,
    Terminal,
}

/// Codex capabilities that must not exist inside a Biorouter-managed child.
///
/// In particular, `read-only` constrains writes but deliberately permits reads
/// anywhere on the host. Leaving either shell implementation enabled lets a
/// prompt-injected child read `$CODEX_HOME/auth.json` (or the original file it
/// links to) and place subscription credentials in model-visible tool output.
/// Biorouter supplies the audited workspace/knowledge surface through its MCP
/// bridge instead.
const DISABLED_CHILD_FEATURES: &[&str] = &[
    "apps",
    "auth_elicitation",
    "browser_use",
    "computer_use",
    "image_generation",
    "in_app_browser",
    "multi_agent",
    "plugin_sharing",
    "plugins",
    "shell_tool",
    "skill_mcp_dependency_install",
    "skill_search",
    "unified_exec",
    "view_image",
    "workspace_dependencies",
];

/// Give a Codex child its subscription credential without its personal config.
///
/// Codex merges `thread/start.config` into `$CODEX_HOME/config.toml`; it does
/// not replace the user's configured MCP servers. Pointing the child at this
/// empty home is therefore the enforcement boundary. The auth file is linked
/// where the platform permits it. Windows cannot hard-link across volumes, so
/// that case uses the OS copy operation into the same ephemeral home; its
/// contents still never pass through a Biorouter-owned buffer.
fn isolated_codex_home(source_home: &Path) -> Result<tempfile::TempDir, ProviderError> {
    let isolated = tempfile::Builder::new()
        .prefix("biorouter-codex-home-")
        .tempdir()
        .map_err(|error| {
            ProviderError::ExecutionError(format!(
                "could not create an isolated Codex config home: {error}"
            ))
        })?;
    let source_auth = source_home.join("auth.json");
    if source_auth.try_exists().map_err(|error| {
        ProviderError::ExecutionError(format!(
            "could not inspect the Codex subscription credential: {error}"
        ))
    })? {
        let source_auth = std::fs::canonicalize(&source_auth).map_err(|error| {
            ProviderError::ExecutionError(format!(
                "could not resolve the Codex subscription credential: {error}"
            ))
        })?;
        let isolated_auth = isolated.path().join("auth.json");
        link_codex_auth(&source_auth, &isolated_auth).map_err(|error| {
            ProviderError::ExecutionError(format!(
                "could not link the Codex subscription credential into its isolated config home: {error}"
            ))
        })?;
    }
    Ok(isolated)
}

#[cfg(unix)]
fn link_codex_auth(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn link_codex_auth(source: &Path, target: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(source, target) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::copy(source, target).map(|_| ()),
    }
}

/// Each window must match what `MODEL_CONTEXT_WINDOWS` declares, because
/// `tests/context_windows.rs` compares the two.
fn known_models() -> Vec<ModelInfo> {
    // Read from the CLI itself, not from a blog post: `codex app-server`
    // answers `model/list` with the catalog the signed-in account actually
    // has. Measured against codex-cli **0.153.4** on 2026-09-08, from an
    // isolated `CODEX_HOME` holding only `auth.json` — exactly what this
    // provider does at runtime, so the list is the one a Biorouter turn sees:
    //
    //   gpt-6-astra          GPT-6-Astra    text+image   (bundled default)
    //   gpt-5.6-sol          GPT-5.6-Sol    text+image
    //   gpt-5.6-terra        GPT-5.6-Terra  text+image
    //   gpt-5.6-luna         GPT-5.6-Luna   text+image
    //   gpt-5.5              GPT-5.5        text+image
    //   gpt-5.3-codex-spark  Spark          TEXT ONLY
    //
    // codex-cli 0.147.0 (the copy on this machine's PATH) answers with the
    // same six **minus Astra**, and makes `gpt-5.6-sol` the default instead;
    // `codex exec -m gpt-6-astra` there fails `400 … requires a newer version
    // of Codex`. So Astra needs **0.153.4 or newer** — see
    // `CODEX_DEFAULT_MODEL`.
    //
    // Two ids that used to be advertised here are **retired**, not merely
    // stale: OpenAI's models page gives `gpt-5.4` and `gpt-5.4-mini` an
    // end-of-life of 2026-08-31 (replacements `gpt-5.6-terra` and
    // `gpt-5.6-luna`), both have left `model/list` on *both* CLI versions, and
    // `codex exec -m gpt-5.4` now fails `400 The 'gpt-5.4' model is not
    // supported when using Codex with a ChatGPT account`. Offering either
    // could only ever fail.
    //
    // Facts about the surviving entries, each of which was or would have been
    // a live defect:
    //
    //   * `gpt-5.3-codex` DOES NOT EXIST. The real id gained a `-spark`
    //     suffix; choosing the old one could only ever fail.
    //   * `gpt-5.3-codex-spark` is text-only, so it must NOT be marked
    //     `with_vision()` — the other five are.
    //   * Astra's 1,050,000 window is OpenAI's published figure
    //     (developers.openai.com/api/docs/models/gpt-6-astra; max output
    //     128,000). `model/list` carries no context-window field on either CLI
    //     version, so the window can never come from the probe.
    //
    // Four ids are deliberately not offered, for two different reasons — and
    // the two must not be collapsed, because only the first group is a model
    // the account cannot select:
    //
    //   * `gpt-5.6`, `gpt-5.6-pro` and `gpt-6` are **refused by the account**.
    //     `codex exec -m <id>` fails on both CLI versions with `400 The
    //     '<id>' model is not supported when using Codex with a ChatGPT
    //     account`. Putting one in the picker would only ever produce that.
    //   * `codex-auto-review` **runs**: `codex exec -m codex-auto-review` is
    //     accepted on both 0.147.0 and 0.153.4. It is absent from `model/list`
    //     on both, so it is a *hidden review model* rather than an unselectable
    //     one, and advertising a model the vendor does not list would be a
    //     guess about a surface that can change with no notice. Its absence
    //     from `model/list` is also why its effort ladder cannot be read — see
    //     `CODEX_MODELS_WITH_MAX` in `coding_agent/effort.rs`.
    //
    // Re-derive rather than trusting this comment:
    //   codex app-server --strict-config   # then: {"id":1,"method":"model/list"}
    vec![
        ModelInfo::new("gpt-6-astra", 1_050_000).with_vision(),
        ModelInfo::new("gpt-5.6-sol", 1_050_000).with_vision(),
        ModelInfo::new("gpt-5.6-terra", 1_050_000).with_vision(),
        ModelInfo::new("gpt-5.6-luna", 1_050_000).with_vision(),
        ModelInfo::new("gpt-5.5", 1_050_000).with_vision(),
        // ⚠ `without_vision()`, not a bare `new()`. A bare one leaves vision
        // UNKNOWN, and `model/list` told us the answer: inputModalities is
        // `["text"]`. Recording a known fact as unknown is its own defect.
        ModelInfo::new("gpt-5.3-codex-spark", 400_000).without_vision(),
    ]
}

/// The sentence to append to a failed turn when the model name is the likely
/// cause.
///
/// Codex's app server reports an unknown model as an `error` notification with
/// **no message and no category**, so the turn surfaces as "the Codex app
/// server reported an error" plus the generic invitation to retry — advice that
/// can never come true, for a request that will fail identically forever.
/// Measured: `gpt-5.5-codex` (a plausible-looking name that does not exist)
/// produced exactly that, and nothing in it named the model.
///
/// Deliberately a *hint on the failure path* rather than a check before the
/// call. An unlisted name is usually a typo and occasionally a model OpenAI
/// shipped after this list was written, and refusing the second to catch the
/// first would make Biorouter the reason a working model cannot be used. So an
/// unlisted model is still sent; it just stops failing anonymously.
fn unknown_model_hint(model: &str) -> String {
    let known = known_models();
    if known.iter().any(|m| m.name == model) {
        return String::new();
    }
    let names: Vec<&str> = known.iter().map(|m| m.name.as_str()).collect();
    format!(
        " — and `{model}` is not one of the models this build knows Codex to \
         offer ({}). If that name is a typo, no retry will fix it",
        names.join(", ")
    )
}

// `Clone` so the streaming path can hand a spawner to its pump task and go
// through `spawn_app_server` rather than building a second command of its own.
// Every field is cheap to clone (a path, a model config, a name).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodexProvider {
    command: PathBuf,
    model: ModelConfig,
    #[serde(skip)]
    name: String,
}

/// What one turn produced.
#[derive(Default, Debug)]
struct TurnOutcome {
    text: Vec<String>,
    usage: Option<Usage>,
    failure: Option<String>,
}

impl CodexProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        // Resolution only, no spawning: `GET /config/providers` builds every
        // provider under a 3s timeout to sample tier and affiliation.
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

    fn app_server_command(
        &self,
        isolated_home: Option<&std::path::Path>,
        features: &[&str],
    ) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.command);
        cmd.arg("app-server");
        // Reject an older or changed CLI instead of silently ignoring one of
        // the isolation settings below and exposing a host-reading built-in.
        cmd.arg("--strict-config");

        for feature in features {
            cmd.arg("--disable").arg(feature);
        }

        // `codex` is an npm shim that execs a sibling native binary and shells out
        // to git and ripgrep on its own account, so the resolved absolute path is
        // not enough — it needs the augmented PATH too.
        //
        // ⚠ It is `#!/usr/bin/env node`, so without `node` on this PATH it does
        // not merely lose a feature — it never starts. A desktop app launched
        // from Finder inherits `/usr/bin:/bin:/usr/sbin:/sbin`, not the user's
        // shell PATH, so a Node installed by Homebrew or (much worse) by a
        // version manager is invisible. The child then exits 127 with
        // `env: node: No such file or directory` before opening stdout, which
        // is the whole of the "app server closed its output" report.
        //
        // `with_leading_dir` is the reliable half: an npm global install puts
        // `node` beside the CLI, so wherever THIS machine put `codex` is the
        // best place to look. `with_node_runtimes` covers nvm / fnm / Volta /
        // asdf, which a GUI child never picks up because it does not run a
        // shell profile.
        let mut search = SearchPaths::builder().with_npm().with_node_runtimes();
        if let Some(dir) = self.command.parent() {
            search = search.with_leading_dir(dir);
        }
        if let Ok(path) = search.path() {
            cmd.env("PATH", path);
        }
        if let Some(home) = isolated_home {
            cmd.env("CODEX_HOME", home);
        }
        // LAST, after every arg and env — see the ordering note on the function.
        agent_env::configure_subscription_child(&mut cmd);
        cmd
    }

    /// The name in `Error: Unknown feature flag: <name>`, if that is what killed
    /// the child.
    ///
    /// Matched on the vendor's exact wording, deliberately: anything looser
    /// would let an unrelated failure silently drop an isolation flag, which is
    /// the one outcome this must never produce.
    fn unknown_feature_flag(stderr: &str) -> Option<String> {
        stderr
            .split("Unknown feature flag:")
            .nth(1)
            .and_then(|rest| {
                let name = rest
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>();
                (!name.is_empty()).then_some(name)
            })
    }

    /// Start `codex app-server`, dropping any feature name this build of the CLI
    /// does not recognise.
    ///
    /// ⚠ **Why this cannot just fail.** `--strict-config` makes an unknown
    /// `--disable` name FATAL, and [`DISABLED_CHILD_FEATURES`] is a hard-coded
    /// list of names owned by the vendor. So the day OpenAI renames or retires
    /// any one of them, every Biorouter user loses Codex completely — the child
    /// exits during `initialize` and no turn can start. That is not
    /// hypothetical: it was reported from the field as
    ///
    /// ```text
    /// app server exited during `initialize`: Error: Unknown feature flag: skill_search
    /// ```
    ///
    /// ⚠ **Why dropping the name is SAFE, which is the part worth checking.**
    /// An unknown feature flag disables nothing: the feature it names does not
    /// exist in this build, so not passing it removes no protection. The flags
    /// that DO exist are still passed and still enforced. The alternative --
    /// dropping `--strict-config` -- would be the unsafe fix, because it makes
    /// the CLI *silently ignore* a name it does not know, and then a genuinely
    /// misspelled `shell_tool` would leave the child holding a shell.
    ///
    /// Bounded by the list length, and each pass drops exactly the one name the
    /// CLI named, so it cannot loop.
    async fn spawn_app_server(&self) -> Result<AppServer, ProviderError> {
        let mut features: Vec<&str> = DISABLED_CHILD_FEATURES.to_vec();
        let mut dropped: Vec<String> = Vec::new();

        for _ in 0..=DISABLED_CHILD_FEATURES.len() {
            let home = isolated_codex_home(&discovery::codex_home())?;
            let command = self.app_server_command(Some(home.path()), &features);
            let server = AppServer::spawn_with_home(command, Some(home)).await?;

            match server
                .request("initialize", Self::initialize_params())
                .await
            {
                Ok(_) => {
                    if !dropped.is_empty() {
                        tracing::warn!(
                            dropped = ?dropped,
                            "this `codex` build does not know these feature names;                              they were not passed. Update DISABLED_CHILD_FEATURES."
                        );
                    }
                    return Ok(server);
                }
                Err(error) => {
                    let text = error.to_string();
                    let Some(unknown) = Self::unknown_feature_flag(&text) else {
                        server.shutdown().await;
                        return Err(error);
                    };
                    server.shutdown().await;
                    let before = features.len();
                    features.retain(|f| *f != unknown.as_str());
                    if features.len() == before {
                        // It named something we never passed — retrying would
                        // spawn the identical command forever.
                        return Err(error);
                    }
                    dropped.push(unknown);
                }
            }
        }

        Err(ProviderError::ExecutionError(
            "`codex app-server` rejected every feature name Biorouter knows".into(),
        ))
    }

    /// `initialize` parameters.
    ///
    /// Identity is declared here: Biorouter says who it is rather than
    /// impersonating the vendor's own first-party client.
    fn initialize_params() -> Value {
        json!({
            "clientInfo": { "name": "biorouter", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": true }
        })
    }

    /// The approval policy every Codex thread runs under.
    ///
    /// ⚠ **Not `"never"`, and the reason is a vendor change rather than a
    /// preference.** An MCP tool call asks for approval unless its server
    /// pre-approved it, and `never` auto-approves that ask only when the sandbox
    /// has full disk write — which Biorouter's read-only child never has. From
    /// codex-cli 0.148.0 the CLI then answers the ask itself, with
    /// `MCP tool call requires approval, but approval policy is never`, before any
    /// request reaches Biorouter. So on 0.153.4 every bridged tool failed on its
    /// first call (QA-E F1, 2026-09-10); 0.147.0, which predates the check, sent
    /// the ask to the host and worked.
    ///
    /// `granular` is the vendor's per-category form of the same policy: a `false`
    /// category is "automatically rejected instead of shown to the user", which is
    /// what `never` does, so the child's own command, rule, skill and permission
    /// requests stay refused inside the CLI with no round trip. Only
    /// `mcp_elicitations` is let through, because that is the channel the
    /// tool-call approval arrives on — see [`Self::answer_elicitation`] for which
    /// of those are then accepted.
    ///
    /// ⚠ The variant is `#[experimental("askForApproval.granular")]` in the
    /// app-server protocol, so it parses only for a client that declared
    /// `experimentalApi` in [`Self::initialize_params`]. Its shape is identical in
    /// the 0.147.0 and 0.153.4 protocol.
    fn approval_policy() -> Value {
        json!({
            "granular": {
                "sandbox_approval": false,
                "rules": false,
                "skill_approval": false,
                "request_permissions": false,
                "mcp_elicitations": true,
            }
        })
    }

    /// `thread/start` parameters.
    ///
    /// `ephemeral` keeps Codex from writing its own session files: Biorouter owns
    /// the transcript, and a second copy on disk would be governed by none of
    /// Biorouter's controls.
    fn thread_params(system: &str, cwd: &str, model: &str, bridge_url: Option<&str>) -> Value {
        // ⚠ Say which tools are real BEFORE the model picks one. Codex keeps
        // advertising `spawn_agent` even though `multi_agent` is in
        // `DISABLED_CHILD_FEATURES`, so a model asked to delegate reaches for it
        // and gets the vendor's `no thread with id` -- which reads as a broken
        // bridge and is not. Appended here rather than at the three call sites,
        // because this is the one place every Codex turn's instructions are
        // built.
        let system = format!("{system}{}", coding_agent::native_tools_notice(bridge_url));
        let mut params = json!({
            "cwd": cwd,
            "ephemeral": true,
            "sandbox": "read-only",
            "approvalPolicy": Self::approval_policy(),
            "baseInstructions": system,
        });
        if !model.trim().is_empty() {
            params["model"] = json!(model);
        }
        // Biorouter's own tools, when this turn has a bridge. `config` is the same
        // override map `-c` writes, so this is `mcp_servers.<name>.url` — the
        // streamable-HTTP form, which is the only one that needs no second process.
        //
        // ⚠ The URL is the credential. Codex sends no Authorization header on MCP
        // requests (observed), so the capability has to travel in the path, and it
        // must not be logged. `dynamicTools` would remove the HTTP hop entirely and
        // is the eventual replacement — but the installed Codex declares the
        // DynamicToolSpec types without any request that accepts them, so it is not
        // reachable yet.
        // Web search is also disabled in config for versions that register it
        // independently of the process feature set. A child reaching the
        // network on its own account is egress Biorouter never saw, and
        // Biorouter has its own web tools behind its own gates.
        let mut config = json!({
            "web_search": "disabled",
        });
        if let Some(url) = bridge_url {
            // Codex has no `--strict-mcp-config` equivalent. The process is
            // therefore spawned under an ephemeral `CODEX_HOME` containing only
            // a link to `auth.json`; there is no user `config.toml` to merge here.
            // This override is consequently the whole MCP server set, just as
            // Claude's `--strict-mcp-config` makes its bridge config the whole set.
            // #110: `tool_timeout_sec` is the per-server **second** wall clock
            // Codex applies to one `tools/call`, and `startup_timeout_sec` the
            // one it applies to the initial connect. Their defaults are far
            // below what Biorouter's slower tools need — `workspace_watch`
            // alone advertises waits of up to 600 s — and a call that outruns
            // the deadline comes back as a transport timeout rather than as the
            // partial result the handler was about to return.
            config["mcp_servers"] = json!({
                "biorouter": {
                    "url": url,
                    "tool_timeout_sec": bridge::child_tool_call_timeout().as_secs(),
                    "startup_timeout_sec": 30,
                }
            });
            // Tell the model that the bridge is the complete usable tool surface.
            // Agent::issue_tool_bridge withholds generic host-reading tools so
            // this surface cannot be used to read the subscription link below.
            // Process-level feature disables remove Codex's local shell and other
            // host-reading built-ins before the credential is made available.
            params["developerInstructions"] = json!(
                "Use the `biorouter` MCP tools for all tool work. Local shell, file, browser, \
                 plugin, and nested-agent capabilities are disabled for this child."
            );
        }
        params["config"] = config;
        params
    }

    /// `turn/start` parameters.
    ///
    /// BR-63's reasoning effort belongs **here** and nowhere else: `thread/start`
    /// has no `effort` field at all (`codex app-server generate-json-schema`),
    /// so a provider that only shapes the thread leaves `/effort` a silent no-op.
    ///
    /// The schema types `effort` as `ReasoningEffort`, which it declares as an
    /// open non-empty string — "a reasoning effort value advertised by the
    /// model" — rather than a closed enum, so the accepted set is per-model and
    /// read from `model/list`. Measured against codex-cli 0.147.0, every model
    /// it lists advertises `low` and `high`; `xhigh` is on all of them too,
    /// `max`/`ultra` only on some. Biorouter therefore sends the same
    /// `low`/`high` pair `provider_effort()` gives every other provider, which
    /// is the subset no model can reject.
    ///
    /// ⚠ `Normal` and `None` must omit the key entirely. `Normal` is documented
    /// as a strict no-op and is the default, so sending `"medium"` for it would
    /// override each model's own `defaultReasoningEffort` — which is not
    /// uniformly medium (gpt-5.6-sol defaults to `low`, gpt-5.3-codex-spark to
    /// `high`) — on every turn of every user who never touched `/effort`.
    fn turn_params(
        thread_id: &str,
        prompt: &str,
        effort: Option<ReasoningEffort>,
        model: &str,
    ) -> Value {
        Self::turn_params_with_images(thread_id, prompt, &[], effort, model)
    }

    fn turn_params_with_images(
        thread_id: &str,
        prompt: &str,
        images: &[transcript::ImageInput],
        effort: Option<ReasoningEffort>,
        model: &str,
    ) -> Value {
        let mut input = vec![json!({ "type": "text", "text": prompt })];
        input.extend(
            images
                .iter()
                .map(|image| json!({ "type": "image", "url": image.data_url() })),
        );
        json!({
            "threadId": thread_id,
            "input": input,
            // Always sent, and on Codex's own per-model ladder rather than the
            // OpenAI-family low/high pair — see `coding_agent::effort`. The model
            // is needed because that ladder differs between models: `max` exists
            // only on part of the 5.6 family, and Biorouter's own four advertised
            // models stop at `xhigh`.
            "effort": crate::providers::coding_agent::effort::codex_effort(effort, model),
        })
    }

    /// Refuse to continue if the run is not actually on the ChatGPT subscription.
    ///
    /// The sibling `ClaudeCodeProvider` has done this on every turn since it
    /// landed, off the `apiKeySource` field in Claude Code's `system/init` frame,
    /// and the reason is the same one here: `agent_env::configure_subscription_child`
    /// strips API credentials out of the child's environment, so reaching a
    /// metered mode is already a defect — but a defect that bills the user's
    /// OpenAI account instead of the subscription they chose, silently and on
    /// every turn until someone reads an invoice. Codex had no equivalent. The
    /// only place `auth_mode` was read was `discovery::probe_codex_auth`, which
    /// feeds the settings card and never runs during a turn, so a `codex` in
    /// api-key mode was simply metered.
    ///
    /// The truth is available on the turn path: the app server answers
    /// `account/read` with `{"account": {"type": "chatgpt" | "apiKey" |
    /// "amazonBedrock", …}}` (measured against codex-cli 0.147.0, which returned
    /// the live `chatgpt` account on this machine). That is one extra JSON-RPC
    /// round trip on the connection the turn is already holding, taken before
    /// `thread/start`, so a refused turn costs no tokens at all.
    ///
    /// Deliberately NOT read from `~/.codex/auth.json` the way discovery does: the
    /// file is what the app server *would* load, but the running process is what
    /// actually bills. `CODEX_HOME`, a login that happened after the file was
    /// read, and an install that authenticates some other way all separate the
    /// two, and this check exists precisely for the case where something outside
    /// Biorouter's expectations is supplying credentials.
    ///
    /// `None` is Ok, matching `ClaudeCodeProvider::assert_subscription_auth`'s
    /// treatment of a missing `apiKeySource`. A build that does not report an
    /// account — an older app server without `account/read`, or one whose answer
    /// omits it — has told us nothing, and turning "nothing" into a refusal would
    /// break every such install for a suspicion. The metered case reports itself
    /// explicitly; it is not something we have to infer from silence.
    fn assert_subscription_auth(account: Option<&Value>) -> Result<(), ProviderError> {
        let kind = account
            .filter(|a| !a.is_null())
            .and_then(|a| a.get("type"))
            .and_then(Value::as_str);
        match kind {
            None | Some("chatgpt") => Ok(()),
            Some(other) => Err(ProviderError::Authentication(format!(
                "This run would have been billed to your Codex `{other}` account rather than to \
                 your ChatGPT subscription, so it was stopped.\n\nBiorouter removes API \
                 credentials from the environment it starts `codex` in, so something outside \
                 that environment is supplying one — most likely an `OPENAI_API_KEY` in a Codex \
                 config file, or a `codex login --api-key` that replaced the subscription \
                 sign-in. Run `codex login` to go back to the subscription."
            ))),
        }
    }

    /// Ask the live app server which account it will bill, and stop if it is not
    /// the subscription.
    ///
    /// A failed *request* is not a failed check. An app server predating
    /// `account/read` rejects the method, and a turn must not die because this
    /// build cannot answer a question about itself — that would take a
    /// working-but-old Codex away from the user to protect them from a billing
    /// mode it never reported. What the response says, when there is one, is
    /// binding; that it could not be obtained is not evidence of anything. The
    /// asymmetry is the same one `assert_subscription_auth` applies to a missing
    /// `account` field, and is why the failure is logged rather than swallowed
    /// silently.
    ///
    /// **Not answering at all is also a failed request**, and it is the one the
    /// fail-open argument above does not cover on its own. "The method was
    /// rejected" is a response; "the method was ignored" is silence, and
    /// `AppServer::request` waits on silence until the child's stdout closes —
    /// which for a healthy-but-unfamiliar app server is never. That would hang
    /// the turn on its first round trip, before `thread/start`, with no error and
    /// no output: a build old enough to lack `account/read` would look like a
    /// Biorouter that stopped working rather than a Codex that stopped answering.
    /// [`ACCOUNT_READ_TIMEOUT`] turns that into the same fail-open branch a
    /// rejection takes.
    async fn assert_subscription(server: &AppServer) -> Result<(), ProviderError> {
        let answered = tokio::time::timeout(
            ACCOUNT_READ_TIMEOUT,
            server.request("account/read", json!({})),
        )
        .await;
        match answered {
            Ok(Ok(response)) => Self::assert_subscription_auth(response.get("account")),
            Ok(Err(e)) => {
                tracing::debug!(
                    error = %e,
                    "codex app-server did not answer account/read; continuing without the \
                     subscription check"
                );
                Ok(())
            }
            Err(_) => {
                tracing::debug!(
                    timeout_secs = ACCOUNT_READ_TIMEOUT.as_secs(),
                    "codex app-server ignored account/read; continuing without the \
                     subscription check rather than holding the turn open"
                );
                Ok(())
            }
        }
    }

    /// Answer a server-originated request.
    ///
    /// Anything that would let the child act on the machine is refused. Local
    /// model-controlled tools are disabled when the app server starts, so an
    /// approval request here means the CLI has exposed an unexpected capability
    /// or is reaching for authority it was not given. The honest answer is no.
    /// The one request accepted is Codex asking whether it may call a tool on
    /// Biorouter's own bridge — see [`Self::answer_elicitation`].
    ///
    /// ⚠ **Each method wants a DIFFERENT response shape**, and they are not
    /// interchangeable. Every one of them used to be answered with
    /// `{"decision": "denied"}`, and `denied` is not a valid value for any of
    /// them — verified against `codex app-server generate-json-schema` (0.147.0,
    /// and unchanged in 0.153.4):
    ///
    /// | Method | Response type | Refusal |
    /// |---|---|---|
    /// | `mcpServer/elicitation/request` | `McpServerElicitationRequestResponse` | `{"action": "decline"}` |
    /// | `item/tool/requestUserInput` | `ToolRequestUserInputResponse` — `{answers: {<question id>: …}}` | no answers: `{"answers": {}}` |
    /// | `item/commandExecution/requestApproval` | `CommandExecutionApprovalDecision` | `"decline"` (`accept`/`acceptForSession`/`acceptWithExecpolicyAmendment`/`applyNetworkPolicyAmendment`/`decline`/`cancel`) |
    /// | `item/fileChange/requestApproval` | `FileChangeApprovalDecision` | `"decline"` |
    /// | `item/permissions/requestApproval` | **not a decision at all** — `{permissions, scope?, strictAutoReview?}` | an empty `GrantedPermissionProfile`: grant nothing |
    /// | `execCommandApproval`, `applyPatchApproval` (legacy) | `ReviewDecision` | the *object* form `{"denied": {"rejection": …}}` (the bare string is `"abort"`, which also ends the turn) |
    ///
    /// `decline` rather than `cancel`, and `denied` rather than `abort`, on
    /// purpose: both refuse the action while letting the turn continue, so the
    /// child can say why it could not proceed instead of the turn dying silently.
    fn decide(method: &str, params: &Value) -> Value {
        match method {
            "mcpServer/elicitation/request" => Self::answer_elicitation(params),
            // Where Codex sends the MCP tool-call approval when its
            // `tool_call_mcp_elicitation` feature is off (and where its own
            // `request_user_input` tool asks). The parameters name no server, so
            // the request cannot be scoped to the bridge and is not accepted; no
            // answers is the refusal, which Codex reads as a cancel.
            "item/tool/requestUserInput" => json!({ "answers": {} }),
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                json!({ "decision": "decline" })
            }
            // Granting no additional permissions IS the refusal here; there is no
            // decision field to say no with.
            "item/permissions/requestApproval" => json!({ "permissions": {} }),
            "applyPatchApproval" | "execCommandApproval" => json!({
                "decision": {
                    "denied": {
                        "rejection": "Biorouter runs the child read-only; tools run on \
                                      Biorouter's side of the bridge instead."
                    }
                }
            }),
            // An unrecognised request still has to be answered or the turn stalls
            // forever. Refuse rather than guess, in the commonest shape.
            _ => json!({ "decision": "decline" }),
        }
    }

    /// Answer `mcpServer/elicitation/request` — which is how codex-cli asks
    /// whether it may make an MCP tool call, not only how an MCP server asks a
    /// person a question.
    ///
    /// The tool-call approval has no method of its own. Codex sends it here,
    /// marked `_meta.codex_approval_kind: "mcp_tool_call"`, with an empty
    /// `requestedSchema` (feature `tool_call_mcp_elicitation`, stable and on by
    /// default). Measured against 0.153.4 — the captured request is the fixture
    /// of `a_tool_call_approval_for_the_bridge_is_accepted_once`. An accept with
    /// empty content and no `persist` choice is Codex's one-time "Allow", so the
    /// next call is asked again rather than remembered in the child.
    ///
    /// It is accepted only for the bridge ([`BRIDGE_SERVER`]): that call is
    /// inspected, permission-checked and privacy-gated again on Biorouter's side,
    /// so Codex's own ask adds nothing Biorouter would say no to. A tool call to
    /// any other server is an isolation regression — the child's `CODEX_HOME`
    /// holds no other — and is declined, which Codex reports to its model as a
    /// rejected call and the turn continues.
    ///
    /// Everything else here is declined too. With `mcp_elicitations` allowed by
    /// [`Self::approval_policy`], an MCP server's own form or URL elicitation now
    /// reaches Biorouter instead of being declined inside Codex, and nobody here
    /// can fill one in: an empty accept would submit a form no person saw.
    fn answer_elicitation(params: &Value) -> Value {
        let tool_call_approval = params
            .pointer("/_meta/codex_approval_kind")
            .and_then(Value::as_str)
            == Some("mcp_tool_call");
        let for_bridge = params.get("serverName").and_then(Value::as_str) == Some(BRIDGE_SERVER);
        if tool_call_approval && for_bridge {
            json!({ "action": "accept", "content": {} })
        } else {
            json!({ "action": "decline" })
        }
    }

    /// Fold one notification into the turn's outcome. Returns true when the turn
    /// is over.
    fn absorb(outcome: &mut TurnOutcome, method: &str, params: &Value) -> bool {
        match method {
            "item/completed" => {
                let item = params.get("item").unwrap_or(&Value::Null);
                // The app server uses camelCase item types while `codex exec`
                // uses snake_case; accept both so a version change on either
                // surface does not silently drop the answer.
                let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if matches!(kind, "agentMessage" | "agent_message") {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            outcome.text.push(text.to_string());
                        }
                    }
                }
                false
            }
            "turn/completed" => {
                outcome.usage = Some(parse_usage(params.get("usage")));
                true
            }
            "turn/failed" => {
                // #180/F8: through `codex_stream::error_message`, which also
                // unwraps a vendor payload that arrives as a JSON envelope
                // rather than a sentence. One reader for both surfaces, so the
                // streaming and non-streaming paths cannot disagree about what
                // a Codex failure says.
                outcome.failure = Some(
                    codex_stream::error_message(params.get("error"))
                        .unwrap_or_else(|| "the Codex turn failed".to_string()),
                );
                true
            }
            // ⚠ The literal is "error", not "thread.error" — it breaks the dotted
            // convention every sibling notification follows, so a match written
            // from the type names alone misses it.
            "error" => {
                // #156: read BOTH spellings, nested first. Current Codex builds
                // put the text at `params.error.message`; an older fold read
                // `params.message`. `codex_stream.rs`'s `error_notice` already
                // reads both and says why — "swapping one guess for the other
                // would just move the blind spot" — but that fix landed on the
                // streaming decoder only, so this arm always fell through to the
                // placeholder and discarded the reason.
                //
                // Measured: the same account limit produced "You've hit your
                // usage limit … try again at Sep 6th" on the streaming path and
                // a bare "the Codex app server reported an error" here.
                //
                // The `turn/failed` arm above already reads the nested spelling,
                // so this file handled both shapes — just not in this arm.
                //
                // Both spellings now come from `codex_stream::error_message`,
                // the one reader the streaming decoder already used — which is
                // also what makes the JSON-envelope unwrapping (#180/F8) apply
                // here without a second copy of it.
                outcome.failure = Some(
                    codex_stream::error_message(params.get("error"))
                        .or_else(|| {
                            params
                                .get("message")
                                .and_then(Value::as_str)
                                .map(coding_agent::unwrap_json_error)
                        })
                        .unwrap_or_else(|| "the Codex app server reported an error".to_string()),
                );
                // Advisory errors can precede `turn.started` and are not fatal, so
                // this does not end the turn; `turn/failed` or `turn/completed`
                // does. Recorded in case nothing else explains an empty answer.
                false
            }
            _ => false,
        }
    }

    /// Drive one complete turn: handshake, thread, turn, then pump until done.
    async fn run_turn(
        &self,
        model: &ModelConfig,
        system: &str,
        prompt: &transcript::Prompt,
    ) -> Result<TurnOutcome, ProviderError> {
        let server = self.spawn_app_server().await?;
        let result = coding_agent::await_turn(
            self.turn_on(&server, model, system, prompt),
            coding_agent::turn_timeout(),
        )
        .await
        .map_err(|elapsed| {
            ProviderError::ExecutionError(format!(
                "the Codex handshake and turn did not finish within {}s and were stopped",
                elapsed.duration().as_secs()
            ))
        })?;
        // Always reap. A leaked `codex app-server` is a live process holding the
        // user's credential.
        server.shutdown().await;
        result
    }

    async fn turn_on(
        &self,
        server: &AppServer,
        model: &ModelConfig,
        system: &str,
        prompt: &transcript::Prompt,
    ) -> Result<TurnOutcome, ProviderError> {
        // `initialize` already happened in `spawn_app_server`, which owns it
        // because it is the request that reveals an unknown feature name and so
        // decides whether the child needs respawning. Identity is declared
        // there: Biorouter says who it is rather than impersonating the vendor's
        // own first-party client, which their terms prohibit.
        server.notify("initialized", Value::Null).await?;

        // Before `thread/start`, so a metered run is refused without sending the
        // user's prompt anywhere or costing a token. `initialize` has to come
        // first — the app server rejects requests before the handshake — which is
        // the only reason this is not the very first thing the turn does.
        Self::assert_subscription(server).await?;

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        let thread = server
            .request(
                "thread/start",
                Self::thread_params(
                    system,
                    &cwd,
                    &model.model_name,
                    bridge::active_bridge_url().as_deref(),
                ),
            )
            .await?;
        let thread_id = thread
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::RequestFailed(
                    "codex app-server did not return a thread id".to_string(),
                )
            })?
            .to_string();

        // `turn/start` is acknowledged immediately and the turn continues as
        // notifications, so the pump has to run alongside it rather than after it.
        let start = server.request(
            "turn/start",
            Self::turn_params_with_images(
                &thread_id,
                &prompt.text,
                &prompt.images,
                model.reasoning_effort,
                &model.model_name,
            ),
        );
        let pump = Self::pump(server);

        let (started, outcome) = coding_agent::await_turn(
            async { tokio::join!(start, pump) },
            coding_agent::turn_timeout(),
        )
        .await
        .map_err(|elapsed| {
            ProviderError::ExecutionError(format!(
                "the Codex turn did not finish within {}s and was stopped",
                elapsed.duration().as_secs()
            ))
        })?;
        started?;
        outcome
    }

    /// The streaming turn: handshake, thread, turn, then pump events out as they
    /// arrive rather than folding them into one final answer.
    ///
    /// Structurally the same as [`Self::turn_on`] up to `turn/start`; from there
    /// every notification goes through [`codex_stream::CodexDecoder`] and each
    /// decoded event is sent on immediately.
    #[allow(clippy::too_many_arguments)]
    async fn stream_turn(
        server: &AppServer,
        model: &ModelConfig,
        system: &str,
        prompt: &transcript::Prompt,
        bridge_url: Option<&str>,
        tx: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
        steering: Option<ProviderSteerReceiver>,
    ) -> Result<(), ProviderError> {
        // `initialize` already happened in `spawn_app_server` -- it is the
        // request that reveals an unknown feature name, so the spawner owns it
        // and respawns on one. Sending it twice makes the app server reject the
        // second as an out-of-order handshake.
        server.notify("initialized", Value::Null).await?;

        // Before `thread/start`, so a metered run is refused without sending the
        // user's prompt anywhere or costing a token.
        Self::assert_subscription(server).await?;

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        // ⚠ A coding-agent turn with no bridge silently has NO Biorouter tools --
        // the child answers from the model alone and the user cannot tell why.
        // Worth a warning rather than a silence: the failure mode is "the agent
        // ignored my request to use a tool", which reads as a model problem.
        if bridge_url.is_none() {
            tracing::warn!(
                "Codex turn is starting WITHOUT a tool bridge: the child will have \
                 none of Biorouter's tools. Expected only when the daemon has not \
                 published its base URL (a CLI process with no HTTP server)."
            );
        } else {
            tracing::debug!("Codex turn has a tool bridge");
        }
        let thread = server
            .request(
                "thread/start",
                Self::thread_params(system, &cwd, &model.model_name, bridge_url),
            )
            .await?;
        let thread_id = thread
            .get("thread")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ProviderError::RequestFailed(
                    "codex app-server did not return a thread id".to_string(),
                )
            })?
            .to_string();

        let (turn_id_tx, turn_id_rx) = tokio::sync::watch::channel(None::<String>);
        let start = async {
            let started = server
                .request(
                    "turn/start",
                    Self::turn_params_with_images(
                        &thread_id,
                        &prompt.text,
                        &prompt.images,
                        model.reasoning_effort,
                        &model.model_name,
                    ),
                )
                .await?;
            let turn_id = started
                .get("turn")
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ProviderError::RequestFailed(
                        "codex app-server did not return a turn id".to_string(),
                    )
                })?
                .to_string();
            turn_id_tx.send_replace(Some(turn_id));
            Ok::<(), ProviderError>(())
        };
        let pump = Self::stream_pump(server, model, &thread_id, turn_id_rx, tx, steering);

        let (started, pumped) = coding_agent::await_turn(
            async { tokio::join!(start, pump) },
            coding_agent::turn_timeout(),
        )
        .await
        .map_err(|elapsed| {
            ProviderError::ExecutionError(format!(
                "the Codex turn did not finish within {}s and was stopped",
                elapsed.duration().as_secs()
            ))
        })?;
        started?;
        pumped
    }

    /// Read notifications, decode them, and forward each decoded event.
    async fn stream_pump(
        server: &AppServer,
        model: &ModelConfig,
        thread_id: &str,
        mut turn_id_rx: tokio::sync::watch::Receiver<Option<String>>,
        tx: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
        mut steering: Option<ProviderSteerReceiver>,
    ) -> Result<(), ProviderError> {
        let mut decoder = codex_stream::CodexDecoder::new();
        let mut streamed_anything = false;
        let mut turn_id = turn_id_rx.borrow().clone();
        let mut pending_steering: std::collections::VecDeque<ProviderSteerRequest> =
            std::collections::VecDeque::new();
        let mut restart_after_terminal: Option<ProviderSteerRequest> = None;

        loop {
            if restart_after_terminal.is_none() {
                if let Some(active_turn_id) = turn_id.as_deref() {
                    if let Some(request) = pending_steering.pop_front() {
                        if let Err(error) =
                            Self::interrupt_turn(server, thread_id, active_turn_id).await
                        {
                            request.reject(error);
                            continue;
                        }
                        restart_after_terminal = Some(request);
                        continue;
                    }
                }
            }

            let Some(message) = Self::next_stream_message(
                server,
                &mut steering,
                &mut turn_id_rx,
                &mut turn_id,
                &mut pending_steering,
            )
            .await?
            else {
                continue;
            };
            let Some(message) = message else {
                break;
            };
            match message {
                Inbound::Request { id, method, params } => {
                    server.respond(&id, Self::decide(&method, &params)).await?;
                }
                Inbound::Notification { method, params } => {
                    match Self::emit_stream_notification(
                        &mut decoder,
                        &model.model_name,
                        &method,
                        &params,
                        tx,
                        &mut streamed_anything,
                    )? {
                        StreamPumpEvent::Continue => {}
                        StreamPumpEvent::ConsumerClosed => return Ok(()),
                        StreamPumpEvent::Terminal => {
                            if Self::handle_stream_terminal(
                                server,
                                model,
                                thread_id,
                                &mut decoder,
                                tx,
                                &mut restart_after_terminal,
                                &mut turn_id,
                            )
                            .await?
                            {
                                continue;
                            }
                            return Ok(());
                        }
                    }
                }
            }
        }

        Err(Self::interrupted_stream_error(server, &decoder, streamed_anything).await)
    }

    async fn next_stream_message(
        server: &AppServer,
        steering: &mut Option<ProviderSteerReceiver>,
        turn_id_rx: &mut tokio::sync::watch::Receiver<Option<String>>,
        turn_id: &mut Option<String>,
        pending_steering: &mut std::collections::VecDeque<ProviderSteerRequest>,
    ) -> Result<Option<Option<Inbound>>, ProviderError> {
        enum PumpInput {
            Inbound(Option<Inbound>),
            Steer(Option<ProviderSteerRequest>),
            TurnId(Result<(), tokio::sync::watch::error::RecvError>),
        }
        let input = tokio::select! {
            message = server.next_inbound() => PumpInput::Inbound(message),
            request = async {
                match steering.as_mut() {
                    Some(steering) => steering.recv().await,
                    None => std::future::pending().await,
                }
            } => PumpInput::Steer(request),
            changed = turn_id_rx.changed(), if turn_id.is_none() => PumpInput::TurnId(changed),
        };
        match input {
            PumpInput::Inbound(message) => Ok(Some(message)),
            PumpInput::Steer(Some(request)) => {
                pending_steering.push_back(request);
                Ok(None)
            }
            PumpInput::Steer(None) => {
                *steering = None;
                Ok(None)
            }
            PumpInput::TurnId(Ok(())) => {
                *turn_id = turn_id_rx.borrow().clone();
                Ok(None)
            }
            PumpInput::TurnId(Err(_)) => Err(ProviderError::RequestFailed(
                "codex app-server closed before returning a turn id".to_string(),
            )),
        }
    }

    async fn handle_stream_terminal(
        server: &AppServer,
        model: &ModelConfig,
        thread_id: &str,
        decoder: &mut codex_stream::CodexDecoder,
        tx: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
        restart_after_terminal: &mut Option<ProviderSteerRequest>,
        turn_id: &mut Option<String>,
    ) -> Result<bool, ProviderError> {
        if let Some(request) = restart_after_terminal.take() {
            match Self::start_followup_turn(server, thread_id, model, request.text()).await {
                Ok(next_turn_id) => {
                    *turn_id = Some(next_turn_id);
                    *decoder = codex_stream::CodexDecoder::new();
                    request.acknowledge();
                    return Ok(true);
                }
                Err(error) => {
                    request.reject(error);
                    return Err(ProviderError::RequestFailed(
                        "Codex stopped the original turn but could not start the steered follow-up"
                            .to_string(),
                    ));
                }
            }
        }

        let usage = decoder.usage().map(|u| u.usage).unwrap_or_default();
        let mut usage = ProviderUsage::new(model.model_name.clone(), usage);
        usage.provider = Some(KIND.provider_id().to_string());
        let _ = tx.send(Ok((None, Some(usage), None)));
        Ok(false)
    }

    async fn interrupted_stream_error(
        server: &AppServer,
        decoder: &codex_stream::CodexDecoder,
        streamed_anything: bool,
    ) -> ProviderError {
        // stdout closed without a terminal frame. This is a failure whether or
        // not text arrived: a turn that streamed half an answer and then lost
        // its app server has NOT completed, and returning `Ok` here would show
        // the user a truncated answer as though it were the whole one. The
        // Claude path errors in the same situation for the same reason.
        let detail = decoder
            .pending_failure()
            .map(str::to_string)
            .unwrap_or_else(|| format!("the app server exited{}", ""));
        let partial = if streamed_anything {
            " after a partial answer"
        } else {
            ""
        };
        ProviderError::RequestFailed(format!(
            "the Codex app server exited before finishing the turn{partial}: {detail}{}",
            server.stderr_suffix().await
        ))
    }

    fn emit_stream_notification(
        decoder: &mut codex_stream::CodexDecoder,
        model_name: &str,
        method: &str,
        params: &Value,
        tx: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
        streamed_anything: &mut bool,
    ) -> Result<StreamPumpEvent, ProviderError> {
        for event in decoder.push(method, params) {
            match event {
                codex_stream::CodexEvent::TextDelta { item_id, text }
                | codex_stream::CodexEvent::TextComplete { item_id, text } => {
                    *streamed_anything = true;
                    // The item id becomes the message id so every chunk of one
                    // answer merges into a single row instead of one row per token.
                    let mut message = Message::new(
                        Role::Assistant,
                        chrono::Utc::now().timestamp(),
                        vec![MessageContent::text(text)],
                    );
                    message.id = Some(item_id);
                    if tx.send(Ok((Some(message), None, None))).is_err() {
                        return Ok(StreamPumpEvent::ConsumerClosed);
                    }
                }
                codex_stream::CodexEvent::ReasoningDelta { item_id, text } => {
                    let mut message = Message::new(
                        Role::Assistant,
                        chrono::Utc::now().timestamp(),
                        // No signature: Codex does not sign its reasoning, and a blank
                        // one keeps this off the signed-turn persistence path.
                        vec![MessageContent::thinking(text, "")],
                    );
                    message.id = Some(item_id);
                    if tx.send(Ok((Some(message), None, None))).is_err() {
                        return Ok(StreamPumpEvent::ConsumerClosed);
                    }
                }
                codex_stream::CodexEvent::Terminal(terminal) => {
                    if let Some(error) = terminal.error {
                        return Err(ProviderError::RequestFailed(format!(
                            "{error}{}",
                            unknown_model_hint(model_name)
                        )));
                    }
                    return Ok(StreamPumpEvent::Terminal);
                }
                codex_stream::CodexEvent::Tool(event) => {
                    if !emit_codex_tool_event(*event, tx) {
                        return Ok(StreamPumpEvent::ConsumerClosed);
                    }
                }
                // Usage is read from the decoder at the terminal frame; notices
                // are advisory and never end a turn.
                codex_stream::CodexEvent::Usage(_)
                | codex_stream::CodexEvent::Notice { .. }
                | codex_stream::CodexEvent::Ignored => {}
            }
        }
        Ok(StreamPumpEvent::Continue)
    }

    async fn interrupt_turn(
        server: &AppServer,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), ProviderError> {
        // Codex 0.147 acknowledged `turn/steer` while silently omitting the
        // input from later model decisions, including across MCP calls. An
        // interrupt followed by `turn/start` on the same thread preserves the
        // partial work and makes accepting a steer mean the model will see it.
        server
            .request(
                "turn/interrupt",
                json!({
                    "threadId": thread_id,
                    "turnId": turn_id,
                }),
            )
            .await?;
        Ok(())
    }

    async fn start_followup_turn(
        server: &AppServer,
        thread_id: &str,
        model: &ModelConfig,
        instruction: &str,
    ) -> Result<String, ProviderError> {
        let started = server
            .request(
                "turn/start",
                Self::turn_params(
                    thread_id,
                    instruction,
                    model.reasoning_effort,
                    &model.model_name,
                ),
            )
            .await?;
        started
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                ProviderError::RequestFailed(
                    "codex app-server did not return a follow-up turn id".to_string(),
                )
            })
    }

    /// Read notifications until the turn ends, answering server requests as they
    /// arrive.
    async fn pump(server: &AppServer) -> Result<TurnOutcome, ProviderError> {
        let mut outcome = TurnOutcome::default();
        while let Some(message) = server.next_inbound().await {
            match message {
                Inbound::Request { id, method, params } => {
                    server.respond(&id, Self::decide(&method, &params)).await?;
                }
                Inbound::Notification { method, params } => {
                    if Self::absorb(&mut outcome, &method, &params) {
                        return Ok(outcome);
                    }
                }
            }
        }
        // stdout closed without a terminal frame.
        if outcome.failure.is_none() && outcome.text.is_empty() {
            outcome.failure = Some(format!(
                "the Codex app server exited before finishing the turn{}",
                server.stderr_suffix().await
            ));
        }
        Ok(outcome)
    }
}

/// The MCP server name Biorouter serves over the tool bridge.
///
/// Any call from another server is unexpected because Codex runs under an
/// isolated config home. It is still attributed to the child rather than to the
/// bridge so an upstream isolation regression remains visible in the transcript.
const BRIDGE_SERVER: &str = "biorouter";

/// How a Codex tool item is presented: the name on the card, the arguments to
/// expand, and who actually ran it.
fn codex_tool_identity(kind: &codex_stream::CodexToolKind) -> (String, Value, mirror::Execution) {
    match kind {
        codex_stream::CodexToolKind::McpToolCall { server, tool } => {
            if server == BRIDGE_SERVER {
                // Biorouter's own tool coming back around: it ran on Biorouter's
                // side, behind every gate.
                (
                    mirror::display_tool_name(tool).to_string(),
                    Value::Null,
                    mirror::Execution::Bridged,
                )
            } else {
                (
                    format!("{server}__{tool}"),
                    Value::Null,
                    mirror::Execution::Child,
                )
            }
        }
        // These items should be unreachable with the process feature gates and
        // read-only sandbox. Keep decoding them as defense in depth: an
        // upstream isolation regression must be visible and marked `Child`,
        // because Biorouter neither approved nor executed it.
        codex_stream::CodexToolKind::CommandExecution { command, cwd } => (
            // `exec_command`, not `exec`: the GUI's row summariser recognises
            // `shell` / `exec_command` / anything containing `command` and shows
            // the command itself, and falls back to a bare "Ran Exec with
            // command, cwd" for anything else. This is the most
            // safety-relevant card the feature produces, so the command has to
            // be legible in the collapsed row rather than one expansion away.
            "exec_command".to_string(),
            json!({ "command": command, "cwd": cwd }),
            mirror::Execution::Child,
        ),
        codex_stream::CodexToolKind::FileChange { changes } => (
            "apply_patch".to_string(),
            json!({ "changes": changes }),
            mirror::Execution::Child,
        ),
    }
}

/// Turn one decoded tool item into what the GUI draws.
///
/// `item/started` raises the skeleton card; `item/completed` mints the marked
/// request/response pair that settles it. The pairing id is the Codex item id,
/// which both halves carry.
fn emit_codex_tool_event(
    event: codex_stream::CodexToolEvent,
    tx: &tokio::sync::mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>,
) -> bool {
    let (name, base_args, exec) = codex_tool_identity(&event.kind);

    match event.lifecycle {
        codex_stream::CodexItemLifecycle::Started => tx
            .send(Ok((
                None,
                None,
                Some(PendingToolCall {
                    id: event.id,
                    name,
                    partial_args: None,
                }),
            )))
            .is_ok(),
        codex_stream::CodexItemLifecycle::Completed => {
            // The completed frame carries the real arguments for an MCP call;
            // for the built-ins the identity above already holds them.
            let arguments = event.arguments.clone().unwrap_or(base_args);
            let request = mirror::request_message(&event.id, &name, arguments, exec);
            if tx.send(Ok((Some(request), None, None))).is_err() {
                return false;
            }

            // A call is a failure if it said so, if it exited non-zero, or if an
            // approval was declined — the last of which is not in the generated
            // status enum but does appear on the wire.
            let declined = event.status.as_deref() == Some("declined");
            let failed = event.status.as_deref() == Some("failed");
            let bad_exit = event.exit_code.is_some_and(|code| code != 0);
            let tool_failed = event
                .result
                .as_ref()
                .and_then(|result| result.get("isError"))
                .and_then(Value::as_bool)
                == Some(true);
            let is_error = event.error.is_some() || failed || bad_exit || declined || tool_failed;

            if event.error.is_none() && event.aggregated_output.is_none() && !declined {
                if let Some(mut result) = event.result.as_ref().and_then(|value| {
                    serde_json::from_value::<rmcp::model::CallToolResult>(value.clone()).ok()
                }) {
                    result.is_error = Some(is_error);
                    let response = mirror::response_message_with_result(&event.id, result, exec);
                    return tx.send(Ok((Some(response), None, None))).is_ok();
                }
            }

            let body = if let Some(error) = &event.error {
                vec![rmcp::model::Content::text(error.clone())]
            } else if let Some(output) = &event.aggregated_output {
                vec![rmcp::model::Content::text(output.clone())]
            } else if declined {
                vec![rmcp::model::Content::text(
                    "Biorouter declined this unexpected local-tool request; use a \
                     Biorouter MCP tool instead."
                        .to_string(),
                )]
            } else if let Some(result) = &event.result {
                mirror::content_from_value(result)
            } else {
                Vec::new()
            };

            let response = mirror::response_message(&event.id, body, is_error, exec);
            tx.send(Ok((Some(response), None, None))).is_ok()
        }
    }
}

/// Map Codex's usage onto Biorouter's four **disjoint** buckets.
///
/// Codex follows OpenAI's convention, where `input_tokens` is the total prompt
/// count and `cached_input_tokens` is the cached *subset* of it. Biorouter's
/// [`Usage`] requires the buckets not to overlap, so the cached part is
/// subtracted out of `input_tokens` here — without that, `billed_total` would
/// double-count every cached token and stop reconciling with a vendor bill.
fn parse_usage(usage: Option<&Value>) -> Usage {
    let Some(u) = usage else {
        return Usage::default();
    };
    let get = |k: &str| u.get(k).and_then(Value::as_i64).map(|v| v as i32);

    let total_input = get("input_tokens");
    let cached = get("cached_input_tokens");
    let cache_write = get("cache_write_input_tokens");
    let output = get("output_tokens");

    let fresh_input = match (total_input, cached) {
        (Some(total), Some(c)) => Some((total - c).max(0)),
        (other, _) => other,
    };

    Usage {
        input_tokens: fresh_input,
        output_tokens: output,
        // Context occupancy for the gauge, which includes the cached prefix.
        total_tokens: match (total_input, output) {
            (None, None) => None,
            _ => Some(total_input.unwrap_or(0) + output.unwrap_or(0)),
        },
        cache_read_input_tokens: cached,
        cache_creation_input_tokens: cache_write,
    }
}

impl CodexProvider {
    async fn stream_inner(
        &self,
        system: &str,
        messages: &[Message],
        steering: Option<ProviderSteerReceiver>,
    ) -> Result<MessageStream, ProviderError> {
        let prompt = transcript::flatten_with_images(messages).ok_or_else(|| {
            ProviderError::RequestFailed("there is no user message for Codex to answer".to_string())
        })?;

        let bridge_url = bridge::active_bridge_url();
        let model_config = self.model.clone();
        let system = system.to_string();
        // ⚠ Spawn through `spawn_app_server`, NOT by building a command here.
        // This is the path the desktop app actually takes, and it used to build
        // and spawn its own command -- which meant the unknown-feature self-heal
        // covered the non-streaming path only, i.e. covered nothing a user
        // would ever hit. Two spawn sites with one recovery between them is the
        // shape of that bug; there is now one.
        let spawner = self.clone();

        let (tx, rx) =
            tokio::sync::mpsc::unbounded_channel::<Result<ProviderStreamItem, ProviderError>>();

        let pump = tokio::spawn(async move {
            let server = match spawner.spawn_app_server().await {
                Ok(server) => server,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };

            let outcome = coding_agent::await_turn(
                Self::stream_turn(
                    &server,
                    &model_config,
                    &system,
                    &prompt,
                    bridge_url.as_deref(),
                    &tx,
                    steering,
                ),
                coding_agent::turn_timeout(),
            )
            .await
            .map_err(|elapsed| {
                ProviderError::ExecutionError(format!(
                    "the Codex handshake and turn did not finish within {}s and were stopped",
                    elapsed.duration().as_secs()
                ))
            })
            .and_then(std::convert::identity);

            if let Err(e) = outcome {
                let _ = tx.send(Err(e));
            }

            server.shutdown().await;
        });

        let guard = coding_agent::AbortOnDrop(pump.abort_handle());
        let stream = async_stream::try_stream! {
            let _guard = guard;
            let mut rx = rx;
            while let Some(item) = rx.recv().await {
                yield item?;
            }
        };

        Ok(Box::pin(stream))
    }
}

#[async_trait]
impl Provider for CodexProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::with_models(
            KIND.provider_id(),
            KIND.display_name(),
            "Uses the ChatGPT subscription you are already signed in to, through your own \
             installed `codex` command. No API key. Requires the Codex CLI to be installed and \
             signed in.",
            CODEX_DEFAULT_MODEL,
            known_models(),
            CODEX_DOC_URL,
            vec![ConfigKey::new(
                KIND.command_config_key(),
                true,
                false,
                Some(KIND.default_command()),
            )],
        )
        .with_unlisted_models()
        // Public, and not `runs_locally`: the subprocess is local, the inference is
        // OpenAI's. Both are the trait defaults, restated so the decision is
        // visible rather than inherited by omission.
        .with_tier(crate::privacy::ProviderTier::Public)
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn restore_binding(&self) -> ProviderRestoreBinding {
        ProviderRestoreBinding::Codex {
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
        let prompt = transcript::flatten_with_images(messages).ok_or_else(|| {
            ProviderError::RequestFailed("there is no user message for Codex to answer".to_string())
        })?;

        let outcome = self.run_turn(model_config, system, &prompt).await?;

        if outcome.text.is_empty() {
            let detail = outcome
                .failure
                .unwrap_or_else(|| "Codex returned an empty response".to_string());
            let detail = format!("{detail}{}", unknown_model_hint(&model_config.model_name));
            return Err(ProviderError::RequestFailed(detail));
        }

        let message = Message::new(
            Role::Assistant,
            chrono::Utc::now().timestamp(),
            vec![MessageContent::text(outcome.text.join("\n\n"))],
        );

        // Attribute the row to this provider. Left unset, accounting falls back to
        // the model name and `canonical_model_pricing` invents an OpenAI per-token
        // price for a run that billed a subscription.
        let mut usage = ProviderUsage::new(
            model_config.model_name.clone(),
            outcome.usage.unwrap_or_default(),
        );
        usage.provider = Some(KIND.provider_id().to_string());
        Ok((message, usage))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_live_steering(&self) -> bool {
        true
    }

    /// Stream one turn: text appears as the model writes it.
    ///
    /// The app server already sends everything needed for this — the previous
    /// implementation simply threw it away, keeping only the one `item/completed`
    /// frame that arrives after the model has finished. [`codex_stream`] decodes
    /// the delta notifications instead; this method owns the process, the
    /// handshake and the pump.
    ///
    /// The same three rules as the Claude path apply, for the same reasons:
    /// the bridge URL is read **here**, at construction, because the task-local
    /// scope is gone once the stream is polled; the app server is owned by a task
    /// the stream aborts on drop, so a cancelled turn cannot leave one running
    /// with the user's credential; and the turn ceiling lives inside the stream,
    /// because the blocking path's timeout wraps a join this path never reaches.
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

    /// Ask the CLI whether this provider can run at all. Spawns, so never call it
    /// from `from_env`.
    async fn fetch_supported_models(&self) -> Result<Option<Vec<String>>, ProviderError> {
        let availability = discovery::probe(KIND).await;
        if !availability.auth.is_subscription() {
            return Err(coding_agent::unavailable_error(KIND, &availability));
        }
        Ok(Some(known_models().into_iter().map(|m| m.name).collect()))
    }
}

#[cfg(test)]
mod tests {
    /// #156. The `error` notification carries its text under EITHER spelling.
    /// Reading only one is a blind spot, and reading only the flat one — which
    /// this arm did — silently replaced a real reason with a placeholder.
    ///
    /// Measured against a live account limit: the streaming decoder surfaced
    /// "You've hit your usage limit … try again at Sep 6th" while this fold
    /// produced "the Codex app server reported an error", so the operator had
    /// nothing to act on and could not tell a code defect from an account limit.
    #[test]
    fn an_error_notification_keeps_its_reason_under_either_spelling() {
        use serde_json::json;

        let reason = "You've hit your usage limit. Try again at Sep 6th.";

        // Nested — what current Codex builds emit.
        let mut outcome = super::TurnOutcome::default();
        super::CodexProvider::absorb(
            &mut outcome,
            "error",
            &json!({ "error": { "message": reason } }),
        );
        assert_eq!(
            outcome.failure.as_deref(),
            Some(reason),
            "#156: the nested spelling must survive; it was being discarded"
        );

        // Bare string under `error`, which `error_message` also accepts.
        let mut outcome = super::TurnOutcome::default();
        super::CodexProvider::absorb(&mut outcome, "error", &json!({ "error": reason }));
        assert_eq!(outcome.failure.as_deref(), Some(reason));

        // Flat — the older shape, which must keep working.
        let mut outcome = super::TurnOutcome::default();
        super::CodexProvider::absorb(&mut outcome, "error", &json!({ "message": reason }));
        assert_eq!(
            outcome.failure.as_deref(),
            Some(reason),
            "the previously-handled spelling must not regress"
        );

        // Neither: the placeholder is correct only when there is nothing to say.
        let mut outcome = super::TurnOutcome::default();
        super::CodexProvider::absorb(&mut outcome, "error", &json!({ "willRetry": true }));
        assert_eq!(
            outcome.failure.as_deref(),
            Some("the Codex app server reported an error")
        );
    }

    use super::*;

    #[test]
    fn restore_binding_pins_the_resolved_codex_command() {
        let command = std::env::current_exe().unwrap();
        let provider = CodexProvider::from_resolved(
            ModelConfig::new_or_fail("gpt-5.5"),
            AbsoluteCommandPath::new(command.clone()).unwrap(),
        )
        .unwrap();
        let encoded = serde_json::to_value(provider.restore_binding()).unwrap();
        assert_eq!(encoded["kind"], "codex");
        assert_eq!(encoded["command"], serde_json::json!(command));
    }

    /// Only a `chatgpt` account is the subscription. Everything else is metered,
    /// and a metered run is refused rather than billed quietly.
    ///
    /// The three named account types are the app server's own closed set
    /// (`GetAccountResponse` in `codex app-server generate-json-schema`), and the
    /// unknown-string case is here because that set is the vendor's to extend: a
    /// type this build has never heard of must not fall through to "fine".
    #[test]
    fn only_a_chatgpt_account_is_the_subscription() {
        assert!(
            CodexProvider::assert_subscription_auth(Some(&json!({"type": "chatgpt"}))).is_ok(),
            "the ChatGPT subscription is the whole point of this provider"
        );

        for metered in ["apiKey", "amazonBedrock", "somethingNewFromOpenAI"] {
            let err = CodexProvider::assert_subscription_auth(Some(&json!({"type": metered})))
                .expect_err("a non-subscription account must stop the turn");
            assert!(
                matches!(err, ProviderError::Authentication(_)),
                "a billing-mode refusal must be typed Authentication so the retry \
                 layer does not retry it: {err:?}"
            );
            assert!(
                err.to_string().contains(metered),
                "the refusal must name what would have been billed: {err}"
            );
        }
    }

    /// No account reported is not evidence of a metered run.
    ///
    /// An app server predating `account/read`, or one whose answer omits the
    /// field, has told us nothing — and turning "nothing" into a refusal would
    /// take a working Codex away from the user over a suspicion. This mirrors
    /// `ClaudeCodeProvider::assert_subscription_auth`, which treats a missing
    /// `apiKeySource` the same way. The metered case announces itself.
    #[test]
    fn an_unreported_account_does_not_stop_the_turn() {
        assert!(CodexProvider::assert_subscription_auth(None).is_ok());
        assert!(CodexProvider::assert_subscription_auth(Some(&Value::Null)).is_ok());
        assert!(CodexProvider::assert_subscription_auth(Some(&json!({}))).is_ok());
    }

    /// The check is ON THE TURN PATH, and it fires before the prompt is sent.
    ///
    /// This is the half that the pure tests above cannot reach and the half that
    /// was actually missing: `auth_mode` was already readable, just only from
    /// `discovery::probe_codex_auth`, which builds the settings card and never
    /// runs during a turn. A correct `assert_subscription_auth` that nothing
    /// calls bills the user exactly as much as no check at all.
    ///
    /// The fake app server below answers `account/read` with an `apiKey` account
    /// and would happily complete a turn if asked. So "the turn failed with an
    /// Authentication error" and "`thread/start` was never reached" are the same
    /// observation from two sides, and the second is what proves the refusal
    /// costs no tokens.
    // ⚠ `cfg(unix)`: this test drives a fake app server written in Python over
    // stdio, and CI installs no Python on Windows (there is no `setup-python`
    // step in `.github/workflows/rust.yml`). On Windows `python3` resolves to
    // the Store's App Execution Alias or to nothing, the child never answers,
    // and `AppServer::request` — which by design has no timeout of its own, see
    // its doc comment — awaits its oneshot forever. That does not fail the job,
    // it HANGS it: `test (windows-latest)` ran 120+ minutes against a ~20 minute
    // norm until the runner's ceiling, on every head carrying this code. The
    // transport semantics under test are not platform-specific; the fake server
    // is.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_metered_codex_is_refused_before_the_prompt_is_sent() {
        let server = fake_app_server(r#"{"type":"apiKey"}"#).await;
        let provider = test_provider();
        let prompt = transcript::Prompt::text_only("hello");

        let err = provider
            .turn_on(
                &server,
                &ModelConfig::new_or_fail("gpt-5.5"),
                "SYSTEM",
                &prompt,
            )
            .await
            .expect_err("a turn on an api-key Codex must not run");

        assert!(
            matches!(err, ProviderError::Authentication(_)),
            "expected an Authentication refusal, got {err:?}"
        );
        assert!(
            err.to_string().contains("apiKey"),
            "the refusal must name the account that would have been billed: {err}"
        );

        let reached = server
            .request("test/reachedThreadStart", json!({}))
            .await
            .expect("the fake server is still alive to be asked");
        assert_eq!(
            reached["value"], false,
            "the prompt must not be sent to a metered account; the refusal has to \
             happen before `thread/start`"
        );
    }

    /// The same fake reporting a `chatgpt` account runs the turn through.
    ///
    /// Without this the test above is satisfied by a check that refuses
    /// everything, which would break the provider for every correctly signed-in
    /// user while looking like a passing suite.
    // ⚠ `cfg(unix)`: this test drives a fake app server written in Python over
    // stdio, and CI installs no Python on Windows (there is no `setup-python`
    // step in `.github/workflows/rust.yml`). On Windows `python3` resolves to
    // the Store's App Execution Alias or to nothing, the child never answers,
    // and `AppServer::request` — which by design has no timeout of its own, see
    // its doc comment — awaits its oneshot forever. That does not fail the job,
    // it HANGS it: `test (windows-latest)` ran 120+ minutes against a ~20 minute
    // norm until the runner's ceiling, on every head carrying this code. The
    // transport semantics under test are not platform-specific; the fake server
    // is.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_subscription_codex_still_runs_its_turn() {
        let server = fake_app_server(r#"{"type":"chatgpt","planType":"pro"}"#).await;
        let provider = test_provider();
        let prompt = transcript::Prompt::text_only("hello");

        let outcome = provider
            .turn_on(
                &server,
                &ModelConfig::new_or_fail("gpt-5.5"),
                "SYSTEM",
                &prompt,
            )
            .await
            .expect("a subscription turn must not be refused");
        assert_eq!(outcome.text, vec!["hi from the fake".to_string()]);
    }

    /// An app server that ignores `account/read` must not hold the turn open.
    ///
    /// The check is documented as fail-open, and the argument for that only
    /// covers a server which *answers* an unknown method with a JSON-RPC error.
    /// Silence is the other way to fail, and `AppServer::request` waits on
    /// silence until the child's stdout closes — so without a timeout this is not
    /// a degraded check, it is a turn that never starts: no error, no output,
    /// nothing to cancel it, on the very first round trip and before
    /// `thread/start`. A build too old to know the method would present as a
    /// Biorouter that stopped working.
    ///
    /// The fake below answers `initialize` and ignores everything else, which is
    /// exactly the shape being defended against — a healthy server that simply
    /// does not know this method. The clock is paused so the real
    /// [`ACCOUNT_READ_TIMEOUT`] elapses in virtual time; the test therefore
    /// exercises the production constant rather than a shortened copy of it, and
    /// still finishes in milliseconds.
    ///
    /// The outer budget is three times the inner one, so removing the `timeout`
    /// from `assert_subscription` fails here instead of hanging the suite.
    // ⚠ `cfg(unix)`: this test drives a fake app server written in Python over
    // stdio, and CI installs no Python on Windows (there is no `setup-python`
    // step in `.github/workflows/rust.yml`). On Windows `python3` resolves to
    // the Store's App Execution Alias or to nothing, the child never answers,
    // and `AppServer::request` — which by design has no timeout of its own, see
    // its doc comment — awaits its oneshot forever. That does not fail the job,
    // it HANGS it: `test (windows-latest)` ran 120+ minutes against a ~20 minute
    // norm until the runner's ceiling, on every head carrying this code. The
    // transport semantics under test are not platform-specific; the fake server
    // is.
    #[cfg(unix)]
    #[tokio::test(start_paused = true)]
    async fn an_app_server_that_ignores_account_read_does_not_hang_the_turn() {
        let server = silent_app_server().await;

        let checked = tokio::time::timeout(
            ACCOUNT_READ_TIMEOUT * 3,
            CodexProvider::assert_subscription(&server),
        )
        .await;

        let verdict = checked.expect(
            "assert_subscription never returned: an app server that ignores \
             account/read holds the turn open forever, because AppServer::request \
             waits on its oneshot until the child's stdout closes",
        );
        assert!(
            verdict.is_ok(),
            "a check that could not be obtained is not evidence of a metered run; \
             a timeout must fail open exactly as a rejected method does: {verdict:?}"
        );
    }

    /// A stand-in for an app server that answers what it knows and silently
    /// ignores what it does not — i.e. any build predating `account/read`.
    #[cfg(unix)]
    async fn silent_app_server() -> AppServer {
        let script = r#"
import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    m = json.loads(line)
    if m.get("method") == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":m["id"],"result":{"codexHome":"/tmp"}}), flush=True)
    # Everything else, account/read included, is read and dropped on the floor.
"#;
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c").arg(script);
        AppServer::spawn(cmd)
            .await
            .expect("the silent app server should start")
    }

    fn test_provider() -> CodexProvider {
        // The command is never spawned: `turn_on` is handed an already-running
        // `AppServer`, which is the seam that makes this testable at all.
        CodexProvider {
            command: PathBuf::from("codex"),
            model: ModelConfig::new_or_fail("gpt-5.5"),
            name: KIND.provider_id().to_string(),
        }
    }

    /// Issue #109, the twin of `claude_code`'s. Codex does not forward `_tools`,
    /// but the provider-driven tool-turn seam supplies the macro dispatcher over
    /// its MCP bridge.
    #[test]
    fn the_provider_declares_that_it_can_drive_a_biorouter_run_tool_loop() {
        assert!(
            test_provider().supports_tool_calls(),
            "the provider-driven bridge makes macro tools reachable"
        );
    }

    /// A scripted stand-in for `codex app-server` that reports `account` as the
    /// caller asks and otherwise completes a turn normally.
    ///
    /// Python for the same reason `appserver.rs`'s own fake is: the protocol is
    /// newline-delimited JSON and a real Codex would need a real subscription,
    /// which no test can have. It also answers a synthetic
    /// `test/reachedThreadStart`, which is the only way to observe from outside
    /// that the refusal happened *before* the prompt went anywhere rather than
    /// after.
    #[cfg(unix)]
    async fn fake_app_server(account: &str) -> AppServer {
        let script = format!(
            r#"
import sys, json
reached = False
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    m = json.loads(line)
    method = m.get("method")
    if method == "initialize":
        print(json.dumps({{"jsonrpc":"2.0","id":m["id"],"result":{{"codexHome":"/tmp"}}}}), flush=True)
    elif method == "account/read":
        print(json.dumps({{"jsonrpc":"2.0","id":m["id"],
                           "result":{{"account":{account},"requiresOpenaiAuth":True}}}}), flush=True)
    elif method == "thread/start":
        reached = True
        print(json.dumps({{"jsonrpc":"2.0","id":m["id"],"result":{{"thread":{{"id":"t-1"}}}}}}), flush=True)
    elif method == "turn/start":
        print(json.dumps({{"jsonrpc":"2.0","method":"item/completed",
                           "params":{{"item":{{"type":"agentMessage","text":"hi from the fake"}}}}}}), flush=True)
        print(json.dumps({{"jsonrpc":"2.0","method":"turn/completed","params":{{"usage":{{}}}}}}), flush=True)
        print(json.dumps({{"jsonrpc":"2.0","id":m["id"],"result":{{}}}}), flush=True)
    elif method == "test/reachedThreadStart":
        print(json.dumps({{"jsonrpc":"2.0","id":m["id"],"result":{{"value":reached}}}}), flush=True)
"#
        );
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c").arg(script);
        AppServer::spawn(cmd)
            .await
            .expect("the fake app server should start")
    }

    /// The thread is read-only, ephemeral, and carries Biorouter's instructions.
    /// Each of the four is a decision rather than a default, so each is pinned.
    #[test]
    fn thread_params_are_locked_down() {
        let p = CodexProvider::thread_params("SYSTEM", "/tmp/work", "gpt-5.5", None);
        assert_eq!(
            p["sandbox"], "read-only",
            "the child must not be able to write"
        );
        assert_eq!(
            p["ephemeral"], true,
            "Codex must not persist its own transcript"
        );
        assert_eq!(p["approvalPolicy"], CodexProvider::approval_policy());
        assert_eq!(
            p["baseInstructions"], "SYSTEM",
            "Biorouter's prompt replaces Codex's own preamble"
        );
        assert_eq!(p["cwd"], "/tmp/work");
        assert_eq!(p["model"], "gpt-5.5");
    }

    /// QA-E F1: under `"never"`, codex-cli ≥ 0.148.0 refuses every MCP tool call
    /// inside the CLI ("MCP tool call requires approval, but approval policy is
    /// never") before Biorouter is asked anything, so the bridge was dead on
    /// 0.153.4. The policy must let exactly the MCP-elicitation category through
    /// — that is the channel the tool-call approval arrives on — and keep every
    /// child-local category refused, which is what `false` means in `granular`.
    #[test]
    fn the_approval_policy_lets_only_mcp_elicitations_through() {
        let policy = CodexProvider::approval_policy();
        assert_ne!(
            policy, "never",
            "`never` makes codex-cli refuse every bridged MCP tool call itself"
        );
        let granular = policy
            .get("granular")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("expected the granular form, got {policy}"));
        assert_eq!(granular.get("mcp_elicitations"), Some(&json!(true)));
        // Every child-local category is refused without a round trip, exactly as
        // `never` refused it. Named explicitly, including the two the schema
        // defaults to false, so a future default flip cannot open one silently.
        for refused in [
            "sandbox_approval",
            "rules",
            "skill_approval",
            "request_permissions",
        ] {
            assert_eq!(
                granular.get(refused),
                Some(&json!(false)),
                "{refused} must stay refused: the child's own command execution and \
                 file changes are not Biorouter's to approve"
            );
        }
        assert_eq!(
            granular.len(),
            5,
            "an unknown category is a new approval surface: {policy}"
        );
    }

    /// `granular` is gated behind `#[experimental("askForApproval.granular")]` in
    /// the app-server protocol, so it is only accepted from a client that
    /// declared the experimental API at `initialize`. Dropping that capability
    /// would make every `thread/start` fail — pinned here so the two cannot drift.
    #[test]
    fn initialize_declares_the_experimental_api_the_policy_needs() {
        let params = CodexProvider::initialize_params();
        assert_eq!(params["capabilities"]["experimentalApi"], true);
        assert_eq!(params["clientInfo"]["name"], "biorouter");
    }

    /// An empty model means "whatever Codex defaults to", which must be expressed
    /// by omitting the key rather than sending an empty string.
    #[test]
    fn an_empty_model_is_omitted_rather_than_sent_blank() {
        let p = CodexProvider::thread_params("S", "/tmp", "   ", None);
        assert!(p.get("model").is_none());
    }

    #[test]
    fn an_isolated_codex_home_carries_auth_but_never_user_mcp_config() {
        let source = tempfile::TempDir::new().unwrap();
        std::fs::write(
            source.path().join("auth.json"),
            r#"{"auth_mode":"chatgpt"}"#,
        )
        .unwrap();
        std::fs::write(
            source.path().join("config.toml"),
            "[mcp_servers.personal]\nurl = 'http://127.0.0.1:9'\n",
        )
        .unwrap();

        let isolated = isolated_codex_home(source.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(isolated.path().join("auth.json")).unwrap(),
            r#"{"auth_mode":"chatgpt"}"#
        );
        assert!(
            !isolated.path().join("config.toml").exists(),
            "the child's config home must not inherit personal MCP servers"
        );

        let command =
            test_provider().app_server_command(Some(isolated.path()), DISABLED_CHILD_FEATURES);
        let args: Vec<_> = command.as_std().get_args().collect();
        assert!(args.iter().any(|arg| *arg == "--strict-config"));
        for feature in DISABLED_CHILD_FEATURES {
            assert!(
                args.windows(2).any(|pair| {
                    pair[0] == std::ffi::OsStr::new("--disable")
                        && pair[1] == std::ffi::OsStr::new(feature)
                }),
                "the child command did not disable {feature}: {args:?}"
            );
        }
        let configured_home = command
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == std::ffi::OsStr::new("CODEX_HOME"))
            .and_then(|(_, value)| value)
            .expect("the isolated home is applied to the child");
        assert_eq!(Path::new(configured_home), isolated.path());
    }

    #[test]
    fn a_relative_codex_home_links_an_absolute_working_credential() {
        let cwd = std::env::current_dir().unwrap();
        let source = tempfile::Builder::new()
            .prefix("relative-codex-home-")
            .tempdir_in(&cwd)
            .unwrap();
        std::fs::write(
            source.path().join("auth.json"),
            r#"{"auth_mode":"chatgpt"}"#,
        )
        .unwrap();
        let relative = source.path().strip_prefix(&cwd).unwrap();

        let isolated = isolated_codex_home(relative).unwrap();
        assert_eq!(
            std::fs::read_to_string(isolated.path().join("auth.json")).unwrap(),
            r#"{"auth_mode":"chatgpt"}"#
        );
    }

    /// A bridge URL becomes an `mcp_servers` config override, which is how
    /// Biorouter's own tools reach the child. Without a bridge no `config` key is
    /// sent at all, so the child gets no tools rather than an empty server map.
    #[test]
    fn a_bridge_url_becomes_an_mcp_server_override() {
        let with = CodexProvider::thread_params(
            "S",
            "/tmp",
            "gpt-5.5",
            Some("http://127.0.0.1:9/tool_bridge/deadbeef"),
        );
        assert_eq!(
            with["config"]["mcp_servers"]["biorouter"]["url"],
            "http://127.0.0.1:9/tool_bridge/deadbeef"
        );
        // #110: and the per-call deadline, in SECONDS — Codex's unit, not Claude
        // Code's milliseconds. Without it Codex's default abandons any bridged
        // call that outruns it, and the model is told the operation timed out
        // rather than handed the partial result the tool had ready.
        assert_eq!(
            with["config"]["mcp_servers"]["biorouter"]["tool_timeout_sec"],
            crate::providers::coding_agent::bridge::child_tool_call_timeout().as_secs()
        );

        let without = CodexProvider::thread_params("S", "/tmp", "gpt-5.5", None);
        assert!(
            without["config"].get("mcp_servers").is_none(),
            "no bridge must mean no server map"
        );
        // …but the web tool is disabled either way: that is not part of the
        // bridge, it is a standing restriction on the child.
        assert_eq!(without["config"]["web_search"], "disabled");
        assert_eq!(with["config"]["web_search"], "disabled");
        assert!(without["config"].get("tools").is_none());
        assert!(with["config"].get("tools").is_none());
        // Codex 0.147's generated ThreadStartParams has no `environments`
        // field. Sending the experimental source-tree field here is silently
        // ignored and must not be mistaken for an isolation control.
        assert!(without.get("environments").is_none());
        assert!(with.get("environments").is_none());
    }

    /// The turn's prompt and thread id are always sent — the effort rides
    /// alongside them and must not displace either.
    #[test]
    fn turn_params_always_carry_the_thread_and_the_prompt() {
        let p = CodexProvider::turn_params("th_1", "why?", Some(ReasoningEffort::Deep), "gpt-5.5");
        assert_eq!(p["threadId"], "th_1");
        assert_eq!(p["input"][0]["type"], "text");
        assert_eq!(p["input"][0]["text"], "why?");
    }

    #[test]
    fn turn_params_send_images_as_structured_codex_inputs() {
        let images = vec![transcript::ImageInput {
            data: "cGl4ZWxz".to_string(),
            mime_type: "image/png",
        }];
        let p =
            CodexProvider::turn_params_with_images("th_1", "describe", &images, None, "gpt-5.5");

        assert_eq!(p["input"][0], json!({ "type": "text", "text": "describe" }));
        assert_eq!(p["input"][1]["type"], "image");
        assert_eq!(p["input"][1]["url"], "data:image/png;base64,cGl4ZWxz");
    }

    /// `/effort` has to arrive on `turn/start`, or it is a silent no-op:
    /// `thread/start` declares no `effort` field at all, so shaping the thread
    /// cannot carry it.
    ///
    /// The rungs are Codex's own ladder rather than the OpenAI-family `low`/`high`
    /// pair — `coding_agent::effort` owns the table and the reasoning. `Deep`
    /// stops at `xhigh` here because `gpt-5.5` does not advertise `max`.
    #[test]
    fn the_effort_ladder_reaches_the_turn() {
        for (effort, expected) in [
            (Some(ReasoningEffort::Quick), "low"),
            (Some(ReasoningEffort::Normal), "high"),
            (Some(ReasoningEffort::Deep), "xhigh"),
            (None, "high"),
        ] {
            let p = CodexProvider::turn_params("th_1", "hi", effort, "gpt-5.5");
            assert_eq!(
                p["effort"], expected,
                "{effort:?} must reach the app server as effort={expected}"
            );
        }
    }

    /// The effort is **always** sent, including for the default. That is a
    /// deliberate departure from every other provider, where `Normal` is silence:
    /// a coding agent is reached for when the work is hard, so Biorouter's default
    /// here is `high` rather than whatever the model would have chosen. Asserted
    /// explicitly because it costs the user thinking tokens on every turn.
    #[test]
    fn the_default_effort_is_high_rather_than_silence() {
        let p = CodexProvider::turn_params("th_1", "hi", None, "gpt-5.5");
        assert_eq!(p["effort"], "high");
        assert!(
            !p["effort"].is_null(),
            "an explicit null is a value sent, not an absence"
        );
    }

    /// Codex's ladder is per-model, so the same `/effort deep` reaches a different
    /// rung on a model that advertises `max`.
    #[test]
    fn deep_follows_the_models_own_ladder() {
        let short = CodexProvider::turn_params("t", "hi", Some(ReasoningEffort::Deep), "gpt-5.5");
        let tall =
            CodexProvider::turn_params("t", "hi", Some(ReasoningEffort::Deep), "gpt-5.6-sol");
        assert_eq!(short["effort"], "xhigh");
        assert_eq!(tall["effort"], "max");
    }

    /// A bridged child is told that Biorouter is its complete tool surface.
    #[test]
    fn a_bridged_child_is_steered_to_the_isolated_tool_surface() {
        let with =
            CodexProvider::thread_params("S", "/tmp", "gpt-5.5", Some("http://x/tool_bridge/n"));
        let advice = with["developerInstructions"].as_str().unwrap_or_default();
        assert!(
            advice.contains("biorouter"),
            "it must name the tools that work: {advice}"
        );
        assert!(advice.contains("disabled"), "got: {advice}");

        let without = CodexProvider::thread_params("S", "/tmp", "gpt-5.5", None);
        assert!(
            without.get("developerInstructions").is_none(),
            "with no bridge there are no Biorouter tools to point at"
        );
    }

    /// The MCP tool-call approval exactly as codex-cli 0.153.4 sent it, captured
    /// from a live `codex app-server` under the granular policy on 2026-09-11
    /// (ids shortened). It is NOT a method of its own: it rides
    /// `mcpServer/elicitation/request`, marked by `_meta.codex_approval_kind`.
    fn captured_tool_call_approval(server: &str) -> Value {
        json!({
            "threadId": "01a08f7e-f25c", "turnId": "01a08f7e-f313",
            "serverName": server, "mode": "form",
            "_meta": {
                "codex_approval_kind": "mcp_tool_call",
                "persist": ["session", "always"],
                "tool_description": "Echo the given text back.",
                "tool_params": {"text": "hi"},
                "tool_params_display": [{"name": "text", "value": "hi", "display_name": "text"}]
            },
            "message": "Allow the biorouter MCP server to run tool \"echo\"?",
            "requestedSchema": {"type": "object", "properties": {}}
        })
    }

    /// QA-E F1: the approval Codex asks before calling a bridged tool is
    /// accepted — the call then runs behind Biorouter's own inspectors,
    /// permission mode, `.biorouterignore`, vault and privacy Gate C.
    ///
    /// The answer carries no `persist` choice on purpose: Codex reads an accept
    /// with none as a one-time `Approved`, so nothing is remembered in the child
    /// and every later call is asked again (and so gated again on our side).
    #[test]
    fn a_tool_call_approval_for_the_bridge_is_accepted_once() {
        let answer = CodexProvider::decide(
            "mcpServer/elicitation/request",
            &captured_tool_call_approval(BRIDGE_SERVER),
        );
        assert_eq!(answer["action"], "accept", "got {answer}");
        assert_eq!(answer["content"], json!({}));
        assert!(
            answer.get("_meta").is_none(),
            "no `persist` choice: `session`/`always` would let the child skip the ask \
             for the rest of the turn (got {answer})"
        );
    }

    /// Only the bridge is Biorouter's to vouch for. The child runs under an
    /// isolated `CODEX_HOME` with no other MCP server, so a tool call to one is
    /// an isolation regression, and the answer to it is no.
    #[test]
    fn a_tool_call_approval_for_any_other_server_is_declined() {
        let answer = CodexProvider::decide(
            "mcpServer/elicitation/request",
            &captured_tool_call_approval("personal-clinical-db"),
        );
        assert_eq!(answer["action"], "decline", "got {answer}");
    }

    /// With `mcp_elicitations` allowed, an elicitation that is NOT a tool-call
    /// approval can now reach Biorouter too (under `never` Codex declined those
    /// itself). None of them is something Biorouter can answer for a person, so
    /// each is declined — in particular an empty accept must not be sent to a
    /// form that asked for fields, or to a URL flow.
    #[test]
    fn an_elicitation_that_is_not_a_tool_call_approval_is_declined() {
        let form = json!({
            "threadId": "t", "serverName": BRIDGE_SERVER, "mode": "form",
            "message": "Which cohort?",
            "requestedSchema": {"type": "object", "properties": {"cohort": {"type": "string"}}}
        });
        let url = json!({
            "threadId": "t", "serverName": BRIDGE_SERVER, "mode": "url",
            "elicitationId": "e1", "message": "Sign in", "url": "https://example.invalid/"
        });
        let mut other_kind = captured_tool_call_approval(BRIDGE_SERVER);
        other_kind["_meta"]["codex_approval_kind"] = json!("tool_suggestion");
        for params in [form, url, other_kind] {
            let answer = CodexProvider::decide("mcpServer/elicitation/request", &params);
            assert_eq!(answer["action"], "decline", "{params} got {answer}");
        }
    }

    /// `item/tool/requestUserInput` is where Codex sends the tool-call approval
    /// when its `tool_call_mcp_elicitation` feature is off, and where its own
    /// `request_user_input` tool would ask. Its response is
    /// `{answers: {<question id>: …}}`; the catch-all's `{decision}` is not
    /// that shape at all. No answers is the valid refusal: the approval parser
    /// reads it as a cancel.
    #[test]
    fn a_user_input_request_is_refused_in_its_own_shape() {
        let params = json!({
            "threadId": "t", "turnId": "u", "itemId": "exec-1", "isBlocking": true,
            "questions": [{
                "id": "mcp_tool_call_approval_exec-1",
                "header": "Approve app tool call?",
                "question": "Allow the biorouter MCP server to run tool \"echo\"?",
                "options": [{"label": "Allow", "description": "Run the tool and continue."}]
            }]
        });
        let answer = CodexProvider::decide("item/tool/requestUserInput", &params);
        assert_eq!(answer, json!({ "answers": {} }));
    }

    /// Every approval that would let the child act on the machine is refused.
    ///
    /// ⚠ This test used to assert `decision == "denied"` for all five methods.
    /// That is not a valid value for **any** of them, so the refusals were being
    /// sent in a shape the app server cannot parse, and the test agreed with the
    /// bug rather than catching it. Each refusal below is now the one its own
    /// response schema defines (`codex app-server generate-json-schema`, 0.147.0)
    /// — which is three different shapes, not one.
    #[test]
    fn child_local_approvals_are_refused() {
        // `*ApprovalDecision`: a plain string. `decline` refuses the action and
        // lets the turn continue; `cancel` would kill the turn.
        for refused in [
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
        ] {
            assert_eq!(
                CodexProvider::decide(refused, &Value::Null)["decision"],
                "decline",
                "{refused} takes a *ApprovalDecision, whose refusal is `decline`"
            );
        }

        // Not a decision at all: the response IS the permission grant, so
        // granting nothing is how it is refused.
        let permissions = CodexProvider::decide("item/permissions/requestApproval", &Value::Null);
        assert!(
            permissions
                .get("permissions")
                .and_then(Value::as_object)
                .is_some_and(serde_json::Map::is_empty),
            "item/permissions/requestApproval takes {{permissions: GrantedPermissionProfile}} \
             — an empty profile grants nothing, which is the refusal; a `decision` \
             field here is not part of the schema at all (got {permissions})"
        );

        // Legacy `ReviewDecision`: the refusal that continues the turn is the
        // OBJECT form. The bare string `denied` is not in the enum.
        for legacy in ["applyPatchApproval", "execCommandApproval"] {
            let answer = CodexProvider::decide(legacy, &Value::Null);
            assert!(
                answer["decision"]["denied"]["rejection"].is_string(),
                "{legacy} takes a ReviewDecision, whose continue-the-turn refusal \
                 is {{denied: {{rejection}}}} (got {answer})"
            );
            assert!(
                !answer["decision"].is_string(),
                "{legacy} must not be answered with a bare string — `denied` is not \
                 one of the enum's string forms (those are `approved`, \
                 `approved_for_session`, `timed_out`, `abort`)"
            );
        }
    }

    /// An unknown request must still receive an answer, or the turn stalls
    /// forever waiting for one.
    #[test]
    fn an_unknown_request_is_still_answered() {
        let answer = CodexProvider::decide("some/future/request", &Value::Null);
        assert!(
            answer.is_object() && !answer.as_object().unwrap().is_empty(),
            "an unanswered server request blocks the turn indefinitely"
        );
    }

    /// The real notification sequence, as captured from a live `codex app-server`.
    #[test]
    fn a_captured_turn_sequence_yields_text_and_usage() {
        let mut outcome = TurnOutcome::default();
        let frames = [
            (
                "item/completed",
                json!({"item":{"id":"i0","type":"userMessage"}}),
            ),
            (
                "item/completed",
                json!({"item":{"id":"i1","type":"reasoning"}}),
            ),
            (
                "item/completed",
                json!({"item":{"id":"i2","type":"mcpToolCall","server":"biorouter",
                               "tool":"spoke_lookup","status":"completed"}}),
            ),
            (
                "item/completed",
                json!({"item":{"id":"i3","type":"agentMessage","text":"The gene is HLA-DRB1."}}),
            ),
        ];
        for (method, params) in &frames {
            assert!(
                !CodexProvider::absorb(&mut outcome, method, params),
                "{method} must not end the turn"
            );
        }
        assert!(CodexProvider::absorb(
            &mut outcome,
            "turn/completed",
            &json!({"usage":{"input_tokens":15317,"cached_input_tokens":9984,
                             "cache_write_input_tokens":0,"output_tokens":7}}),
        ));

        assert_eq!(outcome.text, vec!["The gene is HLA-DRB1."]);
        let usage = outcome.usage.unwrap();
        // input_tokens is cache-INCLUSIVE upstream, so the fresh count is the
        // difference. Overlapping buckets would double-count the cached prefix.
        assert_eq!(usage.input_tokens, Some(15317 - 9984));
        assert_eq!(usage.cache_read_input_tokens, Some(9984));
        assert_eq!(usage.output_tokens, Some(7));
        assert_eq!(usage.total_tokens, Some(15317 + 7));
    }

    /// `codex exec` spells item types in snake_case where the app server uses
    /// camelCase. Accepting both means a version change on either surface cannot
    /// silently swallow the answer.
    #[test]
    fn both_item_type_spellings_are_accepted() {
        for spelling in ["agentMessage", "agent_message"] {
            let mut outcome = TurnOutcome::default();
            CodexProvider::absorb(
                &mut outcome,
                "item/completed",
                &json!({"item":{"type":spelling,"text":"hello"}}),
            );
            assert_eq!(outcome.text, vec!["hello"], "{spelling} should be read");
        }
    }

    /// The error notification's literal is `error`, breaking the dotted convention
    /// its siblings follow — and it is advisory, so it must not end the turn.
    #[test]
    fn an_advisory_error_is_recorded_without_ending_the_turn() {
        let mut outcome = TurnOutcome::default();
        let ended = CodexProvider::absorb(
            &mut outcome,
            "error",
            &json!({"message":"config warning: feature under development"}),
        );
        assert!(
            !ended,
            "an advisory error precedes turn.started and is not fatal"
        );
        assert!(outcome.failure.is_some());

        // …and a later real answer still lands.
        CodexProvider::absorb(
            &mut outcome,
            "item/completed",
            &json!({"item":{"type":"agentMessage","text":"answer"}}),
        );
        assert_eq!(outcome.text, vec!["answer"]);
    }

    #[test]
    fn turn_failed_ends_the_turn_with_its_message() {
        let mut outcome = TurnOutcome::default();
        assert!(CodexProvider::absorb(
            &mut outcome,
            "turn/failed",
            &json!({"error":{"message":"rate limited"}}),
        ));
        assert_eq!(outcome.failure.as_deref(), Some("rate limited"));
    }

    /// An unknown model must not fail anonymously.
    ///
    /// Codex reports one as an `error` notification carrying no message and no
    /// category, so the turn reaches the user as "the Codex app server reported
    /// an error" plus the generic invitation to retry — an instruction that can
    /// never come true for a request that will fail identically forever. This
    /// is the one thing the provider knows and the app server does not: which
    /// names it believes Codex offers.
    #[test]
    fn a_failed_turn_names_an_unknown_model_and_the_ones_that_exist() {
        let hint = unknown_model_hint("gpt-5.5-codex");
        assert!(
            hint.contains("gpt-5.5-codex"),
            "the hint must name the model that was asked for: {hint}"
        );
        // ⚠ Assert against the parenthesised catalog only, never against the
        // whole hint. The hint embeds the id that was asked for, and
        // `gpt-5.5-codex` *contains* `gpt-5.5` — so a bare
        // `hint.contains("gpt-5.5")` passes on a hint that lists no models at
        // all.
        let catalog = hint
            .split_once('(')
            .unwrap_or_else(|| panic!("the hint must carry a parenthesised catalog: {hint}"))
            .1;
        assert!(
            catalog.contains("gpt-6-astra")
                && catalog.contains("gpt-5.5")
                && catalog.contains("gpt-5.3-codex-spark"),
            "and the ones that exist, so the fix is in the message: {hint}"
        );
        assert!(
            hint.contains("no retry will fix it"),
            "and must contradict the retry advice it is appended to: {hint}"
        );
    }

    /// ⚠ And it must stay SILENT for a model that is known, or every unrelated
    /// failure — a rate limit, a dropped connection — gains a paragraph about
    /// model names and sends the reader after the wrong thing.
    #[test]
    fn a_known_model_adds_nothing_to_a_failure() {
        for m in known_models() {
            assert_eq!(
                unknown_model_hint(&m.name),
                "",
                "{} is a declared model and must not be second-guessed",
                m.name
            );
        }
    }

    #[test]
    fn absent_usage_is_not_invented() {
        assert_eq!(parse_usage(None).input_tokens, None);
        assert_eq!(parse_usage(None).total_tokens, None);
    }

    #[test]
    fn metadata_is_public_and_keyless_with_one_defaulted_key() {
        let m = CodexProvider::metadata();
        assert_eq!(m.name, "codex");
        assert_eq!(m.display_name, "Codex");
        assert_eq!(m.tier, crate::privacy::ProviderTier::Public);
        assert!(
            !m.runs_locally,
            "the subprocess is local but the inference is not"
        );
        assert_eq!(m.config_keys.len(), 1);
        assert_eq!(m.config_keys[0].name, "CODEX_COMMAND");
        assert!(m.config_keys[0].required);
        assert!(!m.config_keys[0].secret);
        assert_eq!(m.config_keys[0].default.as_deref(), Some("codex"));
        // ⚠ Vision is PER MODEL, not a property of the provider. This used to
        // assert that every advertised model took images, which was true only
        // while the list happened to contain no text-only model. `model/list`
        // reports `inputModalities` per entry and `gpt-5.3-codex-spark` is
        // `["text"]` alone, so the blanket claim is now false — and asserting it
        // would force the catalog to lie about a real model's capability.
        //
        // What is worth pinning is that each entry says something definite, and
        // that the one known text-only model is not advertised as accepting
        // images: a model wrongly marked vision-capable takes an image, sends
        // it, and fails mid-turn.
        assert!(
            m.known_models
                .iter()
                .all(|model| model.supports_vision.is_some()),
            "every advertised model must state whether it takes images, rather \
             than leaving it unknown"
        );
        let spark = m
            .known_models
            .iter()
            .find(|model| model.name == "gpt-5.3-codex-spark")
            .expect("the Spark model is advertised");
        assert_eq!(
            spark.supports_vision,
            Some(false),
            "`model/list` reports inputModalities [\"text\"] for Spark"
        );
        assert!(
            m.known_models
                .iter()
                .filter(|model| model.name != "gpt-5.3-codex-spark")
                .all(|model| model.supports_vision == Some(true)),
            "every other advertised Codex model accepts image inputs"
        );
        assert!(
            !m.known_models
                .iter()
                .any(|model| model.name == "gpt-5.3-codex"),
            "`gpt-5.3-codex` is not a real model id any more — `model/list` \
             reports only `gpt-5.3-codex-spark`, so offering it can only fail"
        );
        for id in [
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.3-codex-spark",
        ] {
            assert!(
                m.known_models.iter().any(|model| model.name == id),
                "{id} is in the live catalog and must be offered"
            );
        }
        // ⚠ Retired, not merely superseded. OpenAI's models page ends both on
        // 2026-08-31; they have left `model/list` on codex-cli 0.147.0 and
        // 0.153.4 alike, and `codex exec -m gpt-5.4` answers `400 … not
        // supported when using Codex with a ChatGPT account`. Advertising
        // either offers the user a choice that can only fail.
        for retired in ["gpt-5.4", "gpt-5.4-mini"] {
            assert!(
                !m.known_models.iter().any(|model| model.name == retired),
                "{retired} retired on 2026-08-31 and must not be offered"
            );
        }
        assert_eq!(
            m.default_model, "gpt-6-astra",
            "Astra is the vendor's own bundled default from codex-cli 0.153.4 \
             (`model/list` returns it first with isDefault: true)"
        );
    }
}

/// Phase 2 end-to-end: the Codex streaming path, driven by a fake `codex
/// app-server` that emits the delta notifications a real one does.
///
/// The unit tests in `codex_stream` prove the decoder against recorded frames;
/// these prove the wiring around it — the handshake, the pump, the message ids
/// that make chunks merge into one row, and the usage attribution.
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

    /// A stand-in for the `codex` binary. It ignores the `app-server` argument
    /// and speaks just enough of the protocol to carry one streamed turn.
    ///
    /// `TOKEN_USAGE_FRAMES` is the interesting part: two snapshots, exactly as a
    /// tool-using turn produces, so the test can prove the provider reports the
    /// cumulative `total` and not the per-request `last`.
    fn fake_codex() -> FakeCli {
        let script = r#"#!/usr/bin/env python3
import sys, json

def send(obj):
    print(json.dumps(obj), flush=True)

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    m = json.loads(line)
    method = m.get("method")
    if method == "initialize":
        send({"jsonrpc":"2.0","id":m["id"],"result":{"codexHome":"/tmp"}})
    elif method == "account/read":
        send({"jsonrpc":"2.0","id":m["id"],
              "result":{"account":{"type":"chatgpt","planType":"pro"},
                        "requiresOpenaiAuth":True}})
    elif method == "thread/start":
        send({"jsonrpc":"2.0","id":m["id"],"result":{"thread":{"id":"t-1"}}})
    elif method == "turn/start":
        send({"jsonrpc":"2.0","id":m["id"],
              "result":{"turn":{"id":"turn-1"}}})
        send({"jsonrpc":"2.0","method":"item/started",
              "params":{"threadId":"t-1","turnId":"turn-1",
                        "item":{"id":"msg_1","type":"agentMessage","text":""}}})
        for piece in ["Hello", ", ", "world"]:
            send({"jsonrpc":"2.0","method":"item/agentMessage/delta",
                  "params":{"threadId":"t-1","turnId":"turn-1",
                            "itemId":"msg_1","delta":piece}})
        send({"jsonrpc":"2.0","method":"item/completed",
              "params":{"threadId":"t-1","turnId":"turn-1",
                        "item":{"id":"msg_1","type":"agentMessage",
                                "text":"Hello, world"}}})
        # A bridged call: Biorouter's own tool, coming back over the bridge.
        send({"jsonrpc":"2.0","method":"item/started",
              "params":{"threadId":"t-1","turnId":"turn-1",
                        "item":{"id":"call_1","type":"mcpToolCall",
                                "server":"biorouter",
                                "tool":"mcp__biorouter__developer__shell",
                                "status":"inProgress"}}})
        send({"jsonrpc":"2.0","method":"item/completed",
              "params":{"threadId":"t-1","turnId":"turn-1",
                        "item":{"id":"call_1","type":"mcpToolCall",
                                "server":"biorouter",
                                "tool":"mcp__biorouter__developer__shell",
                                "status":"completed",
                                "arguments":{"command":"ls"},
                                "result":"a.txt"}}})
        # An unexpected built-in event, as from an upstream isolation regression.
        send({"jsonrpc":"2.0","method":"item/completed",
              "params":{"threadId":"t-1","turnId":"turn-1",
                        "item":{"id":"exec_1","type":"commandExecution",
                                "command":"/bin/bash -lc \'rm x\'",
                                "cwd":"/tmp",
                                "status":"failed",
                                "aggregatedOutput":"rm: x: No such file",
                                "exitCode":1}}})
        # First model request.
        send({"jsonrpc":"2.0","method":"thread/tokenUsage/updated",
              "params":{"threadId":"t-1","tokenUsage":{
                  "last":{"inputTokens":100,"cachedInputTokens":0,
                          "outputTokens":10,"totalTokens":110},
                  "total":{"inputTokens":100,"cachedInputTokens":0,
                           "outputTokens":10,"totalTokens":110}}}})
        # Second model request: `last` resets, `total` accumulates. Reading
        # `last` here would undercount the turn by the first request.
        send({"jsonrpc":"2.0","method":"thread/tokenUsage/updated",
              "params":{"threadId":"t-1","tokenUsage":{
                  "last":{"inputTokens":200,"cachedInputTokens":0,
                          "outputTokens":20,"totalTokens":220},
                  "total":{"inputTokens":300,"cachedInputTokens":0,
                           "outputTokens":30,"totalTokens":330}}}})
        send({"jsonrpc":"2.0","method":"turn/completed",
              "params":{"threadId":"t-1","turn":{"id":"turn-1","status":"completed"}}})
"#;
        FakeCli::new(script)
    }

    fn steerable_codex() -> FakeCli {
        FakeCli::new(
            r#"#!/usr/bin/env python3
import sys, json

def send(obj):
    print(json.dumps(obj), flush=True)

turn_count = 0
for line in sys.stdin:
    m = json.loads(line)
    method = m.get("method")
    if method == "initialize":
        send({"jsonrpc":"2.0","id":m["id"],"result":{"codexHome":"/tmp"}})
    elif method == "account/read":
        send({"jsonrpc":"2.0","id":m["id"],
              "result":{"account":{"type":"chatgpt","planType":"pro"},
                        "requiresOpenaiAuth":True}})
    elif method == "thread/start":
        send({"jsonrpc":"2.0","id":m["id"],"result":{"thread":{"id":"t-1"}}})
    elif method == "turn/start":
        turn_count += 1
        turn_id = f"turn-{turn_count}"
        if turn_count == 2:
            expected = [{"type":"text","text":"change course"}]
            if m.get("params", {}).get("input") != expected:
                send({"jsonrpc":"2.0","id":m["id"],
                      "error":{"code":-32602,"message":"invalid follow-up input"}})
                continue
        send({"jsonrpc":"2.0","id":m["id"],"result":{"turn":{"id":turn_id}}})
        if turn_count == 2:
            send({"jsonrpc":"2.0","method":"item/agentMessage/delta",
                  "params":{"threadId":"t-1","turnId":"turn-2",
                            "itemId":"msg_1","delta":"changed course"}})
            send({"jsonrpc":"2.0","method":"turn/completed",
                  "params":{"threadId":"t-1",
                            "turn":{"id":"turn-2","status":"completed"}}})
    elif method == "turn/interrupt":
        params = m["params"]
        valid = (params.get("threadId") == "t-1" and
                 params.get("turnId") == "turn-1")
        if not valid:
            send({"jsonrpc":"2.0","id":m["id"],
                  "error":{"code":-32602,"message":"invalid interrupt params"}})
            continue
        send({"jsonrpc":"2.0","id":m["id"],"result":{}})
        send({"jsonrpc":"2.0","method":"turn/completed",
              "params":{"threadId":"t-1",
                        "turn":{"id":"turn-1","status":"interrupted"}}})
"#,
        )
    }

    fn provider_running(script: &FakeCli) -> CodexProvider {
        CodexProvider {
            command: script.path().to_path_buf(),
            model: ModelConfig::new("gpt-5.5").unwrap(),
            name: KIND.provider_id().to_string(),
        }
    }

    async fn drive() -> (Vec<Message>, Vec<ProviderUsage>) {
        let script = fake_codex();
        let provider = provider_running(&script);
        let messages = vec![Message::user().with_text("hello")];

        let stream = provider
            .stream("SYS", &messages, &[])
            .await
            .expect("the stream should open");
        futures::pin_mut!(stream);

        let mut out_messages = Vec::new();
        let mut usages = Vec::new();
        while let Some(item) = stream.next().await {
            let (message, usage, _) = item.expect("no item should error");
            if let Some(message) = message {
                out_messages.push(message);
            }
            if let Some(usage) = usage {
                usages.push(usage);
            }
        }
        (out_messages, usages)
    }

    /// Text arrives as it is written, and every chunk carries the same id so the
    /// store merges them into one row instead of one row per token.
    #[tokio::test]
    async fn a_turn_streams_its_text_in_parts_under_one_message_id() {
        let (messages, _) = drive().await;

        assert!(
            messages.len() > 1,
            "the answer must arrive in parts (got {} message(s))",
            messages.len()
        );

        let ids: std::collections::BTreeSet<_> =
            messages.iter().filter_map(|m| m.id.clone()).collect();
        assert_eq!(
            ids.len(),
            1,
            "every chunk of one answer must share the item id as its message id, \
             or persistence fragments into a row per delta (got {ids:?})"
        );

        let text: String = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|c| match c {
                MessageContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            text, "Hello, world",
            "the streamed pieces must reconstruct the answer exactly once — a \
             completed frame that re-emitted the full text would double it"
        );
    }

    /// The turn's usage is the cumulative `total` of the last snapshot.
    ///
    /// The fake sends two snapshots, as a turn that makes two model requests
    /// does. Reading the final `last` (220) instead of the final `total` (330)
    /// undercounts by the whole first request — silently, and by more the more
    /// tools the turn used.
    #[tokio::test]
    async fn usage_is_the_cumulative_total_not_the_last_request() {
        let (_, usages) = drive().await;

        let last = usages.last().expect("a terminal usage item");
        // The billed output is cumulative over the turn's two model requests
        // (10 + 20 = 30), from `total`. Reading `last` would report 20 and
        // undercount the turn by its first request.
        assert_eq!(
            last.usage.output_tokens,
            Some(30),
            "billed tokens come from tokenUsage.total, not tokenUsage.last"
        );
        // `total_tokens` is the live context gauge, which does NOT accumulate:
        // it is whatever the last request carried (220), not the sum (330).
        assert_eq!(
            last.usage.total_tokens,
            Some(220),
            "context occupancy is the last request's total, or the gauge inflates \
             by a whole context per tool call"
        );
        assert_eq!(
            last.provider.as_deref(),
            Some(KIND.provider_id()),
            "and must be attributed to this provider, or a subscription turn is \
             priced as an API call"
        );
    }

    #[tokio::test]
    async fn a_user_instruction_steers_the_running_codex_turn() {
        let script = steerable_codex();
        let provider = provider_running(&script);
        let messages = vec![Message::user().with_text("hello")];
        let (steering_tx, steering_rx) = crate::providers::base::provider_steer_channel();
        let stream = provider
            .stream_with_steering("SYS", &messages, &[], steering_rx)
            .await
            .expect("the stream should open");

        let collector = tokio::spawn(async move {
            futures::pin_mut!(stream);
            let mut text = String::new();
            while let Some(item) = stream.next().await {
                let (message, _, _) = item.map_err(|error| error.to_string())?;
                if let Some(message) = message {
                    for content in message.content {
                        if let MessageContent::Text(content) = content {
                            text.push_str(&content.text);
                        }
                    }
                }
            }
            Ok::<_, String>(text)
        });

        let (request, acknowledged) = ProviderSteerRequest::new("change course");
        assert!(steering_tx.send(request).is_ok(), "the turn is running");
        acknowledged
            .await
            .expect("the provider kept the acknowledgement")
            .expect("the interrupted thread started the steered follow-up");
        let text = collector
            .await
            .expect("the stream collector should finish")
            .expect("the steered turn should finish");
        assert_eq!(text, "changed course");
    }
    /// One tool card as the assertions care about it.
    struct Card {
        id: String,
        name: String,
        execution: Option<mirror::Execution>,
    }

    /// One settled result: the call id and whether it failed.
    struct Settled {
        id: String,
        failed: bool,
    }

    /// Only the assistant's prose is text; the tool traffic is cards.
    fn tool_pairs(messages: &[Message]) -> (Vec<Card>, Vec<Settled>) {
        let mut requests = Vec::new();
        let mut responses = Vec::new();
        for message in messages {
            for content in &message.content {
                match content {
                    MessageContent::ToolRequest(r) => {
                        let name = r
                            .tool_call
                            .as_ref()
                            .map(|c| c.name.to_string())
                            .unwrap_or_default();
                        requests.push(Card {
                            id: r.id.clone(),
                            name,
                            execution: mirror::request_execution(r),
                        });
                    }
                    MessageContent::ToolResponse(r) => {
                        let is_error = r
                            .tool_result
                            .as_ref()
                            .ok()
                            .and_then(|v| v.is_error)
                            .unwrap_or(false);
                        responses.push(Settled {
                            id: r.id.clone(),
                            failed: is_error,
                        });
                    }
                    _ => {}
                }
            }
        }
        (requests, responses)
    }

    /// A bridged call is shown under the name the user knows and marked as
    /// having run behind Biorouter's gates.
    #[tokio::test]
    async fn a_bridged_call_is_mirrored_as_a_gated_card() {
        let (messages, _) = drive().await;
        let (requests, responses) = tool_pairs(&messages);

        let call = requests
            .iter()
            .find(|c| c.id == "call_1")
            .expect("the bridged call must appear as a card");
        assert_eq!(
            call.name, "developer__shell",
            "the card shows the Biorouter tool name, not the child's MCP spelling"
        );
        assert_eq!(
            call.execution,
            Some(mirror::Execution::Bridged),
            "a call over the bridge ran behind Biorouter's inspectors and gates, \
             and must say so"
        );
        assert!(
            responses.iter().any(|r| r.id == "call_1" && !r.failed),
            "and it succeeded, so its card settles green"
        );
    }

    #[test]
    fn completed_mcp_transport_preserves_the_tools_error_flag() {
        let mut decoder = codex_stream::CodexDecoder::new();
        let events = decoder.push(
            "item/completed",
            &json!({"item": {
                "id":"failed-install", "type":"mcpToolCall", "server":"biorouter",
                "tool":"extensionmanager__install_extension", "status":"completed",
                "arguments":{}, "result":{"isError":true,
                    "content":[{"type":"text","text":"Installation was refused"}]}
            }}),
        );
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        for event in events {
            if let codex_stream::CodexEvent::Tool(event) = event {
                assert!(emit_codex_tool_event(*event, &sender));
            }
        }
        let mut messages = Vec::new();
        while let Ok(Ok((Some(message), _, _))) = receiver.try_recv() {
            messages.push(message);
        }
        let (_, settled) = tool_pairs(&messages);
        assert_eq!(settled.len(), 1);
        assert!(
            settled[0].failed,
            "an MCP tool error is not a successful catalog mutation"
        );
    }

    #[test]
    fn completed_mcp_transport_preserves_typed_content_and_result_metadata() {
        let expected = json!({
            "content": [
                {"type":"text", "text":"{\"task\":{\"id\":\"1\",\"text\":\"Inspect Soul\"}}"},
                {"type":"text", "text":"model-only context", "annotations":{"audience":["assistant"]}},
                {"type":"image", "data":"cGl4ZWw=", "mimeType":"image/png"}
            ],
            "isError":false,
            "structuredContent":{"task":{"id":"1","text":"Inspect Soul"}},
            "_meta":{"biorouter/tool-calls":[{"tool":"todo__todo_update","status":"ok"}]}
        });
        let mut decoder = codex_stream::CodexDecoder::new();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        for event in decoder.push(
            "item/completed",
            &json!({"item": {
                "id":"typed-result", "type":"mcpToolCall", "server":"biorouter",
                "tool":"todo__todo_update", "status":"completed", "arguments":{},
                "result":expected.clone()
            }}),
        ) {
            if let codex_stream::CodexEvent::Tool(event) = event {
                assert!(emit_codex_tool_event(*event, &sender));
            }
        }
        let mut results = Vec::new();
        while let Ok(Ok((Some(message), _, _))) = receiver.try_recv() {
            for content in message.content {
                if let MessageContent::ToolResponse(response) = content {
                    assert_eq!(
                        mirror::response_execution(&response),
                        Some(mirror::Execution::Bridged)
                    );
                    results.push(serde_json::to_value(response.tool_result.unwrap()).unwrap());
                }
            }
        }
        assert_eq!(results, vec![expected]);
    }

    /// An unexpected built-in event is shown and marked `Child`. This should be
    /// unreachable with the feature gates, but hiding an upstream regression or
    /// claiming Biorouter vetted it would both be false.
    #[tokio::test]
    async fn a_child_executed_builtin_is_mirrored_and_attributed_to_the_child() {
        let (messages, _) = drive().await;
        let (requests, responses) = tool_pairs(&messages);

        let exec = requests
            .iter()
            .find(|c| c.id == "exec_1")
            .expect("the child's own command must still be visible");
        assert_eq!(exec.name, "exec_command");
        assert_eq!(
            exec.execution,
            Some(mirror::Execution::Child),
            "an unexpected child-local event never passed Biorouter's gates and \
             must not be presented as though it had"
        );
        assert!(
            responses.iter().any(|r| r.id == "exec_1" && r.failed),
            "it exited non-zero, so its card must be red"
        );
    }

    /// Nothing mirrored may be dispatchable: every mirrored request carries the
    /// marker, or the agent loop would run it again.
    #[tokio::test]
    async fn every_mirrored_request_is_marked() {
        let (messages, _) = drive().await;
        let (requests, _) = tool_pairs(&messages);

        assert!(!requests.is_empty(), "the turn made tool calls");
        for card in &requests {
            assert!(
                card.execution.is_some(),
                "request {} ({}) is unmarked and the loop would execute it",
                card.id,
                card.name
            );
        }
    }
}

/// Phase 5: cancellation on the Codex streaming path.
///
/// The Codex pump owns the `AppServer`, and `AppServer::spawn` sets
/// `kill_on_drop(true)` before spawning (`coding_agent/appserver.rs`). So an
/// aborted pump drops the server, which drops the child, which is reaped —
/// which matters because the pump's own `server.shutdown()` never runs on an
/// abort. This proves that chain rather than asserting it from the code.
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

    /// A fake `codex` that completes the handshake, streams one delta, then
    /// hangs — a child still working when the user hits stop.
    fn hanging_codex(pid_file: &std::path::Path) -> FakeCli {
        let script = format!(
            r#"#!/usr/bin/env python3
import sys, json, os

with open("{pid}", "w") as f:
    f.write(str(os.getpid()))

def send(obj):
    print(json.dumps(obj), flush=True)

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    m = json.loads(line)
    method = m.get("method")
    if method == "initialize":
        send({{"jsonrpc":"2.0","id":m["id"],"result":{{"codexHome":"/tmp"}}}})
    elif method == "account/read":
        send({{"jsonrpc":"2.0","id":m["id"],
              "result":{{"account":{{"type":"chatgpt","planType":"pro"}},
                        "requiresOpenaiAuth":True}}}})
    elif method == "thread/start":
        send({{"jsonrpc":"2.0","id":m["id"],"result":{{"thread":{{"id":"t-1"}}}}}})
    elif method == "turn/start":
        send({{"jsonrpc":"2.0","id":m["id"],
              "result":{{"turn":{{"id":"turn-1"}}}}}})
        send({{"jsonrpc":"2.0","method":"item/agentMessage/delta",
              "params":{{"threadId":"t-1","turnId":"turn-1",
                        "itemId":"msg_1","delta":"working"}}}})
        # No terminal frame: the turn never ends on its own.
        while True:
            pass
"#,
            pid = pid_file.display(),
        );

        FakeCli::new(&script)
    }

    fn alive(pid: i32) -> bool {
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

    /// Dropping the stream mid-turn reaps the app server.
    #[tokio::test]
    async fn dropping_a_live_stream_reaps_the_app_server() {
        let dir = tempfile::tempdir().expect("temp dir");
        let pid_file = dir.path().join("pid");
        let script = hanging_codex(&pid_file);

        let provider = CodexProvider {
            command: script.path().to_path_buf(),
            model: ModelConfig::new("gpt-5.5").unwrap(),
            name: KIND.provider_id().to_string(),
        };
        let messages = vec![Message::user().with_text("hello")];

        // Not `pin_mut!`: that shadows the stream with a `Pin<&mut _>`, so
        // dropping it would drop a reference and leave the stream itself alive —
        // the test would then report a leak of its own making. `stream()`
        // returns a `Pin<Box<_>>`, which is `Unpin`.
        let mut stream = provider
            .stream("SYS", &messages, &[])
            .await
            .expect("stream");

        // Read until the first text arrives, so the child is definitely running.
        let mut saw_text = false;
        while let Some(item) = stream.next().await {
            let (message, _, _) = item.expect("no error before the drop");
            if message.is_some() {
                saw_text = true;
                break;
            }
        }
        assert!(
            saw_text,
            "the fake app server should have streamed some text"
        );

        let pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("the child wrote its pid")
            .trim()
            .parse()
            .expect("a numeric pid");
        assert!(alive(pid), "the app server runs while the turn is live");

        drop(stream);

        let mut reaped = false;
        for _ in 0..CHILD_WAIT_TICKS {
            if !alive(pid) {
                reaped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            reaped,
            "the app server survived the stream being dropped. The pump's own \
             server.shutdown() never runs on an abort, so this relies entirely \
             on AppServer::spawn setting kill_on_drop(true) before spawning"
        );
    }
}
