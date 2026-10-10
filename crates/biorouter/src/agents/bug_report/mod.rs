//! `platform__report_bug` — the agent files a Biorouter bug for the user.
//!
//! ## The shape, and why it is two calls and not one
//!
//! "Report a bug" is not one action. The model has to find out what went
//! wrong before it can write about it, and the two halves want different
//! answers when they fail:
//!
//! * [`Action::Analyze`] reads the session's own failure record and hands back
//!   a digest. It **files nothing** and needs no approval. When it cannot tell
//!   what went wrong and the user has not said, it answers with a question and
//!   an instruction not to file — the push-back.
//! * [`Action::File`] takes the report the model wrote, scrubs it, checks it
//!   against the harness, parks a proof-backed approval showing the exact body,
//!   and only then posts.
//!
//! A single call would have to guess, and the guess it would make is the one
//! that files. The action is inferred when the model does not say — a call
//! carrying a title and a description means "file", anything else means
//! "analyze" — because a model that omits an enum should land on the half that
//! cannot publish.
//!
//! ## What the user is agreeing to
//!
//! The approval card carries the **rendered body, verbatim**, in its preview,
//! and its prompt names the destination repository and says whether pressing
//! the button publishes immediately or opens a page the user still has to
//! submit. Those two facts are the whole consent: a card that said "file a bug
//! report?" would be asking about a category, not about the paragraph that is
//! going to be world-readable.
//!
//! It is modelled on `install_extension` (`extension_manager_extension.rs`),
//! the one other tool here that gathers local state, blocks on a proof-backed
//! card and then takes an outward-facing action — including its ordering:
//! preflight, await approval, **re-check that nothing changed between the card
//! and the click**, then act.
//!
//! ## The privacy ruling
//!
//! A GitHub issue is world-readable and permanent. A session classified
//! `Private` has touched a private model or a private data source, and the
//! report is written *from* that session by a model reading it — so nothing
//! here can certify the distillation carries none of it. Whether it is
//! published anyway is the **user's** decision, and a private chat can report
//! a bug. What the agent may never do is publish from one by itself:
//!
//! * The approval card leads with a loud warning (its `prompt`, which the
//!   desktop draws as a warning banner) naming why the chat is private and
//!   what to read the body for, before the verbatim body.
//! * `gh` is never used from a private chat, and is not even probed: approving
//!   it publishes with no further look. Approval opens a prefilled compose page
//!   instead (or hands the text back when it is too long for one), so the
//!   user's own Submit on GitHub is the disclosure.
//! * A classification that could not be read is treated as private: warned,
//!   and never auto-published. It is not refused — a store read that failed is
//!   no reason to stop a user reporting a bug.
//!
//! ⚠ This used to REFUSE from a private chat and hand the finished report
//! back. That made a private chat the one place a user could not report a bug
//! from, and protected nothing the rule above does not: the property that
//! matters is that a person, with the exact text in front of them, takes the
//! step that makes it public.
//!
//! ⚠ It honours the DR-15 master switch, like every other gate: with the switch
//! off a `Private` chat is treated as public here (a `gh` filing, no warning).
//! A switch that some gates ignore is not a switch, and the card names the
//! session's classification either way, so the fact is in front of the user
//! regardless. The switch is read once, in [`handle_report_bug`], and threaded.

pub mod evidence;
pub mod issue;
pub mod redact;

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{Content, ErrorCode, ErrorData};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::conversation::Conversation;
use crate::mcp_utils::ToolResult;
use crate::pending_user_action::{
    PendingUserActions, ToolApprovalRequest, UserActionOutcome, UserActionRequest,
};
use crate::permission::tool_risk::ToolRisk;
use crate::privacy::SessionClassification;
use crate::session::session_manager::Session;
use crate::session::SessionManager;

use evidence::Evidence;
use issue::{Draft, Filer};

pub const REPORT_BUG_TOOL_NAME: &str = "platform__report_bug";

/// What the dispatch arm in `Agent::dispatch_tool_call` answers a model that
/// names this tool in a process that cannot obtain a person's proof. Written
/// for the MODEL; the `/bug` slash command says the same thing to the person
/// in its own words (`execute_commands::bug_command_unavailable`).
pub const NO_APPROVER_REFUSAL: &str = "Filing a bug report needs a person to approve the \
     exact text before it is published, and this Biorouter cannot ask one — it is running \
     without a way to prove a human acted (a `biorouter serve` daemon, for instance). \
     Nothing was filed and nothing was analysed. Tell the user to report it from the \
     desktop app, or at https://github.com/BaranziniLab/biorouter/issues/new.";

/// How long the approval card stands. Long enough for a user to read a whole
/// issue body — which is the point of showing it — and to fetch a colleague.
const APPROVAL_TTL: Duration = Duration::from_secs(15 * 60);

/// Which half of the tool was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Analyze,
    File,
}

impl Action {
    /// ⚠ Inference lands on `Analyze`, always. A model that omits the argument
    /// must not thereby publish something.
    ///
    /// Matching is case- and whitespace-insensitive, and accepts the British
    /// spelling. That is not politeness: the previous exact match sent `"File"`
    /// to the analyze half, which answers "now call me with `action: file`" —
    /// so a model that capitalises loops forever, being told to do the thing it
    /// just tried. The safety property is unchanged, because it is about the
    /// UNRECOGNISED case: absent, misspelled or nonsense still lands on the
    /// half that cannot publish.
    fn infer(arguments: &Value) -> Self {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase());
        match action.as_deref() {
            Some("file") => Self::File,
            Some("analyze" | "analyse") => Self::Analyze,
            Some(_) => Self::Analyze,
            None => {
                let has = |key: &str| {
                    arguments
                        .get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty())
                };
                if has("title") && has("description") {
                    Self::File
                } else {
                    Self::Analyze
                }
            }
        }
    }
}

/// Undo the two shapes models reliably produce that a strict schema reader
/// would reject.
///
/// ⚠ Both are measured behaviours in this tree, not defensive guessing.
/// `autovisualiser::normalize_dashboard_args` exists for exactly the same two:
/// GPT-5.5 wraps a whole argument object in a `data` envelope and retries
/// identically after a rejection, and nested structures arrive stringified.
/// This tool is reached from every provider Biorouter supports — Anthropic,
/// OpenAI, Versa, Bedrock, Ollama, llama.cpp, and a Claude Code or Codex child
/// over the bridge — so it normalises rather than assuming one house style.
fn normalize_arguments(mut value: Value) -> Value {
    // A JSON object that arrived as a string.
    if let Value::String(raw) = &value {
        if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
            value = parsed;
        }
    }
    // An envelope around the real arguments.
    if let Value::Object(map) = &value {
        let names_a_field = ["action", "title", "description"]
            .iter()
            .any(|key| map.contains_key(*key));
        if !names_a_field {
            if let Some(inner) = ["arguments", "report", "issue", "bug", "data", "params"]
                .iter()
                .find_map(|key| map.get(*key))
            {
                let unwrapped = match inner {
                    Value::String(raw) => {
                        serde_json::from_str::<Value>(raw).unwrap_or_else(|_| inner.clone())
                    }
                    other => other.clone(),
                };
                if unwrapped.is_object() {
                    return unwrapped;
                }
            }
        }
    }
    value
}

fn invalid_params(message: impl std::fmt::Display) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, message.to_string(), None)
}

fn text(body: String) -> ToolResult<Vec<Content>> {
    Ok(vec![Content::text(body)])
}

fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `steps` may arrive as a list or as one newline-separated string; models
/// produce both and neither is wrong.
fn steps_arg(arguments: &Value) -> Vec<String> {
    // A stringified array — the shape `de_stringified` exists for elsewhere in
    // this tree. Parsed first, so it is treated as the list it is rather than
    // split into lines of JSON punctuation.
    let parsed = arguments
        .get("steps")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_array);
    match parsed.as_ref().or_else(|| arguments.get("steps")) {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(|step| step.trim().to_string())
            .filter(|step| !step.is_empty())
            .collect(),
        Some(Value::String(value)) => value
            .lines()
            .map(|line| line.trim().trim_start_matches(['-', '*', '•']).trim())
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Is this chat treated as private for filing: warned on the card, never
/// published by approval alone?
///
/// `tier` is the STORED classification, `None` when the store could not be
/// read. An unknown classification counts as private — the safe reading, and
/// one that costs the user nothing but a warning and a page they submit
/// themselves.
///
/// ⚠ Pure, taking the master switch as an ARGUMENT rather than reading it. The
/// switch is a process-global atomic, and a test that flipped it to exercise
/// the second arm broke six unrelated privacy tests running concurrently in the
/// same binary — a failure that reads as a privacy hole in code nobody touched.
/// The same shape as `CallCapability`: sample once, thread the value, and the
/// decision becomes testable without a global to fight over.
fn treat_as_private(tier: Option<SessionClassification>, tiers_enabled: bool) -> bool {
    tier.is_none() || (tiers_enabled && tier == Some(SessionClassification::Private))
}

/// How the report will reach GitHub, decided before the card so the card can
/// say which.
///
/// ⚠ A private chat NEVER gets [`Filer::GhCli`], however ready `gh` is:
/// approving it creates the public issue on the spot, and the premise that
/// makes filing from a private chat acceptable is that the user still presses
/// Submit on GitHub themselves. Pure, because `issue::gh_ready()` answers
/// `false` in every test binary — this rule can only be pinned here.
///
/// `Manual`'s `gh_unavailable` is `!private`: past the first arm, a chat that
/// is not private had `gh` probed and found not ready, and a private one never
/// probed it, so the hand-back may blame `gh` only in the first case.
fn choose_filer(private: bool, gh_ready: bool, compose: Option<String>) -> Filer {
    if gh_ready && !private {
        return Filer::GhCli;
    }
    match compose {
        Some(url) => Filer::ComposeUrl(url),
        None => Filer::Manual {
            gh_unavailable: !private,
        },
    }
}

/// The chat's classification as the store holds it NOW, with the reason the
/// store recorded for it (`Session::privacy_reason`), so the card can say WHY
/// a chat is private rather than only that it is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredClassification {
    tier: SessionClassification,
    reason: Option<String>,
}

/// What the filing half knows about the chat's privacy, decided once in
/// [`handle_report_bug`].
#[derive(Debug, Clone)]
struct ChatPrivacy {
    /// `None` when the store could not be read.
    stored: Option<StoredClassification>,
    /// [`treat_as_private`]'s answer for it, with the master switch sampled
    /// once.
    private: bool,
}

