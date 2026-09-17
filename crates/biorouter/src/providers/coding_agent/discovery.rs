//! Finding the vendor CLIs, and deciding whether they are usable.
//!
//! Three separate questions, deliberately kept apart because they have very
//! different costs and very different answers:
//!
//! | question | cost | who asks |
//! |---|---|---|
//! | where is the binary? | a few `stat` calls | [`resolve_binary`], called from `from_env` |
//! | which version is it? | one process spawn | [`probe`], called from routes/CLI |
//! | is the user signed in, and to what? | one process spawn or one file read | [`probe`] |
//!
//! ⚠ The split is load-bearing. `GET /config/providers` constructs **every**
//! configured provider under a 3-second timeout in order to sample its tier and
//! affiliation, so a `from_env` that spawned `claude auth status` would slow —
//! or time out — the whole settings page. `from_env` therefore only resolves a
//! path; nothing in this module's spawning half may be reached from it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::search_path::SearchPaths;

/// How long a probe may take before we call the CLI unhealthy. Generous, because
/// a cold `claude` start was measured at ~3.5s on a warm dev machine and the
/// first run after an update can be slower.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Which vendor CLI. Kept as an enum rather than a string so the match arms that
/// differ (and there are several — the auth probe especially) cannot silently
/// fall through for a new variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentKind {
    ClaudeCode,
    Codex,
}

impl CodingAgentKind {
    /// The provider id, which is also the `blocks_fallback_pricing` key.
    ///
    /// ⚠ These exact strings matter beyond naming. `pricing.rs`'s
    /// `blocks_fallback_pricing` still lists `claude_code` and `codex` —
    /// deliberately kept when the old modules were deleted, because stored usage
    /// rows carry the provider name and `canonical_model_pricing` would otherwise
    /// invent a per-token catalogue price for a run that billed a subscription.
    /// Registering under any other id silently re-opens that bug.
    pub const fn provider_id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
        }
    }

    /// The user-facing name.
    ///
    /// ⚠ **A known deviation from Anthropic's branding guidelines, made
    /// deliberately.** Those guidelines (Agent SDK overview, "Branding
    /// guidelines") permit "Claude Agent", "Claude", or "<product> Powered by
    /// Claude", and list "Claude Code" and "Claude Code Agent" as *not permitted*
    /// for a third-party product label. This shipped as "Claude Agent" first for
    /// exactly that reason and was changed to "Claude Code" on the maintainer's
    /// instruction, because it is what users of this tool actually call it and the
    /// indirection cost them more than the guideline saves.
    ///
    /// Recorded here rather than argued again: whoever revisits this should know
    /// it was a decision and not an oversight, and that reverting it is a
    /// one-line change plus the tests that name the string.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    /// Default executable name, and the default of the provider's one config key.
    pub const fn default_command(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
        }
    }

    /// The kind whose provider id is `name`, or `None` for every other provider.
    ///
    /// The inverse of [`Self::provider_id`], so code that sees every provider the
    /// daemon serves — `check_provider_configured` — can ask "is this one of the
    /// coding agents?" without keeping a second list of their ids.
    pub fn from_provider_id(name: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|kind| kind.provider_id() == name)
    }

    /// The config key naming the executable.
    ///
    /// Each provider declares exactly one **required** key with a **default**.
    /// That shape is not cosmetic: `check_provider_configured` treats an empty
    /// `config_keys` list as requiring a `{name}_configured` marker that nothing
    /// in the tree ever writes, so a genuinely zero-key provider would report
    /// `is_configured: false` forever and never appear in the model picker.
    /// `llamacpp` solves it the same way with `LLAMACPP_PORT`.
    ///
    /// ⚠ **A saved key is necessary, not sufficient.** The key only NAMES a
    /// command, so `check_provider_configured` also requires that command to
    /// resolve ([`resolve_configured`]). Before it did, `CODEX_COMMAND` pointed at
    /// a path that did not exist left the row reading "Not installed" and
    /// "Configured" side by side, and Codex selectable in the model picker —
    /// where the bind then failed in `from_env`.
    pub const fn command_config_key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "CLAUDE_CODE_COMMAND",
            Self::Codex => "CODEX_COMMAND",
        }
    }

    /// Everything we know how to install, and how. Surfaced to the user rather
    /// than run automatically — installing another vendor's toolchain is the
    /// user's decision, and the Claude Code installer in particular is a piped
    /// shell script.
    pub const fn install_hint(self) -> &'static str {
        match self {
            Self::ClaudeCode => "curl -fsSL https://claude.ai/install.sh | bash",
            Self::Codex => "npm install -g @openai/codex@latest",
        }
    }

    /// The command the **user** runs to sign in.
    ///
    /// BioRouter never performs this itself. Anthropic's terms are explicit that
    /// third-party developers may not "offer Claude.ai login or route requests
    /// through Free, Pro, or Max plan credentials on behalf of their users", and
    /// OpenAI's position on third-party ChatGPT-plan use could not be confirmed
    /// from a first-party source. Hosting the vendor's own command in a terminal
    /// the user drives keeps the credential entirely between the user and the
    /// vendor — BioRouter never sees, stores, brokers or proxies it.
    pub const fn login_command(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude auth login",
            Self::Codex => "codex login",
        }
    }

    /// One line saying the CLI cannot be found.
    ///
    /// The first sentence of [`super::unavailable_error`]'s not-installed
    /// message, and — on its own — the reason `GET /config/providers` serves for
    /// a row the user set up whose CLI is missing, which the model picker prints
    /// on the disabled row. One definition, so the picker and the error a turn
    /// would have raised cannot come to say different things.
    pub fn not_installed_summary(self) -> String {
        format!(
            "{} is not installed, or is not on a path Biorouter searches",
            self.display_name()
        )
    }

    pub const fn all() -> [Self; 2] {
        [Self::ClaudeCode, Self::Codex]
    }
}

