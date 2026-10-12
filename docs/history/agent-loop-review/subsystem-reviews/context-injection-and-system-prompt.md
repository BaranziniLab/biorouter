# Context injection and system prompt construction — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents how the agent assembles the system prompt and the message array — the three injection cadences, the MOIM ambient-context block, hint files and `@import`, and MCP instruction rendering — and critiques the text of `system.md` and `desktop_prompt.md`.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose findings were then implemented. Gap #1 (no context budget) was fixed by BR-2, gaps #2 and #3 (the frozen clock and MOIM accumulation) by BR-5, gap #7 (uncapped skill inlining) by BR-8, gap #10 (one prompt for 43+ providers) by BR-3, and the missing repo map by BR-1. Read it as the record behind those changes, not as current prompt documentation.
> **Audience:** developers working on prompt construction, context injection, or extension instructions.

**MOIM** is BioRouter's per-action ambient-context block — a fresh `<info-msg>` user message carrying the current time, working directory and each platform extension's contribution, re-injected before every provider call. The acronym is never expanded in the codebase; this review treats it as "message of the moment" and describes it fully under "What MOIM is, where and why it injects". Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what sibling reviews cite as `context-injection.md gap #N` (the file's former name).

## Scope and files reviewed

Read in full: `crates/biorouter/src/agents/prompt_manager.rs`,
`crates/biorouter/src/agents/moim.rs`, `crates/biorouter/src/prompt_template.rs`,
`crates/biorouter/src/hints/{load_hints.rs,import_files.rs,mod.rs}`, `crates/biorouter/src/system.rs`,
`crates/biorouter/src/session_context.rs`, `crates/biorouter/src/slash_commands.rs`,
`crates/biorouter/src/agents/{resource_refs.rs,vault_refs.rs}`, the six `prompts/*.md` files,
plus the call sites in `crates/biorouter/src/agents/{agent.rs,reply_parts.rs,extension_manager.rs}`
and `crates/biorouter-server/src/routes/agent.rs`, `crates/biorouter-cli/src/session/{builder.rs,prompt.rs}`.

> **Note.** Line numbers in the citations below are as-read at review time and have drifted since; treat them as pointers to the right function, not exact locations.

## Overview

The agent's context is assembled from two independent streams:

1. **The system prompt** — rebuilt from scratch on every user turn (and again mid-turn if the
   toolset changes). It is a MiniJinja render of `prompts/system.md` plus appended "extras".
2. **The message array** — the persisted conversation, into which several synthetic messages are
   injected per turn (MOIM info block, explicit-resource-context, soft-interrupt text,
   truncation-continuation nudge).

Text data-flow for one turn:

```text
reply_internal (agent.rs:1520)
 └─ prepare_reply_context (agent.rs:417)
     ├─ fix_conversation() → validated message list
     ├─ explicit_resource_context() → append <explicit-resource-context> user msg (438-449)
     └─ prepare_tools_and_prompt (reply_parts.rs:113)
         └─ PromptManager::builder() (prompt_manager.rs:209)
             .with_extensions(...)          # MCP server instructions, sorted by name
             .with_frontend_instructions(..) # desktop/frontend note as a synthetic "frontend" extension
             .with_code_execution_mode(..)   # drops the whole Extensions section
             .with_hints(working_dir)        # .biorouterhints / AGENTS.md, @import-expanded
             .with_enable_subagents(..)
             .build()                        # render system.md + "# Additional Instructions"
loop (per agent action):
 └─ inject_moim (agent.rs:1596 → moim.rs:12) → insert <info-msg> user msg after trailing tool responses
 └─ stream_response_from_provider(system_prompt, messages, tools)  (reply_parts.rs:174)
 └─ if tools_updated: rebuild tools+system_prompt (agent.rs:2039-2042)
```

`extend_system_prompt` / `override_system_prompt` (called once at session build, out of band) mutate
the `PromptManager` so the *next* `build()` picks them up — that is how the desktop and CLI prompts,
workflow prompts, and `--system` overrides get in.

## Review questions answered

### When contexts are injected (every injection point)

Context is injected at **session/agent construction**, **per user turn**, and **per agent action
inside a turn** — enumerated:

- **Agent construction (once):** `PromptManager::new()` at `agent.rs:253`. Critically it freezes
  `current_date_timestamp` here at hour granularity — `Utc::now().format("%Y-%m-%d %H:00")`
  (`prompt_manager.rs:186`) — for prompt-cache stability. This value is what `{{ current_date_time }}`
  in `system.md:9` renders.
- **Session build (once per session, out-of-band mutation of the PromptManager):**
  - CLI: `extend_system_prompt(get_cli_prompt())` at `builder.rs:624`; optional
    `additional_system_prompt` at `builder.rs:628`; optional file override via
    `BIOROUTER_SYSTEM_PROMPT_FILE_PATH` → `override_system_prompt` at `builder.rs:637`.
  - Desktop/server: `desktop_prompt.md` rendered and `extend_system_prompt`'d at
    `routes/agent.rs:495` and `:765` (two routes); session-level extra at
    `routes/session.rs:356`; per-app prompt at `routes/apps.rs:794`.
- **Per user turn (rebuilt every reply):**
  - System prompt fully re-rendered in `prepare_tools_and_prompt` (`reply_parts.rs:149-156`) — this
    re-reads hint files from disk and re-queries extension instructions on **every** turn.
  - `explicit_resource_context` appends a synthetic, agent-only (`with_visibility(false, true)`)
    user message wrapping `<explicit-resource-context>…</explicit-resource-context>` when the latest
    user text names skills/extensions/knowledge bases (`agent.rs:438-449`, refs parsed by
    `resource_refs.rs:20`). For selected skills this eagerly calls `skills__loadSkill` and inlines the
    skill body (`agent.rs:506-535`).
- **Per agent action inside the turn (the loop body, `agent.rs:1556`):**
  - **MOIM** `inject_moim` at `agent.rs:1596` — runs *every* iteration, so a multi-tool turn
    re-injects a fresh `<info-msg>` each pass.
  - **Soft-interrupt** queued user messages drained at `agent.rs:1589-1594` (a safe boundary between
    tool completion and the next provider call).
  - **Truncation-continuation** nudge (`TRUNCATION_CONTINUATION_MESSAGE`) at `agent.rs:2069` when the
    provider stops with `finish_reason=length` and no tool call.
  - **System prompt rebuild** when `tools_updated` (extensions enabled/disabled mid-turn),
    `agent.rs:2039-2042`.
- **Tool-dispatch (not model context):** vault secret substitution (`vault_refs.rs:49`) replaces
  `{{vault:NAME}}` in tool-call *arguments* only, deliberately keeping plaintext secrets out of the
  model's context.

### How the system prompt is assembled

`SystemPromptBuilder::build()` (`prompt_manager.rs:104-176`) is the single assembly point:

1. Collect `extensions_info`. The frontend/desktop note is folded in as a pseudo-extension named
   `"frontend"` (`prompt_manager.rs:108-114`) so it renders through the same loop.
2. Sort extensions by name (`:116`) — explicitly "for multi-session prompt caching."
3. `sanitize_unicode_tags` every extension's instructions (`:118-124`) to strip Unicode tag-block
   prompt-injection characters (U+E00xx).
4. Build a `SystemPromptContext` (`:129-136`) carrying `extensions`, `current_date_time`,
   `biorouter_mode`, `is_autonomous`, `enable_subagents`, `code_execution_mode`.
5. Render the base: either an override template via `render_inline_once` or the embedded
   `system.md` via `render_global_file` (`:138-146`), with a hardcoded fallback string on render error.
6. Append extras — hints, then a chat-mode note if `BioRouterMode::Chat` (`:148-160`) — under a
   `"\n\n# Additional Instructions:\n\n"` header joined by `\n\n` (`:170-174`).

Templating is MiniJinja with `trim_blocks`/`lstrip_blocks` (`prompt_template.rs:38-40`); core prompts
are embedded at compile time via `include_dir!` (`prompt_template.rs:10`). Note the deliberate
`_PROMPT_RECOMPILE_TRACKERS` hack (`prompt_template.rs:19-29`): `include_dir!` isn't change-tracked, so
each prompt is also `include_str!`'d to force recompiles on edit.

**Sections of `system.md`:** identity/provenance; a "prefer tools over recall" note; the rendered
date; a conditional `# Extensions` block (only when `not code_execution_mode`) containing the
Extension-Manager discovery instructions, a per-extension loop emitting `## <name>` + resource note +
`### Instructions`, and the "pillar awareness" paragraph (about-biorouter / Soul) that renders **only**
when extensions exist; then static `# Working on Tasks`, `# Tool Use`, `# Safety`, and
`# Response Guidelines`.

**Per-provider differences:** the only provider-conditional transform is the **toolshim** path
(`reply_parts.rs:160-166`) — for providers without native tool calling, `modify_system_prompt_for_tool_json`
rewrites the prompt to teach JSON tool syntax and the real tools are moved out of the API `tools` field.
There are no other per-provider prompt variants.

**Desktop vs CLI:** identical base `system.md`. Desktop appends `desktop_prompt.md` (GUI sidebar/marketplace
orientation); CLI appends `get_cli_prompt()` (`biorouter-cli/src/session/prompt.rs:2`) with terminal
terseness + slash-command list. Both land in the same `# Additional Instructions` block.

### What MOIM is, where and why it injects

MOIM is a per-action injected **`<info-msg>` block** — a fresh "state of the world" note. The acronym is
**never expanded anywhere in the codebase** (absence finding); functionally it is the
message-of-the-moment / ambient-context injection. It is assembled by
`ExtensionManager::collect_moim` (`extension_manager.rs:1482-1521`):

```text
<info-msg>
It is currently 2026-07-12 14:30:00        # chrono::Local, minute granularity (:1488)
Working directory: /path/to/cwd
...contributions from each Platform extension via get_moim(session_id)...
</info-msg>
```

Only `ExtensionConfig::Platform` extensions contribute (`:1500`). Two override `get_moim`: the **todo**
extension injects the live task list / "immediately update your todo" nudge
(`todo_extension.rs:197-214`), and the **code_execution** extension injects "ALWAYS batch operations into
ONE execute_code call" plus the module list (`code_execution_extension.rs:927`). The base client returns
`None` (`mcp_client.rs:113`).

`inject_moim` (`moim.rs:12-58`) inserts the block as a **user** message. Placement: it finds the position
after the last `Assistant` message and skips past any trailing tool responses
(`moim.rs:31-40`), then inserts there. The comment states the reason precisely (`moim.rs:27-30`):

> "Inserting between an assistant tool_call and its tool_result would put a user message there, which
> the OpenAI API rejects with a 400 error."

So MOIM lands after all trailing `tool_result`s — never between a `tool_call` and its result — and then
`fix_conversation` merges it with the adjacent user turn. If merging produces any issue other than the
two expected "merged consecutive …" ones, the injection is abandoned and the original conversation is
returned (`moim.rs:43-56`) — a conservative fail-safe. A thread-local `SKIP` flag disables it for tests
(`moim.rs:8-10`). Minute granularity is a deliberate cost optimization
(recorded in the [June 2026 performance review findings](../../performance-2026-06/review-findings.md)):
it keeps the conversation hash stable within a minute so the block doesn't bust caches every second.

### How .biorouterhints / hints files work

Loading is `with_hints` (`prompt_manager.rs:72-97`) → `load_hint_files` (`load_hints.rs:54`). Filenames
default to `.biorouterhints` and `AGENTS.md` (`load_hints.rs:10-11`) but are overridable via the
`CONTEXT_FILE_NAMES` config param (`prompt_manager.rs:74-81`) — tests confirm `CLAUDE.md` works.

Two scopes are gathered:
- **Global:** `~/.config/biorouter/<filename>` (`load_hints.rs:63`), rendered under a `### Global Hints`
  header (`:105`).
- **Project:** walking from the git root (or cwd if no `.git`) down to cwd (`get_local_directories`,
  `:30-52`), collecting a hints file at every level, under `### Project Hints` (`:113`). Without a git
  root, only the cwd file is used (test `test_nested_biorouterhints_without_git_root`).

**@import syntax** (`import_files.rs`): a regex (`:8-11`) matches `@path` tokens (extensioned files,
Capitalized names like `LICENSE`, or path-shaped tokens) while excluding emails/handles/URLs. Each ref is
resolved relative to the including file, `canonicalize`'d, and required to stay within the **import
boundary** (git root, else cwd) — absolute paths and traversal outside the boundary are rejected
(`sanitize_reference_path`, `:15-49`). Matched refs are replaced inline with
`--- Content from X ---\n…\n--- End of X ---` (`:124-129`). Recursion is bounded: `MAX_DEPTH = 3`
(`:13`, `:108-111`), a `visited` set prevents cycles (`:77`, `:113`/`:131`), and `.gitignore` patterns
suppress ignored files (`:88`). Missing/blocked refs are left as literal `@…` text.

**Size limits:** `parse_file_references` refuses any file over **128 KB** (`MAX_CONTENT_LENGTH = 131_072`,
`import_files.rs:53-62`) as ReDoS protection — but note this caps only *reference parsing*; the file's own
text is still concatenated into the prompt with **no total-size budget** (see Gaps).

### How extension/MCP instructions reach the model; teaching tool use & the ecosystem

Each MCP server returns `instructions` in its `InitializeResult`; `get_extensions_info`
(`extension_manager.rs:758-771`) maps enabled extensions to `ExtensionInfo{name, instructions,
has_resources}` via `ext.get_instructions()`. The builder sanitizes and renders them under
`## <name> / ### Instructions` in `system.md:38-48`. So an MCP server's `instructions` string is verbatim
system-prompt content (after Unicode-tag sanitization) — this is exactly the BioOKF/Playwright
"MCP Server Instructions" text visible in this environment.

Tool *use* is taught in two places: (a) the static `# Tool Use` section of `system.md` (parallel calls,
follow the schema, never call a non-provided tool, don't leak internal tool names, call the tool in the
same turn you announce it); and (b) each extension's own instructions. The **ecosystem** is taught in the
conditional pillar paragraph (`system.md:31-37`): it names extensions/skills/workflows/scheduler/knowledge
and the built-in **Soul** base, and tells the model to load the `about-biorouter` skill rather than guess.
Discovery is bootstrapped by the Extension-Manager text (`system.md:19-23`): `search_available_extensions`
then `manage_extensions`. Desktop adds the marketplace pointer (`desktop_prompt.md:12-14`).

### What system.md and desktop_prompt.md enforce

`system.md` is a compact, modern agent prompt and was clearly hardened by a prior review — a contract
test (`prompt_manager.rs:368-415`) pins each behavioral clause against silent removal. What it enforces:

- **Tool selection:** "Follow each tool's schema exactly and never call a tool that isn't provided"; batch
  independent ops in one message for parallelism; call the tool in the same turn you announce it
  (`system.md:70-74`). Good, terse, correct.
- **Verification / anti-fabrication:** "Before editing a file, read the relevant parts — don't guess";
  "Don't fabricate file paths, APIs, or results; verify with tools" (`:61-62`). A biomedical-specific
  clause demands accuracy over agreement and flags claims needing a primary source (`:82-83`).
- **Testing habits:** "After substantive code changes, run the project's build, tests, or lints when
  available and fix what you broke" (`:64`) — present but soft ("when available"), and there is no
  read-your-own-diff / self-review discipline.
- **Ask vs answer:** "If the user asks *how* to do something, answer first rather than immediately acting"
  and "When you genuinely lack information you can't obtain with tools, ask. Don't pester…" (`:57`, `:65-66`).
  A sensible balance, though vaguer than state-of-the-art (no explicit "prefer acting over asking when the
  next step is obvious" calibration, no stop-condition for research loops).
- **Conciseness / output conventions:** "Be concise… often 1-3 sentences," ban preamble/postamble,
  Markdown rules, and the `file_path:line_number` citation convention (`:85-95`).

`desktop_prompt.md` is purely orientational (GUI surfaces, sidebar sections, marketplace link) — 15 lines,
no behavioral content, which is appropriate.

### Weaknesses in the prompt text itself

This is a critique of the prose, separate from the architecture above: no explicit **planning/todo** guidance in `system.md` (that lives
only in the todo extension's MOIM, so a session without that extension never hears it); no guidance on
**large-output / context-budget** discipline; the biomedical-accuracy clause is a single sentence with no
citation-format requirement; and "run tests when available" is not backed by any enforcement or
verification-before-completion norm.

## Notable design choices (worth keeping)

- **Cache-stable assembly:** hour-granularity system-prompt date, minute-granularity MOIM, name-sorted
  extensions and tools — all explicitly justified for multi-session prompt caching
  (`prompt_manager.rs:116,184-186`, `reply_parts.rs:138`).
- **Unicode-tag sanitization** on every untrusted string (override, extras, extension instructions) is a
  real prompt-injection defense, and it is regression-tested (`prompt_manager.rs:118-124`, tests
  `:235-314`).
- **MOIM placement invariant** is a genuinely subtle correctness fix (avoids the OpenAI 400 for a user
  message between tool_call and tool_result) and is well-tested (`moim.rs` tests).
- **Import boundary + depth + cycle + gitignore guards** on `@import` are a thorough security model for
  hint expansion (`import_files.rs`), with adversarial tests (`/etc/passwd`, absolute paths).
- **Vault substitution at dispatch, not in context** (`vault_refs.rs`) keeps plaintext secrets out of the
  model entirely — single-pass, non-recursive, with the residual-risk honestly documented in the module
  header.
- **Contract test on behavioral clauses** (`prompt_manager.rs:368-415`) prevents accidental behavior loss.
- **Single source of truth** for prompts embedded in the binary with a recompile-tracking workaround.

## Gaps and weaknesses

These eleven items fed the improvement phase. They are what other documents in this
review cite as `context-injection.md gap #N`; the numbering below is that scheme and is stable.

1. **No total context budget anywhere.** Hints are concatenated with only a per-file 128 KB *parse* cap
   (`import_files.rs:53`); extension instructions, loaded-skill bodies (`agent.rs:526`), and MOIM are all
   spliced in with no aggregate size/token accounting or truncation. A large `AGENTS.md`, a chatty MCP
   server, or several `@import`s can silently blow the window. State-of-the-art agents budget and rank
   injected context; this system does not.

2. **Frozen system-prompt clock.** `current_date_timestamp` is set once at agent construction
   (`agent.rs:253`, `prompt_manager.rs:186`) and never refreshed, so `{{ current_date_time }}` goes stale
   in any long-lived session. It's *also* UTC hour-granularity in the system prompt but Local
   minute-granularity in MOIM (`extension_manager.rs:1488`) — two different clocks the model sees
   simultaneously, which can read as contradictory.

3. **MOIM re-injected every action, not deduped.** `inject_moim` runs on each loop iteration
   (`agent.rs:1596`); a long multi-tool turn accumulates repeated near-identical `<info-msg>` blocks in
   history. There is no "remove previous MOIM before inserting the new one," so ambient context grows and
   the model sees several timestamps.

4. **MOIM guidance is trapped in extensions.** Core disciplines (maintain a todo list; batch
   execute_code) live only in `get_moim` of the todo/code-execution extensions. Sessions without those
   extensions never receive that guidance, and it isn't in `system.md`. Planning/todo behavior is
   therefore non-deterministic based on which extensions happen to be enabled.

5. **`.biorouterhints` is not sandboxed against injection.** Hint file bodies and their `@import`
   expansions are inserted verbatim (only Unicode-tag stripped, and only on the top-level extras path, not
   necessarily on every nested import) with no "treat as untrusted data" framing. A malicious repo
   `AGENTS.md` becomes system-prompt-level instruction. Compare Claude Code, which treats project files as
   lower-trust context.

6. **Resource-ref parsing is a fragile hand-rolled string parser.** `resource_refs.rs` supports many
   overlapping syntaxes (`<biorouter-ref>` tags, legacy quoted phrases, `/skill:`, `/ext(...)`, `kb_id:`,
   `focus the "X" knowledge base`). This substring scanning is brittle and easy to break; the legacy-phrase
   matcher especially could false-positive on ordinary prose. No single grammar or fuzz tests.

7. **Loaded-skill inlining is unbounded and eager.** `skill_resource_context` (`agent.rs:506`) calls
   `loadSkill` and inlines the entire skill body into a synthetic user message every turn the ref appears,
   with no size cap and no caching across turns.

8. **`build()` swallows render errors into a one-line fallback.** If `system.md` fails to render
   (`prompt_manager.rs:144-146`), the agent silently runs on a 1-sentence identity string with no
   extensions, tools guidance, or safety rules, and nothing surfaces this to the user.

9. **Two desktop-prompt injection sites, append-only.** `extend_system_prompt` only *pushes* to
   `system_prompt_extras` (`prompt_manager.rs:200-202`); nothing dedups. If the desktop update route
   (`routes/agent.rs:495`/`:765`) fires more than once per agent, the desktop prompt is appended multiple
   times. There is no idempotence guard.

10. **No prompt/versioning or eval harness surfaced.** Prompts are static embedded strings guarded only by
    string-contains contract tests and insta snapshots. There's no A/B or per-model prompt variant beyond
    the toolshim rewrite, and no place to tune the "ask vs act," verification, or testing norms per model
    capability. Given 43+ providers of varying strength, one fixed prompt is a real limitation.

11. **Subagent prompt duplicates almost nothing from `system.md`.** `subagent_system.md` re-states role
    but omits the safety, citation, and biomedical-accuracy clauses, so subagents operate under materially
    weaker guidance than the main agent.

## Related documentation

- [State, awareness, todos, goals and version control](state-awareness-and-version-control.md) — the sibling review that covers what MOIM carries and the missing repo map; the two overlap on workspace awareness.
- [Compaction and context management](compaction-and-context-management.md) — the other half of context handling: what happens when the assembled context grows too large.
- [Context and prompts compared with other agents](../competitive-comparison/context-and-prompts.md) — how this prompt architecture measures against eight other coding agents.
- [Context engineering guide](../../../agent-loop/context-engineering.md) — the current, living guidance on shaping BioRouter's context.
- [Wave 1 context and prompts report](../../agent-loop-campaign/wave-reports/wave-1-context-and-prompts.md) — what was actually built in response to these gaps.