/// The conversation to read the failures out of.
///
/// ⚠ **Read from the store, not from `session.conversation`**, even though the
/// row handed to `dispatch_tool_call` usually carries one. That copy is the
/// snapshot `RewriteBasis::read_with_session` took at the TOP of this turn, so
/// it is missing the current user's own message ("report a bug") and every tool
/// call this turn has already made — which, for a report raised the moment
/// something failed, is precisely the evidence being asked about. Preferring it
/// to save a query is the shape of a bug that would only appear in the case the
/// tool exists for.
///
/// The store read is a superset, so the turn-start snapshot is kept only as a
/// fallback for a read that fails outright: a degraded report beats none.
async fn conversation_for(
    session: &Session,
    session_manager: &SessionManager,
) -> ToolResult<(Conversation, Option<StoredClassification>)> {
    match session_manager.get_session(&session.id, true).await {
        Ok(loaded) => {
            // The classification as it is NOW. `session.privacy_tier` is a
            // snapshot taken when the agent loop handed this call its
            // `Session`, and the classification is a RATCHET that a tool call
            // can raise MID-turn — reading a private knowledge base is enough.
            // `max` because the ratchet only ever rises, so a store answering
            // lower is answering about an earlier moment. The reason is taken
            // from whichever side supplied the tier, from this same read.
            let stored = if loaded.privacy_tier >= session.privacy_tier {
                StoredClassification {
                    tier: loaded.privacy_tier,
                    reason: loaded
                        .privacy_reason
                        .clone()
                        .or_else(|| session.privacy_reason.clone()),
                }
            } else {
                StoredClassification {
                    tier: session.privacy_tier,
                    reason: session.privacy_reason.clone(),
                }
            };
            Ok((
                loaded
                    .conversation
                    .or_else(|| session.conversation.clone())
                    .unwrap_or_default(),
                Some(stored),
            ))
        }
        // A read that fails still yields a conversation, because a degraded
        // report beats no report — but it yields NO classification, and the
        // filing half then treats the chat as private (`treat_as_private`):
        // warned, and never published by approval alone. A degraded privacy
        // reading must not be the one that publishes.
        Err(error) => session
            .conversation
            .clone()
            .map(|conversation| (conversation, None))
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("could not read this chat's history: {error}"),
                    None,
                )
            }),
    }
}

/// The digest handed back by [`Action::Analyze`], and spliced into the file
/// path's receipt so the two can never describe the same session differently.
fn render_digest(evidence: &Evidence, scrubbed_working_dir: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Biorouter v{} on {} {} ({}), provider {} / model {}, working directory {}.\n",
        evidence.app_version,
        evidence.os,
        evidence.os_version,
        evidence.architecture,
        evidence.provider.as_deref().unwrap_or("not set"),
        evidence.model.as_deref().unwrap_or("not set"),
        scrubbed_working_dir,
    ));
    out.push_str(&format!(
        "Tool calls in this chat: {} total, {} failed.\n",
        evidence.total_tool_calls, evidence.total_failed_calls
    ));

    if evidence.failures.is_empty() {
        out.push_str("\nNo failed tool calls are recorded in this chat.\n");
        return out;
    }

    out.push_str("\nFailures, most repeated first:\n");
    for failure in &evidence.failures {
        out.push_str(&failure.to_line());
        out.push('\n');
        if let Some(arguments) = &failure.arguments {
            out.push_str(&format!("    arguments: {arguments}\n"));
        }
    }
    if evidence.failures.iter().any(|f| f.looks_deliberate) {
        out.push_str(
            "\nAt least one of those is Biorouter refusing ON PURPOSE — a privacy or \
             permission boundary doing its job. Do not file that as a defect unless the \
             user says the WRONG thing was refused.\n",
        );
    }
    out
}

/// How to tell a defect from intended behaviour before writing the report:
/// the version-pinned source, the documentation, and the outcome when the two
/// say Biorouter behaved as designed.
///
/// ⚠ Here, in the analyze RESULT, and not in the system prompt or the tool
/// description: it is read only when a report is being written, so it costs no
/// tokens on any other turn, and only here is the user's version known — the
/// docs site deploys from `main`, so it can describe behaviour newer than the
/// install. The system prompt carries the bare repository and documentation
/// addresses once; this is the method.
///
/// The source links name [`issue::DEFAULT_REPO`], not `issue::repo()`: the
/// latter is where a report is SENT (a fork, a scratch repository), and the
/// code the user runs is Biorouter's.
///
/// `private` adds the egress rule. Nothing gates a public URL fetch from a
/// private chat, so the one real leak is the model building a URL, search or
/// command out of the conversation itself.
fn investigation_guidance(app_version: &str, private: bool) -> String {
    let repo = issue::DEFAULT_REPO;
    let mut out = format!(
        "Before you write the report, find out whether this is a defect:\n\
         1. Compare what the user expected with what happened, from this chat's own record.\n\
         2. Check whether the behaviour is documented and intended. The documentation is \
         https://biorouter.ucsf.edu/docs (one large page; its sources are \
         `docs/website/pages/<page>.html` in the repository, and contributor documentation \
         is under `docs/`).\n\
         3. Read the code involved at the version the user runs: \
         https://github.com/{repo}/tree/v{app_version} (raw files: \
         https://raw.githubusercontent.com/{repo}/v{app_version}/<path>). If that tag does \
         not exist, use `main`; if `main` differs, the bug may already be fixed, so say so. \
         If the working directory is a Biorouter checkout, read the code there instead. The \
         `develop-biorouter` skill maps where code lives.\n\
         4. Use whatever web-fetch or shell tool you have. If you have none, say in \
         `suspected_cause` that the source and the documentation were not checked.\n\
         5. If the documentation and the code show Biorouter behaved as designed, explain the \
         intended use to the user instead of filing, and file only if they still want to (a \
         misleading document or error message is itself worth reporting).\n"
    );
    if private {
        out.push_str(
            "\nThis chat is private. Fetch only fixed public pages on github.com, \
             raw.githubusercontent.com and biorouter.ucsf.edu; never put text from this \
             conversation into a URL, query string, search or command that reaches the \
             network. Write the report about Biorouter's behaviour, not the user's data: do \
             not quote patient data, query results, file contents or anything else from their \
             work.\n",
        );
    }
    out
}

