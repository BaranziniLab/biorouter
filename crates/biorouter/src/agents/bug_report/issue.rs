//! Rendering the issue, and the two ways it can reach GitHub.
//!
//! ## Why there are two
//!
//! Nothing in this tree has ever authenticated to the GitHub API. Every
//! existing call is read-only and unauthenticated (release checks, the
//! extension updater), the single `gh` shell-out lives in a CLI workflow and
//! its auth helper launches an **interactive** `gh auth login` — unusable from
//! a tool call. So the tool has to answer "how does an issue get created" from
//! scratch, and the honest answer is: it depends on the machine.
//!
//! * [`Filer::GhCli`] — the user's own `gh`, already logged in. This genuinely
//!   creates the issue, under the user's own account, with no credential ever
//!   passing through Biorouter. It is used only when `gh auth status` succeeds
//!   **non-interactively**; a `gh` that would prompt is treated as absent.
//! * [`Filer::ComposeUrl`] — a prefilled `…/issues/new?body=…` opened in the
//!   user's browser ([`open_in_browser`]), with the link handed back as well in
//!   case no browser could be opened. The report is complete and the user's
//!   click is the submit. It is the ONLY path from a chat classified private:
//!   `gh` publishes on approval, and from a private chat the user's own Submit
//!   on GitHub must be the disclosure (see the parent module's privacy ruling).
//!
//! Not a third option: a token Biorouter stores. It would need the credential
//! store, a scope the user has to reason about, and a revocation story, to
//! replace a `gh` that most of this project's users already have.
//!
//! ## The size cliff between them
//!
//! GitHub answers 414 on a compose URL well before any browser's own limit, and
//! the body is percent-encoded on the way in — markdown, which is mostly
//! newlines and punctuation, roughly triples. So the same report can be
//! perfectly fileable through `gh` and far too large for a URL, and
//! [`compose_url`] returns `None` rather than producing a link that 414s. The
//! caller degrades to telling the user to paste, which is worse than a link and
//! much better than a dead one.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use super::evidence::Evidence;
use super::redact::{self, MAX_COMPOSE_URL_CHARS};

/// Where a report goes. A constant, not configuration.
pub const DEFAULT_REPO: &str = "BaranziniLab/biorouter";

/// Override for a fork, and for testing against a scratch repository.
///
/// ⚠ An environment variable is a redirect the agent could set for itself — it
/// has `developer__shell`. That is why the approval card names the destination
/// **repository** explicitly and says when it is not the default: the control
/// is that a person reads where it is going before clicking, not that the value
/// is unreachable. Hiding the variable would not make it unreachable; it would
/// make the redirect invisible.
pub const REPO_ENV: &str = "BIOROUTER_BUG_REPORT_REPO";

/// The label every report carries, matching the repository's own template.
pub const LABEL: &str = "bug";

/// How long `gh` gets. Long enough for a cold keyring unlock, short enough that
/// a wedged `gh` does not hold a turn open.
const GH_TIMEOUT: Duration = Duration::from_secs(45);

/// How long the *readiness probe* may take, as distinct from filing.
///
/// ⚠ It sits on the critical path BEFORE the approval card: `file_report` asks
/// `gh_ready()` so the prompt can name where the report will go. `gh auth
/// status` makes a NETWORK round-trip, so at the filing timeout a user whose
/// `gh` is slow or unauthenticated waits three quarters of a minute staring at
/// nothing before the card appears — which is the "chat that silently stops"
/// shape this feature has already been bitten by once.
///
/// A readiness check that takes longer than this is not readiness. Failing it
/// costs only the compose-URL path, which works everywhere.
const GH_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// The destination, resolved once.
pub fn repo() -> String {
    std::env::var(REPO_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REPO.to_string())
}

/// What the model wrote, before it is rendered or checked.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    pub title: String,
    /// **Describe the bug**.
    pub description: String,
    /// **To Reproduce**, one step per entry.
    pub steps: Vec<String>,
    /// **Expected behavior**.
    pub expected: String,
    /// **Suspected cause**: the reporting agent's diagnosis, kept apart from
    /// the description so observation and hypothesis can be told apart by the
    /// maintainer or debugging agent who takes the report over. Optional as an
    /// ARGUMENT, and not one of `redact::REQUIRED_SECTIONS` (a report written by
    /// hand from the template may leave it out), but the section is always
    /// rendered: see [`NO_SUSPECTED_CAUSE`].
    pub suspected_cause: Option<String>,
    /// **Additional context**.
    pub additional: Option<String>,
}

