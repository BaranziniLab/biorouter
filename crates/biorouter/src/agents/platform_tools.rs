//! The `platform__*` tools: the six capabilities the **Agent** owns rather
//! than any extension.
//!
//! They are advertised by [`Agent::list_tools_for`] and dispatched by
//! [`Agent::dispatch_tool_call`], and they exist nowhere in the
//! [`ExtensionManager`]. That single fact is what the whole file has to keep in
//! view, because two other subsystems read the extension manager to decide what
//! the model may reach:
//!
//! * `code_execution`'s importable-module catalogue is
//!   `ExtensionManager::get_prefixed_tools_excluding`, so **no script can ever
//!   `import` a platform tool** — exactly as no script can import
//!   `workspace__subagent`, which that method strips for the same reason.
//! * `reply_parts::survives_code_execution_filter` narrows the model's directly
//!   callable list to `code_execution__*` when Code Execution is on, on the
//!   premise that anything dropped can be reached by writing code instead.
//!
//! Compose those two and the platform tools were reachable from nowhere (issue
//! #141): dropped from the roster, absent from the catalogue, so
//! `platform.ingest_source` answered `Module not found: platform`. The exemption
//! calls [`is_platform_tool_name`] — which IS a hand-written list of names,
//! [`PLATFORM_TOOL_NAMES`]. The property being bought is not "no list"; it is
//! ONE list instead of a second copy at each gate, so the next platform tool is
//! covered everywhere the day it is added to that one.
//!
//! [`Agent::list_tools_for`]: crate::agents::Agent
//! [`Agent::dispatch_tool_call`]: crate::agents::Agent
//! [`ExtensionManager`]: crate::agents::ExtensionManager

use indoc::indoc;
use rmcp::model::{Tool, ToolAnnotations};
use rmcp::object;
pub const PLATFORM_MANAGE_SCHEDULE_TOOL_NAME: &str = "platform__manage_schedule";
pub const PLATFORM_INGEST_CONVERSATION_TOOL_NAME: &str = "platform__ingest_conversation";
pub const PLATFORM_INGEST_SOURCE_TOOL_NAME: &str = "platform__ingest_source";
pub const PLATFORM_READ_SESSION_BLOB_TOOL_NAME: &str = "platform__read_session_blob";
pub const PLATFORM_MANAGE_WORKFLOW_TOOL_NAME: &str = "platform__manage_workflow";
pub const PLATFORM_REPORT_BUG_TOOL_NAME: &str = "platform__report_bug";

/// The extension key these tools are advertised under. Not a registered
/// extension — `PLATFORM_EXTENSIONS` has no `platform` entry and the
/// `ExtensionManager` map has no `platform` key — but it is the prefix the
/// model sees, and the name `list_tools(extension_name = Some("platform"))`
/// filters on.
pub const PLATFORM_EXTENSION_NAME: &str = "platform";

/// Every `platform__*` tool, in advertisement order.
///
/// One list, so a reader can never be built from a subset. Its membership is
/// asserted against the constants below.
pub const PLATFORM_TOOL_NAMES: &[&str] = &[
    PLATFORM_MANAGE_SCHEDULE_TOOL_NAME,
    PLATFORM_INGEST_CONVERSATION_TOOL_NAME,
    PLATFORM_INGEST_SOURCE_TOOL_NAME,
    PLATFORM_READ_SESSION_BLOB_TOOL_NAME,
    PLATFORM_MANAGE_WORKFLOW_TOOL_NAME,
    PLATFORM_REPORT_BUG_TOOL_NAME,
];

/// Is this the name of a tool the **agent loop** dispatches?
///
/// Deliberately an exact-name test rather than a `platform__` prefix match: the
/// prefix is not reserved (an installed extension normalizing to `platform`
/// would take it), and every gate that asks this question is granting a tool a
/// path around the extension manager. Only the names this file defines, and
/// dispatches, get that.
pub fn is_platform_tool_name(name: &str) -> bool {
    PLATFORM_TOOL_NAMES.contains(&name)
}

