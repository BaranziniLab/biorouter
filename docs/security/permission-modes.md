# Permission modes

> **What this is.** A guide to the four permission modes that decide how much autonomy
> BioRouter has when modifying files, using extensions, and performing automated actions, and
> how to switch between them in the desktop app and the CLI.
> **Status:** Current. The four modes and their CLI values match the `/mode` slash command and
> the `BIOROUTER_MODE` setting.
> **Audience:** end users.

BioRouter's permissions determine how much autonomy it has when modifying files, using
extensions, and performing automated actions. By selecting a permission mode you control how
BioRouter interacts with your development environment. The mode applies to the whole session
and takes effect immediately — you can change it before or during a session.

> **Note.** A permission mode is the *user-owned* tier. An administrator can deploy a
> [managed policy](managed-policy.md) that overrides every mode described here, including
> Completely Autonomous, by forcing specific tools to be denied or to require approval. If a
> tool is blocked with *"Blocked by your organization's managed policy,"* that is the managed
> tier, not your mode setting.

## The four modes

The **Mode** column below is the name shown in the desktop app; **CLI value** is what you pass
to `/mode`; **Configure name** is how the mode appears in `biorouter configure`.

| Mode | CLI value | Configure name | Description | Best for |
|---|---|---|---|---|
| **Completely Autonomous** | `auto` | Auto Mode | BioRouter can modify files, use extensions, and delete files **without requiring approval** | Users who want **full automation** and seamless integration into their workflow |
| **Manual Approval** | `approve` | Approve Mode | BioRouter **asks for confirmation** before using any tools or extensions (supports granular tool permissions) | Users who want to **review and approve** every change and tool usage |
| **Smart Approval** | `smart_approve` | Smart Approve Mode | BioRouter uses a risk-based approach to **automatically approve low-risk actions** and **flag others** for approval (supports granular tool permissions) | Users who want a **balanced mix of autonomy and oversight** based on the action's impact |
| **Chat Only** | `chat` | Chat Mode | BioRouter **only engages in chat**, with no extension use or file modifications | Users who prefer a **conversational AI experience** for analysis, writing, and reasoning tasks without automation |

> **Warning.** Completely Autonomous (`auto`) is applied by default.

> **Note.** In Manual Approval and Smart Approval modes you will see "Allow" and "Deny" buttons
> in your session windows during tool calls. BioRouter only asks for permission for tools it
> deems are 'write' tools — for example any 'text editor write', 'text editor edit', or
> 'bash - rm, cp, mv' command. Read/write approval makes a best-effort attempt at classifying
> read or write tools, and that classification is interpreted by your LLM provider.

## What still asks, whatever your mode

A small, fixed set of actions is put to you for approval even in Completely Autonomous. These
are not mode settings and there is no toggle for them — they are the operations where acting
without asking would be irreversible, or would move your data somewhere you cannot see.

| What | What you see | Why it is not left to the mode |
|---|---|---|
| Writing or deleting under a protected system directory, your home directory itself, or a credential store (`~/.ssh`, keychains, browser logins, launchd) | *"Sensitive system operation in Fully-Automatic mode"* | Ordinary file work in Auto stays promptless; these are the writes you cannot undo. Reads are not affected. |
| Reading a **global** memory category | *"Cross-session memory read"*, naming the category | Global memories are shared by every BioRouter session on the computer, in every project. A session reading one is seeing text another conversation wrote, so you decide each time. |
| Saving or deleting a **global** memory | *"Cross-session memory write"* / *"Cross-session memory change"*, naming the category | Marking a note global is what makes it follow you into every other project. |
| Clearing **all** global memories | *"Deletes every global memory"* | It cannot be undone. |

Four related notes on memory:

