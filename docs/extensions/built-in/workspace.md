# Workspace Control capability

> **What this is.** User guide to the built-in Workspace Control capability: the tool surface that lets BioRouter operate the workspace itself — list conversations, open them, read them, inject prompts into them, change what they are allowed to use, and delegate to subagents you can watch in a live tab.
> **Status:** Current.
> **Audience:** end users.

Every BioRouter conversation is a session: its own agent, enabled capabilities, loaded extensions, skills, knowledge bases and history, plus its own desktop tab when the app is running. Workspace Control gives the agent tools over that layer. Instead of telling you "open Settings and enable the single-cell skill in that other chat", it can do it; instead of a subagent being an opaque spinner, the child runs in a tab you can read, talk to and stop.

This page is the user-facing account — what each tool is for, what it asks you first, and how its writes are labelled. If you are trying to *arrange* work rather than look a tool up, start with [Workspace control](../../agent-loop/workspace-control.md), which covers the tab/pane/window layout, delegating to subagents, and the terminal path. If you need the exact contract — every argument and default, the precise refusal strings, the caps — that is [the Workspace Control tool reference](../../agent-loop/workspace-control-tools.md), written for developers and for diagnosing a tool that behaved unexpectedly.

Workspace Control adds no network access of its own: every tool operates on sessions stored under `~/.config/biorouter/sessions/` and on the local daemon that runs them. What it can *grant* is another matter — pointing a conversation at a different provider sends that conversation's history to the provider's endpoint, and handing one an extension that talks to the network gives it the network. Those are exactly the changes that always ask you first; see [the always-confirm rule](#the-always-confirm-rule).

## Two tiers, and why they differ

Workspace Control ships in **two sizes**, and most people only ever meet the small one.

| Tier | How you get it | What the agent can do |
|------|----------------|-----------------------|
| **Delegation** (default) | Automatic. Any session that may delegate loads the capability with a fixed six-tool list: `subagent`, `workspace_list`, `workspace_read_conversation`, `workspace_send_prompt`, `workspace_close`, `workspace_watch`. | Spawn subagents and supervise them; see which conversations exist and which are running; inject a prompt into one. |
| **Full workspace control** | You enable the `workspace` capability explicitly. | Everything above plus `workspace_set_tools` (change another conversation's capabilities, extensions, skills, model, or knowledge bases), `workspace_open`, and the preview-panel pair. |

The split is no longer "your own children versus everyone else's" — an injection may go to any conversation the session can see. What separates the tiers now is **capability change versus message**: the delegation tier can *talk to* another conversation, and only the explicit opt-in can *re-tool* one or mint and move tabs. Three of the delegation tier's six tools stay child-scoped whatever the write rule says — `workspace_read_conversation`, `workspace_close` and `workspace_watch` are confined to direct subagent children by `refuse_unless_direct_subagent_child`, which is a separate mechanism from the privacy matrix and did not move.

Concretely, when delegation is permitted by your [permission mode](../../security/permission-modes.md), BioRouter can inject the restricted six-tool Workspace roster as derived session state. Explicitly enabling Workspace unlocks the full roster, and that explicit choice is never downgraded to delegation-only mode.

### Turning on the full surface

In the desktop app, **Extensions** is its own destination in the left sidebar (not a Settings tab). Open it and turn on **Workspace Control**.

From the CLI:

```bash
biorouter configure
```

Choose `Toggle Extensions`, then enable `workspace`.

> **Note.** Subagents never get Workspace Control themselves, in either tier — a child cannot spawn grandchildren, and cannot steer its own parent.

## The eight tools

Seven `workspace_*` tools plus `subagent`. You do not call these; you ask in plain language and BioRouter picks. The examples show the request and the call it turns into.

### `workspace_list`

Lists conversations — id, name, type, whether a turn is running, parent, enabled extensions, active knowledge base, and GUI tab placement.

> "What am I running right now?" → `workspace_list { scope: "running" }`

Pass `parent_session_id` (your own id) to enumerate the subagents you delegated to; results are paged (`offset`/`limit`, default 50, max 200) rather than silently truncated.

### `workspace_read_conversation`

A structured read of any conversation, in one of four views: `summary` (head/tail digest), `transcript` (prose), `tool_calls` (exactly what its agent did), `spawn_context` (how a subagent was started).

> "What did that other chat actually do to my repo?" → `workspace_read_conversation { session_id: "…", view: "tool_calls" }`

Hidden sessions are refused. Reads are recorded as tool calls in the *reading* conversation, so there is always an audit trail of who read what.

### `workspace_send_prompt`

Injects text into **any** conversation the agent can see — a subagent it spawned, or a chat you opened that it has never touched. `mode: "turn"` starts its agent on the text (it must be idle), `mode: "steer"` redirects it mid-turn (it must be running), `mode: "note"` leaves context without running anything. The message appears in that conversation's tab **as it is sent**, with no reload.

The one boundary is privacy: a chat on a public model cannot inject into a conversation marked private, and a chat on your institution's own model injecting into a public one asks you **the first time**, showing the exact text it would send. Read "the first time" literally — the approval is remembered per pair of conversations, not per message, so once you have approved one write from chat A into chat B, later writes on that pair go through without asking. That is the deliberate trade (a card on every message is a card nobody reads), and it is the thing to know before approving one: you are agreeing to the channel, not just to the text in front of you. The tool's instructions also tell the agent to use it only when it genuinely needs to — you may be reading the conversation it interrupts.

> "Tell the QC chat to stop at step 3 and summarise." → `workspace_send_prompt { session_id: "…", text: "Stop at step 3 and summarize.", mode: "steer" }`

Add `wait: "final_message"` to park until the target answers and get its reply back inline (default 120 s, max 600 s). The reply comes back whole, together with a **verdict** the agent can act on: `completed`, `declined`, `errored`, `timed_out` or `cancelled`. `declined` is the one to know about. The other conversation receives the text as a message from another agent, not from you, and is told to treat it with less trust than your own words — so it may refuse, especially when asked to run something. When it does, the agent is told it was *declined*, in the other conversation's own words, and that nothing was done. It used to be told only that the turn had finished. Every injection is permanently labelled — see [provenance](#provenance-injected-messages-are-labelled-forever).

### `workspace_set_tools`

Changes what a conversation may use: add or remove extensions, add or remove skills **for that conversation only**, switch its provider and model, or set its knowledge bases.

> "Give the transcriptomics chat the single-cell skill." → `workspace_set_tools { session_id: "…", add_skills: ["single-cell"] }`

A model change takes effect on the target's next turn; a turn already running finishes on the provider it started with. Adding a skill here never edits your machine-wide skill preferences. Some of these changes always ask you first — see [the always-confirm rule](#the-always-confirm-rule).

### `workspace_close`

Closes a conversation down at one of three scopes. `tab` closes its GUI tab only — the session and any running turn survive. `turn` cancels the turn it is running (idempotent; not an error when it is already idle). `agent` cancels and evicts its agent, keeping the session record.

> "Stop that runaway subagent." → `workspace_close { session_id: "…", scope: "turn" }`

### `workspace_watch`

Parks until one (or all) of the named conversations finishes its current turn, and reports why it ended. This is what the agent should use after starting background work — never a polling loop.

> "Tell me as soon as any of those three background jobs is done." → `workspace_watch { session_ids: ["…", "…", "…"], mode: "any" }`

Up to 32 sessions per call; default timeout 120 s, max 600 s. A timeout is not an error — the sessions keep running and the agent can watch again.

### `workspace_open`

Opens or focuses a conversation **you** own. Pass `session_id` to bring an existing one up, or `new` to start a fresh one — then `new.kind` is required and must be `"user"` (working directory defaults to the current conversation's; extensions, knowledge bases and a first prompt are optional).

> "Start a separate chat for the figure work and give it the plotting extension." → `workspace_open { new: { kind: "user", extensions: ["developer"], prompt: "Draft the figure panel layout" } }`

`placement` is `tab` (default), `split` or `window`; `focus` defaults to **false**, so a new tab never steals the composer you are typing in.

**It cannot delegate.** `new.kind: "sub_agent"` is refused, with a result pointing the agent at `subagent`. A conversation this tool creates is yours: it has no parent, so it is never nested under the agent in History and never appears in "Show subagent runs". That separation is structural rather than advisory ([#111](https://github.com/BaranziniLab/biorouter/issues/111)) — see the [session metadata contract](../../agent-loop/session-metadata-contract.md).

### `subagent`

The one spawn tool. Delegates to a fresh agent with its own context window, and — when the app is open — in its own visible tab you can watch and talk to.

> "Delegate checking the test suite to a subagent I can watch." → `subagent { instructions: "Run the test suite and report failures" }`

Children are **visible by default**; pass `visible: false` to run one silently, and `placement` to put it in a split or a window. The parent still receives only the child's final summary, which is why it will often follow up with `workspace_read_conversation view:"tool_calls"` to check what the child really did. Full detail in [Subagents](../../agent-loop/subagents.md).

## The always-confirm rule

Some capability changes ask you first **in every permission mode, including Fully Automatic**. This is deliberate: a background agent quietly handing another conversation a shell is exactly the case an approval mode would otherwise have already answered for.

A confirmation card appears when a `workspace_set_tools` call:

- **adds a process-spawning capability or extension** (Developer, Computer Controller, Code Execution, and anything the config describes as running a command), or one that sends the conversation's traffic to a remote endpoint;
- **removes a security-relevant capability** — today Workspace Control itself or the Extension Manager, both of which are how a change stays visible from inside the target;
- **removes an extension you configured explicitly** in `config.yaml`;
- **switches the conversation's provider**, which sends its whole stored history to that provider's endpoint;
- **adds a skill**, which injects instructions into the target's prompt.

The same rule covers `workspace_open { new: { extensions: […] } }`. The field name is retained for compatibility and may carry capability or extension identifiers; the effective classification remains visible in the new conversation.

The card names the target conversation and the specific reason, and says outright that it appears in every mode.

**You are only asked about changes that can actually be made.** Before the card is raised, Biorouter checks the whole request the way the change itself would: that the conversation exists and may be changed from here, that every extension named is really installed or enabled there, that a knowledge base named really exists, that the provider and model are real. If any of it cannot happen, there is no card — the agent is told why, in the same words it would otherwise have got after you approved. The common case is a **built-in capability** such as Auto Visualiser: it is part of Biorouter, not an installed extension, so this tool cannot remove it from another conversation, and the agent is told exactly that instead of asking you to approve a removal that would then fail.

## Provenance: injected messages are labelled forever

Cross-session writes are labelled in storage, not just in the UI. Every message carries its origin, and the transcript renders a small chip beside it:

- **injected by *&lt;conversation&gt;*** — another agent wrote this through `workspace_send_prompt`;
- **direct user message** — you typed it, including into a subagent's tab;
- **spawn context** — the instructions a subagent was started with.

Ordinary same-session messages have no chip. Because the label is stored rather than drawn, it survives reload and History — you can always tell later which words in a conversation were yours.

One caveat for exports: `biorouter session export` keeps provenance in `--format json` and `--format yaml`, which serialise the stored messages whole. The default `--format markdown` renders content for reading and does not carry it, so export as JSON or YAML if the labels are what you need.

Mutations are also announced live: the target tab gets a toast when another agent injects a prompt, changes its tools, or closes it. Silent cross-session action is not a supported configuration.

## Focus etiquette

By default, tabs the agent opens — including subagent tabs — open **in the background**. They never steal the composer you are typing into.

If you would rather not have tabs appear at all, turn on **Settings → App → Workspace → "Never open tabs automatically"**. With it on, the daemon downgrades every focus-stealing frame (`open_tab`, `open_window`, `activate_tab`) to a notification naming the conversation, and tells the model no tab was opened so it cannot claim otherwise. The work still runs; open it from History when you want it. The setting is stored as `WORKSPACE_ANNOUNCE_ONLY` and is **off** by default.

Subagent tabs have a second limit: at most **4** children *running at once* get a tab from the same parent (`BIOROUTER_WORKSPACE_MAX_VISIBLE_CHILD_TABS` to change it). A fan-out of ten spawns is not a tab storm — the fifth child onward runs in the background, is listed in History under its parent, and is readable with `workspace_read_conversation`. A spawn is never refused for this reason, and the parent is told which children did not get a tab.

The cap counts live children, not open tabs: a slot is released when its child finishes, so a parent that spawns four, waits, and spawns four more gets tabs both times. It bounds the burst, not how many subagent tabs you can end up with in a long conversation — closing them is yours to do.

## Without the desktop app

There is nothing GUI-shaped in the contract, but the surface is not identical without one. Three configurations, and the difference between the second and third is the **daemon**, not the window:

**Desktop app (daemon + window).** Everything above.

**A bare `biorouterd` with no window attached.** All the machinery is there; only the display is missing. `workspace_list` reports `gui_attached: false`, sessions are still created and still run, and the tool result says plainly that no tab was opened rather than pretending one was. `workspace_close { scope: "tab" }` has nothing to close and says so. One real restriction: if your machine is in an **approval** permission mode, `workspace_send_prompt mode:"turn"` is **refused** rather than started, because a tool confirmation raised by a turn nobody is watching would sit unanswered until it timed out. The error says so and points you at `mode: "note"`.

**Standalone `biorouter` in a terminal, with no daemon.** The tools that *inspect* work — `workspace_list`, `workspace_read_conversation`, `workspace_watch` (which reads the background-handle registry, so it still knows a child is running), `workspace_send_prompt mode:"note"`, and `subagent` itself. The tools that need something to *drive* a session do not, and each refuses by name rather than failing obscurely: starting a new session, `mode:"turn"`, `mode:"steer"`, setting knowledge bases, and `workspace_close` at `turn` or `agent` scope all answer *"requires the BioRouter daemon"*. Start `biorouterd` (or open the app) if you need them.

The CLI covers the same ground from the other side. These are commands *you* run, and they do **not** all sit in the same configuration: the two that only read the session store on disk are fine in the third, while the four that drive a live turn go over HTTP to `biorouterd` and need the second — which is why `session watch` says "requires a running daemon" in its own help.

| Capability | CLI | Needs `biorouterd` |
|------------|-----|--------------------|
| List conversations, including subagent runs | `biorouter session list --subagents` | no — but the live/done marks do |
| Read a conversation | `biorouter session export --format …` | no |
| Inject a prompt | `biorouter session send` | yes |
| Wait for a turn to finish | `biorouter session watch` (exits on Finish/Error; add `--follow` to keep watching past it) | yes |
| Cancel a turn | `biorouter session cancel` | yes |
| Watch or steer a live session | `biorouter session attach` (`--of` to pick a subagent, `--read-only` to observe without participating) | yes |

The listing is the one hybrid. Its rows come off disk, so it always prints; only the `● live` / `○ done` marks beside subagent runs are the daemon's to answer, and a run whose state could not be asked for reads `· state unknown` instead of the command failing.

Two capabilities deliberately have no CLI counterpart: spawning (it is a tool the model calls, and it already works inside `biorouter session`) and `workspace_set_tools` (reconfiguring another session from a terminal is out of scope; `biorouter extension` / `biorouter skill` are machine-wide, not session-scoped).

## Pairs well with Chat Recall

Workspace Control operates the **live** workspace; [Chat Recall](chat-recall.md) searches **past** conversations by content. The agent's routing instructions send "what did we conclude about X last week?" to Chat Recall — so with Workspace Control on and Chat Recall off, it is being told to reach for a tool it does not have.

That is why enabling Workspace Control in the desktop app raises a one-time, dismissible suggestion to turn Chat Recall on as well. It only ever suggests; it never enables anything for you, and it does not come back.

## Related documentation

- [Workspace control](../../agent-loop/workspace-control.md) — the how-to companion to this page: arranging tabs, panes and windows, delegating, the caps you will meet, and the terminal path.
- [Workspace Control tool reference](../../agent-loop/workspace-control-tools.md) — the developer-facing contract for the same eight tools: exact arguments, defaults and clamps, every refusal string, and the two places a tool reports success it did not earn.
- [Subagents](../../agent-loop/subagents.md) — the glass-box tab, steering a child, the fan-out cap, and the `subagent_status` migration note.
- [Chat Recall capability](chat-recall.md) — the complementary tool for searching past conversations by content.
- [Tool routing](../../agent-loop/tool-routing.md) — the routing table that separates Workspace Control from Chat Recall, Memory and the knowledge base.
- [Permission modes](../../security/permission-modes.md) — which modes allow autonomous delegation and how mutating tools are graded.
- [Agent workspace control (BR-71 design)](../../agent-loop/designs/agent-workspace-control.md) — the design of record, including the §5 permissions and abuse-resistance analysis.
- [Extensions, skills, and MCP agents](../extensions-and-skills-guide.md) — how extensions are enabled and configured generally.
