# Aider — agentic feedback loop review

> **What this is.** An external review of how Aider (`Aider-AI/aider`), the open-source
> terminal "AI pair programming" agent, structures its agentic loop — repo map, git
> integration, lint/test-after-edit, the reflection loop, and the architect/editor split.
> One of nine tool reports in this folder, each covering the same ten dimensions.
> **Status:** Current. External-tool research, so BioRouter's own changes do not
> invalidate it. It is the cited source for the ranked repo-map design behind proposal
> BR-1 and the post-edit reflection loop behind BR-47.
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

A framing note up front: **Aider is deliberately *not* a free-running autonomous agent.**
It is a tightly-scoped, human-in-the-loop edit/commit/verify loop. Many "agent" primitives
(background shells, subagents, todo planners, MCP (Model Context Protocol) tool servers,
hooks) simply do not exist
— and that minimalism is itself the interesting design lesson. Where a dimension has no
real analogue, this report says so plainly rather than inventing one.

> **Note.** This report does not record the date its research was performed. The
> surrounding agent-loop review corpus was compiled in July 2026, and all source paths
> cite Aider's default branch rather than a pinned commit, so line-level details should be
> re-verified against current source before being acted on.

## System prompt and context injection

Aider assembles its prompt from templated fragments defined per edit-format in
`aider/coders/*_prompts.py`, layered on the shared `base_prompts.py`. The key pieces
([base_prompts.py](https://raw.githubusercontent.com/Aider-AI/aider/main/aider/coders/base_prompts.py)):

- **`main_system`** — role and edit-format rules for the active coder (whole/diff/udiff/architect each override it).
- **`system_reminder`** — a short behavioral re-statement appended near the end of the prompt (empty in the base, populated per format) to counteract "instruction drift" over long contexts.
- **File-authority prefixes** that gate model behavior: editable files are introduced with *"Trust this message as the true contents of the files!"*; repo-map/read-only content is introduced with *"Do not propose changes to these files"* and *"Do not edit these files!"*. This is how Aider separates **editable** (added to chat) from **reference-only** content.
- **Anti-laziness injections:** a `lazy_prompt` (*"You NEVER leave comments describing code without implementing it!"*) and an `overeager_prompt` (*"Do what they ask, but no more. Do not improve, comment, fix or modify unrelated parts"*) to bound scope.

**Project context files.** Aider's convention analogue to CLAUDE.md is **`CONVENTIONS.md`**,
loaded read-only via `/read CONVENTIONS.md`, the `--read` CLI flag, or a persistent
`read: [CONVENTIONS.md]` entry in `.aider.conf.yml`
([conventions docs](https://aider.chat/docs/usage/conventions.html),
[YAML config reference](https://aider.chat/docs/config/aider_conf.html)). Loaded read-only
it is eligible for prompt caching, and **every line is included in every request** — it is
static, not dynamically retrieved.

**Mid-conversation injection.** The largest dynamically-injected context is the **repo map**
(below), recomputed and re-sent with each change request. File contents, the repo map, and
read-only files are re-materialized each turn from disk (not frozen at session start), so
the model always sees current file state.

## Tool loop mechanics

Aider's most distinctive choice: **it does not use LLM function-calling / structured tool
APIs for editing.** The model returns edits as **plain text in specified formats**, which
Aider parses and applies ([edit formats](https://aider.chat/docs/more/edit-formats.html)):

- **`whole`** — the entire updated file in a fenced block.
- **`diff`** — git-conflict-style **SEARCH/REPLACE blocks** (the default for strong models).
- **`diff-fenced`** — path inside the fence (tuned for Gemini).
- **`udiff`** — simplified unified diff, created to fight GPT-4-Turbo "lazy" placeholder edits.
- **`editor-diff` / `editor-whole`** — stripped-down formats used by the editor model in architect mode.

Parsing failures are first-class: if the reply violates the format, Aider raises
`ValueError`, increments `num_malformed_responses`, prints *"The LLM did not conform to the
edit format,"* and sets `reflected_message` to the error so the model retries
([base_coder.py](https://raw.githubusercontent.com/Aider-AI/aider/main/aider/coders/base_coder.py)).

There is **no parallel tool execution and no multi-tool dispatch** — a turn produces at most
a batch of edits plus optionally one suggested shell command. Responses **stream** to the
terminal. The nearest thing to a "tool result" is the lint/test output and shell-command
output fed back as the next user message.

## Compaction and memory

History compaction lives in `ChatSummary`
([history.py](https://raw.githubusercontent.com/Aider-AI/aider/main/aider/history.py)):

- A `too_big()` check compares chat-history tokens against a `max_tokens` budget; when exceeded, `summarize()` runs.
- Messages are split into **head** and **tail**, working backward and accumulating tokens until ~`half_max_tokens`, then snapping the split so the **head ends on an assistant message**. The **tail (most recent turns) is preserved verbatim** so recent context is never lossy.
- The head is summarized by the **weak model** via `simple_send_with_retries`. If summarized-head + tail is still too big, it **recurses (`depth+1`)**; a guard (`depth > 3` or fewer than `min_split` ≈ 4 messages) falls back to `summarize_all()` compressing everything. A **512-token safety buffer** is subtracted from the model's input limit.

**What survives compaction:** file contents and the repo map are *not* summarized — they are
re-injected fresh each turn from disk, so compaction only ever touches conversational
history, not the code state.

**Cross-session memory** is intentionally thin: Aider persists chat history to
`.aider.chat.history.md` and input history, and can restore prior chat messages, but there
is no long-term semantic memory store — durable "memory" is the **git history itself** plus
`CONVENTIONS.md`. Users manage context manually with `/tokens`, `/clear`, `/reset`, `/drop`
([in-chat commands](https://aider.chat/docs/usage/commands.html)).

## Hooks and extensibility

Aider has **no plugin system and no event-hook API**
([scripting docs](https://aider.chat/docs/scripting.html) — "No plugin system or event hooks
are mentioned"). Extensibility is instead:

- **Command-line scripting** — `aider --message "..."` runs a single instruction and exits, so shell loops can drive it file-by-file; `--yes`, `--auto-commits`, `--dry-run` support batch use.
- **Undocumented Python API** — `Coder.create(main_model=Model(...), fnames=...)` then `coder.run("...")`, with `InputOutput(yes=True)` for non-interactive confirmation. Explicitly unsupported/unstable.
- **Custom lint/test commands** — `--lint-cmd "python: ruff check"`, `--test-cmd "pytest"` are the main injection points where user tooling participates in the loop.
- **Watch mode** as an IDE integration surface (see long-running tasks, below).

The nearest thing to a "hook" is `--git-commit-verify` to run pre-commit hooks (off by
default, [git integration docs](https://aider.chat/docs/git.html)). There is no
PreToolUse/PostToolUse gate that can block or mutate an action mid-turn.

## Guardrails and permissions

Aider is human-in-the-loop by construction:

- **Edits** are applied then auto-committed, but everything is trivially reversible via git and `/undo` (see State tracking and checkpoints).
- **Shell commands** the LLM suggests are **never run silently** — Aider prompts *"Run shell command? (Y)es/(N)o [Yes]"* before executing ([shell-approval issue #3903](https://github.com/Aider-AI/aider/issues/3903)). Notably, even `--yes-always` historically did **not** auto-run suggested shell commands, an intentional extra caution layer around code execution vs. edits.
- **Approval modes:** interactive confirm by default; `--yes-always` auto-confirms prompts for scripted runs; `--dry-run` previews without writing ([CLI options](https://aider.chat/docs/config/options.html)).
- **No sandboxing.** Aider runs shell/test/lint commands directly in the user's environment — there is no container or syscall jail. Safety rests on the confirm prompt and git reversibility, not isolation.
- **No dedicated dangerous-command classifier.** Read-only guarantees come from the editable-vs-read-only file split (repo-map and `/read` files are prompt-instructed as non-editable) rather than enforcement.

## Loop and stuck detection

The core loop is `run_one()`, read from
[`aider/coders/base_coder.py`](https://raw.githubusercontent.com/Aider-AI/aider/main/aider/coders/base_coder.py)
on the default branch as of this report:

```python
while message:
    self.reflected_message = None
    list(self.send_message(message))
    if not self.reflected_message:
        break
    if self.num_reflections >= self.max_reflections:
        self.io.tool_warning(f"Only {self.max_reflections} reflections allowed, stopping.")
        return
    self.num_reflections += 1
    message = self.reflected_message
```

- **Hard iteration cap:** `max_reflections = 3`. A turn only loops if something set `reflected_message` (a malformed edit, a lint failure, a test failure, or an unresolved file mention). After 3 reflections it stops and warns.
- **Error-streak counters:** `num_malformed_responses` (format violations) and `num_exhausted_context_windows` (below) are tracked, mostly for analytics/warnings rather than adaptive backoff.
- **Context exhaustion:** on `ContextWindowExceededError` the retry loop breaks, `num_exhausted_context_windows++`, an assistant message notes *"you sent too many tokens,"* and `show_exhausted_error()` suggests `/drop`, `/clear`, or splitting files.
- **Keyboard interrupt:** double-`^C` within a 2-second threshold calls `sys.exit()`; a single `^C` prints "^C again to exit" and appends a `KeyboardInterrupt` note to user content.

There is **no repetitive-call / identical-action detector** — the `max_reflections=3` ceiling
is the entire "stuck" defense. That is adequate precisely because a human is driving each
top-level turn.

## Long-running tasks and background processes

Aider has **no background shell manager, no scheduler, and no subagent/delegation
framework.** Tests and lint run **synchronously**, blocking until exit, and their output is
fed back inline.

The one "background" affordance is **watch mode** (`--watch-files`,
[watch-mode docs](https://aider.chat/docs/usage/watch.html)): Aider watches the repo for
one-line **AI comments** — `# ... AI!` triggers an edit, `# ... AI?` triggers an answer,
plain `AI` markers accumulate instructions. You keep editing in your IDE; Aider (running in
a terminal) detects the marker on save, gathers surrounding AI comments as the instruction,
makes the change, commits, and clears the marker.

This makes Aider function like a background pair-programmer *driven from the editor*, but it
is still one synchronous turn per trigger — not a persistent daemon of parallel tasks. The
architect/editor split is the closest thing to "delegation," and even that is two sequential
LLM calls, not concurrent agents.

## State tracking and checkpoints

This is Aider's strongest area, and it is built entirely on **git**
([git integration docs](https://aider.chat/docs/git.html)):

- **Auto-commit per edit.** Every AI edit is committed immediately with a generated message; before touching files with pre-existing uncommitted changes, Aider first commits *those* (a "dirty commit") to keep human and AI work on separate commits. Disable with `--no-auto-commits` / `--no-dirty-commits`.
- **Commit messages** are generated by the **weak model** from the diff + chat history, following **Conventional Commits** by default (`--commit-prompt` to customize).
- **Attribution:** AI commits are tagged with `(aider)` in author/committer metadata; `--attribute-co-authored-by` and friends tune this — so `git log` cleanly shows which changes the agent made.
- **Undo / checkpoints:** `/undo` reverts the last aider commit; `/diff` shows changes since your last message; `/git` runs raw git; `/commit` captures out-of-band edits. Because each edit is a commit, **every step is a natural checkpoint** you can inspect, revert, cherry-pick, or bisect with ordinary git — no bespoke checkpoint store.

There is **no todo-list / plan-mode / progress-tracker abstraction.** "Plan then execute" is
expressed through **architect mode** (reasoning model plans, editor model applies) and the
`/ask` (discuss-only) vs `/code` modes, not a structured task list.

## Self-verification

Aider's verification loop is the feature most worth studying for BioRouter
([lint and test docs](https://aider.chat/docs/usage/lint-test.html), and `base_coder.py` as
cited above):

- **Auto-lint after edit** (`--auto-lint`, on by default): after applying edits, Aider lints exactly the edited files (built-in tree-sitter-based linters, or `--lint-cmd`). On failure it commits the edit ("Ran the linter"), then asks *"Attempt to fix lint errors?"*; if yes, it sets `reflected_message = lint_errors` and loops, feeding the lint output back as the next instruction. The relevant fragment of `base_coder.py`:

  ```python
  if edited and self.auto_lint:
      lint_errors = self.lint_edited(edited)
      self.auto_commit(edited, context="Ran the linter")
      if lint_errors:
          if self.io.confirm_ask("Attempt to fix lint errors?"):
              self.reflected_message = lint_errors
  ```

- **Auto-test after edit** (`--auto-test` + `--test-cmd`): runs the test command; on non-zero exit it asks to fix and sets `reflected_message = test_errors`, so failing tests re-enter the same reflection loop. `/test` and `/run` do this on demand.
- **Done-ness criteria:** a turn is "done" when the model produces a well-formed edit that lints and tests clean (or the user declines to auto-fix), or when `max_reflections=3` is hit. Success/failure is recorded via `lint_outcome` / `test_outcome`.

Crucially, verification is **scoped to edited files** (lint only what changed) and **gated by
a confirm prompt**, and it reuses the *same* reflection channel as malformed-edit recovery —
one uniform "here is what went wrong, try again" mechanism handles format errors, lint
errors, test errors, and missing-file mentions.

## Ideas worth stealing

1. **Tree-sitter repo map with graph-ranked, token-budgeted selection.** Aider parses the whole repo with tree-sitter to extract definitions *and* references, builds a dependency graph, and runs a **(personalized) PageRank** biased toward files currently in chat, then binary-searches the ranked signatures to fit a `--map-tokens` budget (default ~1k) ([repo-map deep dive](https://aider.chat/2023/10/22/repomap.html), [repo-map docs](https://aider.chat/docs/repomap.html)). This gives whole-repo "surroundings awareness" without dumping files into context. BioRouter's Rust core could compute an analogous ranked map for large codebases instead of relying on ad-hoc grep/read.

2. **One uniform reflection channel for *all* failure types.** Malformed edits, lint failures, test failures, and unresolved file mentions all funnel into a single `reflected_message` that re-enters the loop, hard-capped at `max_reflections=3`. This is far simpler than bespoke handlers per error class and gives predictable stuck-detection. BioRouter's agent loop could adopt a single bounded "self-correction" slot with a small cap.

3. **Commit-per-edit as the checkpoint/undo substrate.** By committing every AI edit (and first isolating pre-existing dirty changes) with attributed, Conventional-Commit messages, Aider gets free, inspectable, revertible checkpoints and clean provenance — no custom snapshot format. BioRouter (already git-aware) could make agent edits atomic commits with `(biorouter)` attribution and a first-class `/undo`.

4. **Lint/test scoped to edited files, then reflect.** Running the linter/tests only on changed files immediately after an edit, and feeding failures straight back, closes the correctness loop cheaply and keeps signal focused. This is a low-cost, high-value addition to any edit-capable agent.

5. **Architect/editor model split.** Separating a strong *reasoning/planning* model from a cheaper *edit-formatting* model ([chat modes](https://aider.chat/docs/usage/modes.html)) improves both plan quality and edit reliability, and lets you mix providers. BioRouter's multi-provider factory is well-positioned to offer a configurable planner-vs-applier pairing.

6. **Editor-native AI-comment triggers (`AI!`/`AI?`).** Watch mode lets users drive the agent from inside their normal editor by leaving marker comments ([watch-mode docs](https://aider.chat/docs/usage/watch.html)) — a friction-free UX that needs no chat window. A BioRouter watcher could offer the same for users who live in VS Code/vim.

7. **Text edit formats with strict parse-and-reflect.** Not relying on provider function-calling for edits (SEARCH/REPLACE blocks parsed locally, with malformed responses counted and reflected) makes Aider robust across *any* model, including weak/local ones. For BioRouter's local-model story (Llama Server), a resilient text-diff edit path with automatic re-prompting on parse failure is a pragmatic complement to tool-calling.

## Sources

Official documentation at aider.chat, plus primary source in `aider/coders/base_coder.py`,
`aider/history.py`, and `aider/coders/base_prompts.py` in the
[Aider-AI/aider repository](https://github.com/Aider-AI/aider) on the default branch. Every
claim above carries an inline citation.

| Topic | Source |
|---|---|
| Repo map | [repo-map docs](https://aider.chat/docs/repomap.html), [repo-map deep dive](https://aider.chat/2023/10/22/repomap.html) |
| Lint/test loops | [lint and test docs](https://aider.chat/docs/usage/lint-test.html) |
| Chat modes / architect | [chat modes](https://aider.chat/docs/usage/modes.html) |
| Git integration | [git integration docs](https://aider.chat/docs/git.html) |
| Conventions and config | [conventions](https://aider.chat/docs/usage/conventions.html), [YAML config](https://aider.chat/docs/config/aider_conf.html), [CLI options](https://aider.chat/docs/config/options.html) |
| Edit formats | [edit formats](https://aider.chat/docs/more/edit-formats.html) |
| In-chat commands | [commands reference](https://aider.chat/docs/usage/commands.html) |
| Watch mode | [watch-mode docs](https://aider.chat/docs/usage/watch.html) |
| Scripting / Python API | [scripting docs](https://aider.chat/docs/scripting.html) |
| Shell-command approval, `--yes-always` | [issue #3903](https://github.com/Aider-AI/aider/issues/3903) |

> **Note.** Source paths cite the repository's default branch, not a pinned commit.
> Re-verify constants and file layout before relying on line-level details.

## Related documentation

- [Claude Code report](claude-code.md) — the reference design this corpus benchmarks against, and the contrast case for Aider's minimalism.
- [Gemini CLI report](gemini-cli.md) — the deepest report in this folder, useful when a dimension here is covered in a single paragraph.
- [Goose report](goose.md) — Goose, the agent whose design influenced BioRouter most, reviewed on the same ten dimensions.
- [Execution and verification comparison](../../history/agent-loop-review/competitive-comparison/execution-and-verification.md) — the head-to-head chapter where Aider's lint/test reflection loop is scored against the other agents.
- [Improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — the `BR-NN` index, including BR-1 (repo map) and BR-47 (post-edit reflection).