/// Which of them the caller's session may see.
///
/// Each field is one gate, sampled by the Agent — the only thing that can read
/// all of them — and passed in.
///
/// Prescriptive, not a report of current practice: the only construction site
/// is `Agent::platform_tool_gates`, and `code_execution`'s catalogue consumes
/// no gates at all (the platform tools are never in it). Should a second reader
/// ever appear it **must** take this value rather than re-derive it — two gates
/// (the scheduler handle, the Knowledge capability) are per-agent state no other
/// caller can see, and the other two are process-global flags that must be
/// sampled once, not re-read inside a gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformToolGates {
    /// `AgentConfig::scheduler_service.is_some()` — without a scheduler the
    /// tool's own handler answers "Scheduler not available", so advertising it
    /// is an invitation to fail.
    pub scheduler: bool,
    /// The Knowledge capability is enabled. Ingestion belongs to it: disabling
    /// Knowledge must take these higher-level write paths with its primitives,
    /// rather than leaving them as an unlabelled bypass.
    pub knowledge: bool,
    /// `message_blobs::lazy_load_enabled()` — with the default hydrating read
    /// the payloads are spliced back in at load time, so the model never sees a
    /// stub and would have nothing to read back.
    pub session_blobs: bool,
    /// Workflow management is reachable.
    ///
    /// ⚠ Unconditionally true today, and the field exists anyway. Not
    /// speculation: the alternative is a sixth `if` at the bottom of
    /// `Agent::list_tools_for`, which is the exact shape issue #141 took — the
    /// roster and `code_execution`'s catalogue disagreeing because the same
    /// decision was written in two places. A condition added later belongs in
    /// `Agent::platform_tool_gates` beside the other four, where every reader
    /// of this struct already looks.
    ///
    /// Note this is NOT the approval gate. Whether a person can be asked is a
    /// property of the daemon, not of the agent, so it narrows the tool's
    /// *action list* inside `manage_workflow_tool` rather than withholding the
    /// tool: `list` and `read` are useful on a `biorouter serve` daemon that can
    /// never approve a write.
    pub workflows: bool,
    /// `pending_user_action::user_proof_available()` — a person can be asked
    /// for proof-backed approval in this process at all.
    ///
    /// Filing publishes to a public tracker, so it parks on an approval that
    /// sets `requires_user_proof`. A `biorouter serve` daemon is spawned with
    /// `Stdio::null()` and holds no proof-of-user key, so that approval refuses
    /// forever there — and a tool whose approval can never be granted must not
    /// be OFFERED. Reporting the refusal honestly is necessary and not
    /// sufficient: a model that is offered a bug reporter will propose filing a
    /// bug, and the user then meets a card whose buttons cannot work. Same
    /// rule, same reason, as `install_extension`'s `can_ask_a_person`.
    ///
    /// ⚠ It withholds the ANALYSIS half too, which is read-only and would work.
    /// A tool advertised as "report a bug" that can only ever analyse is worse
    /// than one that is absent: the model reaches it, produces a report, and
    /// the turn ends with nowhere to put it.
    pub bug_report: bool,
    /// `pending_user_action::user_proof_available()` again — the SAME
    /// process-global, sampled in the same breath as [`Self::bug_report`], for
    /// a different decision: it narrows `platform__manage_workflow`'s and
    /// `platform__manage_schedule`'s action lists rather than deciding whether a
    /// tool is offered at all.
    ///
    /// It is a field rather than a read inside [`Self::tools`] because this
    /// struct's whole contract is that a gate is sampled ONCE by
    /// `Agent::platform_tool_gates` and passed in — the doc above says so, and
    /// `manage_workflow_tool`'s own doc already claimed the value was "sampled
    /// by the caller" while the assembly quietly re-read it. A second read is a
    /// second instant: two tools built from one `PlatformToolGates` could
    /// disagree about whether a person is reachable.
    pub can_ask_a_person: bool,
}