/// Render the body in the shape of the repository's own `bug_report.md`.
///
/// ⚠ It is rendered here and nowhere else. The desktop's Diagnostics modal
/// hand-duplicates this template in the renderer, and the two have already
/// drifted — different heading wording, and two dead links built by pasting
/// repo-relative documentation paths after a `github.com/<org>/<repo>/` prefix.
/// A second copy of a template is a copy that will disagree with it.
///
/// `private` is the filing chat's privacy (`treat_as_private`). It changes the
/// footer only: a private chat's body does not invite the reader to attach the
/// diagnostics bundle, which is that chat's transcript, unredacted. Nothing in
/// the body says the chat was private — that is itself a fact about the user's
/// work, and the body is public.
pub fn render_body(draft: &Draft, evidence: &Evidence, private: bool) -> String {
    let mut body = String::new();

    body.push_str("**Describe the bug**\n\n");
    body.push_str(draft.description.trim());
    body.push_str("\n\n---\n\n**To Reproduce**\n");
    if draft.steps.is_empty() {
        body.push_str(
            "\nThe reporter did not give explicit steps. What the session was doing is \
             summarised under *Additional context*.\n",
        );
    } else {
        body.push('\n');
        for (index, step) in draft.steps.iter().enumerate() {
            body.push_str(&format!("{}. {}\n", index + 1, step.trim()));
        }
    }

    body.push_str("\n---\n\n**Expected behavior**\n\n");
    body.push_str(if draft.expected.trim().is_empty() {
        "Not stated by the reporter."
    } else {
        draft.expected.trim()
    });

    // After the observation, before the environment: the order a reader takes
    // a report over in. Labelled as unconfirmed, because a hypothesis rendered
    // in the same voice as the description reads as established fact.
    //
    // ⚠ ALWAYS rendered. A report is meant to hand over to a debugging agent,
    // and a missing section cannot be told apart from a section somebody forgot
    // — so a report with no diagnosis says so, in a fixed line.
    body.push_str("\n\n---\n\n**Suspected cause**\n\n");
    match draft
        .suspected_cause
        .as_deref()
        .map(str::trim)
        .filter(|cause| !cause.is_empty())
    {
        Some(cause) => body.push_str(&format!(
            "_From the reporting agent's analysis of the session, the documentation and the \
             source at v{}. Not yet confirmed by a maintainer._\n\n{cause}",
            evidence.app_version
        )),
        None => body.push_str(NO_SUSPECTED_CAUSE),
    }

    body.push_str("\n\n---\n\n**Please provide the following information**\n");
    body.push_str(&format!(
        "- **OS & Arch:** {} {} ({})\n",
        evidence.os, evidence.os_version, evidence.architecture
    ));
    body.push_str("- **Interface:** Biorouter agent (reported from a chat)\n");
    body.push_str(&format!("- **Version:** v{}\n", evidence.app_version));
    body.push_str(&format!(
        "- **Extensions enabled:** {}\n",
        if evidence.enabled_extensions.is_empty() {
            "none".to_string()
        } else {
            evidence.enabled_extensions.join(", ")
        }
    ));
    body.push_str(&format!(
        "- **Provider & Model:** {} – {}\n",
        evidence.provider.as_deref().unwrap_or("not set"),
        evidence.model.as_deref().unwrap_or("not set"),
    ));

    body.push_str("\n---\n\n**Additional context**\n");
    if let Some(additional) = draft.additional.as_ref().map(|a| a.trim()) {
        if !additional.is_empty() {
            body.push_str(&format!("\n{additional}\n"));
        }
    }

    if evidence.failures.is_empty() {
        body.push_str("\nNo failed tool calls were recorded in the reporting session.\n");
    } else {
        body.push_str(&format!(
            "\n<details>\n<summary>Failed tool calls in the reporting session \
             ({} of {} calls)</summary>\n\n",
            evidence.total_failed_calls, evidence.total_tool_calls
        ));
        for failure in &evidence.failures {
            body.push_str(&failure.to_line());
            body.push('\n');
            if let Some(arguments) = &failure.arguments {
                body.push_str(&format!("  - arguments: `{arguments}`\n"));
            }
        }
        body.push_str("\n</details>\n");
    }

    if evidence.externalized_results > 0 {
        body.push_str(&format!(
            "\n{} tool result(s) were too large to keep in the transcript and are not \
             quoted above.\n",
            evidence.externalized_results
        ));
    }

    body.push_str(
        "\n---\n\n<sub>Filed by Biorouter's own bug reporter from an in-app chat. The \
         environment and failure list above are read from the reporting session; home \
         paths, usernames and credential-shaped strings are removed before posting.",
    );
    // ⚠ Not from a private chat: the bundle IS that chat's transcript,
    // unredacted, and this body is the page the user reads on github.com just
    // before pressing Submit. The receipt tells them not to attach it.
    if !private {
        body.push_str(
            " A full diagnostics bundle (transcript, redacted config, logs) can be attached \
             from **Chat summary → Diagnostics → Generate diagnostics**.",
        );
    }
    body.push_str("</sub>\n");

    body
}

