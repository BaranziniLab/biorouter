# Custom system prompts

> **What this is.** How a CLI session builds its system prompt when you add your own text: `--system`, a workflow's instructions, a prompt file in `BIOROUTER_SYSTEM_PROMPT_FILE_PATH` in its two modes, and the block a non-interactive run receives.
> **Status:** Current (1.94.6, issues [#539](https://github.com/BaranziniLab/biorouter/issues/539) and [#544](https://github.com/BaranziniLab/biorouter/issues/544)).
> **Audience:** end users who script the CLI, and developers working on the prompt (`crates/biorouter/src/agents/prompt_manager.rs`).

Biorouter's stock system prompt (`crates/biorouter/src/prompts/system.md`) tells the model which
tools it has this turn, how to use each one, how to carry a task through and verify it, what the
frame around every tool result means, and the safety rules. There are three ways to put your own
text in front of the model. Two of them add to the stock prompt. The third stands in for it, and
since 1.94.6 it keeps a small core of Biorouter's own text after yours unless you ask it not to.

## Adding text to the stock prompt

`biorouter run --system TEXT` adds your text after the stock prompt, under
`# Additional Instructions:`. It works with every input: `--text`, `--instructions FILE` and
`--instructions -` (stdin). Before 1.94.6 the `--instructions FILE` form dropped `--system`
without saying so; it now keeps it. A workflow's `instructions` are added the same way, and
`--system` cannot be combined with `--workflow`.

Use this when you want Biorouter's full prompt plus your own sentences, for example a
benchmark that adds task rules on top of the shipped behaviour.

## Standing in for the stock prompt with a file

`BIOROUTER_SYSTEM_PROMPT_FILE_PATH` names a file whose text takes the place of `system.md` in
CLI sessions that run in the CLI's own process: `biorouter session`, `run`, `term run` and
`bench`. The desktop app and `--shared-daemon` sessions do not read it. The file is a template
with the same variables as the stock prompt, so it can use `{{ current_date_time }}` or loop over
`capabilities`.

`BIOROUTER_SYSTEM_PROMPT_OVERRIDE_MODE` decides what stays:

| Mode | What the model receives |
|---|---|
| `compose` (the default) | Your file, then Biorouter's core, then the additional instructions (`--system`, the CLI block, hints). |
| `replace` | Your file, then the additional instructions. Nothing of the stock prompt remains. This is how every prompt file behaved before 1.94.6. |

Any other value prints a warning and uses `compose`, the mode that keeps the tool roster and the
safety rules.

### What the core keeps

The core is `crates/biorouter/src/prompts/system_core.md`. It holds:

- the tool state for this turn, shared word for word with the stock prompt
  (`system_tool_state.md`): each enabled capability and loaded extension, its effective tool
  roster, the "do not call or claim" line for tools it was not offered, and its own
  instructions;
- the task rules: carry a task through to completion, read before editing, do not fabricate
  paths, APIs or results, and verify before reporting a task complete;
- one waiting line for each waiting tool on the roster (`shell_status`,
  `code_execution__wait_for`), and none when neither is offered;
- what the `<tool-output untrusted="true">` frame and the `[BIOROUTER GUARDRAIL]` line mean;
- the safety rules: no secrets, no destructive or irreversible commands unless asked, no
  fabricated facts, figures or citations.

It leaves out the identity paragraph, the date, the checklist rules, the ambiguity and
delegation rules, the batching and data-handling advice, tool routing, response formatting and
the desktop control section. Put any of those you want in your file. Each capability's own
instructions still arrive through the tool state, so the checklist tools, for example, still
explain themselves.

### Developer guidance follows the roster

The Developer capability's own instructions describe all of its tools. When a session offers
only some of them (for example `available_tools: [shell]` in `config.yaml`), the core replaces
that text with a sentence for each tool on the roster, followed by the environment facts the
capability reported: the operating system, the working directory, the shell, the container
flag and, inside a git repository, the version control block. So a `shell`-only session is never
told to prefer `text_editor` or to wait with `shell_status`. A session with the whole Developer
roster keeps the capability's text unchanged. The stock prompt is not narrowed this way.

### When the file does not render

A file that is not a valid template is dropped with a warning in the log, and a one-sentence
Biorouter identity takes its place. In `compose` mode the core still follows it, so a typo in
the file never removes the tool roster or the frame rules. In `replace` mode the one sentence is
all that remains, as before.

### What a prompt file does not change

- Subagents always replace: their prompt (`subagent_system.md`) already carries its own frame
  rules and never gains the core.
- On a small local model the small-model overlay follows the core in `compose` mode, as it
  follows the stock prompt. A `replace` file stays complete as written and gets no overlay.
- Tool descriptions travel separately from the system prompt, so neither mode changes them.

## What a non-interactive run is told

Every CLI session gets one block of CLI guidance among its additional instructions, so it
reaches the model in both prompt file modes. Which block depends on how the session runs:

| Session | Block |
|---|---|
| `biorouter session`, `run --interactive` | The terminal block: keep replies short, the slash commands and the keyboard shortcuts. |
| `run --text`, `run --instructions`, `term run`, `bench` with `text` output | The terminal format sentence, then the headless block. |
| `run --output-format json` or `stream-json` | The headless block alone. |

The headless block says that nobody can answer questions or approve actions, that the turn ends
at a reply without a tool call (so keep working and work around a failed step), that requested
files should be verified with a tool call before the final reply, that a file must not be
claimed unless a tool call wrote it, and that brevity applies to chat replies, not to files. It
says the turn ends rather than the run: a reply cut at the length limit, a workflow's final
output tool and the done gate can each send the model back to work. The text is in
`crates/biorouter-cli/src/session/prompt.rs`.

## Related documentation

- [Environment variables](../configuration/environment-variables.md): the two prompt file variables beside the other session settings.
- [CLI command reference](../cli/command-reference.md#run-options): `run`'s `--system`, `--instructions` and `--output-format`.
- [Subagents](subagents.md): the prompt a delegated task runs under.
- [Tool routing](tool-routing.md): the routing guidance the stock prompt carries and the core leaves out.