impl PlatformToolGates {
    /// The tools this session may see, filtered by the `extension_name` a
    /// caller scoped its listing to (`None` means "everything").
    ///
    /// ONE assembly. It has a single caller today — the Agent's tool list — and
    /// the rule for any future one is that it comes here rather than pushing its
    /// own copy: these `if`s used to be separate pushes at the bottom of
    /// `Agent::list_tools_for`, which is how the model's roster and
    /// `code_execution`'s view of the world came to disagree about the same
    /// tools (issue #141).
    ///
    /// A pure function of `self`. Nothing here reads the environment, the
    /// config layer or a process-global — every such value is a field, sampled
    /// once by `Agent::platform_tool_gates`. That is what lets a test state the
    /// platform roster as an input instead of inheriting whatever the binary's
    /// other tests last left behind.
    pub fn tools(self, extension_name: Option<&str>) -> Vec<Tool> {
        if !matches!(extension_name, None | Some(PLATFORM_EXTENSION_NAME)) {
            return Vec::new();
        }
        let mut tools = Vec::new();
        if self.scheduler {
            tools.push(manage_schedule_tool(self.can_ask_a_person));
        }
        if self.knowledge {
            tools.push(ingest_conversation_tool());
            tools.push(ingest_source_tool());
        }
        if self.session_blobs {
            tools.push(read_session_blob_tool());
        }
        if self.workflows {
            tools.push(manage_workflow_tool(self.can_ask_a_person));
        }
        if self.bug_report {
            tools.push(report_bug_tool());
        }
        tools
    }
}

