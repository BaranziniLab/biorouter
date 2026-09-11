# Subagents

> **What this is.** A guide to subagents — the temporary biorouter instances the main agent spawns to run a task in isolation — covering how to invoke them in natural language, configure them with a workflow file, control their extensions and return mode, and what they are forbidden to do.
> **Status:** Current.
> **Audience:** end users

Subagents are independent instances that execute tasks while keeping your main conversation clean and focused. They bring process isolation and context preservation by offloading work to separate instances. Think of them as temporary assistants that handle specific jobs without cluttering your chat with tool execution details.

> **Note.** A *subagent* is not a *subworkflow*. A subagent is a temporary instance the main agent spawns during a chat session, configured either by your natural-language request or by naming a workflow file. A [subworkflow](../workflows/subworkflows.md) is a workflow file registered in another workflow's `sub_workflows` field, which the parent workflow exposes as a tool. Subagents are driven from a conversation; subworkflows are driven from a workflow definition.

## How to use subagents

> **Note.** biorouter can autonomously decide to use subagents when it determines they would be beneficial for your task - you don't always need to explicitly request them. This happens automatically in autonomous [permission mode](../security/permission-modes.md) (the default). Subagents are disabled in manual approval, smart approval, and chat-only modes.

To use subagents, ask biorouter to delegate tasks using natural language. biorouter automatically decides when to spawn subagents and handles their lifecycle. You can:

1. **Request specialized help**: "Use a code reviewer to analyze this function for security issues"
2. **Reference specific workflows**: "Use the 'security-auditor' workflow to scan this endpoint"
3. **Run parallel tasks**: "Create three HTML templates simultaneously"
4. **Delegate complex work**: "Research quantum computing developments and summarize findings"
5. **Control extension access**: "Create a subagent with only the developer extension to refactor the code"

You can run multiple subagents sequentially or in parallel.

| Type | Description | Example phrasing | Example request |
|------|-------------|------------------|---------|
| **Sequential** (Default) | Tasks execute one after another | "first...then", "after" | `"First analyze the code, then generate documentation"` |
| **Parallel** | Tasks execute simultaneously | "parallel", "simultaneously", "at the same time", "concurrently" | `"Create three HTML templates in parallel"` |

> **Note.** If a subagent fails or times out (5-minute default), you will receive no output from that subagent. For parallel execution, if any subagent fails, you get results only from the successful ones.

## Watching a subagent work

Subagents used to be opaque: you saw a spinner, then a summary. In the desktop app they now run as ordinary chat tabs you can read, talk to and stop — the same conversation the parent agent is delegating to, rendered live.

### The subagent tab

When a subagent starts, a tab opens **in the background** (it never steals the composer you are typing into) carrying a `subagent` badge and a header that shows:

- **spawned by** — a link back to the conversation that delegated the work;
- **the spawn context** — the exact instructions and system prompt the child was started with, expandable;
- **the child's grants** — which extensions, skills and knowledge bases it was given;
- **Stop** — the kill switch.

Below the header is the child's live transcript, streaming its tool calls as they happen.

### Watch, steer, stop

Three things you can do from that tab:

- **Watch.** Read the transcript as it streams. You are not interrupting anything.
- **Steer.** Type into the tab's ordinary composer. While the child's turn is running your message is injected as a mid-turn correction ("stop at step 3 and summarise"); between turns it starts a new turn or leaves a note. Either way it is labelled in the transcript as a **direct user message**, permanently.
- **Stop.** The header's Stop control cancels the child's turn. The parent's tool call then resolves promptly, carrying whatever the child had produced — it is not left hanging. What it resolves *as* depends on how far the child got: a child stopped mid-tool-call, with no text to show for it, comes back `incomplete`; a child that had already written a summary returns that summary and can still be labelled `completed`, because the envelope is classified from the transcript rather than from the fact of the cancellation. Do not read `completed` as proof the child finished on its own.

**Closing the tab never kills the child.** That is the same rule as every other tab in BioRouter: closing is a view operation. Stop is the only kill switch, and a child whose tab you closed is still reachable from History.

If you typed into the tab, the parent is told. Its tool result carries `human_intervened` and gains a line — *"Note: the user intervened directly in this subagent's tab during the run."* — so it weighs the child's self-report accordingly instead of assuming an untouched run. Nothing is said when you did not: silence there would read as a claim that someone checked.

The flag tracks **messages you sent**, so it is Steer that sets it, not Stop. Pressing Stop without typing cancels the run without marking the result as intervened.

### Visible by default