/// What the vendor CLI's credential store says right now.
///
/// `SignedInWithApiKey` is a distinct state rather than an error: the CLI works,
/// but the run bills a metered API account instead of the subscription, which is
/// the entire thing this feature exists to avoid. The user is told, rather than
/// silently getting a bill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum AuthState {
    /// The binary could not be found on PATH.
    NotInstalled,
    /// Found, but no credential — the user has not run the login command.
    SignedOut,
    /// Signed in on a subscription. This is the state the feature requires.
    SignedInSubscription {
        /// `"max"`, `"pro"`, … as the vendor reports it. Advisory only.
        plan: Option<String>,
        account: Option<String>,
    },
    /// Signed in, but with an API key, so usage is metered per token.
    SignedInWithApiKey,
    /// The probe ran but its output could not be understood. Carries the reason
    /// rather than collapsing to "signed out", because telling a user to log in
    /// when they already are is worse than admitting we do not know.
    Indeterminate { detail: String },
}

impl AuthState {
    /// True only for the subscription state. The gate the providers use.
    pub fn is_subscription(&self) -> bool {
        matches!(self, Self::SignedInSubscription { .. })
    }
}

/// One CLI's full situation, as the settings card renders it.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentAvailability {
    pub kind: CodingAgentKind,
    pub provider_id: String,
    pub display_name: String,
    /// Absolute path we resolved, when we found one.
    pub path: Option<String>,
    /// Raw first line of `--version`.
    pub version: Option<String>,
    pub auth: AuthState,
    /// The command to run when `auth` says the user must act.
    pub login_command: String,
    pub install_hint: String,
}

impl AgentAvailability {
    /// Ready to serve a subscription-billed turn.
    pub fn is_ready(&self) -> bool {
        self.path.is_some() && self.auth.is_subscription()
    }
}