/// Read the session and report what is there — or push back.
///
/// `private` is [`treat_as_private`]'s answer; it adds the private-chat egress
/// rule to the investigation guidance.
async fn analyze(
    evidence: &Evidence,
    user_description: Option<&str>,
    scrubbed_working_dir: &str,
    private: bool,
) -> ToolResult<Vec<Content>> {
    let digest = render_digest(evidence, scrubbed_working_dir);

    // ⚠ The push-back. Nothing here is an error: an error invites a retry, and
    // a retry cannot produce information the model does not have. It has to go
    // and ask.
    if user_description.is_none() && !evidence.is_conclusive() {
        // The user's own recent prose, quoted back. The heuristic above already
        // rejected it as a problem statement, but it is the model that can read
        // it in context — and the failure being guarded against is a loop where
        // the tool asks for something the user has already said.
        let their_words = if evidence.recent_user_messages.is_empty() {
            String::new()
        } else {
            format!(
                "What the user has said in this chat, most recent last:\n{}\n\n",
                evidence
                    .recent_user_messages
                    .iter()
                    .map(|message| format!("  > {message}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        // ⚠ Both next steps go back through `analyze`, never straight to
        // `file`: a description is what this branch lacks, and the call that
        // supplies one gets the investigation guidance below — the source, the
        // documentation, the suspected cause — which a direct `file` skips.
        return text(format!(
            "I could not tell what went wrong from this chat on its own, so nothing has \
             been filed and nothing will be until you say what to report.\n\n\
             {digest}\n{their_words}\
             If those messages ALREADY describe the problem, do not ask again: call this \
             tool again with `action: \"analyze\"` and the user's own words in \
             `description`, and it will say how to investigate before you write the \
             report.\n\n\
             Otherwise ASK THE USER what they want to report, and be specific: name what \
             you can see above and ask whether that is the problem, or whether it is \
             something else (something looked wrong on screen, an answer was incorrect, \
             the app was slow, a control did nothing). Then call this tool again with \
             `action: \"analyze\"` and their answer, in their own words, in \
             `description`.\n\n\
             Do NOT invent a report from the failures above."
        ));
    }

    let lead = match (user_description, evidence.headline()) {
        (Some(description), _) => format!("The user is reporting: {description}\n\n"),
        (None, Some(headline)) => {
            format!("The clearest failure in this chat is: {headline}\n\n")
        }
        (None, None) => String::new(),
    };
    let guidance = investigation_guidance(&evidence.app_version, private);

    text(format!(
        "{lead}{digest}\n{guidance}\n\
         Then write the report and call this tool again with `action: \"file\"`. Give a \
         `title` a maintainer can recognise in a list; a `description` of what was \
         observed, facts only: what happened and why it is wrong; `steps` to reproduce it \
         if you can honestly state them; `expected`; and a `suspected_cause` with your \
         diagnosis, grounded in what the transcript, the code and the documentation show. \
         Keep observation and diagnosis apart. Always give `suspected_cause`: if nothing \
         grounds a diagnosis, say so there (what you checked, what you could not check, \
         for example no web-fetch or shell tool, confidence low) rather than guess.\n\n\
         The environment, the version and the failure list above are added automatically; \
         do not repeat them. Home paths, usernames and anything credential-shaped are \
         removed before posting, and the user must approve the exact text before it goes \
         anywhere."
    ))
}

/// Why a private chat is private, as a clause completing "This chat …", or
/// `None` when the recorded reason is one the vocabulary does not name.
///
/// `turn:*` → "ran on a private model"; every other named reason (`mcp:`,
/// `inherited:`, `diverged:`, `backfill:`, `imported`) takes
/// `declassify::strong_confirmation_reason`'s wording, the one every surface
/// that explains a private chat shares.
///
/// ⚠ That function's catch-all (an absent or unrecognised reason) is a
/// statement about the RECORD, written for the declassify control ("does not
/// record an observed turn on a private model as the reason it is private").
/// Completing "This chat is PRIVATE: it …" with it is nonsense, so it is
/// detected by comparing with the answer for an absent reason, which is the
/// catch-all by construction, rather than by restating its text here.
fn private_reason_clause(reason: Option<&str>) -> Option<&'static str> {
    use crate::privacy::declassify::strong_confirmation_reason;
    match strong_confirmation_reason(reason) {
        None => Some("ran on a private model"),
        Some(clause) if Some(clause) == strong_confirmation_reason(None) => None,
        Some(clause) => Some(clause),
    }
}

/// The sentence that leads a private chat's approval card.
///
/// Prepended to the card's `prompt`, which the desktop draws as a warning
/// banner (and which hides "Always Allow"), so it is loud with no change to any
/// surface. `stored` is `None` when the classification could not be read.
///
/// The reason clause comes from [`private_reason_clause`]; a chat whose
/// recorded reason the vocabulary does not name gets the bare statement.
fn private_warning(stored: Option<&StoredClassification>) -> String {
    let lead = match stored {
        Some(stored) => match private_reason_clause(stored.reason.as_deref()) {
            Some(clause) => format!("⚠ This chat is PRIVATE: it {clause}."),
            None => "⚠ This chat is classified PRIVATE.".to_string(),
        },
        None => "⚠ Biorouter could not confirm this chat's privacy classification, so it is \
                 treated as PRIVATE."
            .to_string(),
    };
    format!(
        "{lead} The report below was written from it and becomes public if you submit it. \
         Before you approve, read it for patient or participant data, credentials, \
         unpublished results or institutional information; the failure list quotes raw tool \
         output. From a private chat nothing is posted automatically: approving only opens a \
         prefilled GitHub page (its address carries the text to github.com, but nothing is \
         published) or hands the text back, and you decide there whether to submit."
    )
}

/// The preview's own ceiling, in characters of pretty-printed JSON.
///
/// ⚠ NOT [`ToolPreview::for_tool_call`]'s shared 4,000, and the difference is
/// the whole approval. That constant governs every tool's card, where the
/// arguments are context for a judgement about the CALL; here the argument IS
/// the artefact, and `redact`'s module doc states the dependency plainly — "the
/// person is the last check and the design assumes it". A card that showed the
/// first 4,000 characters of a longer body would collect consent for text
/// nobody was shown, and it would say so only in a one-line truncation note.
///
/// Doubling [`redact::MAX_ISSUE_BODY_CHARS`] is the pre-image of a body that
/// has ALREADY passed `validate_issue`: pretty JSON escapes every newline and
/// quote to two characters and indents, so a 60,000-character body renders to
/// well under 120,000. The frame stays bounded, by this tool's own validation
/// rather than by a cap shared with tools that have none.
const APPROVAL_PREVIEW_CHARS: usize = redact::MAX_ISSUE_BODY_CHARS * 2;

/// Build the approval card.
///
/// The body rides in `arguments` rather than in `preview`'s structured shape:
/// `ToolPreview::Arguments` renders the arguments as pretty JSON, which is
/// exactly the right frame for "here is the text that will be published", and
/// it is the one variant every surface already knows how to draw.
///
/// `chatPrivacyTier` is the stored classification as a fact — `"unknown"` when
/// the store could not be read — whatever the master switch says; the warning
/// in `prompt` follows [`ChatPrivacy::private`], which honours it.
fn approval_request(
    title: &str,
    body: &str,
    repo: &str,
    filer: &Filer,
    privacy: &ChatPrivacy,
    quoted_from_transcript: bool,
) -> UserActionRequest {
    let arguments = serde_json::json!({
        "repository": format!("github.com/{repo}"),
        "title": title,
        "body": body,
        "labels": [issue::LABEL],
        "chatPrivacyTier": match privacy.stored.as_ref().map(|stored| stored.tier) {
            Some(SessionClassification::Private) => "private",
            Some(SessionClassification::Public) => "public",
            None => "unknown",
        },
    })
    .as_object()
    .expect("the approval arguments are an object")
    .clone();

    // The model omitted `description`, so "Describe the bug" is the user's own
    // last message quoted back at them (`evidence::described_problem`). Say so:
    // reading one's own sentence under a heading one did not write is exactly
    // the case where a reader skims past text they assume the model composed.
    let quoted = if quoted_from_transcript {
        " ⚠ \"Describe the bug\" is YOUR last message, quoted verbatim — the model did \
         not write it."
    } else {
        ""
    };

    let redirected = if repo == issue::DEFAULT_REPO {
        String::new()
    } else {
        format!(
            " ⚠ This is NOT the Biorouter project's own tracker (github.com/{}); it has \
             been redirected to github.com/{repo}.",
            issue::DEFAULT_REPO
        )
    };

    // First, so it is the first thing read: the rest of the card is the same
    // for every chat, and this is the one fact that is not.
    let warning = if privacy.private {
        format!("{} ", private_warning(privacy.stored.as_ref()))
    } else {
        String::new()
    };

    UserActionRequest::ToolApproval(ToolApprovalRequest {
        tool_name: REPORT_BUG_TOOL_NAME.to_string(),
        preview: Some(
            crate::conversation::tool_preview::ToolPreview::arguments_within(
                &arguments,
                APPROVAL_PREVIEW_CHARS,
            ),
        ),
        arguments,
        prompt: Some(format!(
            "{warning}Publish this bug report to the public issue tracker? {}{redirected}{quoted} \
             Read the body below: everything in it becomes world-readable and permanent.",
            filer.describe(repo)
        )),
        // High, not Medium: the act is irreversible and public. `install_extension`
        // is graded the same way for a change that is at least undoable.
        risk: Some(ToolRisk::High),
        requires_user_proof: true,
    })
}

/// The "Describe the bug" text, and where it came from.
///
/// The provenance is not bookkeeping: a description the MODEL wrote is prose
/// composed for a public tracker, and one lifted from the transcript is the
/// user's own sentence, published verbatim under a heading they did not choose.
/// Both are legitimate — see [`evidence::described_problem`] for the measured
/// reason the fallback exists — but only one of them needs the approval card to
/// say so before it becomes permanent.
struct ProblemDescription {
    text: String,
    /// Quoted from the user's own last message rather than written by the model.
    from_transcript: bool,
}

/// Everything the report is written FROM, gathered once at the top of the call.
///
/// A parameter object rather than eight positional arguments, and the reason is
/// not tidiness: these five travelled together through `handle_report_bug` →
/// `file_report` → `prepare_report`, in an order no reader could check, and
/// `evidence` and `scrubbed_working_dir` are two views of the same thing — the
/// second is the first's `working_dir`, already through the scrubber. Passing
/// them separately is how a caller comes to scrub one and not the other.
struct ReportInputs<'a> {
    arguments: &'a Value,
    evidence: Evidence,
    /// See [`ProblemDescription`]: `None` means neither the model nor the
    /// transcript gave one, which is a question rather than a failure.
    user_description: Option<ProblemDescription>,
    /// `evidence.working_dir`, scrubbed. Held separately because `analyze` needs
    /// it before `evidence` is consumed.
    scrubbed_working_dir: &'a str,
    home: Option<&'a std::path::Path>,
}

/// The whole tool.
///
/// `session_manager` is a parameter rather than reached through an `Agent`, so
/// every branch below is exercisable without one.
pub async fn handle_report_bug(
    arguments: Value,
    session: &Session,
    session_manager: Arc<SessionManager>,
    cancel: Option<CancellationToken>,
) -> ToolResult<Vec<Content>> {
    let arguments = normalize_arguments(arguments);
    let home = dirs::home_dir();
    let home = home.as_deref();
    let (conversation, stored) = conversation_for(session, session_manager.as_ref()).await?;
    // The master switch, read ONCE and threaded: both halves act on it (the
    // analyze guidance and the card), and two reads are two instants.
    let privacy = ChatPrivacy {
        private: treat_as_private(
            stored.as_ref().map(|stored| stored.tier),
            crate::privacy::privacy_tiers_enabled(),
        ),
        stored,
    };
    let evidence = evidence::collect(session, &conversation);
    let scrubbed_working_dir = redact::scrub(&evidence.working_dir, home).text;
    // ⚠ The transcript is the third source, and it is not a nicety. Measured
    // live: asked *"Report a bug to BioRouter: the Auto Visualiser renders a
    // blank panel for a single-row dataset"*, gpt-5.5 reached for this tool
    // correctly and called it with `{"action": "analyze"}` and nothing else --
    // so the tool asked the user to describe a problem they had described one
    // message earlier. A model omitting an optional argument is the ordinary
    // case; reading the user's own last message is how the tool stops depending
    // on it.
    let user_description = string_arg(&arguments, "description")
        .or_else(|| string_arg(&arguments, "user_description"))
        .map(|text| ProblemDescription {
            text,
            from_transcript: false,
        })
        .or_else(|| {
            evidence::described_problem(&evidence.recent_user_messages).map(|text| {
                ProblemDescription {
                    text,
                    from_transcript: true,
                }
            })
        });

    if Action::infer(&arguments) == Action::Analyze {
        return analyze(
            &evidence,
            user_description.as_ref().map(|d| d.text.as_str()),
            &scrubbed_working_dir,
            privacy.private,
        )
        .await;
    }
    file_report(
        ReportInputs {
            arguments: &arguments,
            evidence,
            user_description,
            scrubbed_working_dir: &scrubbed_working_dir,
            home,
        },
        session,
        privacy,
        cancel,
    )
    .await
}

/// Scrub the model's prose, render the body, and run the harness over it.
///
/// Returns the draft, the rendered body and the title's own scrub result — the
/// last so the receipt can say what was removed without re-running anything.
///
/// Split out of [`file_report`] to stay under the `too_many_lines` baseline,
/// and because everything here is pure: no approval, no network, no clock.
///
/// `private` is [`ChatPrivacy::private`]; it reaches [`issue::render_body`],
/// which leaves a private chat's body without the diagnostics-bundle invitation.
fn prepare_report(
    inputs: ReportInputs<'_>,
    private: bool,
) -> Result<(Draft, String, redact::Scrubbed), ErrorData> {
    let ReportInputs {
        arguments,
        evidence,
        user_description,
        scrubbed_working_dir,
        home,
    } = inputs;
    let title = string_arg(arguments, "title").ok_or_else(|| {
        invalid_params(
            "`title` is required to file. Call this tool with `action: \"analyze\"` first \
             if you do not yet know what the report should say.",
        )
    })?;
    let description = user_description.map(|d| d.text).ok_or_else(|| {
        invalid_params(
            "`description` is required to file: it becomes the report's \"Describe the \
             bug\" section. Ask the user what went wrong rather than inventing one.",
        )
    })?;

    // ⚠ Scrub the model's own prose FIRST. It writes the report from the
    // transcript, so it quotes the error it is reporting -- and a real error
    // message is where a home path, a bearer token or a signed URL lives.
    let title_scrub = redact::scrub(&title, home);
    let draft = Draft {
        title: title_scrub.text.clone(),
        description: redact::scrub(&description, home).text,
        steps: steps_arg(arguments)
            .iter()
            .map(|step| redact::scrub(step, home).text)
            .collect(),
        expected: string_arg(arguments, "expected")
            .map(|expected| redact::scrub(&expected, home).text)
            .unwrap_or_default(),
        // Scrubbed like the rest of the prose, and it needs it more: a
        // diagnosis quotes code and paths, which is where `redact`'s
        // type-annotation and placeholder exemptions matter.
        suspected_cause: string_arg(arguments, "suspected_cause")
            .map(|cause| redact::scrub(&cause, home).text),
        additional: string_arg(arguments, "additional")
            .map(|additional| redact::scrub(&additional, home).text),
    };

    // The environment block is assembled from the evidence, whose working
    // directory is a real path; scrub the evidence too, not only the prose.
    let evidence = Evidence {
        working_dir: scrubbed_working_dir.to_string(),
        failures: evidence
            .failures
            .iter()
            .map(|failure| evidence::ToolFailure {
                message: redact::scrub(&failure.message, home).text,
                arguments: failure
                    .arguments
                    .as_ref()
                    .map(|args| redact::scrub(args, home).text),
                ..failure.clone()
            })
            .collect(),
        ..evidence
    };

    let body = issue::render_body(&draft, &evidence, private);

    // ⚠ The harness. It re-runs the scrub and refuses what it still finds,
    // rather than trusting the passes above to have been complete. A refusal
    // here is returned to the model with the reasons, so it can fix and retry —
    // this is the one failure in the tool that a retry can actually resolve.
    let mut violations = redact::validate_issue(&draft.title, &body, home);
    // A refusal the model can fix by shortening, like every other rule here.
    // See `MAX_SUSPECTED_CAUSE_CHARS` for why a long cause is not merely long.
    if let Some(length) = draft
        .suspected_cause
        .as_deref()
        .map(|cause| cause.trim().chars().count())
        .filter(|length| *length > redact::MAX_SUSPECTED_CAUSE_CHARS)
    {
        violations.push(redact::Violation {
            rule: "size",
            detail: format!(
                "the suspected cause is {length} characters; keep it under {} (about 100 to \
                 300 words: the likely code, the hypothesis, the evidence, the confidence)",
                redact::MAX_SUSPECTED_CAUSE_CHARS
            ),
        });
    }
    if !violations.is_empty() {
        return Err(invalid_params(format!(
            "The report was NOT filed: it did not pass Biorouter's own checks.\n{}\n\n\
             Fix these and call the tool again. If the problem is that identifying \
             material survived redaction, rewrite the offending text rather than \
             quoting it.",
            violations
                .iter()
                .map(|violation| format!("  - {violation}"))
                .collect::<Vec<_>>()
                .join("\n")
        )));
    }

    Ok((draft, body, title_scrub))
}

/// The `file` half: write the report, check it, ask, post.
///
/// Split from [`handle_report_bug`] so that neither half is over the
/// `too_many_lines` baseline, and because the boundary is a real one — nothing
/// below here runs for an `analyze` call.
async fn file_report(
    inputs: ReportInputs<'_>,
    session: &Session,
    privacy: ChatPrivacy,
    cancel: Option<CancellationToken>,
) -> ToolResult<Vec<Content>> {
    // Sampled BEFORE `prepare_report` consumes it, because the card is the only
    // place this distinction can still be acted on.
    let quoted_from_transcript = inputs
        .user_description
        .as_ref()
        .is_some_and(|description| description.from_transcript);
    let (draft, body, title_scrub) = prepare_report(inputs, privacy.private)?;
    let repo = issue::repo();

    // ⚠ A private chat does not even PROBE `gh`: the probe is a network round
    // trip on the path to the card, and its answer could not be used.
    let gh_ready = !privacy.private && issue::gh_ready().await;
    let filer = choose_filer(
        privacy.private,
        gh_ready,
        issue::compose_url(&repo, &draft.title, &body),
    );

    if session.id.is_empty() {
        return Err(invalid_params(
            "Filing a bug report needs a visible chat, because the user has to approve \
             the exact text before it is published.",
        ));
    }

    let parked = PendingUserActions::global().park(
        Some(&session.id),
        None,
        approval_request(
            &draft.title,
            &body,
            &repo,
            &filer,
            &privacy,
            quoted_from_transcript,
        ),
    );
    match parked.wait(APPROVAL_TTL, cancel.as_ref()).await {
        UserActionOutcome::Approved { .. } => {}
        outcome => {
            return text(format!(
                "Nothing was filed: the approval {}. The report is written and ready — \
                 say the word and I will ask again.",
                outcome.refusal_detail()
            ));
        }
    }

    // `install_extension`'s re-check, for the same reason: approval binds the
    // exact thing shown, not merely the intention. A destination that changed
    // between the card and the click gets a new card.
    if issue::repo() != repo {
        return Err(invalid_params(format!(
            "The destination repository changed after you approved (it was \
             github.com/{repo}). Nothing was filed."
        )));
    }
    if cancel.as_ref().is_some_and(CancellationToken::is_cancelled) {
        return text(
            "The turn was cancelled after you approved, so nothing was filed.".to_string(),
        );
    }

    post_report(filer, &repo, &draft, &body, &title_scrub, privacy.private).await
}

/// The compose path's receipt, after [`issue::open_in_browser`] answered.
///
/// ⚠ The link is ALWAYS in it, and the model is told to pass it on exactly as
/// written: an opened page can still be closed or land behind another window,
/// and the link is hundreds of percent-encoded characters a model will
/// otherwise "tidy" into a dead one.
fn compose_url_receipt(url: &str, opened: bool) -> String {
    if opened {
        format!(
            "Opened a prefilled new-issue page in the user's browser. Nothing is posted until \
             they press Submit there. Tell them so, and give them this link exactly as \
             written in case the page did not appear:\n\n{url}"
        )
    } else {
        format!(
            "Nothing is posted yet, and no browser could be opened from here. Give the user \
             this link exactly as written; they open it and press Submit to file the \
             report:\n\n{url}"
        )
    }
}

/// Everything after the approval: do the thing, and say what happened.
///
/// Every arm returns an Ok result rather than an error, including the ones that
/// could not post. The user approved this exact text; handing it back to be
/// pasted is a worse outcome than an issue URL and a much better one than an
/// error that loses the report.
///
/// `private` changes one sentence, and it is the one that matters: the
/// diagnostics bundle is the conversation itself, unredacted, and from a
/// private chat it does not belong on a public issue at all. ⚠ The private
/// sentence is an instruction to pass on NOW, on every arm including the
/// hand-back, not "if they ask": the user is about to be on a GitHub page with
/// an attach button, and a question they never thought to ask is no guard.
async fn post_report(
    filer: Filer,
    repo: &str,
    draft: &Draft,
    body: &str,
    title_scrub: &redact::Scrubbed,
    private: bool,
) -> ToolResult<Vec<Content>> {
    let attach = if private {
        "\n\nTell the user this now, in your reply: do NOT attach the diagnostics bundle \
         (Chat summary → Diagnostics → Generate diagnostics) to this issue. It contains this \
         private conversation unredacted, and the issue is public."
    } else {
        "\n\nTo attach the full diagnostics bundle — transcript, redacted config, logs — \
         open Chat summary → Diagnostics → Generate diagnostics and drag the zip onto the \
         issue. Read it first: it contains the whole conversation."
    };

    match filer {
        Filer::GhCli => {
            let body_file = std::env::temp_dir()
                .join(format!("biorouter-bug-report-{}.md", uuid::Uuid::new_v4()));
            match issue::file_with_gh(repo, &draft.title, body, &body_file).await {
                Ok(url) => text(format!(
                    "Filed: {url}\n\nTitle: {}\nRedacted before posting: {}.{attach}",
                    draft.title,
                    title_scrub.summary()
                )),
                Err(error) => {
                    // Falling back rather than failing: the user approved this
                    // exact text, and a `gh` that broke is not a reason to make
                    // them start over. Opening the page is a lesser act than
                    // the one they approved — it posts nothing.
                    let fallback = issue::compose_url(repo, &draft.title, body);
                    text(match fallback {
                        Some(url) => {
                            let opened = issue::open_in_browser(&url).await;
                            format!(
                                "The GitHub CLI could not create the issue ({error}), so \
                                 nothing was posted. {}{attach}",
                                compose_url_receipt(&url, opened)
                            )
                        }
                        None => format!(
                            "The GitHub CLI could not create the issue ({error}), so nothing \
                             was posted, and the report is too large for a prefilled link. \
                             Open https://github.com/{repo}/issues/new and paste this:\n\n\
                             ----- title -----\n{}\n\n----- body -----\n{body}",
                            draft.title
                        ),
                    })
                }
            }
        }
        // The card promised a page in the browser; this is where it opens.
        Filer::ComposeUrl(url) => {
            let opened = issue::open_in_browser(&url).await;
            text(format!(
                "{}\n\nRedacted before posting: {}.{attach}",
                compose_url_receipt(&url, opened),
                title_scrub.summary()
            ))
        }
        // A private chat's usual fallback when the body is too long for a link,
        // so it carries the private warning too. The public sentence is left
        // off here: it tells the user how to attach the bundle to an issue that
        // does not exist yet, and the body they are about to paste already
        // says where the bundle is.
        Filer::Manual { gh_unavailable } => text(manual_receipt(
            repo,
            &draft.title,
            body,
            gh_unavailable,
            if private { attach } else { "" },
        )),
    }
}

/// The hand-back's receipt: the whole report, to paste.
///
/// Its reason is [`issue::manual_reason`], the clause the card used, so it
/// names `gh` only when `gh` was probed and failed — never from a private chat,
/// where it was not probed and may well be signed in.
///
/// `note` (empty, or starting with a blank line) goes BEFORE the text to paste:
/// after it, nothing marks where the body ends, and a note there reads as the
/// last paragraph of the report.
fn manual_receipt(repo: &str, title: &str, body: &str, gh_unavailable: bool, note: &str) -> String {
    format!(
        "The report is ready, but nothing was posted because {}.{note}\n\nOpen \
         https://github.com/{repo}/issues/new and paste this:\n\n----- title -----\n{title}\n\n\
         ----- body -----\n{body}",
        issue::manual_reason(gh_unavailable)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::{Message, MessageContent, ToolRequest, ToolResponse};
    use crate::pending_user_action::{DecisionAuthority, ResolveOutcome};
    use crate::permission::Permission;
    use crate::session::session_manager::SessionType;
    use rmcp::model::{CallToolRequestParams, CallToolResult, Content};
    use std::borrow::Cow;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn args(value: serde_json::Value) -> Value {
        value
    }

    /// A session with `n` hard failures already in its transcript, over an
    /// isolated store. The `TempDir` is returned because dropping it deletes the
    /// SQLite file the manager still holds.
    async fn session_with_failures(count: usize) -> (TempDir, Arc<SessionManager>, Session) {
        let dir = TempDir::new().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let mut session = manager
            .create_session(
                PathBuf::from("/workspace/demo"),
                "bug-report".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        let mut content = Vec::new();
        for index in 0..count {
            let id = format!("call-{index}");
            content.push(MessageContent::ToolRequest(ToolRequest {
                id: id.clone(),
                tool_call: Ok(CallToolRequestParams {
                    task: None,
                    name: Cow::from("developer__shell"),
                    arguments: serde_json::json!({ "command": format!("build {index}") })
                        .as_object()
                        .cloned(),
                    meta: None,
                }),
                metadata: None,
                tool_meta: None,
            }));
            content.push(MessageContent::ToolResponse(ToolResponse {
                id,
                tool_result: Ok(CallToolResult {
                    content: vec![Content::text(format!(
                        "the panel rendered blank (failure {index})"
                    ))],
                    structured_content: None,
                    is_error: Some(true),
                    meta: None,
                }),
                metadata: None,
            }));
        }
        // ⚠ PERSISTED, not merely attached to the in-memory row. The handler
        // reads the store, because the row `dispatch_tool_call` is handed
        // carries a snapshot from the top of the turn and is missing everything
        // the turn has done since. A fixture that only set `session.conversation`
        // would exercise a path production never takes — and it did, until this
        // comment's change caught it.
        let message = content
            .into_iter()
            .fold(Message::assistant(), Message::with_content);
        manager.add_message(&session.id, &message).await.unwrap();
        session.conversation = Some(Conversation::new_unvalidated(vec![message]));
        (dir, manager, session)
    }

    fn body_of(result: &ToolResult<Vec<Content>>) -> String {
        result
            .as_ref()
            .expect("an Ok result")
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn file_args() -> Value {
        args(serde_json::json!({
            "action": "file",
            "title": "Chart panel renders blank for a single-row dataset",
            "description": "The Auto Visualiser panel is empty when the dataset has one row.",
            "steps": ["Open a chat", "Ask for a chart of one row"],
            "expected": "The chart renders."
        }))
    }

    /// ⚠ The failure the report is about must be FOUND, even when the row the
    /// agent hands over does not know about it.
    ///
    /// `dispatch_tool_call` receives the `Session` that
    /// `RewriteBasis::read_with_session` snapshotted at the TOP of the turn, so
    /// its `conversation` is missing the current user's message and every tool
    /// call this turn has already made. Reading it "because it is already in
    /// hand" saves a query and loses exactly the evidence a report raised the
    /// moment something failed is about — a bug that appears only in the case
    /// the tool exists for, and in no test that builds its fixture by hand.
    ///
    /// This fixture reproduces the divergence deliberately: the store holds a
    /// hard failure, and the in-memory row holds an EMPTY conversation, which is
    /// what a turn-start snapshot looks like on the first turn.
    #[tokio::test]
    async fn the_store_is_read_rather_than_the_turn_start_snapshot() {
        let (_dir, manager, mut session) = session_with_failures(2).await;
        session.conversation = Some(Conversation::default());

        let result = handle_report_bug(
            args(serde_json::json!({"action": "analyze"})),
            &session,
            manager,
            None,
        )
        .await;
        let body = body_of(&result);
        assert!(
            body.contains("2 total, 2 failed"),
            "the turn-start snapshot was read instead of the store, so the failure \
             being reported is invisible: {body}"
        );
        assert!(!body.contains("ASK THE USER"), "{body}");
    }

    /// ⚠ The push-back. An empty session plus no user description must produce
    /// a QUESTION, not an issue — and not an error either, because an error
    /// invites a retry and a retry cannot produce information the model does
    /// not have.
    #[tokio::test]
    async fn an_inconclusive_session_with_no_description_asks_instead_of_filing() {
        let (_dir, manager, session) = session_with_failures(0).await;
        let result = handle_report_bug(args(serde_json::json!({})), &session, manager, None).await;
        let body = body_of(&result);
        assert!(body.contains("nothing has been filed"), "{body}");
        assert!(body.contains("ASK THE USER"), "{body}");
        assert!(
            body.contains("Do NOT invent a report from the failures above"),
            "{body}"
        );
    }

    /// ⚠ Measured live, then pinned. gpt-5.5 called this tool with
    /// `{"action": "analyze"}` and nothing else while the user's description
    /// sat one message above, and the tool asked them to describe a problem
    /// they had just described.
    #[tokio::test]
    async fn the_users_own_message_stands_in_for_the_argument_the_model_omitted() {
        let (_dir, manager, session) = session_with_failures(0).await;
        manager
            .add_message(
                &session.id,
                &Message::user().with_text(
                    "Report a bug to BioRouter: when I ask for a chart of a one-row \
                     dataset the artifact panel renders completely blank.",
                ),
            )
            .await
            .unwrap();

        let result = handle_report_bug(
            args(serde_json::json!({"action": "analyze"})),
            &session,
            manager,
            None,
        )
        .await;
        let body = body_of(&result);
        assert!(
            body.contains("The user is reporting"),
            "the user described the problem; asking again is a loop: {body}"
        );
        assert!(
            body.contains("the artifact panel renders completely blank"),
            "{body}"
        );
        assert!(!body.contains("ASK THE USER"), "{body}");
    }

    /// A bare "report a bug" still gets the question — and now sees the user's
    /// own words quoted back, so a case the heuristic misses is recoverable by
    /// the model's own judgement rather than lost.
    #[tokio::test]
    async fn a_bare_request_still_asks_but_shows_the_model_what_was_said() {
        let (_dir, manager, session) = session_with_failures(0).await;
        manager
            .add_message(&session.id, &Message::user().with_text("report a bug"))
            .await
            .unwrap();

        let result = handle_report_bug(
            args(serde_json::json!({"action": "analyze"})),
            &session,
            manager,
            None,
        )
        .await;
        let body = body_of(&result);
        assert!(body.contains("ASK THE USER"), "{body}");
        assert!(
            body.contains("> report a bug"),
            "the user's own words must be quoted back: {body}"
        );
        assert!(
            body.contains("If those messages ALREADY describe"),
            "{body}"
        );
    }

    /// The same empty session, but the user said what is wrong. There is
    /// nothing left to ask, so the tool gets on with it.
    #[tokio::test]
    async fn a_user_supplied_description_removes_the_need_to_push_back() {
        let (_dir, manager, session) = session_with_failures(0).await;
        let result = handle_report_bug(
            args(serde_json::json!({
                "action": "analyze",
                "description": "the window will not resize below 1200px"
            })),
            &session,
            manager,
            None,
        )
        .await;
        let body = body_of(&result);
        assert!(body.contains("The user is reporting"), "{body}");
        assert!(!body.contains("ASK THE USER"), "{body}");
    }

    /// Conclusive evidence stands in for a description.
    #[tokio::test]
    async fn a_conclusive_session_is_reported_without_pushing_back() {
        let (_dir, manager, session) = session_with_failures(3).await;
        let result = handle_report_bug(
            args(serde_json::json!({"action": "analyze"})),
            &session,
            manager,
            None,
        )
        .await;
        let body = body_of(&result);
        assert!(
            body.contains("The clearest failure in this chat is"),
            "{body}"
        );
        assert!(body.contains("`developer__shell`"), "{body}");
        assert!(!body.contains("ASK THE USER"), "{body}");
    }

    fn evidence_for(app_version: &str) -> Evidence {
        Evidence {
            session_id: "s".into(),
            failures: Vec::new(),
            total_failed_calls: 0,
            total_tool_calls: 0,
            recent_user_messages: Vec::new(),
            externalized_results: 0,
            app_version: app_version.into(),
            os: "macos".into(),
            os_version: "27.0".into(),
            architecture: "aarch64".into(),
            provider: None,
            model: None,
            enabled_extensions: Vec::new(),
            working_dir: "~/demo".into(),
        }
    }

    /// ⚠ The conclusive branch sends the model to investigate before it
    /// writes: the documentation, the source AT THE USER'S VERSION (the docs
    /// site deploys from `main`, so it can describe a newer build), and the
    /// "behaved as designed" outcome — and it asks for a suspected cause kept
    /// apart from the observation, instead of forbidding one.
    #[tokio::test]
    async fn the_analysis_says_how_to_check_the_docs_and_the_source_at_the_user_s_version() {
        let body = body_of(
            &analyze(
                &evidence_for("1.92.1"),
                Some("the chart panel is blank for one row"),
                "~/demo",
                false,
            )
            .await,
        );
        for needle in [
            "https://biorouter.ucsf.edu/docs",
            "docs/website/pages/<page>.html",
            "https://github.com/BaranziniLab/biorouter/tree/v1.92.1",
            "https://raw.githubusercontent.com/BaranziniLab/biorouter/v1.92.1/<path>",
            "use `main`",
            "`develop-biorouter` skill",
            "were not checked",
            "behaved as designed",
            "`suspected_cause`",
            "Always give `suspected_cause`",
            "say in `suspected_cause` that the source and the documentation were not checked",
            "rather than guess",
        ] {
            assert!(body.contains(needle), "missing `{needle}`: {body}");
        }
        assert!(
            !body.contains("do not pad the report with guesses about the cause"),
            "the old rule forbade the diagnosis the report now asks for: {body}"
        );
        assert!(!body.contains("ASK THE USER"), "{body}");
        assert!(
            !body.contains("This chat is private"),
            "a public chat gets no egress rule: {body}"
        );
    }

    /// A private chat may read the public docs and source, but never send the
    /// conversation anywhere to do it, and writes about Biorouter rather than
    /// about the user's data.
    #[tokio::test]
    async fn a_private_chat_s_analysis_adds_the_egress_rule() {
        let body = body_of(
            &analyze(
                &evidence_for("1.92.1"),
                Some("the chart panel is blank for one row"),
                "~/demo",
                true,
            )
            .await,
        );
        for needle in [
            "This chat is private",
            "Fetch only fixed public pages",
            "never put text from this conversation into a URL",
            "do not quote patient data",
        ] {
            assert!(body.contains(needle), "missing `{needle}`: {body}");
        }
    }

    /// ⚠ The push-back routes the model back through `analyze`, never straight
    /// to `file`: the call that supplies a description is the one that gets the
    /// investigation guidance, and a direct `file` would skip it.
    #[tokio::test]
    async fn the_push_back_sends_the_model_back_through_analysis() {
        let (_dir, manager, session) = session_with_failures(0).await;
        let body = body_of(
            &handle_report_bug(
                args(serde_json::json!({"action": "analyze"})),
                &session,
                manager,
                None,
            )
            .await,
        );
        assert!(body.contains("ASK THE USER"), "{body}");
        assert!(
            body.contains("call this tool again with `action: \"analyze\"`"),
            "{body}"
        );
        assert!(
            !body.contains("`action: \"file\"`"),
            "the push-back must not skip the investigation: {body}"
        );
    }

    /// The diagnosis reaches the card in its own section, scrubbed like the
    /// rest of the prose.
    #[tokio::test]
    async fn a_suspected_cause_reaches_the_card_scrubbed() {
        let (_dir, manager, session) = session_with_failures(2).await;
        let session_id = session.id.clone();
        let home = dirs::home_dir()
            .map(|home| home.display().to_string())
            .filter(|home| home.len() > 1)
            .unwrap_or_else(|| "/Users/jsmith".to_string());
        let mut arguments = file_args();
        arguments["suspected_cause"] = Value::String(format!(
            "`render_single_row` in {home}/biorouter/crates/biorouter-mcp/src/autovisualiser/\
             tools_charts.rs takes `fn render(api_key: String)` and divides by `rows - 1`. \
             Confidence: medium."
        ));

        let running = tokio::spawn({
            let manager = Arc::clone(&manager);
            let session = session.clone();
            async move { handle_report_bug(arguments, &session, manager, None).await }
        });
        let (id, arguments, _prompt, running) = raised_card(&session_id, running).await;
        let body = arguments
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        assert!(body.contains("**Suspected cause**"), "{body}");
        assert!(body.contains("divides by `rows - 1`"), "{body}");
        assert!(
            body.contains("fn render(api_key: String)"),
            "a type annotation is code, not a credential: {body}"
        );
        assert!(
            !body.contains(&home),
            "the cause is scrubbed like the rest of the prose: {body}"
        );

        PendingUserActions::global().resolve_in_session(
            &session_id,
            &id,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            DecisionAuthority::for_test_proven(),
        );
        let _ = running.await.unwrap();
    }

    /// A cause long enough to push an ordinary report off the prefilled page
    /// is refused for the model to shorten, like every other harness rule.
    #[tokio::test]
    async fn an_over_long_suspected_cause_is_refused_with_its_reason() {
        let (_dir, manager, session) = session_with_failures(1).await;
        let mut arguments = file_args();
        arguments["suspected_cause"] = Value::String(
            "The renderer divides by the row count minus one. "
                .repeat(redact::MAX_SUSPECTED_CAUSE_CHARS / 40),
        );
        let error = handle_report_bug(arguments, &session, manager, None)
            .await
            .expect_err("an over-long cause must not reach a card");
        assert!(error.message.contains("was NOT filed"), "{error:?}");
        assert!(error.message.contains("suspected cause"), "{error:?}");
    }

    /// ⚠ Nothing is filed without an approval, and a refusal is reported
    /// honestly rather than as a success.
    ///
    /// `without_human_surface` is the production statement of "there is nobody
    /// to answer this": `park` registers nothing and the handle answers
    /// `Cancelled` at once, which is the same outcome a dismissal produces.
    #[tokio::test]
    async fn a_refused_approval_files_nothing_and_says_so() {
        let (_dir, manager, session) = session_with_failures(2).await;
        let result = crate::user_surface::without_human_surface(handle_report_bug(
            file_args(),
            &session,
            manager,
            None,
        ))
        .await;
        let body = body_of(&result);
        assert!(body.contains("Nothing was filed"), "{body}");
        assert!(
            body.contains("was cancelled before anyone answered it"),
            "{body}"
        );
    }

    /// The card carries the exact body, names the destination, and demands
    /// proof of a person.
    ///
    /// ⚠ All three are the consent. A card that said "file a bug report?" would
    /// be asking about a category; the user is agreeing to a specific paragraph
    /// becoming world-readable at a specific address.
    /// ⚠ These tests probe the real `gh` on the machine they run on, so the
    /// filer they pick differs between a developer's laptop and CI. Every
    /// assertion below is therefore branch-INDEPENDENT; do not add one that
    /// reads `Filer::GhCli`'s wording without pinning the branch first. Nothing
    /// here can post: `issue::file_with_gh` refuses outright under `cfg!(test)`,
    /// and this test denies the card in any case.
    #[tokio::test]
    async fn the_approval_card_shows_the_exact_body_and_the_destination() {
        let (_dir, manager, session) = session_with_failures(2).await;
        let session_id = session.id.clone();

        let running = tokio::spawn({
            let manager = Arc::clone(&manager);
            let session = session.clone();
            async move { handle_report_bug(file_args(), &session, manager, None).await }
        });

        let registry = PendingUserActions::global();
        let card = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let pending: Vec<_> = registry
                    .pending_cards_for_session(&session_id)
                    .into_iter()
                    .collect();
                if let Some(message) = pending.into_iter().next() {
                    if let Some(MessageContent::ActionRequired(action)) =
                        message.content.into_iter().next()
                    {
                        if let crate::conversation::message::ActionRequiredData::ToolConfirmation {
                            id,
                            tool_name,
                            arguments,
                            prompt,
                            ..
                        } = action.data
                        {
                            return (id, (tool_name, arguments, prompt));
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        // ⚠ A bare `Elapsed` says only that no card arrived, which is the same
        // observation whether the handler is slow or returned early — and this
        // test failed three times on windows-latest saying exactly that, while
        // passing everywhere else. So on a timeout, ASK THE HANDLER what it did
        // rather than reporting the silence. Its result names the branch it took
        // (a store read that failed, an invalid-params exit, a harness refusal),
        // which is the thing a remote runner otherwise cannot tell you.
        let (id, request) = match card {
            Ok(found) => found,
            Err(_) => {
                let outcome = tokio::time::timeout(Duration::from_secs(5), running).await;
                panic!(
                    "the file half must raise an approval card; no card arrived within 10s. \n\
                     The handler's own outcome was: {outcome:#?}\n\
                     (an Ok(..) here means it RETURNED instead of parking — read its text; \n\
                     a timeout here means it is genuinely still running.)"
                );
            }
        };

        let (tool_name, arguments, prompt) = request;
        assert_eq!(tool_name, REPORT_BUG_TOOL_NAME);
        assert!(
            registry.requires_user_proof_in_session(&session_id, &id),
            "publishing must not be approvable through daemon HTTP by the model"
        );

        let card_body = arguments
            .get("body")
            .and_then(Value::as_str)
            .expect("the card carries the body verbatim");
        assert!(card_body.contains("**Describe the bug**"), "{card_body}");
        assert!(
            card_body.contains("The Auto Visualiser panel is empty"),
            "{card_body}"
        );
        assert_eq!(
            arguments.get("repository").and_then(Value::as_str),
            Some(format!("github.com/{}", issue::repo()).as_str())
        );
        let prompt = prompt.unwrap_or_default();
        assert!(prompt.contains("public issue tracker"), "{prompt}");
        assert!(
            prompt.contains("world-readable and permanent"),
            "the card must say what publishing means: {prompt}"
        );
        // A public chat's card carries no private warning: a warning on every
        // card is a warning nobody reads.
        assert_eq!(
            arguments.get("chatPrivacyTier").and_then(Value::as_str),
            Some("public")
        );
        assert!(!prompt.contains("PRIVATE"), "{prompt}");

        // Deny it, and the tool must file nothing.
        assert_eq!(
            registry.resolve_in_session(
                &session_id,
                &id,
                UserActionOutcome::Denied {
                    permission: Permission::DenyOnce
                },
                DecisionAuthority::for_test_proven(),
            ),
            ResolveOutcome::Delivered
        );
        let body = body_of(&running.await.unwrap());
        assert!(body.contains("was refused by the user"), "{body}");
        assert!(!body.contains("Filed:"), "{body}");
    }

    /// The file half's approval card, or a panic that says WHY none came.
    ///
    /// ⚠ Every test here spawns `handle_report_bug` and then polls the pending
    /// registry, never looking at the task — so an early return (a store read
    /// that failed, an invalid-params exit, a harness refusal) is
    /// indistinguishable from silence, and a remote runner reports
    /// `Elapsed(())` for both. That ambiguity cost three windows-latest runs
    /// and two wrong diagnoses on
    /// `the_approval_card_shows_the_exact_body_and_the_destination`. On a
    /// timeout this asks the handler what it actually did.
    ///
    /// ⚠ That test keeps its own inlined copy of this for now, deliberately:
    /// it is being iterated against live Windows evidence, and converging the
    /// two while that is in flight would fight an edit rather than help it.
    /// Fold it in once the Windows run is green.
    ///
    /// Hands the join handle back, so the caller can still resolve the card and
    /// read the tool's own result.
    async fn approval_card(
        session_id: &str,
        running: tokio::task::JoinHandle<ToolResult<Vec<Content>>>,
    ) -> (
        crate::conversation::message::ActionRequiredData,
        tokio::task::JoinHandle<ToolResult<Vec<Content>>>,
    ) {
        let registry = PendingUserActions::global();
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(message) = registry
                    .pending_cards_for_session(session_id)
                    .into_iter()
                    .next()
                {
                    if let Some(MessageContent::ActionRequired(action)) =
                        message.content.into_iter().next()
                    {
                        return action.data;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;

        match found {
            Ok(data) => (data, running),
            Err(_) => {
                let outcome = tokio::time::timeout(Duration::from_secs(5), running).await;
                panic!(
                    "the file half must raise an approval card; none arrived within 10s.\n\
                     The handler's own outcome was: {outcome:#?}\n\
                     (an Ok(..) here means it RETURNED instead of parking — read its text; \
                     a timeout here means it is genuinely still running.)"
                );
            }
        }
    }

    /// The card must show the WHOLE body, not the first four thousand
    /// characters of it.
    ///
    /// `ToolPreview::for_tool_call` clips at `MAX_ARGS_CHARS`, which is right
    /// for a tool whose arguments are context for a judgement about the call.
    /// Here the argument IS the artefact: `redact`'s own module doc says "the
    /// person is the last check and the design assumes it", and a preview that
    /// stopped at 4,000 characters would collect consent for text nobody was
    /// shown — announced only by a one-line truncation note.
    #[tokio::test]
    async fn a_long_body_reaches_the_card_whole() {
        let (_dir, manager, session) = session_with_failures(2).await;
        let session_id = session.id.clone();

        // Comfortably past the shared 4,000-character cap, and made of prose so
        // nothing in it trips the redaction harness.
        let long_context = "The panel stays blank and the console is quiet. ".repeat(150);
        let marker = "ONLY-AT-THE-VERY-END-OF-THE-BODY";
        let mut arguments = file_args();
        arguments["additional"] = Value::String(format!("{long_context}{marker}"));

        let running = tokio::spawn({
            let manager = Arc::clone(&manager);
            let session = session.clone();
            async move { handle_report_bug(arguments, &session, manager, None).await }
        });

        let (card, running) = approval_card(&session_id, running).await;
        let crate::conversation::message::ActionRequiredData::ToolConfirmation {
            id,
            preview,
            arguments,
            ..
        } = card
        else {
            panic!("the parked card must be a tool confirmation: {card:?}");
        };
        let body = arguments
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        assert!(
            body.chars().count() > 4_000,
            "the fixture must exceed the shared preview cap or it proves nothing: {} chars",
            body.chars().count()
        );
        let Some(crate::conversation::tool_preview::ToolPreview::Arguments { json, truncated }) =
            preview
        else {
            panic!("the card must carry an Arguments preview");
        };
        assert!(
            !truncated,
            "a bug report's card must not be clipped: the user is asked to read the exact \
             text that becomes world-readable and permanent"
        );
        assert!(
            json.contains(marker),
            "the END of the body never reached the card, so it was approved unseen"
        );

        let registry = PendingUserActions::global();
        registry.resolve_in_session(
            &session_id,
            &id,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            DecisionAuthority::for_test_proven(),
        );
        let _ = running.await.unwrap();
    }

    /// When the model omits `description`, the "Describe the bug" section is
    /// the user's own last message, published verbatim under a heading they did
    /// not write. Legitimate — see `evidence::described_problem` for the
    /// measured reason the fallback exists — but the card has to SAY so, because
    /// a reader skims past text they assume the model composed.
    #[tokio::test]
    async fn a_card_says_when_the_description_is_the_users_own_words() {
        let (_dir, manager, session) = session_with_failures(2).await;
        let session_id = session.id.clone();
        manager
            .add_message(
                &session_id,
                &Message::user().with_text(
                    "Report a bug: the Auto Visualiser renders a blank panel for a \
                     single-row dataset",
                ),
            )
            .await
            .unwrap();

        // No `description`: the tool has to reach for the transcript.
        let arguments = args(serde_json::json!({
            "action": "file",
            "title": "Chart panel renders blank for a single-row dataset",
            "steps": ["Open a chat", "Ask for a chart of one row"],
            "expected": "The chart renders."
        }));

        let running = tokio::spawn({
            let manager = Arc::clone(&manager);
            let session = session.clone();
            async move { handle_report_bug(arguments, &session, manager, None).await }
        });

        let (card, running) = approval_card(&session_id, running).await;
        let crate::conversation::message::ActionRequiredData::ToolConfirmation {
            id, prompt, ..
        } = card
        else {
            panic!("the parked card must be a tool confirmation: {card:?}");
        };
        let prompt = prompt.unwrap_or_default();

        assert!(
            prompt.contains("YOUR last message"),
            "the card must say the description is quoted from the user: {prompt}"
        );

        let registry = PendingUserActions::global();
        registry.resolve_in_session(
            &session_id,
            &id,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            DecisionAuthority::for_test_proven(),
        );
        let _ = running.await.unwrap();
    }

    /// The mirror: a description the MODEL wrote must NOT be announced as the
    /// user's own words. A note that appears on every card is a note nobody
    /// reads.
    #[tokio::test]
    async fn a_model_written_description_is_not_announced_as_the_users_own() {
        let (_dir, manager, session) = session_with_failures(2).await;
        let session_id = session.id.clone();

        let running = tokio::spawn({
            let manager = Arc::clone(&manager);
            let session = session.clone();
            async move { handle_report_bug(file_args(), &session, manager, None).await }
        });

        let (card, running) = approval_card(&session_id, running).await;
        let crate::conversation::message::ActionRequiredData::ToolConfirmation {
            id, prompt, ..
        } = card
        else {
            panic!("the parked card must be a tool confirmation: {card:?}");
        };
        let prompt = prompt.unwrap_or_default();

        assert!(
            !prompt.contains("YOUR last message"),
            "the model wrote this description: {prompt}"
        );

        let registry = PendingUserActions::global();
        registry.resolve_in_session(
            &session_id,
            &id,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            DecisionAuthority::for_test_proven(),
        );
        let _ = running.await.unwrap();
    }

    /// ⚠ The private-chat rule, tested as the pure predicate it is.
    ///
    /// It used to be tested by flipping the process-global master switch, which
    /// broke six unrelated privacy tests in the same binary — a race that reads
    /// as a privacy hole in code nobody had touched. Nothing here mutates any
    /// global, so nothing can.
    ///
    /// ⚠ The DR-15 master switch turns the private treatment off with every
    /// other gate: a switch some gates ignore is not a switch, and with it off
    /// a session marked `Private` was marked by machinery the user disabled.
    /// An UNKNOWN classification is private either way — the switch is about
    /// the classification machinery, and here there is no classification.
    #[test]
    fn who_counts_as_private_is_the_stored_tier_the_switch_and_nothing_else() {
        use SessionClassification::{Private, Public};
        assert!(treat_as_private(Some(Private), true));
        assert!(!treat_as_private(Some(Public), true));
        assert!(
            !treat_as_private(Some(Private), false),
            "the master switch turns this off with every other gate"
        );
        assert!(!treat_as_private(Some(Public), false));
        assert!(
            treat_as_private(None, true) && treat_as_private(None, false),
            "a classification that could not be read is treated as private"
        );
    }

    /// ⚠ The rule that makes filing from a private chat acceptable: the user
    /// still presses Submit on GitHub themselves. `gh` creates the issue the
    /// moment the card is approved, so a private chat must never get it — and
    /// `issue::gh_ready()` is `false` in every test binary, so this is the only
    /// place the rule can be pinned.
    #[test]
    fn a_private_chat_never_gets_the_filer_that_publishes_on_approval() {
        let url = || Some("https://github.com/x/y/issues/new?body=z".to_string());
        for gh_ready in [true, false] {
            for compose in [url(), None] {
                let filer = choose_filer(true, gh_ready, compose.clone());
                assert!(
                    !filer.publishes_on_approval(),
                    "private, gh_ready={gh_ready}, compose={compose:?} chose {filer:?}"
                );
                assert_eq!(
                    filer,
                    match compose {
                        Some(url) => Filer::ComposeUrl(url),
                        // `gh` was never probed, so the hand-back must not
                        // blame it.
                        None => Filer::Manual {
                            gh_unavailable: false,
                        },
                    }
                );
            }
        }
        // A public chat keeps the behaviour it always had.
        assert_eq!(choose_filer(false, true, url()), Filer::GhCli);
        assert_eq!(choose_filer(false, true, None), Filer::GhCli);
        assert_eq!(
            choose_filer(false, false, url()),
            Filer::ComposeUrl(url().unwrap())
        );
        assert_eq!(
            choose_filer(false, false, None),
            Filer::Manual {
                gh_unavailable: true
            }
        );
    }

    /// The hand-back receipt carries the whole report and, like the card,
    /// names `gh` only when it was probed and failed.
    #[test]
    fn the_hand_back_receipt_blames_gh_only_when_it_was_probed() {
        let receipt = |gh_unavailable| {
            manual_receipt(
                issue::DEFAULT_REPO,
                "Chart panel renders blank",
                "**Describe the bug**\n\nblank",
                gh_unavailable,
                "",
            )
        };
        for gh_unavailable in [true, false] {
            let text = receipt(gh_unavailable);
            for needle in [
                "nothing was posted",
                "too large for a prefilled link",
                "https://github.com/BaranziniLab/biorouter/issues/new",
                "----- title -----\nChart panel renders blank",
                "----- body -----\n**Describe the bug**\n\nblank",
            ] {
                assert!(text.contains(needle), "missing `{needle}`: {text}");
            }
        }
        assert!(receipt(true).contains("GitHub CLI"));
        let private = receipt(false);
        for needle in ["GitHub CLI", "`gh`", "signed in"] {
            assert!(!private.contains(needle), "{private}");
        }
    }

    fn tool_approval(request: UserActionRequest) -> ToolApprovalRequest {
        match request {
            UserActionRequest::ToolApproval(request) => request,
            other => panic!("the bug reporter parks a tool approval: {other:?}"),
        }
    }

    fn stored(tier: SessionClassification, reason: Option<&str>) -> Option<StoredClassification> {
        Some(StoredClassification {
            tier,
            reason: reason.map(str::to_string),
        })
    }

    /// The card a private chat gets: loud, first, and specific about why —
    /// and still demanding proof of a person, at the highest risk grade.
    #[test]
    fn a_private_chat_s_card_leads_with_why_it_is_private() {
        let compose = Filer::ComposeUrl("https://example.invalid".into());
        for (reason, clause) in [
            ("turn:versa_azure", "it ran on a private model."),
            ("mcp:cdwagent", "it reached a private data source."),
            (
                "inherited:20260101_1",
                "it was created inside a private chat.",
            ),
        ] {
            let request = tool_approval(approval_request(
                "Chart panel renders blank for a single-row dataset",
                "**Describe the bug**\n\nblank",
                issue::DEFAULT_REPO,
                &compose,
                &ChatPrivacy {
                    stored: stored(SessionClassification::Private, Some(reason)),
                    private: true,
                },
                false,
            ));
            let prompt = request.prompt.unwrap_or_default();
            assert!(
                prompt.starts_with(&format!("⚠ This chat is PRIVATE: {clause}")),
                "the warning must come first and say why ({reason}): {prompt}"
            );
            for needle in [
                "patient or participant data",
                "the failure list quotes raw tool output",
                "nothing is posted automatically",
                "you decide there whether to submit",
                // The card's own text after the warning is unchanged.
                "world-readable and permanent",
            ] {
                assert!(prompt.contains(needle), "missing `{needle}`: {prompt}");
            }
            assert_eq!(
                request
                    .arguments
                    .get("chatPrivacyTier")
                    .and_then(Value::as_str),
                Some("private")
            );
            assert!(request.requires_user_proof);
            assert_eq!(request.risk, Some(ToolRisk::High));
            assert_eq!(
                request.arguments.get("body").and_then(Value::as_str),
                Some("**Describe the bug**\n\nblank"),
                "the body rides verbatim"
            );
        }
    }

    /// Every reason the vocabulary names completes "This chat is PRIVATE: it …"
    /// with a true clause; anything else (an absent reason, a future entry,
    /// `declassified_by_user`) yields none, never the declassify control's
    /// catch-all, which is a sentence about the record rather than the chat.
    #[test]
    fn the_private_reason_clause_is_specific_or_absent() {
        for (reason, clause) in [
            ("turn:versa_azure", "ran on a private model"),
            ("mcp:cdwagent", "reached a private data source"),
            ("inherited:20260101_1", "was created inside a private chat"),
            ("diverged:20260101_1", "was branched out of a private chat"),
            ("imported", "was imported already marked private"),
        ] {
            assert_eq!(
                private_reason_clause(Some(reason)),
                Some(clause),
                "{reason}"
            );
        }
        assert!(
            private_reason_clause(Some("backfill:versa_azure")).is_some_and(
                |clause| clause.starts_with("was marked private by the one-time migration")
            )
        );

        for reason in [
            None,
            Some("declassified_by_user"),
            Some("future:thing"),
            Some(""),
        ] {
            assert_eq!(private_reason_clause(reason), None, "{reason:?}");
            let warning = private_warning(stored(SessionClassification::Private, reason).as_ref());
            assert!(
                warning.starts_with("⚠ This chat is classified PRIVATE. The report below"),
                "{reason:?}: {warning}"
            );
            assert!(!warning.contains("does not record"), "{warning}");
        }
    }

    /// An unreadable classification is said to be unreadable, not dressed up
    /// as a reason.
    #[test]
    fn an_unknown_classification_says_so_on_the_card() {
        let request = tool_approval(approval_request(
            "Chart panel renders blank for a single-row dataset",
            "body",
            issue::DEFAULT_REPO,
            &Filer::Manual {
                gh_unavailable: false,
            },
            &ChatPrivacy {
                stored: None,
                private: true,
            },
            false,
        ));
        let prompt = request.prompt.unwrap_or_default();
        assert!(
            prompt.starts_with("⚠ Biorouter could not confirm this chat's privacy"),
            "{prompt}"
        );
        assert!(prompt.contains("treated as PRIVATE"), "{prompt}");
        assert_eq!(
            request
                .arguments
                .get("chatPrivacyTier")
                .and_then(Value::as_str),
            Some("unknown")
        );
    }

    /// With the master switch off the card still NAMES the classification —
    /// it is a fact — but does not warn, because nothing acts on it.
    #[test]
    fn with_the_switch_off_the_card_names_the_tier_without_the_warning() {
        let request = tool_approval(approval_request(
            "Chart panel renders blank for a single-row dataset",
            "body",
            issue::DEFAULT_REPO,
            &Filer::GhCli,
            &ChatPrivacy {
                stored: stored(SessionClassification::Private, Some("turn:versa_azure")),
                private: false,
            },
            false,
        ));
        assert!(!request.prompt.unwrap_or_default().contains("PRIVATE"));
        assert_eq!(
            request
                .arguments
                .get("chatPrivacyTier")
                .and_then(Value::as_str),
            Some("private")
        );
    }

    /// The card raised for a chat, read off the pending registry, and its
    /// arguments, prompt and id.
    async fn raised_card(
        session_id: &str,
        running: tokio::task::JoinHandle<ToolResult<Vec<Content>>>,
    ) -> (
        String,
        serde_json::Map<String, Value>,
        String,
        tokio::task::JoinHandle<ToolResult<Vec<Content>>>,
    ) {
        let (card, running) = approval_card(session_id, running).await;
        let crate::conversation::message::ActionRequiredData::ToolConfirmation {
            id,
            arguments,
            prompt,
            ..
        } = card
        else {
            panic!("the parked card must be a tool confirmation: {card:?}");
        };
        (id, arguments, prompt.unwrap_or_default(), running)
    }

    /// ⚠ A private chat CAN report a bug: it gets a card, not a refusal — the
    /// card warns first and still needs proof of a person, and denying it files
    /// nothing.
    ///
    /// Reads the ambient switch rather than setting it, and SKIPS if it is off.
    /// A skip is honest; flipping it is the race described above.
    #[tokio::test]
    async fn a_private_chat_raises_a_warned_card_and_files_nothing_when_denied() {
        if !crate::privacy::privacy_tiers_enabled() {
            return;
        }
        let (_dir, manager, mut session) = session_with_failures(2).await;
        // The SNAPSHOT is the private side here, so its reason is the one the
        // card must give.
        session.privacy_tier = SessionClassification::Private;
        session.privacy_reason = Some("turn:versa_azure".to_string());
        let session_id = session.id.clone();

        let running = tokio::spawn({
            let manager = Arc::clone(&manager);
            let session = session.clone();
            async move { handle_report_bug(file_args(), &session, manager, None).await }
        });
        let (id, arguments, prompt, running) = raised_card(&session_id, running).await;

        let registry = PendingUserActions::global();
        assert!(
            registry.requires_user_proof_in_session(&session_id, &id),
            "a private chat's report must not be approvable by the model"
        );
        assert!(
            prompt.starts_with("⚠ This chat is PRIVATE: it ran on a private model."),
            "{prompt}"
        );
        assert_eq!(
            arguments.get("chatPrivacyTier").and_then(Value::as_str),
            Some("private")
        );
        assert!(
            arguments
                .get("body")
                .and_then(Value::as_str)
                .is_some_and(|body| body.contains("The Auto Visualiser panel is empty")),
            "{arguments:?}"
        );
        assert!(
            !prompt.contains("CREATE a public issue"),
            "a private chat must never be offered the filer that publishes on approval: \
             {prompt}"
        );

        registry.resolve_in_session(
            &session_id,
            &id,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            DecisionAuthority::for_test_proven(),
        );
        let body = body_of(&running.await.unwrap());
        assert!(body.contains("was refused by the user"), "{body}");
        assert!(!body.contains("Filed:"), "{body}");
    }

    /// ⚠ THE RATCHET CAN FIRE MID-TURN, and the snapshot will not show it.
    ///
    /// `session.privacy_tier` is captured when the agent loop hands this call
    /// its `Session`. A chat that began PUBLIC and read a private knowledge base
    /// three tool calls ago is private NOW, and its snapshot still says public.
    /// Treating it as public would let `gh` publish it on a click, with no
    /// warning on the card.
    ///
    /// So this fixture is the inverse of the one above: the snapshot stays
    /// Public and only the STORE is raised. The card must warn, name the
    /// STORE's reason, and carry the stored tier.
    ///
    /// ⚠ The read moved. It was `current_classification`, a second
    /// `get_session` taken just before the decision — which deadlocked against
    /// `conversation_for`'s open on Windows. `conversation_for` now returns the
    /// tier (and its reason) alongside the transcript, so this test guards the
    /// same property through a different function; do not go looking for the
    /// old name.
    #[tokio::test]
    async fn a_chat_privatised_mid_turn_is_warned_on_the_stored_tier_not_the_snapshot() {
        if !crate::privacy::privacy_tiers_enabled() {
            return;
        }
        let (_dir, manager, session) = session_with_failures(2).await;
        assert_ne!(
            session.privacy_tier,
            SessionClassification::Private,
            "the fixture only means anything if the SNAPSHOT starts non-private"
        );
        let session_id = session.id.clone();

        manager
            .update(&session.id)
            .raise_privacy(SessionClassification::Private, "mcp:cdwagent")
            .apply()
            .await
            .expect("the store accepts the ratchet");

        let running = tokio::spawn({
            let manager = Arc::clone(&manager);
            let session = session.clone();
            async move { handle_report_bug(file_args(), &session, manager, None).await }
        });
        let (id, arguments, prompt, running) = raised_card(&session_id, running).await;

        assert_eq!(
            arguments.get("chatPrivacyTier").and_then(Value::as_str),
            Some("private"),
            "the snapshot said public and the store said private; the store wins"
        );
        assert!(
            prompt.starts_with("⚠ This chat is PRIVATE: it reached a private data source."),
            "{prompt}"
        );

        PendingUserActions::global().resolve_in_session(
            &session_id,
            &id,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
            DecisionAuthority::for_test_proven(),
        );
        let body = body_of(&running.await.unwrap());
        assert!(!body.contains("Filed:"), "{body}");
    }

    /// ⚠ A store read that fails is no longer a refusal: the chat is treated
    /// as private, warned, and filed through the page the user submits.
    ///
    /// Independent of the master switch, so it runs everywhere — and it is the
    /// one test that APPROVES a card, which is safe by construction: a private
    /// chat never gets `gh`, `file_with_gh` refuses from a test binary anyway,
    /// and `open_in_browser` opens nothing from one. What it proves is what the
    /// user is told after approving: the link, verbatim, and that the
    /// diagnostics bundle must stay off a public issue.
    #[tokio::test]
    async fn an_unreadable_classification_is_warned_and_filed_through_the_compose_page() {
        let (_dir, _manager, session) = session_with_failures(2).await;
        // A store that has never heard of this chat: the read fails, the
        // turn-start snapshot supplies the transcript, and there is no tier.
        let elsewhere = TempDir::new().unwrap();
        let unreadable = Arc::new(SessionManager::new(elsewhere.path().to_path_buf()));
        let session_id = session.id.clone();

        let running = tokio::spawn({
            let session = session.clone();
            async move { handle_report_bug(file_args(), &session, unreadable, None).await }
        });
        let (id, arguments, prompt, running) = raised_card(&session_id, running).await;
        assert_eq!(
            arguments.get("chatPrivacyTier").and_then(Value::as_str),
            Some("unknown")
        );
        assert!(prompt.contains("treated as PRIVATE"), "{prompt}");

        PendingUserActions::global().resolve_in_session(
            &session_id,
            &id,
            UserActionOutcome::Approved {
                permission: Permission::AllowOnce,
            },
            DecisionAuthority::for_test_proven(),
        );
        let body = body_of(&running.await.unwrap());
        assert!(!body.contains("Filed:"), "{body}");
        assert!(
            body.contains("https://github.com/") && body.contains("/issues/new?"),
            "the compose link must be handed back: {body}"
        );
        assert!(body.contains("exactly as written"), "{body}");
        assert!(
            body.contains("Tell the user this now")
                && body.contains("do NOT attach the diagnostics bundle"),
            "a private chat's bundle is the conversation, unredacted, and the warning is \
             not conditional on the user asking: {body}"
        );
        // The prefilled page itself must not invite the bundle either: that
        // page is what the user reads just before pressing Submit.
        assert!(
            !body.contains("diagnostics%20bundle"),
            "the prefilled body invites the bundle: {body}"
        );
    }

    /// ⚠ The hand-back is a private chat's usual fallback (a body too long for
    /// a link), so it carries the do-not-attach warning too. A public chat's
    /// hand-back says nothing about the bundle: the body it hands over already
    /// says where the bundle is.
    #[tokio::test]
    async fn a_private_hand_back_tells_the_user_not_to_attach_the_bundle() {
        let draft = Draft {
            title: "Chart panel renders blank".into(),
            description: "blank".into(),
            steps: Vec::new(),
            expected: String::new(),
            suspected_cause: None,
            additional: None,
        };
        let title_scrub = redact::scrub(&draft.title, None);
        for private in [true, false] {
            let receipt = body_of(
                &post_report(
                    Filer::Manual {
                        gh_unavailable: !private,
                    },
                    issue::DEFAULT_REPO,
                    &draft,
                    "**Describe the bug**\n\nblank",
                    &title_scrub,
                    private,
                )
                .await,
            );
            assert!(
                receipt.ends_with("----- body -----\n**Describe the bug**\n\nblank"),
                "the text to paste must end the receipt: {receipt}"
            );
            assert_eq!(
                receipt.contains("do NOT attach the diagnostics bundle"),
                private,
                "private={private}: {receipt}"
            );
        }
    }

    /// ⚠ The harness refuses rather than posting. This is the one failure in
    /// the tool a retry CAN fix, so it is an error carrying the reasons.
    #[tokio::test]
    async fn a_report_that_fails_the_harness_is_refused_with_its_reasons() {
        let (_dir, manager, session) = session_with_failures(1).await;
        let result = handle_report_bug(
            args(serde_json::json!({
                "action": "file",
                "title": "bug",
                "description": "it broke"
            })),
            &session,
            manager,
            None,
        )
        .await;
        let error = result.expect_err("an unusable title must not reach a card");
        assert!(error.message.contains("was NOT filed"), "{error:?}");
        assert!(error.message.contains("title"), "{error:?}");
    }

    #[tokio::test]
    async fn filing_without_a_title_or_description_says_which_is_missing() {
        let (_dir, manager, session) = session_with_failures(1).await;
        let error = handle_report_bug(
            args(serde_json::json!({"action": "file", "description": "it broke"})),
            &session,
            Arc::clone(&manager),
            None,
        )
        .await
        .expect_err("filing needs a title");
        assert!(error.message.contains("`title` is required"), "{error:?}");

        let error = handle_report_bug(
            args(serde_json::json!({"action": "file", "title": "A believable title here"})),
            &session,
            manager,
            None,
        )
        .await
        .expect_err("filing needs a description");
        assert!(
            error.message.contains("`description` is required"),
            "{error:?}"
        );
    }

    /// ⚠ A model that omits `action` must land on the half that cannot publish.
    #[test]
    fn an_ambiguous_call_analyses_rather_than_files() {
        assert_eq!(Action::infer(&args(serde_json::json!({}))), Action::Analyze);
        assert_eq!(
            Action::infer(&args(serde_json::json!({"title": "something broke"}))),
            Action::Analyze,
            "a title alone is a draft, not an instruction to publish"
        );
        assert_eq!(
            Action::infer(&args(serde_json::json!({"description": "it broke"}))),
            Action::Analyze
        );
        assert_eq!(
            Action::infer(&args(serde_json::json!({"action": "nonsense"}))),
            Action::Analyze,
            "an unrecognised action must not fall through to filing"
        );
        assert_eq!(
            Action::infer(&args(serde_json::json!({"action": "Analyze"}))),
            Action::Analyze,
            "matching is case-insensitive, so this is the analyze half by name"
        );
        assert_eq!(
            Action::infer(&args(serde_json::json!({"action": "analyse"}))),
            Action::Analyze,
            "the British spelling is the same request"
        );
    }

    #[test]
    fn a_complete_draft_or_an_explicit_action_files() {
        assert_eq!(
            Action::infer(&args(serde_json::json!({
                "title": "Charts render blank",
                "description": "The panel is empty."
            }))),
            Action::File
        );
        assert_eq!(
            Action::infer(&args(serde_json::json!({"action": "file"}))),
            Action::File
        );
    }

    /// Blank strings are not a draft.
    #[test]
    fn whitespace_arguments_do_not_count_as_a_draft() {
        assert_eq!(
            Action::infer(&args(
                serde_json::json!({"title": "  ", "description": "\n"})
            )),
            Action::Analyze
        );
    }

    /// ⚠ The same call, in the shapes different model families actually emit.
    ///
    /// This tool is reached from every provider Biorouter supports — Anthropic,
    /// OpenAI, Versa, Bedrock, Ollama, llama.cpp, and a Claude Code or Codex
    /// child over the bridge — and the tool schema is the only contract between
    /// them. Two divergences are measured behaviours in this tree rather than
    /// hypotheses: `autovisualiser::normalize_dashboard_args` exists because
    /// GPT-5.5 wraps a whole argument object in an envelope and retries
    /// identically after a rejection, and `de_flexible`/`de_stringified` exist
    /// because nested structures arrive stringified.
    ///
    /// Every row below must reach the SAME decision. A tool that only works for
    /// the house style of whichever model it was written against is a tool that
    /// silently stops working when the user switches models — which they do,
    /// from the composer, mid-chat.
    #[test]
    fn every_model_family_s_argument_shape_reaches_the_same_decision() {
        let report = serde_json::json!({
            "action": "file",
            "title": "Auto Visualiser renders a blank panel for a single-row dataset",
            "description": "A chart of a one-row table produces an empty panel."
        });
        let shapes: Vec<(&str, Value)> = vec![
            ("plain object", report.clone()),
            (
                "capitalised action",
                serde_json::json!({ "action": "File", "title": "t", "description": "d" }),
            ),
            (
                "padded action",
                serde_json::json!({ "action": "  file  ", "title": "t", "description": "d" }),
            ),
            (
                "`arguments` envelope",
                serde_json::json!({ "arguments": report.clone() }),
            ),
            (
                "`report` envelope",
                serde_json::json!({ "report": report.clone() }),
            ),
            (
                "`data` envelope",
                serde_json::json!({ "data": report.clone() }),
            ),
            (
                "stringified envelope",
                serde_json::json!({ "arguments": report.to_string() }),
            ),
            ("wholly stringified", Value::String(report.to_string())),
        ];
        for (name, shape) in shapes {
            assert_eq!(
                Action::infer(&normalize_arguments(shape.clone())),
                Action::File,
                "`{name}` must file: a model's house style is not a different intention"
            );
        }
    }

    /// ⚠ And the safety direction survives all of it: anything unrecognised
    /// still lands on the half that cannot publish.
    #[test]
    fn normalisation_never_turns_an_unrecognised_call_into_a_publish() {
        for shape in [
            serde_json::json!({}),
            serde_json::json!({ "action": "publish" }),
            serde_json::json!({ "action": "FILE_IT_NOW" }),
            serde_json::json!({ "arguments": { "action": "analyse" } }),
            serde_json::json!({ "data": { "title": "only a title" } }),
            Value::String("not json at all".to_string()),
            Value::Array(vec![]),
        ] {
            assert_eq!(
                Action::infer(&normalize_arguments(shape.clone())),
                Action::Analyze,
                "{shape} must not publish"
            );
        }
    }

    /// An envelope is unwrapped only when the outer object is genuinely an
    /// envelope — a real call carrying a `data` field of its own keeps it.
    #[test]
    fn a_real_call_is_not_mistaken_for_an_envelope() {
        let call = serde_json::json!({
            "action": "analyze",
            "data": { "title": "not the real arguments" }
        });
        let normalised = normalize_arguments(call.clone());
        assert_eq!(
            normalised, call,
            "an object that names its own fields is not an envelope"
        );
    }

    #[test]
    fn steps_are_accepted_as_a_list_or_as_prose() {
        assert_eq!(
            steps_arg(&args(serde_json::json!({"steps": ["one", " two "]}))),
            vec!["one", "two"]
        );
        assert_eq!(
            steps_arg(&args(serde_json::json!({"steps": "- one\n- two\n\n"}))),
            vec!["one", "two"]
        );
        assert!(steps_arg(&args(serde_json::json!({}))).is_empty());
        // A stringified JSON array — parsed as the list it is, not split into
        // lines of punctuation.
        assert_eq!(
            steps_arg(&args(serde_json::json!({"steps": "[\"one\", \"two\"]"}))),
            vec!["one", "two"]
        );
    }
}
