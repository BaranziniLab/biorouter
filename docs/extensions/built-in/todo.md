# Todo capability

> **What this is.** User guide to the built-in Todo capability, which makes BioRouter break multi-step work into a tracked checklist and report progress as it goes.
> **Status:** Current. The capability is enabled by default, so no manual setup is normally needed. The rules under [What BioRouter enforces](#what-biorouter-enforces) are enforced by the agent loop as of 2026-09-11; before that they were advice in the system prompt that nothing checked.
> **Audience:** end users.

The Todo capability keeps BioRouter organized on long tasks. When your request has several steps, BioRouter writes a checklist before it starts, ticks items off as it works, and does not end the turn with items left open unless it tells you which ones and why. The checklist appears in the chat summary's **To Do** section as soon as it exists, so you can see where BioRouter is rather than waiting for a single opaque answer.

## What BioRouter enforces

For a request with several steps, keeping the checklist is not left to the model's judgement. Three rules apply, and each one is bounded so that a model with a good reason can still proceed:

1. **Plan first.** When your message reads as several steps and the chat's checklist is empty, BioRouter tells the model, for that turn, to write the checklist with `todo_write` before doing anything else.
2. **One redirect.** If the model reaches for another tool anyway, BioRouter refuses that first call (or that first batch of parallel calls) and points it back at `todo_write`. The refused call never runs, and the model sees why. A batch that writes the checklist alongside its other calls is not refused. This happens **once per turn**: if the model repeats the call — for instance after explaining why a checklist is not worth keeping here — it runs.
3. **No silent stop.** When a turn created or changed the checklist, BioRouter does not let it end while items are unfinished, unless the model's final message names each open item, by its `#N` id or by its text. Otherwise BioRouter sends the model back to finish, and shows you a notice saying which items are open. It does this at most five times per turn, and the count does not reset when the model runs tools. After the fifth time the turn ends anyway, with a notice that items were left open.

The model is asked to say *why* an item is left open, but only the naming is checked. A mechanical check cannot tell a reason from a status recap; a named item is one you can see was not done, which is what the rule protects.

### What counts as "several steps"

- A numbered or bulleted list of actions, such as `1. create a temp dir 2. write hello.txt into it 3. count its bytes 4. delete it`, or a list of three or more items introduced by an instruction ("Build a page with: 1. a header 2. a pricing table 3. a footer").
- Three or more instructions in a row ("Download the CSV, clean the missing values, and plot the distribution").
- Two or more instructions joined by "then", "after that", "afterwards", "finally", "followed by" or "steps" ("Create the directory, then write hello.txt into it").

The detection is deliberately conservative, and it reads English cues only. A question about how to do something ("How do I create a venv and then install the dependencies?") never counts, and neither does text inside code blocks or a single instruction. When the detection does not fire, the model can still keep a checklist on its own, and the system prompt asks it to.

### When none of this applies

- The Todo capability is disabled, or `todo_write` is not on the model's tool list for this chat.
- Chat mode, where no tools run.
- Subagents. A delegated task's turn ends with an observe-only `SubagentStop` rather than a blockable stop, and its instructions come from the parent model.
- The Claude Code and Codex providers. Their tool calls reach BioRouter through the coding-agent bridge rather than BioRouter's own loop.
- Slash commands.

While a `/goal` is active, the goal's evaluator decides when a turn may end, so the "no silent stop" rule stands aside; the first two rules still apply.

The system prompt describes these rules only in a chat where they are enforced.

> **Note.** This capability is **enabled by default**. Its internal registration still uses the legacy `PlatformExtensionDef` type; that storage name does not make it an installed extension. The configuration walkthrough below is only needed if you previously disabled it, or want to confirm its state.

## Configuration

1. Run the `configure` command:

   ```bash
   biorouter configure
   ```

2. Choose `Toggle Extensions`, then confirm `todo` is enabled:

   ```text
   ┌   biorouter-configure
   │
   ◇  What would you like to configure?
   │  Toggle Extensions
   │
   ◆  Enable capabilities and extensions: (use "space" to toggle and "enter" to submit)
   │  ● todo
   └  Extension settings updated successfully
   ```

## Available tools

These five tools stay directly callable when Code Execution mode is on. Most other tools are then reached only by writing a script, and a checklist the model could only update from inside a script was one it did not keep.

| Tool | Description |
|------|-------------|
| `todo_write` | Replace the entire checklist with a markdown checklist. Used to seed the initial list. It discards every existing item and renumbers the `#N` ids, so the tools below are what BioRouter uses to change a list that already exists. |
| `todo_expand` | Break one coarse item into the concrete steps it turned out to be, **in place**. The steps take that item's position, so the list still reads in the order the work happens, and no other item is renumbered. |
| `todo_add` | Add pending items without rewriting the existing ones — appended by default, or inserted directly after a given `#N` id. Each new item gets a fresh id. |
| `todo_update` | Update a single item by its `#N` id — change its status, its text, or both — without touching the rest of the list. |
| `plan_write` | Set or update the living plan: a step-by-step plan kept current as work proceeds, re-injected into the model's context each turn alongside the checklist. An empty string clears it. |

## Checklist item states

Items carry one of four states, written in the familiar markdown checkbox syntax:

| Marker | State |
|--------|-------|
| `- [ ] task` | pending |
| `- [~] task` | in progress |
| `- [!] task` | blocked |
| `- [x] task` | completed |

The `[~]` in-progress marker is what makes the checklist useful while a long task is still running: it shows you which single item BioRouter is working on right now.

`[!] blocked` is for work that cannot proceed until something outside BioRouter's
control arrives — most often an answer from you. It is worth watching for: a
blocked item is usually BioRouter telling you it is waiting on a decision.

## When a task turns out to be bigger than it looked

Plans start coarse. An item like "do the actual work" often turns out to be ten
steps once BioRouter gets to it, and where those steps land matters: appended to
the end, they would sit *after* "test and verify", and the checklist would stop
describing the real order of the work.

`todo_expand` is the operation for this. The steps replace the coarse item where
it already sits, and by default that item stays as a heading with its steps
nested under it:

```md
- [x] Survey the inputs
- [x] Set up the workspace
- [~] Do the actual work
  - [x] Write the parser
  - [~] Wire it into the pipeline
  - [ ] Handle the error cases
- [ ] Test and verify
```

Nesting stops at one level, on purpose: the checklist is there to show progress,
not to become a project taxonomy. Every id already on the list keeps the number
it had, so anything BioRouter was tracking still points at the same task.

## Example usage

In this example BioRouter fixes formatting inconsistencies across two documentation files, and uses the Todo capability to work through them systematically.

> **Note.** This example is self-referential — it shows BioRouter editing BioRouter's own documentation, so its checklist items refer to an internal docs style guide ("reference pattern", "reference format") that you do not need to know. Read it for the *shape* of the checklist, not for its content. Note also that every item is already ticked `[x]`, because the transcript was captured after the task finished; during a real run you would see a mix of `[ ]`, `[~]` and `[x]`.

### BioRouter prompt

```text
I need to update our documentation to fix formatting inconsistencies between two tutorial files. There are several sections that don't match the expected pattern. Can you help me systematically work through these issues and track our progress?
```

### BioRouter output

```md
I'll help you fix the documentation inconsistencies systematically. Let me break this down into organized tasks and track our progress.

## Fix Documentation Tutorial Inconsistencies

### Missing Components (vs Reference Pattern)
- [x] Add video embed at the top
- [x] Add LLM info box in Example Usage section  
- [x] Remove extra sections not in reference pattern

### Configuration Section Issues
- [x] Change terminology to match reference format
- [x] Fix CLI configuration section format
  - [x] Show exact command flow
  - [x] Include proper menu output with syntax highlighting
  - [x] Remove vague instructions

### Introduction/Description Issues
- [x] Simplify intro paragraph to match reference style
- [x] Fix tutorial description line to match pattern
- [x] Fix capitalization consistency throughout

### Example Structure Issues
- [x] Simplify example to match reference format
  - [x] Use clear "Prompt" and "Output" sections
  - [x] Add timing information
  - [x] Show actual response in proper code blocks
- [x] Use focused, achievable example
- [x] Show clear before/after states

### Formatting Consistency
- [x] Ensure code blocks use proper syntax highlighting
- [x] Match tone and structure of reference
- [x] Use consistent terminology throughout
- [x] Fix capitalization issues

## Documentation Update Complete! ✅

All tutorial inconsistencies have been resolved:

✅ **Structure aligned** - Matches reference pattern exactly
✅ **Components added** - Video embed, info boxes, proper tabs  
✅ **Terminology fixed** - Consistent language throughout
✅ **Example updated** - Simple, focused, with clear timing
✅ **CLI instructions** - Exact command flow with highlighting
✅ **Formatting consistent** - Proper code blocks and syntax

The documentation now follows a consistent pattern and provides a clear, organized experience for users.
```

## Related documentation

- [Extensions and skills guide](../extensions-and-skills-guide.md) — how to enable, disable and configure extensions generally.
- [Installation](../../getting-started/installation.md) — the list of capabilities enabled out of the box.
- [Subagents](../../agent-loop/subagents.md) — the other mechanism for structuring long, multi-step work.
- [Context engineering](../../agent-loop/context-engineering.md) — how the checklist and living plan are re-injected into the model's context each turn.
- [Hooks reference](../../agent-loop/hooks/hooks-reference.md) — where the "no silent stop" check sits among the other checks that run when the agent tries to finish a turn.
- [Code Execution capability](code-execution.md) — the mode that routes most tools through scripts, and the tools it leaves directly callable.
