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

Subagents used to be opaque: you saw a spinner, then a summary. Now every subagent is an ordinary chat you can follow, open, talk to and stop: the same conversation the parent agent is delegating to.

### Subagents run in the background

A subagent runs in the background and **opens no tab** unless someone asks for one. The chat that started it lists it in its **chat summary**, the side panel to the right of the conversation (or the popover from the summary button in the chat header when the window is too narrow for the panel), in its **Agents** section, or under its phase in the **Plan** section when the chat has a phase plan. Each subagent is one small mark:

- **its shape** says what kind of model does the work: a square for a language model, a circle for a decision model (the circle is also every phase gate and every monitor in the Decisions section);
- **its fill** says where it stands, so you never need the colour: hollow (waiting to start), hollow with a core (running, or stopping), half full (it needs an answer, it was stopped before it finished, `incomplete` in records, it was interrupted when the app or daemon stopped, or it has gone quiet), solid (completed), a cross (failed);
- **its colour** says the same again: blue while it runs, amber when it waits for you or has gone quiet, red when it failed, a calm green when it finished.

With four subagents or fewer, each is a row led by its mark: its role (the short label the parent gave it, or its name when it has none) and how long it has run, then the model and **one line of what it is doing**: "Working…" while it runs, its result line once it ended, or **"Quiet for 4m"** when a running subagent has recorded no activity for a while. Quiet is a hint that it may be waiting on something slow, not a verdict that it has stopped. With more, the marks form a grid, one square per subagent in the order they started, with the five that most need a look listed under it (waiting for you, failed, running, then the newest) and **Show N more** for the rest; from 100 subagents the squares get smaller. A phase with a single subagent shows its row alone, without a grid above it.

Everything else is in the subagent's **card**. Hover over a square or a row and the card appears beside it: its name, its model and where it runs (and, for a decision model, that it is one), which of the chat's monitors also watch it, the task, the result and how long it ran, and under Details the extensions, skills and knowledge bases it was given (by name) and what it used. Click and the card stays open, so you can use **Open in a tab**, which opens that subagent as a tab. A click never opens the tab by itself. This is the app's one rule for the pointer: hover peeks, click pins.

Three things open a tab when the subagent starts, instead of leaving it in the background:

- **You ask for one.** Ask the agent to let you watch a subagent in its own tab, and it passes `visible: true` (or a `placement`) on the spawn.
- **The "Open subagent tabs automatically" setting.** Turn it on in Settings > App > General (config key `BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS`, off by default) and every new subagent opens in its own tab, as it did before the side panel.
- **A `placement`** on the spawn (`tab`, `split` or `window`) is itself a request for a tab.

