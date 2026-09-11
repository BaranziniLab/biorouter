# Workspace control

> **What this is.** A task-oriented guide to running more than one BioRouter conversation at once: laying them out across tabs, panes and windows, delegating work to subagents you can watch, checking on what is running, and doing all of it from the terminal as well as the desktop app.
> **Status:** Current.
> **Audience:** end users — researchers organising a piece of work across several conversations.

A BioRouter conversation is a session: its own agent, its own working directory, its own extensions, skills and knowledge bases, its own history, and — in the desktop app — its own tab. Workspace control is the set of tools that lets the agent operate on that layer instead of only inside one chat. The practical effect is that "run the alignment while I write the methods" stops meaning *you* open a second chat, paste the context in, and remember to come back to it.

This page is the how-to. The per-tool reference — what each tool takes, the confirmation rules, how injected messages are labelled — is the [Workspace Control capability page](../extensions/built-in/workspace.md); the child-agent detail is [Subagents](subagents.md). Read this one when you have the app (or a terminal) open and want to lay work out.

## What it is for

Three jobs account for nearly all of it.

| Job | What you say | What you get |
|---|---|---|
| **Run several conversations side by side** | "Open a second chat for the QC pass, in a split, and start it on the flowcell report." | A new session in its own pane, already working, sharing your working directory unless you asked otherwise |
| **Delegate and keep working** | "Delegate the test suite to a subagent and tell me when it's done." | A child conversation in its own tab you can read, steer and stop, and a parent that waits on it properly instead of polling |
| **Fix another conversation's setup from here** | "Give the transcriptomics chat the single-cell skill." | That chat reconfigured, with a confirmation card first and a toast in its tab afterwards |

Everything below is the mechanics behind those three.

## Turning it on

Workspace control ships in two sizes. The small one is automatic: any session allowed to delegate gets `subagent`, the child-scoped supervision tools (`workspace_read_conversation`, `workspace_close`, `workspace_watch`), and the two that make cross-chat injection usable — `workspace_list` to see which conversations exist and which are running, and `workspace_send_prompt` to write into one. The full surface — changing another conversation's tool set with `workspace_set_tools`, opening and moving tabs with `workspace_open`, reading the preview panel — is an explicit opt-in.

The line between the two is **message versus capability change**: the automatic tier can talk to another conversation, and only the opt-in can re-tool one.

In the desktop app, open **Extensions** in the left sidebar and turn on **Workspace Control**. From the terminal, run `biorouter configure`, choose `Toggle Extensions`, and enable `workspace`. The extension is registered `default_enabled: false`; nothing enables it for you.

Jobs 1 and 3 need that full surface. Job 2 does not — but "allowed to delegate" is a real gate, and it is the likeliest reason a request for a subagent quietly does nothing.

### When delegation is allowed

`Agent::subagents_enabled` (`crates/biorouter/src/agents/agent.rs:3711`) decides whether the spawn tool is offered at all, and it is consulted twice: once when the tool list is built, and again when a call arrives — so a model that simply remembers the name cannot spawn where delegation is off. It refuses with *Subagent delegation is not available in this session*.

All of the following must hold:

- **The permission mode is Completely Autonomous** (`auto`). In Manual Approval, Smart Approval and Chat Only — three of the [four modes](../security/permission-modes.md) — there is no spawn tool at all. Autonomous is the default, so most people never meet this; anyone who has turned the mode down will, and the symptom is an ordinary answer where a child conversation was expected.
- **The session is not itself a subagent.** A child cannot spawn grandchildren, which is the same rule that stops a child being granted workspace control. This is a *lineage* rule and it survives; it is not the retired §7 write rule, which no longer looks at lineage at all.
- **At least one ordinary extension is loaded.** The auto-injected `workspace` entry is deliberately not counted — otherwise one turn's grant would justify the next one's, and an agent that dropped its last real extension would keep delegating forever off a grant it derived from itself.
- **The active model name does not begin with `gemini`.** This is a flat exclusion in the gate, with no rationale recorded in the source, so treat it as observed behaviour rather than a rule with a reason: on a Gemini model, delegation is off whatever your mode says.
- **The session is not a BioRouter app that delegates through `consult`.** Apps with worker profiles route delegation through their own mechanism and have the generic tool withdrawn so the two cannot both be offered.