/// Resolve the executable for `kind`, honouring the provider's config key.
///
/// Cheap enough for `from_env`: a handful of `stat` calls, no spawning.
///
/// The augmented search path is why this exists rather than a bare
/// `Command::new("claude")`. `biorouterd` is launched by the Electron main
/// process with a `PATH` of essentially `<dir of biorouterd>:<inherited>`, and a
/// GUI app's inherited `PATH` on macOS excludes `/opt/homebrew/bin`,
/// `~/.local/bin` and every npm prefix — so the naive spawn reports "not
/// installed" on a machine where the user's terminal finds the binary instantly.
/// [`SearchPaths`] is the house answer to exactly that.
pub fn resolve_binary(kind: CodingAgentKind, configured: Option<&str>) -> Option<PathBuf> {
    let name = configured
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| kind.default_command());

    // An absolute or relative path the user pinned wins outright — this is the
    // escape hatch for toolchain managers `SearchPaths` does not know about
    // (nvm, volta, bun, asdf all install outside every directory it searches).
    let as_path = Path::new(name);
    if as_path.components().count() > 1 {
        return as_path.exists().then(|| as_path.to_path_buf());
    }

    SearchPaths::builder().with_npm().resolve(name).ok()
}

/// Read the configured command for `kind` out of global config, if set.
pub fn configured_command(kind: CodingAgentKind) -> Option<String> {
    crate::config::Config::global()
        .get_param::<String>(kind.command_config_key())
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// [`resolve_binary`] under the command the user configured — "is it installed?"
/// asked the way every consumer must ask it.
///
/// ⚠ **One question, one function.** [`probe`] (behind `/coding_agents/status`,
/// whose "Not installed" pill the provider row shows) and
/// `check_provider_configured` (behind the "Configured" check on the SAME row,
/// and behind which providers the model picker offers) both call this. Two
/// spellings of it are how the row came to say both things at once. Cheap
/// enough for the provider list: `stat` calls, never a spawn.
pub fn resolve_configured(kind: CodingAgentKind) -> Option<PathBuf> {
    resolve_binary(kind, configured_command(kind).as_deref())
}

// ---------------------------------------------------------------------------
// The spawning half. Never call these from `from_env` — see the module header.
// ---------------------------------------------------------------------------

/// Full status for one CLI: path, version, and credential state.
///
/// Both spawns run under the **same scrubbed environment as a real turn**
/// ([`super::env::configure_subscription_child`]). That is deliberate and it is
/// the difference between reporting what is stored and reporting what will
/// happen: `claude auth status` answers "claude.ai" even when a stray
/// `ANTHROPIC_API_KEY` is exported, so probing with the ambient environment
/// would describe a credential our own runs will never use.
pub async fn probe(kind: CodingAgentKind) -> AgentAvailability {
    let path = resolve_configured(kind);

    let (version, auth) = match &path {
        None => (None, AuthState::NotInstalled),
        Some(exe) => {
            let version = probe_version(exe).await;
            let auth = match kind {
                CodingAgentKind::ClaudeCode => probe_claude_auth(exe).await,
                CodingAgentKind::Codex => probe_codex_auth(exe).await,
            };
            (version, auth)
        }
    };

    AgentAvailability {
        kind,
        provider_id: kind.provider_id().to_string(),
        display_name: kind.display_name().to_string(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        version,
        auth,
        login_command: kind.login_command().to_string(),
        install_hint: kind.install_hint().to_string(),
    }
}

/// Probe every CLI at once. The two spawns are independent, so they overlap.
pub async fn probe_all() -> Vec<AgentAvailability> {
    let futures = CodingAgentKind::all().map(probe);
    futures::future::join_all(futures).await
}

/// Run `<exe> <args...>` with the subscription-safe environment and a timeout.
///
/// Returns `None` on spawn failure or timeout — both mean "cannot tell", never
/// "signed out".
async fn run_probe(exe: &Path, args: &[&str]) -> Option<std::process::Output> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Give the child the augmented PATH too: `codex` is an npm shim that execs a
    // sibling native binary, so a truncated PATH can break it even when the shim
    // itself resolved.
    if let Ok(path) = SearchPaths::builder().with_npm().path() {
        cmd.env("PATH", path);
    }
    // LAST, after every env write. See the ordering warning on the function.
    super::env::configure_subscription_child(&mut cmd);

    match tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => Some(out),
        Ok(Err(e)) => {
            tracing::debug!("coding-agent probe {:?} failed to spawn: {e}", exe);
            None
        }
        Err(_) => {
            tracing::warn!(
                "coding-agent probe {:?} timed out after {PROBE_TIMEOUT:?}",
                exe
            );
            None
        }
    }
}

