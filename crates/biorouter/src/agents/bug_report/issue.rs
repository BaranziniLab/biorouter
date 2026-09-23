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
//!   user's browser. The report is complete and the user's click is the submit.
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
pub fn render_body(draft: &Draft, evidence: &Evidence) -> String {
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
         paths, usernames and credential-shaped strings are removed before posting. A \
         full diagnostics bundle (transcript, redacted config, logs) can be attached \
         from **Chat summary → Diagnostics → Generate diagnostics**.</sub>\n",
    );

    body
}

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
    /// `gh` is installed and already authenticated non-interactively.
    GhCli,
    /// Open a prefilled compose page; the user presses Submit.
    ComposeUrl(String),
    /// Neither: no `gh`, and the report is too large for a URL.
    Manual,
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
            Self::ComposeUrl(_) => format!(
                "This will open a prefilled new-issue page for github.com/{repo} in your \
                 browser. Nothing is posted until you press Submit there."
            ),
            Self::Manual => format!(
                "The report is too large for a prefilled link and `gh` is not signed in, \
                 so it will be handed back to you to paste into github.com/{repo} yourself. \
                 Nothing is posted."
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
            additional: None,
        }
    }

    /// The rendered body satisfies the harness. This is the pairing that
    /// matters: a renderer and a validator that disagree produce a tool that
    /// refuses its own output.
    #[test]
    fn a_rendered_body_passes_the_validator() {
        let body = render_body(&draft(), &evidence());
        let violations = redact::validate_issue(&draft().title, &body, None);
        assert!(violations.is_empty(), "{violations:#?}\n---\n{body}");
    }

    #[test]
    fn the_body_carries_the_environment_the_template_asks_for() {
        let body = render_body(&draft(), &evidence());
        assert!(body.contains("**Version:** v1.90.0"), "{body}");
        assert!(body.contains("macos 27.0.0 (aarch64)"), "{body}");
        assert!(body.contains("versa_azure – gpt-5.5"), "{body}");
        assert!(body.contains("developer, knowledge"), "{body}");
    }

    #[test]
    fn the_failure_list_is_rendered_with_its_counts_and_arguments() {
        let body = render_body(&draft(), &evidence());
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
        let body = render_body(&draft, &evidence());
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
        assert!(!Filer::Manual.publishes_on_approval());
    }
}