/// The **Suspected cause** line of a report that supplied no diagnosis.
///
/// A fixed sentence rather than an absent section, so whoever takes the issue
/// over knows the reporting agent did not diagnose it, rather than wondering
/// whether the section was dropped.
pub const NO_SUSPECTED_CAUSE: &str =
    "Not determined by the reporting agent (no diagnosis was supplied).";

/// The prefilled compose URL, or `None` when the body cannot fit in one.
///
/// The cap is applied to the ENCODED url — see the module header.
pub fn compose_url(repo: &str, title: &str, body: &str) -> Option<String> {
    let url = format!(
        "https://github.com/{repo}/issues/new?template=bug_report.md&labels={}&title={}&body={}",
        urlencoding::encode(LABEL),
        urlencoding::encode(title),
        urlencoding::encode(body),
    );
    (url.chars().count() <= MAX_COMPOSE_URL_CHARS).then_some(url)
}

/// How a report will be filed, decided before the user is asked to approve it —
/// so the card can say which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filer {
    /// `gh` is installed and already authenticated non-interactively. Never
    /// chosen for a chat treated as private (`choose_filer` in the parent
    /// module), because approving it publishes with no further look.
    GhCli,
    /// Open a prefilled compose page; the user presses Submit.
    ComposeUrl(String),
    /// Neither: the report is too large for a URL, and `gh` was either found
    /// unavailable or, from a chat treated as private, never considered.
    Manual {
        /// `gh` was probed and is not installed or not signed in. `false` from
        /// a private chat, where it is never probed: there, "not signed in"
        /// would be a claim nobody checked.
        gh_unavailable: bool,
    },
}

/// Why a [`Filer::Manual`] report is handed back rather than filed, as one
/// clause the card ([`Filer::describe`]) and the receipt share, so the two
/// cannot disagree about it. It names `gh` only when `gh` really was found
/// unavailable.
pub fn manual_reason(gh_unavailable: bool) -> &'static str {
    if gh_unavailable {
        "it is too large for a prefilled link and the GitHub CLI (`gh`) is not installed \
         or not signed in on this machine"
    } else {
        "it is too large for a prefilled link and is not filed automatically"
    }
}

impl Filer {
    /// One sentence for the approval card, so the user knows what the button
    /// does before pressing it.
    pub fn describe(&self, repo: &str) -> String {
        match self {
            Self::GhCli => format!(
                "This will CREATE a public issue on github.com/{repo} immediately, using \
                 your own signed-in GitHub CLI account."
            ),
            // "or …" because `open_in_browser` can fail (no browser on this
            // machine, a headless host); the link is handed back either way.
            Self::ComposeUrl(_) => format!(
                "Approving opens a prefilled new-issue page for github.com/{repo} in your \
                 browser, or gives you its link if no browser can be opened. Nothing is \
                 posted until you press Submit there."
            ),
            // ⚠ `gh` is named only when it was probed and failed: from a
            // private chat it is never probed, so "not signed in" would be
            // untrue there.
            Self::Manual { gh_unavailable } => format!(
                "The report will be handed back to you to paste into github.com/{repo} \
                 yourself, because {}. Nothing is posted.",
                manual_reason(*gh_unavailable)
            ),
        }
    }

    /// Does approving this actually publish?
    pub fn publishes_on_approval(&self) -> bool {
        matches!(self, Self::GhCli)
    }
}