async fn probe_version(exe: &Path) -> Option<String> {
    let out = run_probe(exe, &["--version"]).await?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

/// `claude auth status` emits JSON on stdout by default (there is a `--text`
/// flag for humans), so this is a parse and not a scrape.
async fn probe_claude_auth(exe: &Path) -> AuthState {
    let Some(out) = run_probe(exe, &["auth", "status"]).await else {
        return AuthState::Indeterminate {
            detail: "could not run `claude auth status`".into(),
        };
    };
    parse_claude_auth(&out.stdout, out.status.code())
}

fn parse_claude_auth(stdout: &[u8], exit_code: Option<i32>) -> AuthState {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(stdout) else {
        return AuthState::Indeterminate {
            detail: "Claude Code returned an unreadable sign-in status. Check that the command path points to a current Claude Code installation, then check again.".into(),
        };
    };
    match value.get("loggedIn").and_then(serde_json::Value::as_bool) {
        Some(false) if matches!(exit_code, Some(0 | 1)) => AuthState::SignedOut,
        Some(true) if exit_code == Some(0) => {
            match value.get("authMethod").and_then(serde_json::Value::as_str) {
                Some("claude.ai") => AuthState::SignedInSubscription {
                    plan: value.get("subscriptionType").and_then(serde_json::Value::as_str).map(str::to_string),
                    account: value.get("email").and_then(serde_json::Value::as_str).map(str::to_string),
                },
                Some("api_key" | "apiKey" | "console" | "bedrock" | "vertex" | "foundry") => AuthState::SignedInWithApiKey,
                _ => AuthState::Indeterminate {
                    detail: "Claude Code did not identify its sign-in method. Check its authentication status in a terminal, then check again.".into(),
                },
            }
        }
        _ => AuthState::Indeterminate {
            detail: "Claude Code could not confirm its sign-in status. Check its authentication status in a terminal, then check again.".into(),
        },
    }
}

/// Ask the CLI first: authentication can live in a keyring, and an old auth file
/// is not evidence that the current CLI can use it. Never forward probe output:
/// API-key status and wrapper failures can contain credentials.
async fn probe_codex_auth(exe: &Path) -> AuthState {
    probe_codex_auth_in_home(exe, &codex_home()).await
}

async fn probe_codex_auth_in_home(exe: &Path, home: &Path) -> AuthState {
    let Some(out) = run_probe(exe, &["login", "status"]).await else {
        return AuthState::Indeterminate {
            detail: "Codex did not finish checking its sign-in status. Check the command path and try again.".into(),
        };
    };
    let auth = parse_codex_auth(&out.stdout, &out.stderr, out.status.code());
    if !auth.is_subscription() {
        return auth;
    }

    // A turn uses an isolated CODEX_HOME with only the linked auth file. A
    // keyring-only login cannot be advertised as ready for that execution path.
    #[derive(Deserialize)]
    struct StoredAuthMode {
        auth_mode: Option<String>,
    }
    let mode = tokio::fs::read(home.join("auth.json"))
        .await
        .ok()
        .and_then(|raw| serde_json::from_slice::<StoredAuthMode>(&raw).ok())
        .and_then(|stored| stored.auth_mode);
    if mode.as_deref() == Some("chatgpt") {
        auth
    } else {
        AuthState::Indeterminate {
            detail: "Codex is signed in, but Biorouter cannot use its saved sign-in in an isolated session. Check Codex credential storage in the official authentication instructions; Biorouter currently requires file-based ChatGPT sign-in.".into(),
        }
    }
}

fn parse_codex_auth(stdout: &[u8], stderr: &[u8], exit_code: Option<i32>) -> AuthState {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let lines: Vec<_> = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .collect();
    if exit_code == Some(1) && lines.contains(&"Not logged in") {
        return AuthState::SignedOut;
    }
    if exit_code == Some(0) {
        if lines.contains(&"Logged in using ChatGPT") {
            return AuthState::SignedInSubscription {
                plan: None,
                account: None,
            };
        }
        if lines
            .iter()
            .any(|line| line.starts_with("Logged in using an API key"))
        {
            return AuthState::SignedInWithApiKey;
        }
    }
    AuthState::Indeterminate {
        detail: "Codex could not confirm its sign-in status. Run codex login status in a terminal to check the CLI, then check again.".into(),
    }
}

/// `$CODEX_HOME`, else `~/.codex`. Turn execution links this home's `auth.json`
/// into an ephemeral config home so Codex keeps subscription authentication
/// without loading the user's `config.toml`.
pub fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_status_requires_explicit_authentication_evidence() {
        assert_eq!(
            parse_claude_auth(br#"{"loggedIn":false}"#, Some(1)),
            AuthState::SignedOut
        );
        assert!(
            parse_claude_auth(br#"{"loggedIn":true,"authMethod":"claude.ai"}"#, Some(0))
                .is_subscription()
        );
        assert_eq!(
            parse_claude_auth(br#"{"loggedIn":true,"authMethod":"api_key"}"#, Some(0)),
            AuthState::SignedInWithApiKey
        );
        for (output, exit) in [
            ("{}", Some(0)),
            ("private-token-do-not-display", Some(1)),
            (r#"{"loggedIn":true,"authMethod":"claude.ai"}"#, Some(1)),
            (r#"{"loggedIn":true,"authMethod":"unknown"}"#, Some(0)),
            (r#"{"loggedIn":false}"#, None),
        ] {
            let auth = parse_claude_auth(output.as_bytes(), exit);
            assert!(matches!(auth, AuthState::Indeterminate { .. }), "{auth:?}");
            assert!(!serde_json::to_string(&auth)
                .unwrap()
                .contains("private-token"));
        }
    }

    #[test]
    fn codex_status_requires_a_successful_recognized_login() {
        assert!(parse_codex_auth(b"", b"Logged in using ChatGPT\n", Some(0)).is_subscription());
        assert_eq!(
            parse_codex_auth(b"Not logged in\n", b"", Some(1)),
            AuthState::SignedOut
        );
        assert_eq!(
            parse_codex_auth(b"", b"Logged in using an API key - private-token", Some(0)),
            AuthState::SignedInWithApiKey
        );
        for (output, exit) in [
            ("Logged in using ChatGPT", Some(1)),
            ("Not logged in", Some(0)),
            ("private-token-do-not-display", Some(1)),
            ("Logged in using ChatGPT", None),
            ("", Some(0)),
        ] {
            let auth = parse_codex_auth(b"", output.as_bytes(), exit);
            assert!(matches!(auth, AuthState::Indeterminate { .. }), "{auth:?}");
            assert!(!serde_json::to_string(&auth)
                .unwrap()
                .contains("private-token"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_probe_does_not_trust_stale_or_inaccessible_saved_auth() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let exe = home.path().join("fake-codex");
        let write_cli = |body: &str| {
            std::fs::write(&exe, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        std::fs::write(home.path().join("auth.json"), r#"{"auth_mode":"chatgpt"}"#).unwrap();
        write_cli("echo 'Not logged in' >&2; exit 1");
        assert_eq!(
            probe_codex_auth_in_home(&exe, home.path()).await,
            AuthState::SignedOut
        );
        write_cli("echo 'Logged in using ChatGPT' >&2");
        assert!(probe_codex_auth_in_home(&exe, home.path())
            .await
            .is_subscription());
        std::fs::remove_file(home.path().join("auth.json")).unwrap();
        assert!(matches!(
            probe_codex_auth_in_home(&exe, home.path()).await,
            AuthState::Indeterminate { .. }
        ));
        std::fs::remove_file(&exe).unwrap();
        assert!(matches!(
            probe_codex_auth_in_home(&exe, home.path()).await,
            AuthState::Indeterminate { .. }
        ));
    }

    /// The ids are the pricing keys. Spelled out as a literal assertion because
    /// a rename here re-opens the fabricated-pricing bug that
    /// `blocks_fallback_pricing` exists to prevent, and nothing else would catch
    /// it — the pricing table would simply stop matching.
    #[test]
    fn provider_ids_match_the_pricing_block_list() {
        assert_eq!(CodingAgentKind::ClaudeCode.provider_id(), "claude_code");
        assert_eq!(CodingAgentKind::Codex.provider_id(), "codex");
    }

    /// The label is "Claude Code", a knowing deviation from Anthropic's branding
    /// guidelines — see [`CodingAgentKind::display_name`] for why. Pinned so the
    /// string cannot drift back and forth silently: it has already changed once,
    /// and either value is a decision rather than a default.
    ///
    /// The provider **id** is asserted separately above and must stay
    /// `claude_code` whatever the label says, because pricing keys on it.
    #[test]
    fn the_claude_label_is_the_one_that_was_chosen() {
        assert_eq!(CodingAgentKind::ClaudeCode.display_name(), "Claude Code");
        assert_eq!(CodingAgentKind::Codex.display_name(), "Codex");
        // The label and the id are independent; a rename of one must not silently
        // become a rename of the other.
        assert_eq!(CodingAgentKind::ClaudeCode.provider_id(), "claude_code");
    }

    /// A pinned path is taken verbatim rather than looked up, which is the only
    /// escape hatch for nvm/volta/bun installs.
    #[test]
    fn an_explicit_path_bypasses_the_search_path() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("claude");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();

        let found = resolve_binary(CodingAgentKind::ClaudeCode, Some(exe.to_str().unwrap()));
        assert_eq!(found.as_deref(), Some(exe.as_path()));

        let missing = resolve_binary(
            CodingAgentKind::ClaudeCode,
            Some(dir.path().join("absent").to_str().unwrap()),
        );
        assert!(
            missing.is_none(),
            "a pinned path that does not exist is not a fallback"
        );
    }

    /// An empty or whitespace config value falls back to the default name rather
    /// than resolving the empty string.
    #[test]
    fn blank_configuration_falls_back_to_the_default_command() {
        for blank in ["", "   "] {
            // Cannot assert the resolution result (depends on the host), but it
            // must not panic and must not treat "" as a relative path.
            let _ = resolve_binary(CodingAgentKind::Codex, Some(blank));
        }
        assert_eq!(CodingAgentKind::Codex.default_command(), "codex");
    }

    #[test]
    fn auth_state_only_reports_subscription_for_the_subscription_variant() {
        assert!(AuthState::SignedInSubscription {
            plan: Some("max".into()),
            account: None
        }
        .is_subscription());
        for other in [
            AuthState::NotInstalled,
            AuthState::SignedOut,
            AuthState::SignedInWithApiKey,
            AuthState::Indeterminate { detail: "x".into() },
        ] {
            assert!(
                !other.is_subscription(),
                "{other:?} must not count as subscription-backed"
            );
        }
    }

    /// `from_provider_id` is the exact inverse of `provider_id`, and answers
    /// nothing for any other provider — `check_provider_configured` runs it over
    /// every provider the daemon serves, and a false match there would hold an
    /// API provider to a CLI it has no reason to have.
    #[test]
    fn from_provider_id_inverts_provider_id_and_nothing_else() {
        for kind in CodingAgentKind::all() {
            assert_eq!(
                CodingAgentKind::from_provider_id(kind.provider_id()),
                Some(kind)
            );
        }
        for other in ["anthropic", "openai", "claude", "Codex", ""] {
            assert_eq!(CodingAgentKind::from_provider_id(other), None, "{other:?}");
        }
    }

    /// The picker's reason and the turn's error open with the same words.
    #[test]
    fn the_not_installed_summary_is_the_errors_first_sentence() {
        for kind in CodingAgentKind::all() {
            let error = super::super::unavailable_error(
                kind,
                &AgentAvailability {
                    kind,
                    provider_id: kind.provider_id().to_string(),
                    display_name: kind.display_name().to_string(),
                    path: None,
                    version: None,
                    auth: AuthState::NotInstalled,
                    login_command: kind.login_command().to_string(),
                    install_hint: kind.install_hint().to_string(),
                },
            )
            .to_string();
            assert!(
                error.contains(&format!("{}.", kind.not_installed_summary())),
                "{kind:?}: {error}"
            );
        }
    }
}