/// BR-7: read back a tool result that was too large to keep inline in the
/// conversation. Only offered when lazy blob loading is on — otherwise the
/// payloads are spliced back in at load time and the model never sees a stub.
pub fn read_session_blob_tool() -> Tool {
    Tool::new(
        PLATFORM_READ_SESSION_BLOB_TOOL_NAME.to_string(),
        indoc! {r#"
            Read back the full output of an earlier tool call that was too large
            to keep in the conversation.

            When a tool returns a very large result, Biorouter stores it with the
            session and leaves a stub in its place: a size summary, a head/tail
            preview, and a blob id. Use this tool with that blob id to get the
            rest, instead of re-running the original tool.

            Read a slice, don't dump the whole thing:
              - `pattern`: a regular expression; only matching lines are returned
                (the fastest way to find what you need in a big result).
              - `offset` / `limit`: a 1-based line range.
            Output is capped; narrow the query if it comes back truncated.
        "#}
        .to_string(),
        object!({
            "type": "object",
            "required": ["blob_id"],
            "properties": {
                "blob_id": {"type": "string", "description": "The blob id printed in the stub that replaced the tool result"},
                "pattern": {"type": "string", "description": "Regular expression; return only lines that match it"},
                "offset": {"type": "integer", "description": "1-based first line to return (default 1)"},
                "limit": {"type": "integer", "description": "Maximum number of lines to return (default 200)"}
            }
        }),
    ).annotate(ToolAnnotations {
        title: Some("Read a stored tool result".to_string()),
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        open_world_hint: Some(false),
    })
}

/// Tool that lets the user, mid-chat, fold conversation history (this session
/// and/or other sessions) into a knowledge base — "remember this chat".
pub fn ingest_conversation_tool() -> Tool {
    Tool::new(
        PLATFORM_INGEST_CONVERSATION_TOOL_NAME.to_string(),
        indoc! {r#"
            Digest Biorouter conversation/chat history into a knowledge base.

            Use this when the user asks to "save", "remember", "ingest", or
            "add this conversation to my knowledge base". It captures the full
            transcript (user input, model output, every tool call with its
            arguments, and every tool response), renders it to markdown, and runs
            the standard knowledge ingestion pipeline so it becomes wiki pages
            with credibility, links and git history.

            By default it ingests the CURRENT session. Pass `session_ids` to
            ingest specific (or multiple) sessions instead. Choose a target with
            exactly one of:
              - `kb_id`: ingest into an existing knowledge base, or
              - `new_kb_name`: create a new knowledge base with this display name
                and ingest into it.
            If neither is given it uses this chat's primary knowledge base, and
            the result names the base it wrote to. If the chat has no primary,
            the error lists the bases you can pass as `kb_id`.
        "#}
        .to_string(),
        object!({
            "type": "object",
            "properties": {
                "kb_id": {"type": "string", "description": "Existing knowledge base id to ingest into"},
                "new_kb_name": {"type": "string", "description": "Display name for a new knowledge base to create and ingest into"},
                "session_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Session ids to ingest. Defaults to the current session when omitted."
                },
                "focus": {"type": "string", "description": "Optional guidance on what to emphasise while digesting"}
            }
        }),
    ).annotate(ToolAnnotations {
        title: Some("Ingest conversation into knowledge base".to_string()),
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        open_world_hint: Some(false),
    })
}

/// Tool that folds documents, folders and URLs into a knowledge base through
/// the **same** transactional ingest macro the desktop's dropzone uses.
///
/// It exists because a chat had no way to reach that macro at all: the knowledge
/// extension offers only low-level primitives, so a model asked to "ingest these
/// PDFs" hand-rolled extraction and page writes and ended with raw sources and no
/// curated pages (issue #108). The description below is deliberately emphatic
/// about not doing that, because the primitives are still there and still
/// callable.
pub fn ingest_source_tool() -> Tool {
    Tool::new(
        PLATFORM_INGEST_SOURCE_TOOL_NAME.to_string(),
        indoc! {r#"
            Ingest documents, folders or URLs into a knowledge base.

            Use this whenever the user asks to "ingest", "digest", "add", "load"
            or "read into" a knowledge base any file, folder, PDF, paper, note,
            spreadsheet, page or link. It runs Biorouter's real ingestion
            pipeline: it stages the raw source, opens a git transaction, runs the
            bounded knowledge sub-agent to write curated wiki pages, validates
            them, commits (or aborts and leaves the base untouched), rebuilds the
            graph, and re-scans what it committed. The result reports, per source,
            whether curated pages were actually written.

            ALWAYS use this instead of assembling pages yourself. Do NOT extract
            text, call kb_add_raw_source, and then write pages with kb_write_page
            in a script: that path has no transaction, no abort on failure and no
            verification, and it is how ingestion ends with raw files on disk and
            no knowledge in the base.

            Sources — give either one or a batch:
              - `sources`: a list. Each entry is a local path, an http(s) URL, or
                an object with `path`, `url`, or `text` (+ optional `title`).
              - or a single `path`, `url`, or `text` (+ `title`).
            A folder or archive path is expanded to the readable files inside it.

            Choose a target with exactly one of:
              - `kb_id`: an existing knowledge base, or
              - `new_kb_name`: create a new knowledge base with this display name.
            If neither is given it uses this chat's primary knowledge base; if the
            chat has no primary, the error lists the bases you can pass.

            The ingest runs on this chat's model unless you pass `model`. If the
            chat's model cannot drive the pipeline's tools, this tool says so and
            ingests nothing — it never moves the work to a different provider on
            its own.
        "#}
        .to_string(),
        object!({
            "type": "object",
            "properties": {
                "sources": {
                    "type": "array",
                    "description": "Sources to ingest. Each entry is a local path or http(s) URL as a string, or an object with one of `path`, `url`, `text` (plus optional `title`).",
                    "items": {}
                },
                "path": {"type": "string", "description": "A single local file or folder to ingest"},
                "url": {"type": "string", "description": "A single http(s) URL to ingest"},
                "text": {"type": "string", "description": "A single pasted document to ingest"},
                "title": {"type": "string", "description": "Title for `text`"},
                "kb_id": {"type": "string", "description": "Existing knowledge base id to ingest into"},
                "new_kb_name": {"type": "string", "description": "Display name for a new knowledge base to create and ingest into"},
                "focus": {"type": "string", "description": "Optional guidance on what to emphasise while digesting"},
                "model": {
                    "type": "object",
                    "description": "Run this ingest on a specific model instead of the chat's. Only when the user asked for it, or when the chat's model cannot drive the pipeline's tools.",
                    "properties": {
                        "provider": {"type": "string"},
                        "model": {"type": "string"}
                    },
                    "required": ["provider", "model"]
                }
            }
        }),
    ).annotate(ToolAnnotations {
        title: Some("Ingest documents into knowledge base".to_string()),
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        open_world_hint: Some(false),
    })
}

/// Manage the user's saved workflows: list, read, generate from this chat, save,
/// delete, validate, share and schedule.
///
/// ⚠ The `enum` this builds is DERIVED from
/// [`crate::agents::workflow_tool::available_actions`], not written out here.
/// The two used to be the same kind of hand-copied pair that issue #141 was:
/// a schema listing an action the handler refuses is a tool advertising a verb
/// that always fails, and a handler arm the schema omits is a capability the
/// model never learns exists.
///
/// `can_ask_a_person` is `pending_user_action::user_proof_available()`, sampled
/// by the caller. On a `biorouter serve` daemon it is false forever — no
/// proof-of-user digest is ever installed — so the four mutating actions are
/// absent from the enum and the description says why, rather than leaving the
/// model to discover it by being refused (SD-8).
pub fn manage_workflow_tool(can_ask_a_person: bool) -> Tool {
    let actions = crate::agents::workflow_tool::available_actions(can_ask_a_person);
    let action_values: Vec<serde_json::Value> = actions
        .iter()
        .map(|action| serde_json::Value::String((*action).to_string()))
        .collect();

    let mutation_note = if can_ask_a_person {
        "Saving, deleting, importing and scheduling ask the user to approve the \
         change first, and show them exactly what it is."
    } else {
        "This Biorouter cannot ask the user to approve anything (it is a browser \
         session), so saving, deleting, importing and scheduling are not available \
         here. Reading and generating still work; tell the user to make changes in \
         the desktop app or with the `biorouter` command."
    };

    let description = format!(
        "Manage the user's saved Biorouter workflows.\n\n\
         A workflow is a saved, reusable setup for a chat: instructions, the \
         extensions and knowledge bases it needs, the skills it uses, and \
         optionally a starting prompt and parameters. Use this whenever the user \
         talks about their workflows, agents or saved setups — \"what workflows do \
         I have\", \"turn this chat into a workflow\", \"delete the old report one\", \
         \"run this every Monday\".\n\n\
         Actions:\n\
         - \"list\": every saved workflow, with its id, title and slash command. Do \
         this first — every other action names a workflow by that id (its exact \
         title works too).\n\
         - \"read\": the full YAML of one workflow.\n\
         - \"generate\": build a workflow out of THIS conversation and return the \
         draft. Saves nothing; show it to the user and save it only if they want \
         it.\n\
         - \"validate\": check a workflow without saving it.\n\
         - \"export\": turn a workflow into a shareable link.\n\
         - \"save\": write a workflow to the user's library. Pass `workflow` (YAML \
         or an object); pass `id` as well to overwrite an existing one.\n\
         - \"delete\": permanently remove a saved workflow.\n\
         - \"import\": add a workflow from a shared link.\n\
         - \"schedule\": run a saved workflow automatically on a cron schedule. \
         Prefer this over platform__manage_schedule's \"create\", which takes a raw \
         file path.\n\n\
         {mutation_note}"
    );

    Tool::new(
        PLATFORM_MANAGE_WORKFLOW_TOOL_NAME.to_string(),
        description,
        object!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": action_values,
                    "description": "What to do. Start with \"list\"."
                },
                "id": {"type": "string", "description": "Which saved workflow to act on: its id from \"list\", or its exact title"},
                "workflow": {"description": "A workflow document, as YAML text or as an object. Used by \"save\", \"validate\" and \"export\"."},
                "deeplink": {"type": "string", "description": "A shared workflow link, for \"import\""},
                "cron": {"type": "string", "description": "Cron expression for \"schedule\". 5-field (minute hour day month weekday) or 6-field (with leading seconds)."},
                "title": {"type": "string", "description": "Override the title of a workflow produced by \"generate\""}
            }
        }),
    )
    .annotate(ToolAnnotations {
        title: Some("Manage saved workflows".to_string()),
        read_only_hint: Some(false),
        // "delete" removes a file the user cannot get back. The hint covers the
        // tool, so it is set from the most dangerous action the tool can take.
        destructive_hint: Some(can_ask_a_person),
        idempotent_hint: Some(false),
        open_world_hint: Some(false),
    })
}