Two things suppress a tab whatever was asked: no desktop window attached (a terminal session or a bare daemon), and the **"Never open tabs automatically"** setting described in the [Workspace Control guide](../extensions/built-in/workspace.md#focus-etiquette). A subagent still runs in either case, and the parent agent is told no tab opened, so it does not tell you one did.

A background subagent still carries its `subagent` badge, so when you open it from the side panel or from History, its tab shows which chat it belongs to.

### The subagent tab

When a subagent's tab opens (from the side panel, from History, or at spawn when a tab was asked for), it opens **in the background** (it never steals the composer you are typing into) and carries a `subagent` badge. The tab is titled by the subagent's role, with "{role}: {task}" as its tooltip; a subagent with no role is titled by its task. At the top of its transcript, one line shows:

- **Subagent of** and the parent chat's name: a link back to the chat that delegated the work. The parent's id is not shown; the tab's context menu has **Copy chat ID**;
- **What it can use**: an InfoTip naming the extensions and knowledge bases it was given, at most three of each, then "and N more";
- **Instructions**: the exact instructions and system prompt the child was started with, closed by default, in the app's ordinary text. This is their one place: the transcript no longer repeats them as its first message, although the saved chat and its exports keep it;
- **Stop**: the kill switch.

Below that line is the child's live transcript, streaming its tool calls as they happen. A tab that cannot send (in a browser, for instance) offers no edit pencil on your messages, because an edit there could not be saved; Copy stays.

### Watch, steer, stop

Three things you can do from that tab:

- **Watch.** Read the transcript as it streams. You are not interrupting anything.
- **Steer.** Type into the tab's ordinary composer. While the child's turn is running, the composer works as in any running chat: Enter queues your message for when the turn ends, and Cmd+Enter (Ctrl+Enter on Windows and Linux), or **Add now** on a queued message, injects it into the running turn as a mid-turn correction ("stop at step 3 and summarise"). Between turns Enter starts a new turn. Either way it is labelled in the transcript as a **direct user message**, permanently.
- **Stop.** The line's Stop control cancels the child's turn. The parent's tool call then resolves promptly, carrying whatever the child had produced — it is not left hanging. What it resolves *as* depends on how far the child got: a child stopped mid-tool-call, with no text to show for it, comes back `incomplete`; a child that had already written a summary returns that summary and can still be labelled `completed`, because the envelope is classified from the transcript rather than from the fact of the cancellation. Do not read `completed` as proof the child finished on its own.

> **Watch is the only one of the three in a browser.** On a page served by `biorouter serve` the tab has no composer and no Stop: steering and stopping a subagent need proof that a person acted, which only the desktop application holds. The tab says so in place of both controls rather than refusing on click — see [Browser access](../deployment/browser-access.md#what-a-browser-can-and-cannot-do).

**Closing the tab never kills the child.** That is the same rule as every other tab in BioRouter: closing is a view operation. Stop is the only kill switch, and a child whose tab you closed is still reachable from History.

If you typed into the tab, the parent is told. Its tool result carries `human_intervened` and gains a line — *"Note: the user intervened directly in this subagent's tab during the run."* — so it weighs the child's self-report accordingly instead of assuming an untouched run. Nothing is said when you did not: silence there would read as a claim that someone checked.

The flag tracks **messages you sent**, so it is Steer that sets it, not Stop. Pressing Stop without typing cancels the run without marking the result as intervened.

### Monitors on a subagent

A subagent can be watched by decision-model monitors like any chat, and they can come from the chat that started it as well as from its own tab:

- **Yours, on the parent.** Tick "Also watch the subagents this chat starts" on a monitor of the parent chat (`--subagents` with `biorouter session monitors`), and every subagent it starts gets a copy. Saving the parent's monitors again updates the copies on the subagents still running.
- **The agent's.** The parent agent can give a subagent monitors when it spawns it (the `subagent` tool's `monitors` field), through a phase of its plan, or later with `workspace_monitor` (which also reaches the parent's own chat when `session_id` is left out). They are yes/no questions that stop or steer the subagent, at most 4 per subagent within the chat's 8. The agent may change or remove only the monitors it set, never yours.
- **Yours, on the subagent.** Open its tab and use its Monitors dialog, or `biorouter session monitors <subagent id>`. You may change or remove any of its monitors, the agent's included; in the app, editing one of the agent's makes it yours.
- **A sub-workflow's.** A subagent spawned from a sub-workflow that declares `decision_settings` starts with its monitors, as yours.

A subagent's chat runs its `turn_end` monitors at the end of each of its turns, and a `turn_end` steer sends it back to work. Every monitor is checked against the parent chat's privacy tier when the subagent is spawned, so a private chat's subagent is never started with a hosted decision model. The full rules are in [Monitors on subagents](decision-models.md#monitors-on-subagents).

**When a monitor stops a subagent,** its result reads `incomplete` and opens with which monitor stopped it and why, as in "Stopped by monitor 'Off task' (set by this chat's agent): P 0.93 reached its threshold 0.80. Its question: "Is the agent editing files outside src/?". The run did not finish." That sentence is the first line the side panel, `biorouter session subagents` and the terminal pane show for it. The parent agent is told it was stopped and did not finish. After its own monitor's stop it may read the subagent, continue it or change the monitor; after yours it is told to tell you and wait, not to work around it. A copy of one of your "also watch" monitors that stops one subagent also stops its siblings and the chat that started them.

### When the chat that started it stops

Stopping a chat stops its subagents. A plain Stop in the chat that started them (the Stop button, `biorouter session cancel`, Ctrl-C in the terminal) and a decision-model monitor's stop of that chat both stop every subagent it started that still has work in flight: a delegated run that is running or queued, a later turn in its tab, or a Stop-and-Send continuation waiting to start. Finished subagents are left alone. Each stopped subagent ends `incomplete`, keeps what it had done, and its result and its own tab say why:

- "Stopped because the person stopped the chat that started this subagent."
- "Stopped because monitor 'Form reached' stopped the chat that started this subagent."
- "Stopped because monitor 'Form reached', set by the person to watch every subagent of that chat, stopped subagent 'Fetch the data'."

A sentence names a subagent by its role when it has one ('data loader'), as the side panel does, and by its task only when it has no role.

Stop-and-Send in the parent (sending a correction while its turn runs) leaves the subagents running, because the next turn keeps supervising them, and so does cancelling a tool approval. After a monitor's stop, the parent's turn waits up to 30 seconds (`BIOROUTER_MONITOR_STOP_SETTLE_SECS`) for the subagents to stop, so it can collect what each had done, and names any that had not stopped by then. See [A stop stops the chat's subagents](decision-models.md#a-stop-stops-the-chats-subagents).

### When the child needs your permission

Even in Completely Autonomous mode some operations always ask — a recursive delete of a directory the session did not create, a write into a credential store, anything the security floor stops. A subagent running one of those has to reach **you**, and a card that lives only in a conversation you are not looking at is the same thing as no card at all.

So the ask appears in **two** places: the child's own tab, and the conversation *you* started — the top of the chain, if the work was delegated more than once, because every layer below it is itself an agent. It is one decision with one identity: answer it in either place and the child continues. That matters most where the child has no tab to answer in: a background subagent (the default), a child past the four-tab fan-out cap, a fan-out you never opened.

Two things it is not:

- **The parent agent is not asked.** The card is shown to a person, in the chat a person is watching; the delegating model never sees it and cannot answer it. An approval a security inspector raised may only ever be answered by a person, at any depth.
- **It is not a record.** An approval card exists only while somebody has to answer it, so it is never written into the delegating conversation's history — answered or expired, it leaves nothing behind in a chat it was never part of.

If nobody answers, nothing is quietly allowed: the call is refused, the child is told the approval expired, and it carries on without it.

**Folder access works the same way.** When a subagent's file operation needs a folder outside its working folder, its folder card is shown in its own tab and in the chat you are watching, as one decision. Until 2026-11-10 that card waited only in the child's own session, so a background subagent's request sat unseen until it expired. A denial ends the child's call with "Folder authorization required for <folder>. … Access was not approved." and an unanswered card with "… The request expired before anyone answered it, so access was not given."

A **direct** subagent may also use the folders you allowed with **Allow for this session** in the chat that started it, when both are at the same privacy tier. They are the same grants, so revoking one in the parent ends it for the subagent too. A grandchild does not inherit them, a grant you give in a subagent's own card stays on that subagent, and an "Allow once" never carries over. The inherited folders are not listed in the subagent's own working-folder card. See [External folder access](../security/folder-access.md#subagents).

### The fan-out cap

When subagents do open tabs at spawn (you asked, or the setting is on), at most **4** children *running at the same time* get a tab from one parent. Ask for ten subagents in parallel with the setting on and you get four tabs, not a tab storm; children five through ten run in the background, appear in the side panel and in History nested under their parent, and can be read with `workspace_read_conversation`. A spawn is **never refused** because of this cap, and the parent is told which children did not get a tab so it does not claim one exists. Raise or lower it with `BIOROUTER_WORKSPACE_MAX_VISIBLE_CHILD_TABS`.

It is a cap on the burst, not a running total of open tabs. Each slot is released when that child's run ends, so a parent that spawns four, waits for them, and spawns four more gets tabs for all eight. Tabs you open yourself from the side panel or History never count toward it.

### Phases and roles

When you ask the agent to organise a larger piece of work into stages (plan, research, implementation, verification, testing and so on), it can declare a phase plan with the `workspace_orchestrate` tool and start each subagent inside a phase. Two spawn parameters carry this:

- **`phase`** names the phase the subagent works in. A spawn into a phase the plan does not have, or one already completed or skipped, is refused before anything is created. Results handed off from earlier phases are added to the subagent's instructions, so a later phase starts from what an earlier one found.
- **`role`** is a short label for what the subagent does ("reviewer", "data loader"), at most 40 characters. It is the subagent's title in the side panel and on its tab.

The side panel, `biorouter session subagents` and the terminal pane then show the plan with each phase's progress: how many of its subagents are done, running, quiet or failed. A phase may carry a decision gate, which holds every phase after it until a decision model is confident the phase met its goal, or until you pass or skip it; the agent cannot change, remove or skip a gate. A phase may also carry monitors that every subagent spawned into it starts with. See [Orchestration phases](orchestration-phases.md) for how phases are declared, gated, completed and handed off.

### Finding subagent runs later

The side panel of the chat that started them is the quickest way back to a chat's subagents, and the only place that follows one live. History hides subagent runs by default, so your session list stays a list of *your* chats. Turn on **Show subagent runs** in History and each run appears nested under the chat that spawned it as a one-line row: its name, when it ran, the model it ran on, and how it ended, as a shape and a word: Completed (a check), Blocked (a question mark), Failed (a cross) or Stopped (a stop square, for a run that ended before it finished, `incomplete` in records). A stopped run never shows the running ring. Clicking a run row opens it. A run that has not ended yet shows no status there. A chat with more than three runs shows the newest three and a **Show N more runs** row that opens the rest in place; while you search History, every matching run is shown. A run you have focused with the keyboard stays shown, in its place in the order, when newer activity pushes it out of the newest three, so the list never takes the focus away from under you.

A subagent's status survives a restart of the app or daemon: when a run ends, its final status (completed, blocked, incomplete or failed) and the first line of its result are stored with the subagent's own session. A subagent that was still running when its process went away reads as **interrupted**, never as running.

From the CLI, `biorouter session subagents <session-id>` lists a chat's subagents with the same status, model and setup the side panel shows, and the phase plan first when there is one (`--format json` for scripts, `--watch` to redraw every two seconds). Without a running daemon it reads the local session store, so a subagent that was running reads as interrupted or unknown.

In the terminal chat (`biorouter session`), the subagent pane lists the current chat's subagents with their status, under their phases when the chat has a phase plan, in the app's notation: each row leads with the same mark, a square (■) for a language model and a circle (●) for a decision model, filled by status (□ waiting to start, ▣ running, ◧ waiting for you or quiet, ■ completed, ⊠ failed, ⊟ status unknown, and ○ ◉ ◒ ● ⊗ ⊖ for a decision model) and coloured blue for running and green for completed, a run waiting for you or quiet in the terminal's own ink and a failure dim, never red or amber (the word "failed" is on its row), with the app's status words ("waiting to start", "needs an answer", "stopped"). Past three subagents, a line of marks in the order they started sits under each phase's heading (or once under the pane's when there is no plan) and wraps into a block, except under a phase of one subagent, whose row already leads with its mark, and a phase's gate leads its words with a flag, never a circle (`⚑ gate passed`, `⚐ gate blocked · needs you`, the flag in the accent while it needs you). The pane is sections, never frames, as the desktop's rail is: it opens on a dim `AGENTS` heading, the rail's word, with the counts; while decision-model subagents mix with language-model ones a key follows it, `models: ■ language  ● decision`; a dim `PLAN` heading with the phases done and the plan's title opens the phases; each phase is headed in words (never a mark, so a `●` is always decision-model work); and a dim `─` rule parts each phase from the one above. Past three subagents each group lists five rows, the ones that need a look first (waiting for you or quiet, then failed, then running, then the rest newest first, as the desktop orders them), and a dim `+N more`, so every phase of a long plan stays on screen; the selected row and the open one are drawn wherever they rank, so ↑ and ↓ still reach every subagent. The narrow strip carries the whole line of marks after its counts. It appears on its own once the chat has a subagent: beside the transcript in a terminal at least 120 columns wide, and as a strip above the input in a narrower one. The keys:

| Key or command | What it does |
|----------------|--------------|
| `Ctrl-B` | Shows or hides the pane. |
| `Ctrl-O` | Moves focus into the pane, and back to the input. |
| `↑` `↓` `Home` `End` | With focus in the pane, choose a subagent. The pane then shows its setup: model, extensions, skills, knowledge bases, tokens and tool calls. |
| `Enter` | With focus in the pane, opens the chosen subagent's chat to read. |
| `Esc` | In the pane, returns focus to the input. In a subagent's chat, returns to the parent chat, and to the pane if that is where you opened it. |
| `/subagents` | Shows or hides the pane, like `Ctrl-B`. `/subagents show` and `/subagents hide` set it either way. |
| `/subagents list` | Prints the chat's subagents, numbered in the order they started. |
| `/attach <n\|id\|name>` | Opens one subagent's chat by its number from `/subagents list`, its session id, or its name (a unique prefix of either is enough). |

A subagent's chat opened this way is read-only: it follows the child as it works (`PageUp`, `PageDown`, `↑` and `↓` scroll it), and steering or stopping it is done with `biorouter session attach` and `biorouter session cancel`, below. Typing while the pane has focus returns focus to the input, so the start of a message is never lost, and `Ctrl-C` still stops a running turn wherever focus is. `Ctrl-B` is also tmux's default prefix key; inside tmux, use `Ctrl-O` or `/subagents` instead.

`biorouter session list --subagents` nests children under their parents the way History does, `biorouter session attach` joins a live child (`--of` to pick one by parent, `--read-only` to watch without steering), and `biorouter session cancel` stops it. Steering or stopping a child needs proof that a person acted: a daemon started with a user-action key asks the terminal for the key first, and one started without — `biorouter serve` — refuses, and says so; `--read-only` still follows the child there. See [the user-action key](workspace-control.md#as-subcommands-you-type).

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

A subagent run is still recorded as its own session, and deleting the chat that started it does **not** delete the run: its transcript stays, and it moves to the top level of History (visible with **Show subagent runs** on) and of `biorouter session list --subagents`. A run can hold text you typed into it, so it is removed only when you remove it — from History, or with `biorouter session remove --session-id <id>`. The deleted chat's id is never given to a later chat, so no other chat can come to own the run.

## Subagent configuration

Subagents use the following pre-configured settings, but you can override any defaults using natural language in your prompts.

### Default settings

| Parameter | Default | How to customize |
|-----------|---------|------------------|
| **Max turns** | 25 | Use natural language or set `BIOROUTER_SUBAGENT_MAX_TURNS` |
| **Timeout** | 5 minutes | Request longer timeout in your prompt |
| **Extensions** | Inherited from parent | Specify which extensions to use in your prompt |
| **Model** | The default worker model, or the parent's model when none is set | Choose a default worker model in Settings, or name another chat model in your prompt (see [Model control](#model-control)) |
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

### Model control

Which model a subagent runs on is decided in this order:

1. **A model the spawn names.** You can ask for another model in your prompt, and biorouter passes it to the subagent as a provider, model or temperature override. A named provider or model always wins. A provider named without a model runs on that provider's own default model. The agent is told which chat providers are set up on this computer, and a spawn that names one that is not set up is refused before the subagent is created, with the names of the ones that are.
2. **The default worker model**, when one is set. It is optional and nothing is set by default. Choose it in **Settings > Models > Default model > Default worker model**, or with `biorouter configure` (Settings, then Default Worker Model). It is the model a subagent runs on when the agent names none, typically a lighter or more token-efficient model than the chat's own. It is stored as `BIOROUTER_WORKER_PROVIDER` and `BIOROUTER_WORKER_MODEL`, and both must be set for it to apply.
3. **The model of the chat that started it**, which is how every subagent ran before the default worker model existed.

The [privacy tier rules](#a-child-cannot-change-the-privacy-tier) below apply to whichever model is chosen, including the default worker model. A default worker model never fails a spawn: when the rules refuse it for a chat (a public model under a private chat, or a private model under a public one), when it is a decision model, or when it cannot be started (its provider is not set up on this computer, for example), the subagent runs on the chat's own model instead, and the result the agent gets back says so once, with the reason. So a public default worker model is used by your public chats and your private chats keep their own model, with nothing to change.

The default worker model replaced lead/worker mode, in which a lead model answered a chat's first turns and a worker took over. A lead/worker setup in your config file is carried across the first time the app or the command line tool starts on this version: the old lead model becomes the model new chats start on, and the old worker model becomes the default worker model. A chat saved in lead/worker mode keeps running that way.

The model must be a chat model. A [decision model](decision-models.md) (any `baseten_decision` or `ollama_decision` model) answers a yes/no, choice or score question about the text it is sent and cannot carry out a task, so a spawn that would run on one is refused before the subagent is created. The check is made on the provider the subagent would really run on, so it also covers one that would inherit a decision model, although a decision chat never runs tools and so never spawns one today. The refusal tells the agent to give the subagent a chat model, or to call `decisions__decide`, which asks a decision model its questions about each item and returns the answers.

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

The background *handle* mechanism itself is unchanged, and every spawn now uses it: a `subagent` call always returns the child's session id immediately instead of blocking the parent's turn, and the parent collects the result with the tools above. What changed is the identifier: the child's **session id** is now what every one of these tools takes, in place of the old handle id.

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

When a private chat is refused a public subagent, the person sees one more line under the refusal,
in the desktop card and in both terminal views (full-screen and line by line), and the agent never
does: if the whole chat may be public, they can make it public from History and then switch it to a
public model.

Every *check* above is off when privacy tiers are off (Settings → Privacy): no spawn is refused for
a tier reason and no extension is dropped for one. **The tier itself still propagates, though** — a
child born to a private parent is still stamped private, permanently, because that stamp is column
propagation rather than a check. It is deliberately outside the switch: a spawn that laundered a
private parent's mark to public while the feature was off would write that permanently, and
re-enabling the feature never revisits an existing row.

## Related documentation

- [Workspace Control capability](../extensions/built-in/workspace.md) — the extension that advertises the spawn tool, plus the cross-session tools this page's migration table points at.
- [Orchestration phases](orchestration-phases.md): declaring a phase plan, spawning into phases, and handing results forward.
- [Chat summary rail](../desktop-ui/chat-summary-rail.md): the side panel that lists a chat's subagents.
- [Workspace control](workspace-control.md): running several chats at once, and the CLI commands that reach them.
- [Subworkflows](../workflows/subworkflows.md) — the workflow-driven counterpart to subagents, registered via `sub_workflows` and exposed as tools.
- [Workflows](../workflows/README.md) — how to author the workflow files a subagent can be configured from.
- [Workflow schema reference](../workflows/workflow-schema-reference.md) — every field available in a workflow file, including where workflows are discovered.
- [Permission modes](../security/permission-modes.md) — which modes allow autonomous subagent dispatch and which disable it.
- [Privacy tiers](../security/privacy-tiers.md) — why a spawn cannot change a chat's privacy tier in either direction, and what a public child loses from its inherited extension set.
- [Decision models](decision-models.md): the yes/no, choice and score models a subagent cannot run on, the `decide` tool that screens items with them, and the monitors that watch and stop a subagent.
- [Environment variables](../configuration/environment-variables.md) — `BIOROUTER_SUBAGENT_MAX_TURNS` and the other session-management settings.