## Running several conversations side by side

### The layout you are arranging

The desktop window holds **tab groups**. A group is a pane with its own tab strip; splitting gives you two panes side by side, each with its own tabs. A window can hold up to **six panes** (`MAX_GROUPS` in `ui/desktop/src/components/chatGroups/chatGroupsLayout.ts`), and you can open more windows.

What you can do by hand:

| Action | How |
|---|---|
| New chat tab | `Cmd`/`Ctrl`+`T` |
| New window | `Cmd`+`N` (macOS) / `Ctrl`+`N` |
| Split a pane | Drag a tab onto the **outer quarter** of a pane — left, right, top or bottom edge. Dropping in the middle half moves the tab into that pane instead of splitting |
| Reorder tabs within a pane | Drag a tab sideways inside its own tab strip. A drag is only promoted after 5 px of movement, so an ordinary click still just selects the tab (`useTabDragReorder.ts`) |
| Resize two panes | Drag the divider between them. Only the two panes either side of the divider move — the rest of the window stays put — and neither can be squeezed below 12% of the pair (`ChatGroupSplitter.tsx`) |
| Close the tab | `Cmd`/`Ctrl`+`W` |
| Close the window | `Shift`+`Cmd`/`Ctrl`+`W` |

The six-pane ceiling is the edge of what was measured, not a hard limit of the renderer — panes are cheap, but a pane holding a long transcript is not, so it was left where the evidence stopped.

### Whether a layout comes back

A layout survives a **reload** of the window and does not survive a **quit**. Worth knowing before you spend five minutes arranging six panes.

The arrangement — the pane tree, which tabs sit in which pane, where the dividers are — is written to `localStorage` on every change (`saveChatGroups`, `ui/desktop/src/components/chatGroups/chatGroupsStorage.ts:57`). But the key it is written under is scoped per window, `biorouter.chatgroups.v1:<windowId>`, and that window id is minted into **`sessionStorage`** (`getWindowId`, same file, line 21). `sessionStorage` belongs to one `BrowserWindow` and goes away when that window closes. So:

- **Reloading the window** — same window, same id, `loadChatGroups` finds the record and the panes come back. (A tab whose session never finished being created is dropped on the way in, and a pane left with no tabs at all is folded away, so a reload can tidy as well as restore.)
- **Quitting and relaunching**, or closing the window and opening a new one — a fresh id is minted, nothing matches it, and you get the cold-boot single pane. The old record stays behind in `localStorage` as unreachable residue.

The per-window scoping is deliberate rather than an oversight: two windows share one origin, so a single unscoped key would have them clobbering each other's layout. It is the *lifetime* of the id, not the scoping, that ends at quit, and nothing restores a closed window's panes. Treat a multi-pane layout as an arrangement for the sitting you are in; the conversations themselves are durable and reopen from History.

### Asking for a layout in words

The agent places conversations with the same vocabulary you use by hand:

- **`tab`** (the default) — a new tab in the current pane.
- **`split`** — a new pane beside the current one.
- **`window`** — a separate window.

Nothing else is accepted. A misspelling such as `"windows"` is refused outright rather than quietly treated as a tab, so an odd placement is an error message and never a silent surprise.

Two defaults are worth knowing because they are the opposite of what people expect:

- **Tabs open in the background.** Focus stays where you are typing. The agent has to ask for focus explicitly, and it rarely should.
- **A new conversation inherits your working directory.** If the agent puts one somewhere else, you are told — in a toast on the new tab, and in what the agent itself is told, so it can repeat it to you.

If the split is refused because the window is already at six panes, the agent is told the conversation was **NOT opened** and why. It should say so rather than claim a pane exists.