/// The agent-callable bug reporter.
///
/// The description is written to make the model reach for `analyze` first and
/// to make it ASK rather than guess, because both are behaviours a schema
/// cannot enforce: nothing stops a model calling `file` with an invented
/// report, and the only defences after that are the harness (which checks the
/// text, not its truth) and the approval card (which asks a person to read a
/// paragraph they did not ask for). The cheapest place to prevent a fabricated
/// bug report is here.
pub fn report_bug_tool() -> Tool {
    Tool::new(
        PLATFORM_REPORT_BUG_TOOL_NAME.to_string(),
        indoc! {r#"
            Report a bug in Biorouter itself to its issue tracker.

            Use this when the user says "report a bug", "file an issue",
            "something is broken in Biorouter", or describes Biorouter
            misbehaving and asks you to tell someone. This is for defects in
            Biorouter — the app, the agent, an extension, the interface — not
            for problems in the user's own code or data.

            Call it TWICE.

            1. `action: "analyze"` first, always. It reads this chat's own
               record of failed tool calls, grades them, and hands back the
               environment and the failure list. It files nothing.
               - If it says it cannot tell what went wrong, ASK THE USER what
                 they want to report and wait for their answer. Do not invent a
                 report from the failures it listed. Ask a specific question
                 naming what you can see.
               - If a failure is labelled as a deliberate refusal, that is
                 Biorouter's privacy or permission boundary working. Do not
                 file it unless the user says the wrong thing was refused.

            2. `action: "file"` with the report you wrote: `title`,
               `description` (what happened and why it is wrong), `steps` to
               reproduce, `expected`, and `additional` context. Write what was
               actually observed; do not pad it with guesses about the cause.

            Biorouter adds the version, OS, provider, model, enabled extensions
            and the failure list itself — do not repeat them. It removes home
            paths, usernames and anything credential-shaped, refuses the report
            if identifying material survives, and requires the user to approve
            the exact text before anything is published. A GitHub issue is
            public and permanent; a chat classified private will not be filed
            from at all.
        "#}
        .to_string(),
        object!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["analyze", "file"],
                    "description": "`analyze` reads this chat and reports what it finds, filing nothing. `file` submits the report you wrote. Omitting this analyses unless both `title` and `description` are present."
                },
                "title": {"type": "string", "description": "One line a maintainer can recognise in a list of issues"},
                "description": {"type": "string", "description": "What happened and why it is wrong. Becomes the report's `Describe the bug` section. With `action: \"analyze\"`, the user's own words for the problem, if they gave any."},
                "steps": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Steps to reproduce, one per entry. Omit rather than invent."
                },
                "expected": {"type": "string", "description": "What should have happened instead"},
                "additional": {"type": "string", "description": "Anything else that helps: what the user was trying to do, when it started, what is different about this machine"}
            }
        }),
    ).annotate(ToolAnnotations {
        title: Some("Report a Biorouter bug".to_string()),
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        // It reaches github.com.
        open_world_hint: Some(true),
    })
}