Children are **visible by default** whenever the desktop app is open. To run one silently, ask for it — the agent passes `visible: false` on the spawn — and the child runs exactly as subagents did before, reachable only from History and from the parent's summary.

Two things also suppress the tab without changing what runs: no GUI attached (a terminal session or a bare daemon), and the **"Never open tabs automatically"** setting described in the [Workspace Control guide](../extensions/built-in/workspace.md#focus-etiquette).

### The fan-out cap

At most **4** children *running at the same time* get a tab from one parent. Ask for ten subagents in parallel and you get four tabs, not a tab storm; children five through ten run in the background, appear in History nested under their parent, and can be read with `workspace_read_conversation`. A spawn is **never refused** because of this cap, and the parent is told which children did not get a tab so it does not claim one exists. Raise or lower it with `BIOROUTER_WORKSPACE_MAX_VISIBLE_CHILD_TABS`.

It is a cap on the burst, not a running total of open tabs. Each slot is released when that child's run ends, so a parent that spawns four, waits for them, and spawns four more gets tabs for all eight — you can finish a long conversation with more than four subagent tabs open, and closing them is up to you. That is the intended behaviour: the cap exists to stop a single parallel fan-out from burying the workspace, and a batch you have already read is not a fan-out.

### Finding subagent runs later

History hides subagent runs by default, so your session list stays a list of *your* conversations. Turn on **Show subagent runs** in History and each child appears nested under the conversation that spawned it, with a live marker while it is still running.

From the CLI, `biorouter session list --subagents` does the same, `biorouter session attach` joins a live child (`--of` to pick one by parent, `--read-only` to watch without steering), and `biorouter session cancel` stops it. Steering or stopping a child needs proof that a person acted: a daemon started with a user-action key asks the terminal for the key first, and one started without — `biorouter serve` — refuses, and says so; `--read-only` still follows the child there. See [the user-action key](workspace-control.md#as-subcommands-you-type).

## Internal subagents

Internal subagents spawn biorouter instances to handle tasks using your current session's context and extensions. There are two ways to configure and execute internal subagents:

1. **Direct prompts** - Quick, one-off tasks using natural language instructions
2. **Workflows** - Reusable, structured configurations for specialized subagent behavior

### Direct prompts

Direct prompts are for one-off tasks, expressed in natural language. The main agent automatically configures the subagent based on your request.

Prompt:

```text
"Use 2 subagents to create hello.html with 'Hello World' content and goodbye.html with 'Goodbye World' content in parallel"
```

Illustrative tool output — an example of the shape you get back, not a recorded transcript:

```json
{
  "execution_summary": {
    "total_tasks": 2,
    "successful_tasks": 2,
    "failed_tasks": 0,
    "execution_time_seconds": 16.2
  },
  "task_results": [
    {
      "task_id": "create_hello_html",
      "status": "success",
      "result": "Successfully created hello.html with Hello World content"
    },
    {
      "task_id": "create_goodbye_html", 
      "status": "success",
      "result": "Successfully created goodbye.html with Goodbye World content"
    }
  ]
}
```

### Workflows

Use [workflow](../workflows/README.md) files to define specific instructions, extensions, and behavior for subagents. Workflows provide reusable configurations that can be shared and referenced by name.

Create a workflow file, `code-reviewer.yaml`:

```yaml
id: code-reviewer
version: 1.0.0
title: "Code Review Assistant"
description: "Specialized subagent for code quality and security analysis"
instructions: |
  You are a code review assistant. Analyze code and provide feedback on:
  - Code quality and readability
  - Security vulnerabilities
  - Performance issues
  - Best practices adherence
activities:
  - Analyze code structure
  - Check for security issues
  - Review performance patterns
extensions:
  - type: builtin
    name: developer
    display_name: Developer
    timeout: 300
    bundled: true
parameters:
  - key: focus_area
    input_type: string
    requirement: optional
    description: "Specific area to focus on (security, performance, readability, etc.)"
    default: "general"
prompt: |
  Please review the following code focusing on {{focus_area}} aspects.
  Provide specific, actionable feedback with examples.
```

Place your workflow file where biorouter can find it:

- Set the [`BIOROUTER_WORKFLOW_PATH`](../workflows/workflow-schema-reference.md#workflow-location) environment variable to your workflow directory
- Or place it in your current working directory

Prompt:

```text
Use the "code-reviewer" workflow to analyze the authentication feature I implemented
```

Illustrative output — a constructed example showing the shape of a review, not a real finding in any BioRouter source file:

```text
I'll use your code-reviewer workflow to create a specialized subagent for this analysis.

🤖 Subagent created using code-reviewer workflow
💭 Analyzing authentication function for security issues...
🔧 Scanning code structure and patterns...
⚠️  Security vulnerabilities detected!

## Code Review Results

### Critical Issues Found:
1. **SQL Injection Vulnerability**: Direct string interpolation in SQL query
2. **Missing Password Hashing**: Plain text password comparison

### Recommendations:
- Use parameterized queries or ORM
- Implement proper password hashing (bcrypt, scrypt)
- Add input validation and sanitization
```

## External subagents

External subagents let you bring in AI agents from other providers and platforms, enabling biorouter to coordinate and integrate your workflow with the broader ecosystem. The example below — running OpenAI's Codex CLI as an MCP (Model Context Protocol) server — is the only external subagent configuration documented here; adapting it to another agent means substituting that agent's own command and configuration file.

[Biorouter configuration file](../configuration/config-file-reference.md) (`~/.config/biorouter/config.yaml`):

```yaml
subagent:
  args:
  - mcp-server
  bundled: true
  cmd: codex
  description: OpenAI Codex CLI Subagent
  enabled: true
  env_keys:
  - OPENAI_API_KEY
  envs: {}
  name: subagent
  timeout: 300
  type: stdio
```

External tool configuration (`~/.codex/config.toml`):

```toml
# Use fast model for quick responses
# model = "codex-mini-latest"
disable_response_storage = true

# Never prompt for approval - auto-execute
approval_policy = "never"

[sandbox]
mode = "workspace-write"
```

Prompt:

```text
"Use the codex subagent to analyze my codebase structure and identify the main components"
```

Illustrative output — a constructed example of what such an analysis looks like:

```md
Based on my analysis of your codebase, here are the main components:

1. **Core Agent System** (`crates/biorouter/src/agents/`)
   - Agent orchestration and session management
   - Tool execution framework
   - Extension system integration

2. **CLI Interface** (`crates/biorouter-cli/`)
   - Command-line interface and session handling
   - Configuration management

3. **Server Components** (`crates/biorouter-server/`)
   - HTTP API endpoints
   - WebSocket communication for real-time interaction

4. **Desktop UI** (`ui/desktop/`)
   - Electron-based desktop application
   - TypeScript frontend with React components

The architecture follows a modular design with clear separation between the core agent logic, interfaces, and UI components.
```

## Suggested use cases

**Independent operations**

- Creating multiple files with similar structure
- Basic data processing tasks
- File transformations and generations

**Context preservation**

- Complex analysis that generates lots of tool output
- Specialized tasks better handled by dedicated agents
- Keeping main conversation focused on high-level decisions

**Process isolation**

- Tasks that might fail without affecting main workflow
- Operations requiring different configurations
- Experimental or exploratory work

## Lifecycle and cleanup

Subagents are temporary instances that exist only for task execution. After the task is completed, no manual intervention is needed for cleanup.

## Subagent configuration

Subagents use the following pre-configured settings, but you can override any defaults using natural language in your prompts.

### Default settings

| Parameter | Default | How to customize |
|-----------|---------|------------------|
| **Max turns** | 25 | Use natural language or set `BIOROUTER_SUBAGENT_MAX_TURNS` |
| **Timeout** | 5 minutes | Request longer timeout in your prompt |
| **Extensions** | Inherited from parent | Specify which extensions to use in your prompt |
| **Return mode** | All subagent information provided in main session | Specify how much detail you want in your prompt |

### Customizing settings in prompts

You can override any default by including the setting in your natural language request. For example:

```text
"Use subagents to analyze code, limit each to 5 turns"
```

```text
"Use a research subagent with 30 turns and 20-minute timeout to investigate quantum computing trends"
```

**Environment variable:** Set [`BIOROUTER_SUBAGENT_MAX_TURNS`](../configuration/environment-variables.md#session-management) to change the default max turns for all subagents.

### Extension control

Control which tools and capabilities subagents can access. By default, subagents inherit all extensions from your main session, but you can restrict access for security, focus or performance. For example:

```text
"Create a subagent to write a summary, but don't give it file access"
```

```text
"Use a subagent with only code editing tools to refactor main.py"
```

### Return mode control

Choose how much information biorouter provides from its subagents in your main session.

**Full details (default):** See all tool executions and reasoning steps.

```text
"Create a subagent to debug this issue - I want to see the full investigation process"
```

**Summary only:** Get just the final result to keep your conversation clean.

```text
"Use a subagent to research this topic and summarize the key findings"
```

## `subagent_status` was removed

If a prompt, skill or workflow of yours names `subagent_status`, this section is for you. That tool no longer exists — its three jobs moved to the workspace tools, each of which also works for *foreground* children and for you, not just for background handles.

| `subagent_status` mode | Replacement |
|---|---|
| list (no `handle`) | `workspace_list { scope: "all", include_subagents: true, parent_session_id: "<me>" }` |
| poll one (`handle`) | `workspace_read_conversation { session_id, view: "summary" }` |
| block (`wait: true`) | `workspace_watch { session_ids: [...], timeout_s }` |
| cancel (`cancel: true`) | `workspace_close { session_id, scope: "turn" }` |

The background *handle* mechanism itself is unchanged, and so is `BIOROUTER_SUBAGENT_BACKGROUND`: a subagent started with `background: true` still returns immediately instead of blocking the parent's turn. What changed is the identifier — the child's **session id** is now what every one of these tools takes, in place of the old handle id.

## Security constraints

Subagents operate with restricted tool access to ensure safe execution and prevent interference with the main session.

### Allowed operations

Subagents have access to these safe operations:

- **Extension discovery**: Search for available extensions to understand what tools are available
- **Resource access**: Read and list resources from enabled extensions for context
- **Extension tools**: Use tools from extensions specified in workflows or inherited from the parent session

### Restricted operations

The following operations are blocked to ensure subagents remain focused on their assigned tasks without affecting the broader system state:

- **Subagent spawning**: Cannot create additional subagents to prevent infinite recursion
- **Extension management**: Cannot enable, disable, or modify extensions to avoid conflicts with the main session
- **Schedule management**: Cannot create, modify, or delete scheduled tasks to prevent interference with parent workflows

> **Note.** Subagents can browse extensions for suggestions but cannot enable them to avoid modifying the parent session.

### A child cannot change the privacy tier

A subagent is a session of its own, so delegation would otherwise be a way around the
[privacy tier](../security/privacy-tiers.md) of the chat that delegated. It is not: **a spawn cannot
change the tier in either direction.**

| Parent chat | Child asks for | What happens |
|---|---|---|
| Public | a public model | Runs. The child is public. |
| Public | a private model | **Refused.** A public chat cannot mint a private helper — that would be a way for a public conversation to reach private data through a proxy. |
| Private | a private model | Runs. The child starts private in its own right — its classification is derived from its own model, not copied from the parent — and the reason is recorded as `inherited:<parent id>` so the lineage is auditable. |
| Private | a public model | **Refused.** The task prompt a private chat writes is itself private context, so handing it to an externally hosted model is the leak the tier exists to stop. |

Two consequences worth knowing before you write a prompt that delegates:

- **The child's tier is decided by its own model, not its parent's.** A private parent's child is
  private because the only model it is allowed to run on is a private one — not because privacy is
  inherited by lineage. That is also why a private parent cannot read another chat *through* a
  public child: the child is judged on its own capability.
- **A public child silently loses private extensions, and is told so.** If the parent's extension
  set contains a private data extension (UCSF OMOP, CDW), a public-capability child is created
  without it rather than being refused, and the result the parent gets back names what was dropped.
  A child you asked for `extensions: []` loses nothing and is told nothing — silence there would
  read as a claim that something was removed. The same applies to an extension belonging to a
  different institution than the child's model.

If a spawn is refused, the fix is not to retry: switch the chat you are delegating *from*, or start
a new chat on the model you want the work done on.

Every *check* above is off when privacy tiers are off (Settings → Privacy): no spawn is refused for
a tier reason and no extension is dropped for one. **The tier itself still propagates, though** — a
child born to a private parent is still stamped private, permanently, because that stamp is column
propagation rather than a check. It is deliberately outside the switch: a spawn that laundered a
private parent's mark to public while the feature was off would write that permanently, and
re-enabling the feature never revisits an existing row.

## Related documentation

- [Workspace Control capability](../extensions/built-in/workspace.md) — the extension that advertises the spawn tool, plus the cross-session tools this page's migration table points at.
- [Subworkflows](../workflows/subworkflows.md) — the workflow-driven counterpart to subagents, registered via `sub_workflows` and exposed as tools.
- [Workflows](../workflows/README.md) — how to author the workflow files a subagent can be configured from.
- [Workflow schema reference](../workflows/workflow-schema-reference.md) — every field available in a workflow file, including where workflows are discovered.
- [Permission modes](../security/permission-modes.md) — which modes allow autonomous subagent dispatch and which disable it.
- [Privacy tiers](../security/privacy-tiers.md) — why a spawn cannot change a chat's privacy tier in either direction, and what a public child loses from its inherited extension set.
- [Environment variables](../configuration/environment-variables.md) — `BIOROUTER_SUBAGENT_MAX_TURNS` and the other session-management settings.