- **You can read the store yourself.** These approvals name a category; **Settings → Chat →
  Memory** is where you see what is in it, and delete anything you would rather BioRouter did
  not have. See [seeing and deleting what was remembered](../extensions/built-in/memory.md#seeing-and-deleting-what-was-remembered).
- **Project-local memories never prompt.** They live in your project's `.biorouter/memory`, so
  they only reach a session you opened that directory in. Nothing on this page changes them.
- **There is no "read all my global memories" call.** BioRouter refuses it. Global memory is
  read one named category at a time, precisely so each approval names something you can decide
  about — an approval covering the whole store would be a prompt you cannot answer informedly.
  Every global memory is still reachable, one approved category at a time.
- **There is no way round the card, either.** The global store is a directory of text files, so
  the approval would be worth little if the same files could simply be opened. They cannot:
  the file tools refuse any path inside the store and point at the memory tools instead; a
  script running under `execute_code` and the `/agent/call_tool` API are both refused, because
  neither can show you the operation; and BioRouter's memory server started on its own — outside
  the app, for another program — serves project-local memory only. In each case the machine-wide
  store is reached the one way that asks you first.

Two things these approvals are guaranteed against, so that "put to you" means what it says:

- **Nothing can answer them for you.** In particular a `PermissionRequest`
  [hook](../agent-loop/hooks/hooks-reference.md) returning `allow` — the automation you write to
  stop being prompted for routine calls — is ignored on these, and the card is shown anyway. A
  hook can still *deny* anything.
- **They never reopen a refusal.** If the same call was already refused — by your own "never
  allow" for that tool, by an administrator's [managed policy](managed-policy.md), or by the
  always-on catastrophic-command block — it stays refused and no card appears. An approval prompt
  is never a second chance at something that was already denied.

An administrator's [managed policy](managed-policy.md) can add further tools to this list.

## Scripts: every call is decided on its own

With the [Code Execution capability](../extensions/built-in/code-execution.md) on — the default
— the model reaches most tools by writing a short script (`code_execution__execute_code`) rather
than calling each tool directly. Your mode and your per-tool settings still apply to **every
call the script makes, under that tool's own name**:

- In **Manual Approval** a script's `developer__shell` call gets its own card, naming
  `developer__shell` and showing the command, even when the script itself is on your
  **Always allow** list. **Always allow** or **Never allow** for `developer__shell` applies
  inside a script exactly as it does outside one.
- In **Smart Approval** each call is graded by that tool's own risk annotations, so a read-only
  lookup runs and anything else asks.
- In **Completely Autonomous** the calls run without a card, apart from the operations in the
  table above, which ask in every mode — inside a script too.
- A refused or declined call does not run; the script receives it as an error it can catch.

So an **Always allow** entry for `code_execution__execute_code` means "do not ask me before
running a script" and nothing more — it is not a grant for the tools the script calls. It is
also never a default: the entry exists in your `permission.yaml` only if you added it. See
[the Code Execution guide](../extensions/built-in/code-execution.md#permissions-every-call-a-script-makes-is-decided-on-its-own)
for the full table, including what **Always allow** on a script's card records.

## Changing the mode in the desktop app

You can change modes before or during a session, and the change takes effect immediately.

From the chat window, click the mode button in the bottom menu and pick a mode.

Or, from Settings:

1. Click the sidebar button in the top-left to open the sidebar.
2. Click the `Settings` button on the sidebar.
3. Click `Chat`.
4. Under `Mode`, choose the mode you'd like.

## Changing the mode in the CLI

To change modes mid-session, use the `/mode` command:

- Autonomous: `/mode auto`
- Smart Approve: `/mode smart_approve`
- Approve: `/mode approve`
- Chat: `/mode chat`

To set the default mode, use `biorouter configure`:

1. Run the following command:

   ```bash
   biorouter configure
   ```

2. Select `biorouter settings` from the menu and press Enter.

   ```text
   ┌ biorouter-configure
   │
   ◆ What would you like to configure?
   | ○ Configure Providers
   | ○ Add Extension
   | ○ Toggle Extensions
   | ○ Remove Extension
   | ● biorouter settings (Set the biorouter mode, Tool Output, Tool Permissions, Experiment, biorouter workflow github repo and more)
   └
   ```

3. Choose `biorouter mode` from the menu and press Enter.

   ```text
   ┌   biorouter-configure
   │
   ◇  What would you like to configure?
   │  biorouter settings 
   │
   ◆  What setting would you like to configure?
   │  ● biorouter mode (Configure biorouter mode)
   │  ○ Router Tool Selection Strategy 
   │  ○ Tool Permission 
   │  ○ Tool Output 
   │  ○ Max Turns 
   │  ○ Toggle Experiment 
   │  ○ biorouter workflow github repo 
   │  ○ Scheduler Type 
   └
   ```

4. Choose the biorouter mode you would like to configure.

   ```text
   ┌   biorouter-configure
   │
   ◇  What would you like to configure?
   │  biorouter settings
   │
   ◇  What setting would you like to configure?
   │  biorouter mode
   │
   ◆  Which biorouter mode would you like to configure?
   │  ● Auto Mode (Full file modification, extension usage, edit, create and delete files freely)
   |  ○ Approve Mode
   |  ○ Smart Approve Mode    
   |  ○ Chat Mode
   |
   └  Set to Auto Mode - full file modification enabled
   ```

## Related documentation

- [Managed enterprise policy](managed-policy.md) — the admin-owned tier that overrides every
  mode on this page.
- [CLI command reference](../cli/command-reference.md) — the `/mode` slash command alongside
  the rest of the CLI surface.
- [Configuration file reference](../configuration/config-file-reference.md) — where the
  persisted mode and `permission.yaml` live.
- [Hooks reference](../agent-loop/hooks/hooks-reference.md) — lifecycle hooks, which gate tool
  calls independently of the permission mode.
- [Code Execution capability](../extensions/built-in/code-execution.md) — scripts, and how each
  call inside one is decided.
- [Data privacy and patient data](data-privacy-and-phi.md) — the other decision that matters
  before a session touches sensitive data.