/// Manage the user's scheduled jobs.
///
/// ⚠ Like [`manage_workflow_tool`], the `enum` is DERIVED from
/// [`crate::agents::schedule_tool::available_actions`] rather than written out
/// here, and for the same two reasons: a schema listing an action the handler
/// refuses advertises a verb that always fails, and on a `biorouter serve`
/// daemon — where `can_ask_a_person` is false forever — the six actions that
/// need an approval can never get one, so they are absent and the description
/// says why (SD-8).
pub fn manage_schedule_tool(can_ask_a_person: bool) -> Tool {
    let action_values: Vec<serde_json::Value> =
        crate::agents::schedule_tool::available_actions(can_ask_a_person)
            .iter()
            .map(|action| serde_json::Value::String((*action).to_string()))
            .collect();

    let approval_note = if can_ask_a_person {
        "Creating, running, pausing, resuming, deleting and stopping a job ask the user to \
         approve it first, and the approval tells them when the job runs, what it runs and how."
    } else {
        "This Biorouter cannot ask the user to approve anything (it is a browser session), so \
         creating, running, pausing, resuming, deleting and stopping jobs are not available here. \
         Reading still works; tell the user to make changes in the desktop app or with the \
         `biorouter` command."
    };

    let description = format!(
        "{}\n{approval_note}",
        indoc! {r#"
            Manage biorouter's internal scheduled workflow execution.

            Actions:
            - "list": List all biorouter scheduled jobs
            - "create": Create a new biorouter scheduled job from a workflow file
            - "run_now": Execute a biorouter scheduled job immediately
            - "pause": Pause a biorouter scheduled job
            - "unpause": Resume a paused biorouter scheduled job
            - "delete": Remove a biorouter scheduled job
            - "kill": Terminate a currently running biorouter scheduled job
            - "inspect": Get details about a running biorouter scheduled job
            - "sessions": List execution history for a biorouter scheduled job
            - "session_content": Get the full content (messages) of a specific session
        "#}
    );

    Tool::new(
        PLATFORM_MANAGE_SCHEDULE_TOOL_NAME.to_string(),
        description,
        object!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": action_values
                },
                "job_id": {"type": "string", "description": "Job identifier for operations on existing jobs"},
                "workflow_path": {"type": "string", "description": "Path to workflow file for create action"},
                "cron_expression": {"type": "string", "description": "A cron expression for create action. Supports both 5-field (minute hour day month weekday) and 6-field (second minute hour day month weekday) formats. 5-field expressions are automatically converted to 6-field by prepending '0' for seconds."},
                "limit": {"type": "integer", "description": "Limit for sessions list", "default": 50},
                "session_id": {"type": "string", "description": "Session identifier for session_content action"}
            }
        }),
    ).annotate(ToolAnnotations {
        title: Some("Manage scheduled workflows".to_string()),
        read_only_hint: Some(false),
        // `delete` and `kill` are destructive; without a person to ask neither
        // is offered, so the hint follows the most dangerous action on offer.
        destructive_hint: Some(can_ask_a_person),
        idempotent_hint: Some(false),
        open_world_hint: Some(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `platform__*` constant this file declares is in
    /// [`PLATFORM_TOOL_NAMES`].
    ///
    /// The list is what `is_platform_tool_name` answers from, and that predicate
    /// is what exempts a tool from the Code Execution filter. A fifth tool added
    /// here but not to the list would therefore be advertised, dropped from the
    /// model's roster the moment Code Execution is on, and absent from the JS
    /// catalogue — issue #141 exactly, reintroduced silently. Nothing else
    /// catches it: the tool builder compiles, the dispatch branch compiles, and
    /// no count anywhere disagrees.
    ///
    /// ⚠ Membership, not length. This assertion used to compare
    /// `declared.len()` with `PLATFORM_TOOL_NAMES.len()`, which passes for an
    /// edit that adds a fifth constant while dropping an existing entry from the
    /// list — the exact silent regression above, wearing a passing test. The
    /// expected VALUES are derived from the scan (the constant's value is on the
    /// same source line as its name), so the two sets are compared directly.
    ///
    /// This is the anti-rot guard for the *file*; the load-bearing behavioural
    /// assertion is
    /// `agents::agent::tests::each_platform_tool_tracks_its_own_gate_and_the_listing_scope`,
    /// which pins `PlatformToolGates::tools(all, None) == PLATFORM_TOOL_NAMES` —
    /// i.e. that the list is also what the Agent actually advertises, in order,
    /// and that each entry tracks its own gate.
    #[test]
    fn the_name_list_holds_every_platform_tool_constant() {
        let source = std::fs::read_to_string(std::path::Path::new(file!()))
            .or_else(|_| {
                std::fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("src/agents/platform_tools.rs"),
                )
            })
            .expect("the audit must not pass vacuously: this file must be readable");
        let declared: std::collections::BTreeSet<String> = source
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("pub const PLATFORM_")?;
                let (_ident, value) = rest.split_once("_TOOL_NAME: &str = ")?;
                // `"platform__manage_schedule";` -> `platform__manage_schedule`
                let value = value.trim().strip_prefix('"')?;
                let (value, _) = value.split_once('"')?;
                Some(value.to_string())
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "the scan found no constants at all, so it proves nothing"
        );
        let listed: std::collections::BTreeSet<String> = PLATFORM_TOOL_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        assert_eq!(
            declared, listed,
            "PLATFORM_TOOL_NAMES must hold exactly the platform tools this file declares"
        );
    }

    /// Every advertised platform tool has its own branch in
    /// `Agent::dispatch_tool_call`.
    ///
    /// The two halves live in different files and neither refers to the other,
    /// so a tool can be added to the roster with no route: the model calls it,
    /// the call falls through to the extension manager, and the answer is
    /// `Tool '…' not found`. The constant's identifier is DERIVED from its
    /// value rather than listed here, so this covers a fifth tool too.
    #[test]
    fn every_advertised_platform_tool_has_a_dispatch_branch() {
        let agent_rs = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agents/agent.rs"),
        )
        .expect("the audit must not pass vacuously: agent.rs must be readable");
        for name in PLATFORM_TOOL_NAMES {
            let ident = format!("{}_TOOL_NAME", name.to_uppercase().replace("__", "_"));
            let branch = format!("if tool_call.name == {ident} {{");
            assert!(
                agent_rs.contains(&branch),
                "`{name}` is advertised but `Agent::dispatch_tool_call` has no `{branch}`"
            );
        }
    }

    /// The knowledge extension's own instructions tell the model to reach for
    /// this tool instead of hand-rolling ingestion out of the `kb_*` primitives
    /// — which is the failure issue #108 describes. The two live in different
    /// crates (`biorouter-mcp` cannot depend on `biorouter`), so nothing but
    /// this assertion keeps the name in that prose from drifting away from the
    /// constant, and a stale name there is a silent instruction to do the wrong
    /// thing.
    #[test]
    fn the_knowledge_extension_instructions_name_this_tool() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("biorouter-mcp/src/knowledge/instructions.md");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "the audit must not pass vacuously: {} could not be read: {e}",
                path.display()
            )
        });
        assert!(
            text.contains(PLATFORM_INGEST_SOURCE_TOOL_NAME),
            "the knowledge extension's instructions must name `{PLATFORM_INGEST_SOURCE_TOOL_NAME}`"
        );
        assert!(
            text.contains("Do not hand-roll ingestion"),
            "the instructions must still say NOT to rebuild ingestion from the primitives"
        );
    }

    /// Every way a caller may name a source is in the schema. A missing property
    /// is not a compile error and not a runtime error either — the model simply
    /// never learns the argument exists.
    #[test]
    fn the_ingest_source_schema_offers_a_batch_and_each_single_source_form() {
        let tool = ingest_source_tool();
        assert_eq!(tool.name, PLATFORM_INGEST_SOURCE_TOOL_NAME);
        let props = tool
            .input_schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("the schema has properties");
        for key in [
            "sources",
            "path",
            "url",
            "text",
            "title",
            "kb_id",
            "new_kb_name",
            "focus",
            "model",
        ] {
            assert!(props.contains_key(key), "the schema must offer `{key}`");
        }
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(
            description.contains("kb_write_page"),
            "the description must name the primitive it is replacing, or the model \
             reaches for it anyway"
        );
    }
    // The guard that stood here — `no_test_parks_the_session_blob_flag_in_
    // the_process_environment` — was absorbed into the table-driven
    // `model::tests::no_test_parks_a_shared_setting_in_the_process_environment`,
    // which watches this key alongside eight others on the same two
    // principles. It was not deleted: a key with no guard is a key whose
    // next writer nobody notices.
}