/// Is `gh` present AND already authenticated, without prompting?
///
/// ⚠ `stdin` is `null` and prompting is disabled, because the failure mode this
/// guards against is not "gh is missing" but "gh is installed and would open an
/// interactive login". `github_workflow.rs`'s own helper does exactly that, and
/// from a tool call it would hang a turn until the TTL killed it, with the user
/// seeing nothing at all.
pub async fn gh_ready() -> bool {
    // ⚠ The same guard `file_with_gh` uses, for the same reason and one more.
    // A test binary must not shell out to `gh` at all: the probe is a network
    // round-trip, so without this the tests' behaviour depends on whether the
    // machine running them happens to have `gh` authenticated. That is why four
    // card tests passed on a developer's Mac and failed on windows-latest — the
    // runner's `gh` is installed but signed out, so the probe outlived the
    // tests' budget and the card was never reached.
    if running_under_test() {
        return false;
    }
    let mut probe = tokio::process::Command::new("gh");
    probe
        .args(["auth", "status", "--hostname", "github.com"])
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // `gh` is a console program and `biorouterd` is started DETACHED on Windows,
    // so it owns no console for a child to inherit and Windows gives this one a
    // brand-new, VISIBLE console unless it is asked not to (#368).
    biorouter_mcp::developer::shell::no_console_window(&mut probe);
    let Ok(result) = tokio::time::timeout(GH_PROBE_TIMEOUT, probe.status()).await else {
        return false;
    };
    result.is_ok_and(|status| status.success())
}

/// The result of actually filing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    /// The issue URL, when one is known. `ComposeUrl` returns the compose page,
    /// not an issue: nothing has been created yet.
    pub url: Option<String>,
    pub filer: Filer,
}

/// Is this process a test binary?
///
/// ⚠ `cfg!(test)` alone is NOT the answer, and the comment that used to say the
/// compiler enforced this was wrong in the direction that matters. `cfg(test)`
/// is set only while the crate under compilation is built with `--test`, so it
/// is true for this crate's unit tests and **false** inside every integration
/// test — `crates/biorouter/tests/*.rs` link the library compiled normally.
/// `bug_report_agent_loop` is exactly such a test, it drives a real
/// `Agent::reply` at this tool, and the only thing standing between it and a
/// live `gh issue create` was that it happens to DENY the approval card. The
/// privileged `DecisionAuthority` constructors it would need to approve one are
/// `pub`, so an integration test added tomorrow needs no unsafe, no new
/// dependency and no ill intent to file a real issue on the project's tracker.
///
/// The second half is a runtime check, and deliberately a structural one rather
/// than an env var a test would have to remember to set — a guard that depends
/// on being armed is exactly the guard the next test forgets. Cargo builds and
/// runs every test binary out of `<target>/<profile>/deps/`, unit and
/// integration alike, and no shipped binary lives there: the desktop app stages
/// its backends under `Contents/Resources/bin`, the deb/rpm install to
/// `/usr/bin`, and a dev run is `target/debug/biorouter`. Benchmarks are also
/// caught, which is correct — they must not file issues either.
///
/// Fails SAFE in both directions. A false negative is impossible for the case
/// that matters (a test binary is always under `deps/`), and a false positive
/// costs nothing worse than a fallback: [`super::post_report`] answers a
/// `file_with_gh` error with the prefilled compose URL, so the user still gets
/// their report.
fn running_under_test() -> bool {
    if cfg!(test) {
        return true;
    }
    std::env::current_exe().is_ok_and(|exe| {
        exe.parent()
            .and_then(Path::file_name)
            .is_some_and(|dir| dir == "deps")
    })
}

/// Open the prefilled compose page in the user's browser. `true` when a
/// browser accepted it.
///
/// ⚠ Without this the compose path only RETURNED the link, for the model to
/// relay — and the link is up to [`MAX_COMPOSE_URL_CHARS`] of percent-encoded
/// body, which is exactly the kind of string models truncate or "tidy". The
/// approval card already promised a page would open. The caller hands the link
/// back as well, because `false` is an ordinary outcome: a headless host, no
/// default browser, a daemon on another machine.
///
/// Runs on the blocking pool: `webbrowser::open` waits for `xdg-open` on Linux.
/// Never opens anything from a test binary, for the same reason
/// [`file_with_gh`] never posts: a test that approved a card would otherwise
/// pop a browser on whatever machine ran it.
pub async fn open_in_browser(url: &str) -> bool {
    if running_under_test() {
        return false;
    }
    let url = url.to_string();
    tokio::task::spawn_blocking(move || webbrowser::open(&url).is_ok())
        .await
        .unwrap_or(false)
}