> **Note.** Opening a conversation never moves an existing one's working directory. Only a newly created session gets a directory, and it gets it at creation.

### Turning tabs off entirely

If you would rather nothing ever appeared on its own, turn on **Settings → App → Workspace → "Never open tabs automatically"**. Conversations and subagents still run; you get a notification naming them and open them from History when you want. The agent is explicitly told no tab opened, so it cannot report one. The setting is stored as `WORKSPACE_ANNOUNCE_ONLY` and is off by default.

## Delegating work you can watch

This section assumes delegation is available in the session — if asking for a subagent produces an ordinary answer instead, check [When delegation is allowed](#when-delegation-is-allowed) first.

Ask for a subagent in plain language ("delegate the QC pass to a subagent") and, whenever the desktop app is open, the child opens in its own background tab carrying a **`sub`** badge and a link back to the conversation that spawned it. Inside that tab you can read the child's transcript as it streams, type into it to steer it mid-run, and stop it from the header.

**Delegation and "open another chat" are different operations, and only one of them makes a subagent.** A subagent is a `sub_agent` session with its parent stamped on it before its first turn; that is what History nests, what the `sub` badge reads, and what "Show subagent runs" reveals. Only the `subagent` tool creates one. `workspace_open` starts a conversation **you** own, born with no parent — useful, but never your agent's delegate — and it refuses outright if asked for a subagent, pointing the agent at the right tool. So if you asked for three subagents and got three ordinary chats, that was [#111](https://github.com/BaranziniLab/biorouter/issues/111), now closed structurally rather than by advice; the full rule is the [session metadata contract](session-metadata-contract.md).

Three things about that tab that people get wrong:

- **Closing it does not kill the child.** Closing is a view operation everywhere in BioRouter. Stop is the kill switch, and a child whose tab you closed is still in History.
- **If you typed into it, the parent is told.** The parent's tool result carries a note that you intervened, so it weighs the child's self-report accordingly. Nothing is said when you did not.
- **`sub` badges are keyed to the session, not the tab.** A child that never got a tab is still badged when you open it later from History.

### Fanning out

At most **four** children *running at the same time* get a tab from one parent (`BIOROUTER_WORKSPACE_MAX_VISIBLE_CHILD_TABS`). Ask for ten in parallel and you get four tabs; the rest run in the background and appear in History nested under their parent. A spawn is never refused for hitting the cap — it is downgraded to a background run and the parent is told which children that happened to.

The cap counts *live* children, so a slot frees when a child finishes. Spawn four, wait, spawn four more, and you get tabs both times. It bounds the burst, not the running total.

Tabs are not the only cap on a fan-out, though, and the other two are what actually decide how fast ten pieces of work get done. Only **eight** subagents run at a time (`SUBAGENT_SEMAPHORE`, `crates/biorouter/src/agents/subagent_tool.rs:52`): numbers nine and ten *queue* on that semaphore and do not start until a slot frees. With background handles off — the default — a queued child has no session and no tab yet; with `BIOROUTER_SUBAGENT_BACKGROUND` on, the session is created and the tab announced *before* the semaphore is acquired, so a queued child is visible but idle. The cap is per process, one daemon shared by every conversation in it, where the four-tab cap is per parent. And past **64** subagents in flight, queued and running counted together, a spawn is not queued but **refused**, with *Subagent limit reached: N already in flight (max 64)* (`subagent_tool.rs:709`). Both bounds, and the caveat about changing one of them, are in [the limits table](#the-limits-you-will-actually-meet).

### Long jobs

**`background: true` is off by default,** which is the first thing to check if a request for a detached child quietly blocks anyway. `BIOROUTER_SUBAGENT_BACKGROUND` resolves through `Config::get_param` and falls back to `false` (`crates/biorouter/src/agents/subagent_handle.rs:53`), and while it is off the `background` argument is not advertised on the `subagent` tool at all — a model that sends it regardless is ignored and the call blocks like any other.

It is a **config parameter**, not an env var only, so either of these turns it on:

- a line in `~/.config/biorouter/config.yaml` — `BIOROUTER_SUBAGENT_BACKGROUND: true`
- the environment — `BIOROUTER_SUBAGENT_BACKGROUND=true`, which wins over the config file (`Config::get_param`, `crates/biorouter/src/config/base.rs:755`)

With it on, a subagent started with `background: true` hands the parent the child's session id straight back and the parent keeps working. The right way to collect the result later is to **wait**, not poll — the agent parks on `workspace_watch` until one (or all) of up to 32 named conversations finishes, up to a 600-second bound. A timeout there is not an error; the conversations keep running and it can wait again.

## Checking on what is running

Ask "what am I running right now?" and the agent lists the workspace: id, name, whether a turn is in flight, which parent spawned it, the extensions and knowledge bases it has, and — when the app is attached — which window, pane and tab it sits in. The default scope is the *open* set (anything with a live agent, a turn in flight, or a tab); `all` and `running` are the other two. Results are paged, 50 rows at a time, up to 200 per page.

To look inside one conversation, four views are available, and the narrowest honest one is the right choice:

| View | What it shows | Reach for it when |
|---|---|---|
| `summary` | working directory, message count, first three and last three messages | "where has that chat got to?" |
| `transcript` (default) | the user-visible messages, with tool payloads collapsed to one-line stubs | you want the prose |
| `tool_calls` | request/response pairs, correlated by id, responses clipped | "what did it actually *do* to my repo?" |
| `spawn_context` | the instructions a subagent was started with | auditing a delegation |

Reads are clipped at 20,000 characters by default (200,000 maximum) and the clip names the controls that would narrow it. `last: N` tails the conversation. There is also a `from_msg_uid` slice that starts from a specific durable message id — but note that none of the four views print message ids, so in practice narrowing happens with `last`.

Hidden sessions are refused in every view. And because a read is an ordinary tool call, it is recorded in the *reading* conversation: there is always a trail of who read what.

## Fixing another conversation's setup

`workspace_set_tools` is the one tool that changes what a different conversation may use — add or remove extensions, scope skills to that conversation alone, switch its provider and model, or set its knowledge bases. A model switch takes effect on the target's **next** turn; a turn already running finishes on the provider it started with. Skills added this way never touch your machine-wide skill preferences.

Some of these changes raise a confirmation card **in every permission mode, including the fully automatic one** — handing a conversation a process-spawning extension, removing a security-relevant one, switching its provider, or adding a skill. The full list and the reasoning are in the [extension page's always-confirm rule](../extensions/built-in/workspace.md#the-always-confirm-rule). Whatever changes, the target's tab gets a toast saying what happened and who did it. Silent cross-session action is not a supported configuration.

A change that cannot be made never reaches you as a card: the request is checked first — the conversation, every extension, knowledge base, provider and model it names — and a request that would fail is refused with the reason before you are asked anything. The confirmation card only ever describes a change that will happen if you approve it.

Three refusals you may see and should not try to work around:

- A built-in capability — Auto Visualiser, Developer, Memory, Workspace Control itself — is part of Biorouter, not an installed extension, and cannot be removed from a conversation this way.
- An extension an operator wrote `enabled: false` for cannot be enabled here. The refusal names the operator's decision and tells the agent to ask you rather than route around it.
- A subagent session can never be granted workspace control, so a child cannot spawn grandchildren or steer its parent.

The same honesty applies to talking to another conversation. When the agent sends one a prompt and waits for the answer, it gets the whole reply back with a verdict — and if the other conversation *declined* (it treats injected text as coming from another agent rather than from you, and may refuse to act on it), the agent is told that nothing was done rather than that the turn finished. Ask for the work in that conversation yourself if it has to happen there.

## The limits you will actually meet

Every one of these is a real, named bound rather than a guideline.

| Limit | Value | Change it with |
|---|---|---|
| Panes in one window | 6 | not configurable |
| Subagent tabs per parent, at once | 4 | `BIOROUTER_WORKSPACE_MAX_VISIBLE_CHILD_TABS` |
| Subagents *running* at once, per process | 8 | `BIOROUTER_SUBAGENT_MAX_CONCURRENT` (see the caveat below) |
| Subagents *in flight* at once — queued plus running, per process | 64 | `BIOROUTER_SUBAGENT_MAX_INFLIGHT` |
| Turns one conversation may inject into others at once | 4 | `BIOROUTER_WORKSPACE_MAX_INJECTED_TURNS` |
| Conversations one wait may watch | 32 | not configurable |
| Wait / injected-reply timeout | 120 s default, 600 s maximum | per call |
| Rows per listing page | 50 default, 200 maximum | per call |
| Characters per conversation read | 20,000 default, 200,000 maximum | per call |

The injected-turn cap is the one that surfaces as a puzzling message. If the agent reports that *this session already has 4 injected turns in flight*, it means this conversation is already driving four others and is being made to wait rather than saturate the daemon. A slot is released when the turn it started actually ends.

The two subagent caps look alike and behave in opposite ways at the boundary, which is the point of having both. At eight concurrent, the ninth spawn **queues** and runs when a slot frees — slower, never lost. At 64 in flight, the next spawn is **refused**, with an error the model has to handle. One is a throttle; the other is the hard stop on a runaway. The second sits so far above the first because it is a whole-process ceiling: it counts every conversation's children together, not one parent's.

One caveat on raising the concurrency cap: the semaphore is a `LazyLock` sized on first use (`subagent_tool.rs:52`), so `BIOROUTER_SUBAGENT_MAX_CONCURRENT` is read exactly once — at the **first** spawn in that process — and the size is fixed for its lifetime. Set it before you launch the app or start `biorouterd`; exporting it into a shell after a subagent has already run there changes nothing, with no error to say so. `BIOROUTER_SUBAGENT_MAX_INFLIGHT` is re-read on every spawn and has no such caveat. Both are read straight from the environment (`subagent_tool.rs:38-51`), so unlike `BIOROUTER_SUBAGENT_BACKGROUND` neither can be set in `config.yaml`.

## Doing all of this from the terminal

The CLI is a first-class front end here, and it reaches the feature in **two different ways**. Conflating them is the main source of confusion.

### As tools, inside `biorouter session`

The CLI links the same core library the daemon does, so an interactive terminal chat advertises the same tool surface. `subagent` needs no extension enabled in `biorouter session` — only the same delegation gate as everywhere else, so `/mode auto` in the terminal chat is a prerequisite — and enabling the `workspace` extension adds the seven `workspace_*` tools there too. You ask for delegation in the terminal exactly as you would in the app.

What changes without a daemon is not the tool list but what the handlers can reach. These keep working headlessly: listing conversations, reading them, waiting on them (the wait consults the parent's own background-subagent registry, so it still knows a child is running), leaving a note in another conversation, and spawning subagents. These refuse **by name**, with "requires the BioRouter daemon" rather than an obscure failure: starting a new session, starting or steering a turn in another conversation, setting knowledge bases, and cancelling a turn or stopping an agent. With no app attached, closing a tab is a stated no-op and a newly created session reports plainly that no tab was opened.

### As subcommands you type

These are ordinary shell commands, and they split in two. The first three touch only the session store on disk and work with nothing else running. The last four — `send`, `watch`, `attach`, `cancel` — drive a live turn through a running `biorouterd` over HTTP and SSE, and fail without one. Having the desktop app open does **not** satisfy that requirement, which surprises nearly everyone: see [The desktop app's daemon is not the one your terminal finds](#the-desktop-apps-daemon-is-not-the-one-your-terminal-finds).

| What you want | Command | Needs a daemon |
|---|---|---|
| See what exists, including subagent runs nested under their parents | `biorouter session list --subagents` | no |
| Read a conversation | `biorouter session export --session-id <id>` (or `--name <name>`; `--format markdown\|json\|yaml`) | no |
| Start or name a session | `biorouter session --name <name>` | no |
| Send a prompt into a session and stream its turn | `biorouter session send <id> "<text>"` (`--no-wait` to return as soon as the turn starts) | yes |
| Wait for a turn to end | `biorouter session watch <id>` (exits on finish or error; `--follow` keeps watching past it) | yes |
| Join a live session, follow it, and steer it | `biorouter session attach <id>` — `--name` to pick it by name, `--of <parent>` to attach to that parent's running subagent, `--read-only` to observe without participating | yes |
| Stop the turn a session is running | `biorouter session cancel <id>` | yes |

Give `session export` an identifier. With neither `--session-id` nor `--name` it drops into an interactive session picker instead of exporting the conversation you meant — which is a surprise in a script.

`session list --subagents` reads the store for the rows but has to ask the daemon who is still live, so it marks each run `● live`, `○ done`, or — when it could not ask — `· state unknown`. It deliberately does *not* blame a missing daemon for that third state: a stripped `BIOROUTER_SERVER__SECRET_KEY` (which is what an agent-spawned shell gets) produces it with a daemon running perfectly well. The actual reason is printed once on stderr, so `--format json` on stdout stays clean.

The four daemon-bound commands need one running. It listens on `127.0.0.1:3000` unless
`BIOROUTER_PORT` says otherwise, and `send`, `watch`, `attach` and `cancel` authenticate with the
same `BIOROUTER_SERVER__SECRET_KEY`. `biorouterd` invents a random key when that variable is unset,
in which case no client can authenticate — so set it on both sides. A mismatch shows up as HTTP 401.

Whether stopping and steering also need a **user-action key** depends on how the daemon was
started, and the command asks the daemon rather than you:

- **A daemon started with a key** — its SHA-256 digest piped to `biorouterd` on stdin, as below,
  which is how you start the daemon the desktop app shares with your terminal — wants the raw key
  before it lets anyone stop or steer a turn, and before it takes a message into a subagent's
  session, or into a private chat when your terminal runs a public model. When it refuses a
  request for want of the key, the command asks you for it once, without echo, and makes the
  request again with it. `attach` asks as it joins, before it reads anything you type, and
  checks the key there rather than at your first steer.
- **A daemon started without one** — `biorouter serve`, or `biorouterd agent` with nothing piped in
  — has nothing to check a key against, so you are never asked for one. It lets you stop and steer
  any chat it lets you reach (a public one always, a private one when your terminal runs a private
  model), except a subagent's: stopping, steering or sending to a subagent needs proof that a person
  acted, which only a daemon holding a key can check, and the command prints that daemon's refusal
  saying so ([SD-11](../deployment/serve-decisions.md#sd-11--stop-and-steering-work-on-a-daemon-with-no-key-a-subagents-tab-stays-the-persons)).
  `attach --read-only` still follows such a session.

```bash
read -r -s action_key
printf '\n'
printf '%s' "$action_key" | shasum -a 256 | cut -d ' ' -f 1 | \
  BIOROUTER_SERVER__SECRET_KEY=<daemon-key> biorouterd agent
```

`watch` and `attach --read-only` never send the key. Trusted automation can supply it as the first
line of stdin with `--user-action-key-stdin` (on `send`, `attach` and `cancel`), which sends it at
once instead of waiting for the daemon to ask; never put it in argv, an environment variable,
config, or logs. The raw key is held in a zeroizing CLI buffer and sent only on the attached event
stream, `/interrupt`, `/agent/cancel` and `/reply` — and only when you supplied it or the daemon
asked for it — while the daemon retains only its digest.

Use `session attach` rather than `session --resume` on a session that is running right now: resuming opens a second agent on the same conversation, and the two do not share the daemon's turn lock.

### The desktop app's daemon is not the one your terminal finds

This is the single likeliest way those four commands fail, and the mental model almost everyone arrives with — *the app is open, so a daemon is running, so the CLI can talk to it* — is precisely the wrong one. Read this before concluding the commands are broken.

The desktop app does start a `biorouterd`. It starts it somewhere your shell cannot find and with a key your shell cannot know:

- **An ephemeral port.** `startBiorouterd` calls `findAvailablePort()` and passes the result to the child as `BIOROUTER_PORT` (`ui/desktop/src/biorouterd.ts:283`, `:305`). It is a different port every launch, recorded only in the app's log.
- **A per-launch random secret.** `GENERATED_SECRET = crypto.randomBytes(32).toString('hex')` is minted once per Electron process (`ui/desktop/src/main.ts:900`) and handed to the daemon as `BIOROUTER_SERVER__SECRET_KEY` (`biorouterd.ts:306`). It is never written to disk.

The CLI reads neither of those. `configured_port()` takes `BIOROUTER_PORT` from *your* environment and otherwise assumes 3000 (`crates/biorouter-cli/src/commands/apps.rs:210`), and `secret_key()` takes `BIOROUTER_SERVER__SECRET_KEY` from *your* environment or refuses with an actionable error rather than guessing (`crates/biorouter-cli/src/commands/session_watch.rs:38`). So with the app open and mid-conversation, `send`, `watch`, `attach` and `cancel` fail one of two ways: nothing is listening on 3000, or something is and the key does not match, which is an HTTP 401.

**Starting your own `biorouterd agent` does not connect you to the app.** It gives you a *second, separate* daemon, and the distinction is the part worth internalising:

- **The session store is shared**, because it is a directory on disk. `biorouter session list` and `session export` see the app's conversations, and always did — that is why they need no daemon at all.
- **Live turns are not shared**, because they are process memory. `GET /sessions/running` answers from that daemon's own state (`state.active_turn_session_ids()`, `crates/biorouter-server/src/routes/session.rs:1065`), so a turn running in an app tab is invisible to your daemon: `watch` streams nothing, and `cancel` has no agent to stop.

Nor is two daemons over one store a configuration to *write* from. The lock that stops two agents trampling one conversation lives in the daemon that holds it, which is the same reason [`attach` beats `--resume`](#as-subcommands-you-type) on a live session — one daemon, one turn lock, or neither.

#### Making the app and the terminal share one daemon

There is a supported way to have both halves talk to the same process, and it works by turning the ownership around: *you* run the daemon, on a port and key you chose, and the app connects to it instead of spawning its own. Two hooks in the main process implement it, both keyed off `settings.externalBiorouterd`:

- `startBiorouterd` returns `connectToExternalBackend(dir, url)` — spawning nothing — when `enabled` and a `url` are both set (`biorouterd.ts:271`).
- `getServerSecret` returns `settings.externalBiorouterd.secret` in preference to the generated one when `enabled` and a `secret` are set (`main.ts:902`).

To use it:

1. Start the daemon yourself and keep it running with the user-action digest piped on stdin, as shown above (add `BIOROUTER_PORT=<port>` for anything other than 3000).
2. Add `"externalBiorouterd": { "enabled": true, "url": "http://127.0.0.1:3000", "secret": "<key>" }` to the app's `settings.json`, which lives in Electron's `userData` directory (`ui/desktop/src/utils/settings.ts:26`) — on macOS `~/Library/Application Support/Biorouter/settings.json`.
3. Restart the app. The setting is read once at launch.
4. Export the same `BIOROUTER_SERVER__SECRET_KEY` (and `BIOROUTER_PORT`, if not 3000) in the terminal you run `biorouter session` from.

The app's tabs and your `send`/`watch`/`attach`/`cancel` are then the same process, and you can steer from the terminal a conversation you are watching in the GUI. Two costs: the daemon is yours to keep alive — if it is unreachable at launch the app offers only *Disable External Backend & Retry* or *Quit* (`main.ts:1131`) — and the shared secret now sits in a plain settings file.

> **Note on step 2.** There *is* a settings component for this, `ui/desktop/src/components/settings/app/ExternalBackendSection.tsx`, rendering a "Biorouter Server" card with an enable switch, a URL field and a secret field. In this build it is not imported by `SettingsView` or anything else, so it does not appear anywhere in the app — hand-editing the file is currently the only way in. Everything it would have written is read normally by the main process, so the mechanism itself works; only its front door is missing.

Developers have a shortcut worth knowing and not shipping: with `BIOROUTER_EXTERNAL_BACKEND` set, the app skips its own daemon and connects to `http://127.0.0.1:3000` with the literal secret `test` (`biorouterd.ts:275`, `main.ts:906`) — which is what `just debug-ui` and `just debug-server` pair up. Convenient on a dev machine, and a fixed, published key everywhere else.

### What has no terminal equivalent

Exactly two capabilities, and both are declared rather than accidental. The mapping between the tools and the subcommands is not folklore — it is a compiled table in `crates/biorouter-cli/src/commands/workspace_parity.rs` whose rows are checked against the real command tree, against the advertised tool surface, and against the daemon's published routes. A new capability with no CLI row fails the build, and the number of declared exceptions is capped at two by a test.

- **Spawning.** It is a tool the model calls, and it already works inside `biorouter session`. There is nothing for a subcommand to add.
- **Reconfiguring another session's tools.** Out of scope for the terminal; `biorouter extension` and `biorouter skill` are machine-wide rather than session-scoped.

That gate is coarse by design, and it is worth knowing where it is coarse: `session cancel` covers only the *turn* scope of a three-scope tool, and `session export` dumps a whole transcript rather than offering the four views. The rows prove a subcommand **exists**, not that it is equivalent.

## When a tab does not appear

Most of the time the answer is one of the deliberate behaviours above — announce-only is on, the fan-out cap downgraded the child, or the window was already at six panes. Three honest caveats beyond those:

- **A subagent spawn does not wait for the window to answer.** When a conversation is opened directly, the agent parks on the renderer's reply and reports "NOT opened" with the reason if it was refused. A *subagent* announcement is fire-and-forget, deliberately: waiting would couple every spawn to the window, and one wedged window would stall a whole fan-out. So in the narrow case where the renderer refuses a subagent's tab, the agent may believe a tab opened when none did. The child still exists, still runs, and is still reachable from History and from a conversation read — check there before assuming the run failed.
- **A request for a separate window is reported as *requested*.** The window is asked for and the answer comes back before the window has actually been created, so a window that fails to appear is not distinguished from one that did.
- **A split can silently arrive as a plain tab.** The six-pane refusal above is announced; a second one is not. The renderer will not split a pane off a tab that is that pane's *only* tab — that would trade one pane for another and leave an empty one behind. So a `split` whose conversation ends up alone in its pane does not split: most often that is a conversation which already has a tab of its own in a pane by itself, which is simply focused instead. The planner asks the reducer first, so nothing pointless is dispatched, but the agent is told `opened` rather than `opened in split` and may not pass that difference on. If you asked for a pane and got a tab, drag the tab to a pane edge yourself.

## Related documentation

- [Session metadata contract](session-metadata-contract.md) — conversation ID, session kind, parent, and what makes something a subagent run.
- [Workspace Control capability](../extensions/built-in/workspace.md) — the user-facing reference: the two tiers, what each tool asks you first, the always-confirm rule, and how injected messages are labelled forever.
- [Workspace Control tool reference](workspace-control-tools.md) — the developer-facing contract for the same eight tools: exact arguments and defaults, every refusal string, the caps and clamps, and the cases where a tool reports success it did not earn.
- [Subagents](subagents.md) — the glass-box tab in full, steering and stopping a child, and configuring one from a workflow file.
- [Tool routing](tool-routing.md) — which tool the agent should reach for, and what separates workspace control from Chat Recall, Memory and the knowledge base.
- [Permission modes](../security/permission-modes.md) — the four modes, what each one still puts to you for approval, and how to switch between them. It does not discuss delegation; that gate is [above](#when-delegation-is-allowed).
- [biorouter CLI command reference](../cli/command-reference.md) — the rest of the `biorouter session` surface.
- [Agent workspace control (BR-71 design)](designs/agent-workspace-control.md) — the design of record behind this feature, including its permissions and abuse-resistance analysis.