/// Create the issue with the user's own `gh`.
///
/// The body goes through a file rather than an argument: an issue body is tens
/// of kilobytes, exceeds `ARG_MAX` on some platforms, and would be visible in
/// `ps` on all of them.
pub async fn file_with_gh(
    repo: &str,
    title: &str,
    body: &str,
    body_file: &Path,
) -> anyhow::Result<String> {
    // ⚠ A test that approved the card would create a real, public, permanent
    // issue on someone's tracker — from `cargo test`, on whatever machine
    // happened to have `gh` signed in. No test does today; this makes that a
    // property of the code rather than of everyone who ever adds one, and the
    // fallback path the caller takes on an error is exercised by the same
    // refusal.
    if running_under_test() {
        anyhow::bail!("refusing to create a GitHub issue from a test build; nothing was posted");
    }
    tokio::fs::write(body_file, body).await?;
    let mut create = tokio::process::Command::new("gh");
    create
        .args([
            "issue",
            "create",
            "--repo",
            repo,
            "--title",
            title,
            "--body-file",
            &body_file.to_string_lossy(),
            "--label",
            LABEL,
        ])
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null());
    biorouter_mcp::developer::shell::no_console_window(&mut create);
    let output = tokio::time::timeout(GH_TIMEOUT, create.output())
        .await
        .map_err(|_| anyhow::anyhow!("`gh issue create` did not finish within {GH_TIMEOUT:?}"))??;

    // Best effort: the file holds the report, not a credential, but it does not
    // need to outlive the call.
    let _ = tokio::fs::remove_file(body_file).await;

    if !output.status.success() {
        // stderr, scrubbed: `gh` quotes the repository path and sometimes the
        // user's own login, and this string goes back into the conversation.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let scrubbed = redact::scrub(stderr.trim(), dirs::home_dir().as_deref());
        anyhow::bail!(
            "`gh issue create` failed: {}",
            if scrubbed.text.is_empty() {
                "no output".to_string()
            } else {
                scrubbed.text
            }
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .rev()
        .find(|line| line.trim().starts_with("https://"))
        .map(|line| line.trim().to_string())
        .ok_or_else(|| anyhow::anyhow!("`gh issue create` reported success but printed no URL"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::bug_report::evidence::{Evidence, ToolFailure};
    use crate::agents::tool_errors::ToolErrorKind;

    fn evidence() -> Evidence {
        Evidence {
            session_id: "20260905_1".into(),
            failures: vec![ToolFailure {
                tool_name: Some("developer__shell".into()),
                kind: ToolErrorKind::NotFound,
                retryable: false,
                message: "No such file or directory (os error 2)".into(),
                occurrences: 2,
                arguments: Some("{\"command\":\"cargo build\"}".into()),
                looks_deliberate: false,
            }],
            total_failed_calls: 2,
            total_tool_calls: 7,
            recent_user_messages: vec!["build the crate".into()],
            externalized_results: 0,
            app_version: "1.90.0".into(),
            os: "macos".into(),
            os_version: "27.0.0".into(),
            architecture: "aarch64".into(),
            provider: Some("versa_azure".into()),
            model: Some("gpt-5.5".into()),
            enabled_extensions: vec!["developer".into(), "knowledge".into()],
            working_dir: "~/Projects/demo".into(),
        }
    }

    fn draft() -> Draft {
        Draft {
            title: "Shell tool reports not-found for a path that exists".into(),
            description: "Running `cargo build` fails with os error 2.".into(),
            steps: vec!["Open a chat".into(), "Ask it to build".into()],
            expected: "The build runs.".into(),
            suspected_cause: None,
            additional: None,
        }
    }

    /// The diagnosis gets its own section, between the observation and the
    /// environment, labelled as unconfirmed — and the body still passes the
    /// harness, because the section is optional rather than required.
    #[test]
    fn a_suspected_cause_is_rendered_between_expected_and_the_environment() {
        let draft = Draft {
            suspected_cause: Some(
                "`developer/shell.rs` resolves the command against the daemon's own \
                 working directory, not the chat's (`fn validate_shell_command(cwd: \
                 String)`). Confidence: medium."
                    .into(),
            ),
            ..draft()
        };
        let body = render_body(&draft, &evidence(), false);
        let expected = body.find("**Expected behavior**").unwrap();
        let cause = body
            .find("**Suspected cause**")
            .unwrap_or_else(|| panic!("no Suspected cause section: {body}"));
        let environment = body
            .find("**Please provide the following information**")
            .unwrap();
        assert!(expected < cause && cause < environment, "{body}");
        assert!(
            body.contains("the source at v1.90.0. Not yet confirmed by a maintainer."),
            "{body}"
        );
        assert!(body.contains("Confidence: medium."), "{body}");
        let violations = redact::validate_issue(&draft.title, &body, None);
        assert!(violations.is_empty(), "{violations:#?}\n---\n{body}");
    }

    /// No cause still gets the section, saying so in a fixed line, so a
    /// debugging agent can tell "not diagnosed" from "section dropped" — and a
    /// blank cause is no cause. The body still passes the harness.
    #[test]
    fn without_a_suspected_cause_the_section_says_none_was_supplied() {
        for cause in [None, Some("   \n".to_string())] {
            let draft = Draft {
                suspected_cause: cause,
                ..draft()
            };
            let body = render_body(&draft, &evidence(), false);
            let section = body
                .find("**Suspected cause**")
                .unwrap_or_else(|| panic!("no Suspected cause section: {body}"));
            let line = body
                .find(NO_SUSPECTED_CAUSE)
                .unwrap_or_else(|| panic!("no fallback line: {body}"));
            let environment = body
                .find("**Please provide the following information**")
                .unwrap();
            assert!(section < line && line < environment, "{body}");
            assert!(
                !body.contains("Not yet confirmed by a maintainer"),
                "no analysis to attribute: {body}"
            );
            let violations = redact::validate_issue(&draft.title, &body, None);
            assert!(violations.is_empty(), "{violations:#?}\n---\n{body}");
        }
    }

    /// ⚠ A private chat's body is the page the user reads on github.com before
    /// pressing Submit, and it must not invite them to attach the diagnostics
    /// bundle, which is that chat's transcript, unredacted. Nor may it say the
    /// chat was private: that is a fact about the user's work, on a public page.
    #[test]
    fn a_private_chat_s_body_does_not_invite_the_diagnostics_bundle() {
        let public = render_body(&draft(), &evidence(), false);
        assert!(
            public.contains(
                "A full diagnostics bundle (transcript, redacted config, logs) can be attached"
            ),
            "{public}"
        );

        let private = render_body(&draft(), &evidence(), true);
        for needle in ["diagnostics bundle", "Generate diagnostics", "private"] {
            assert!(
                !private.contains(needle),
                "`{needle}` in a private body: {private}"
            );
        }
        assert!(private.trim_end().ends_with("</sub>"), "{private}");
        let violations = redact::validate_issue(&draft().title, &private, None);
        assert!(violations.is_empty(), "{violations:#?}\n---\n{private}");
    }

    /// A test binary must never open a browser, for the same reason it must
    /// never run `gh issue create`.
    #[tokio::test]
    async fn the_compose_page_is_never_opened_from_a_test() {
        assert!(!open_in_browser("https://example.invalid/never-opened").await);
    }

    /// The rendered body satisfies the harness. This is the pairing that
    /// matters: a renderer and a validator that disagree produce a tool that
    /// refuses its own output.
    #[test]
    fn a_rendered_body_passes_the_validator() {
        let body = render_body(&draft(), &evidence(), false);
        let violations = redact::validate_issue(&draft().title, &body, None);
        assert!(violations.is_empty(), "{violations:#?}\n---\n{body}");
    }

    #[test]
    fn the_body_carries_the_environment_the_template_asks_for() {
        let body = render_body(&draft(), &evidence(), false);
        assert!(body.contains("**Version:** v1.90.0"), "{body}");
        assert!(body.contains("macos 27.0.0 (aarch64)"), "{body}");
        assert!(body.contains("versa_azure – gpt-5.5"), "{body}");
        assert!(body.contains("developer, knowledge"), "{body}");
    }

    #[test]
    fn the_failure_list_is_rendered_with_its_counts_and_arguments() {
        let body = render_body(&draft(), &evidence(), false);
        assert!(body.contains("2 of 7 calls"), "{body}");
        assert!(body.contains("`developer__shell` ×2"), "{body}");
        assert!(body.contains("cargo build"), "{body}");
    }

    /// A report with no steps still renders a `To Reproduce` section, because
    /// the validator requires one and a body that fails its own harness is a
    /// tool that can never file.
    #[test]
    fn a_report_with_no_steps_still_satisfies_the_template() {
        let draft = Draft {
            steps: Vec::new(),
            expected: String::new(),
            ..draft()
        };
        let body = render_body(&draft, &evidence(), false);
        assert!(redact::validate_issue(&draft.title, &body, None).is_empty());
        assert!(body.contains("**To Reproduce**"), "{body}");
        assert!(body.contains("Not stated by the reporter"), "{body}");
    }

    #[test]
    fn the_compose_url_is_prefilled_and_encoded() {
        let url = compose_url(DEFAULT_REPO, "A title with spaces", "**Body** & more")
            .expect("a short body fits");
        assert!(url.starts_with("https://github.com/BaranziniLab/biorouter/issues/new?"));
        assert!(url.contains("labels=bug"), "{url}");
        assert!(url.contains("A%20title%20with%20spaces"), "{url}");
        assert!(
            url.contains("%26%20more"),
            "the ampersand must not split the query: {url}"
        );
    }

    /// ⚠ The cap is on the ENCODED url. A body that looks comfortably small in
    /// characters can triple through percent-encoding, and a link that 414s is
    /// worse than no link: the user clicks it, sees an error page, and the
    /// report is gone.
    #[test]
    fn a_body_that_would_414_yields_no_link_rather_than_a_dead_one() {
        // Newlines encode to three characters each, so this is well under the
        // cap in characters and well over it encoded.
        let body = "\n".repeat(MAX_COMPOSE_URL_CHARS / 2);
        assert!(body.chars().count() < MAX_COMPOSE_URL_CHARS);
        assert!(compose_url(DEFAULT_REPO, "t", &body).is_none());
    }

    #[test]
    fn the_destination_defaults_to_the_project_and_can_be_pointed_elsewhere() {
        let _guard = env_lock::lock_env([(REPO_ENV, None::<&str>)]);
        assert_eq!(repo(), DEFAULT_REPO);
        drop(_guard);
        let _guard = env_lock::lock_env([(REPO_ENV, Some("acme/fork"))]);
        assert_eq!(repo(), "acme/fork");
    }

    /// The card has to say whether pressing the button publishes. A user who
    /// believes they are opening a draft page and actually creates a public
    /// issue has not consented to the thing that happened.
    #[test]
    fn each_filer_says_whether_approving_publishes() {
        assert!(Filer::GhCli.publishes_on_approval());
        assert!(Filer::GhCli.describe(DEFAULT_REPO).contains("CREATE"));
        let compose = Filer::ComposeUrl("https://example.invalid".into());
        assert!(!compose.publishes_on_approval());
        assert!(compose
            .describe(DEFAULT_REPO)
            .contains("Nothing is posted until you press Submit"));
        // True whether or not a browser could be opened.
        assert!(compose
            .describe(DEFAULT_REPO)
            .contains("gives you its link"));
        for gh_unavailable in [true, false] {
            let manual = Filer::Manual { gh_unavailable };
            assert!(!manual.publishes_on_approval());
            assert!(manual.describe(DEFAULT_REPO).contains("Nothing is posted"));
        }
    }

    /// The hand-back names `gh` only when `gh` was probed and failed. From a
    /// private chat it is never probed, and "not signed in" there would be a
    /// claim nobody checked (and is often false).
    #[test]
    fn the_hand_back_blames_gh_only_when_gh_was_found_unavailable() {
        let probed = Filer::Manual {
            gh_unavailable: true,
        }
        .describe(DEFAULT_REPO);
        assert!(probed.contains("GitHub CLI"), "{probed}");
        assert!(
            probed.contains("too large for a prefilled link"),
            "{probed}"
        );

        let never_probed = Filer::Manual {
            gh_unavailable: false,
        }
        .describe(DEFAULT_REPO);
        for needle in ["GitHub CLI", "`gh`", "signed in"] {
            assert!(!never_probed.contains(needle), "{never_probed}");
        }
        assert!(
            never_probed.contains("too large for a prefilled link and is not filed automatically"),
            "{never_probed}"
        );
    }
}
