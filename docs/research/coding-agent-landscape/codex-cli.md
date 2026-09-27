# OpenAI Codex CLI — agentic feedback loop review

> **What this is.** An external review of OpenAI Codex CLI (`openai/codex`), a Rust agent
> workspace, covering per-model system-prompt files, the `execpolicy` Starlark command
> policy, OS sandboxing, and its self-maintained ranked memories layer. One of nine tool
> reports in this folder, each covering the same ten dimensions.
> **Status:** Current. External-tool research; the cited source for per-model prompt
> variants (BR-3), the `project_doc_max_bytes` context cap (BR-2), hook `updated_input`
> rewriting (BR-19) and the auditable policy engine (BR-21).
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

Codex CLI is architecturally a close cousin of BioRouter and of Goose, the agent whose design
influenced BioRouter most: a turn loop over the Responses API with a small native tool set plus
MCP (the Model Context Protocol). That makes it the most directly
transferable design in this folder, and the one whose crate layout maps onto BioRouter's own.

> **Note.** Researched July 2026 — month granularity only, for a repository that ships
> daily. Model names cited below (`gpt-5.2-codex`, `gpt-5.1-codex-max`) and the crate layout
> will date quickly. Each section here is a single dense paragraph; the
> [Gemini CLI](gemini-cli.md) and [Claude Code](claude-code.md) reports go considerably
> deeper on the same ten dimensions.

## Repository layout

Codex CLI is a Rust monorepo (`codex-rs/`) with roughly 80 crates. The ones this review
touches:

| Crate | Role |
|---|---|
| `core` | Agent loop, tools, compaction |
| `exec` | Non-interactive mode |
| `tui` | Interactive terminal UI |
| `linux-sandbox`, `bwrap`, `windows-sandbox-rs`, `sandboxing` | Process isolation |
| `execpolicy` | Command policy engine |
| `hooks` | Lifecycle hook events |
| `memories` | Cross-session memory |
| `rollout`, `thread-store` | Session persistence |
| `mcp-server`, `rmcp-client` | MCP in both directions |
| `app-server` | IDE/desktop backend |

Source tree: [`openai/codex` `codex-rs/`](https://github.com/openai/codex/tree/main/codex-rs).

## System prompt and context injection

The base system prompt is a checked-in Markdown file selected per model — e.g.
`gpt-5.2-codex_prompt.md`, `gpt-5.1-codex-max_prompt.md`, `gpt_5_codex_prompt.md`, plus
`prompt_with_apply_patch_instructions.md`
([`codex-rs/core`](https://github.com/openai/codex/tree/main/codex-rs/core)). It defines a
terse "collaborative teammate" persona and hard rules: prefer `rg`, **never** run destructive
commands like `git reset --hard`/`git checkout --` without approval, preserve unrelated
changes in a dirty worktree, use the planning tool for multi-step work (skip it for trivial
tasks), and *"Do not dump large files you've written; reference paths only."*
Permissions/sandbox awareness is injected via generated templates
(`codex-rs/prompts/templates/permissions/{approval_policy,sandbox_mode}`), so the model is
told exactly what it may do under the active mode.

Project context uses the **AGENTS.md** convention (an open standard Codex co-promotes).
Discovery builds an instruction chain once per run/session in precedence order: global
`~/.codex/AGENTS.override.md` else `~/.codex/AGENTS.md`, then every directory from the Git
root down to the cwd (override file preferred at each level). Files are concatenated
root-first joined by blank lines, so **deeper files override shallower ones by appearing
later**, and accumulation stops at `project_doc_max_bytes` (32 KiB default).
[AGENTS.md documentation](https://learn.chatgpt.com/docs/agent-configuration/agents-md)

Mid-conversation, context is added through `context-fragments`/`context_manager` (world
state, environment, current time via `current_time.rs`), `@`-mention file expansion
(`mention_syntax.rs`), and — distinctively — a **memories** layer (below). Piped stdin is
injected in `exec` mode (`curl … | codex exec "…"`).
[non-interactive mode documentation](https://learn.chatgpt.com/docs/non-interactive-mode)

## Tool loop mechanics

The native tool set (from `codex-rs/core/src/tools/spec_plan.rs` handler registration)
includes `shell`/`exec_command` (with `write_stdin` for interactive processes), `apply_patch`
(the diff-based editor), `update_plan` (`PlanHandler`), `view_image`, MCP tool/resource
handlers, `request_permissions`, `request_user_input`, `get_context_remaining`/
`new_context_window` (the model can query its own budget and reset the window), plus a
multi-agent family (`spawn_agent`, `wait_agent`, `send_input`, `resume_agent`, `close_agent`,
`spawn_agents_on_csv`).
[`spec_plan.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/spec_plan.rs)

Tool calls are dispatched through a `router` + `orchestrator` with genuine **parallelism gated
by a read/write lock**: a tool is asked `tool_supports_parallel()`; parallel-safe tools acquire
a shared read lock and run concurrently, while any non-parallel (mutating) tool takes an
exclusive write lock that serializes everything.
[`parallel.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/tools/parallel.rs)

Output streams over the Responses API (`client.rs`, `ResponseEvent`); in `exec --json` this
surfaces as a JSONL event stream (`thread.started`, `turn.started`, `item.*`,
`turn.completed/failed`). Command output is truncated by a `TruncationPolicy`
(`codex_utils_output_truncation`) before re-entering context. Errors return as tool results
the model can react to; a sandbox denial can trigger an approval re-prompt rather than a hard
fail (see Guardrails and permissions).

## Compaction and memory

Codex has an unusually elaborate compaction subsystem — a dozen `compact*.rs` modules in
`core/src`. Two strategies exist:

1. **Summarization ("handoff") compaction** — the model is re-prompted with `SUMMARIZATION_PROMPT` (`prompts/templates/compact/prompt.md`): *"Create a handoff summary for another LLM that will resume the task,"* preserving current progress and key decisions, important context/constraints/user preferences, remaining next steps, and critical data/references — concise and structured. [compaction prompt template](https://github.com/openai/codex/blob/main/codex-rs/prompts/templates/compact/prompt.md)
2. **Token-budget compaction** — *"skips model/server summarization and installs a fresh context window instead"* (`compact_token_budget.rs`), modeled through the same lifecycle so hooks still fire. [`compact_token_budget.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/compact_token_budget.rs)

Compaction can be **manual** (`/compact`), **auto** (triggered when context fills), or
**remote** (`compact_remote*.rs`, when the provider supports server-side compaction). It runs
either **pre-turn** or **mid-turn**, and injection strategy differs: mid-turn compaction
re-injects initial context *just above the last user message*
(`InitialContextInjection::BeforeLastUserMessage`) because the model is trained to treat the
summary as the final history item; pre-turn/manual uses `DoNotInject` and lets the next turn
re-hydrate. Each compaction emits a `ContextCompaction` turn item and analytics
(trigger/reason/phase/strategy).
[`compact.rs`](https://github.com/openai/codex/blob/main/codex-rs/core/src/compact.rs)

**Cross-session memory** is a first-class `memories` crate: past session "rollouts" are
distilled into a `raw_memory` + `rollout_summary`, stored under `~/.codex/memories/`
(`raw_memories.md`, `rollout_summaries/`, consolidated `MEMORY.md`), with
`generated_at`/`last_usage`/`usage_count` metadata and a Phase-2 consolidation sub-agent. On
the read path these are injected as developer instructions with citation parsing.
[memories crate README](https://github.com/openai/codex/blob/main/codex-rs/memories/README.md)
This is a step beyond AGENTS.md: durable, self-maintained, ranked memory.

## Hooks and extensibility

Codex ships a dedicated `hooks` crate with a Claude-Code-style event model. Events:
`PreToolUse`, `PostToolUse`, `PermissionRequest`, `SessionStart`, `Stop`, `UserPromptSubmit`,
and `Compact` (Pre/Post).
[hook events module](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/events/mod.rs)

A `PreToolUse` handler receives `tool_name`, `tool_input`, `cwd`, `permission_mode`,
session/turn ids, and returns a `PreToolUseOutcome` that can **block** (`should_block` +
`block_reason`), **inject** model-visible context (`additional_contexts`), or **rewrite the
call** (`updated_input`).
[`pre_tool_use.rs`](https://github.com/openai/codex/blob/main/codex-rs/hooks/src/events/pre_tool_use.rs)

Pre/Post-compact hooks can even abort a compaction (`PreCompactHookOutcome::Stopped` →
`TurnAborted`). Hooks are configured declaratively (`config_rules.rs`, `declarations.rs`) with
matcher aliases per tool, and there's a legacy `notify` bridge. Extensibility also comes via a
`plugin`/`core-plugins` system, `skills`/`core-skills` (installable skill packs, surfaced
through `/status`), and MCP servers.

## Guardrails and permissions

Two independent axes, exactly like the docs frame it: **sandbox** (what's technically
possible) and **approval policy** (when Codex must ask).
[approvals and security documentation](https://learn.chatgpt.com/docs/agent-approvals-security)

- **Sandbox modes** (`sandbox_mode`): `read-only`, `workspace-write` (default; edits + routine commands confined to the workspace, network off by default), `danger-full-access` (no limits). `[sandbox_workspace_write]` tunes `writable_roots`, `network_access`, `exclude_tmpdir_env_var`. [sandboxing documentation](https://learn.chatgpt.com/docs/sandboxing)
- **Approval policy** (`approval_policy`): `untrusted` (only known-safe reads auto-run), `on-request` (agent works in the sandbox, asks to exceed it), `never`. The `--dangerously-bypass-approvals-and-sandbox` / `--yolo` flag drops both. Named presets: **Read Only** (`--sandbox read-only`), **Auto** (`--sandbox workspace-write --ask-for-approval on-request`), **Full Access**. `/permissions` switches profile live.

**Platform enforcement** is native and deny-by-default:

- **macOS**: Seatbelt via `sandbox-exec -p` with a parameterized profile (writable roots injected), `network-outbound` denied. [Simon Willison's sandbox investigation](https://simonwillison.net/2025/Nov/9/codex-sandbox-investigation/)
- **Linux/WSL2**: a `codex-linux-sandbox` helper combining **Landlock** (filesystem, `landlock.rs`) + **seccomp** (blocks network syscalls) + **bubblewrap** namespaces (`bwrap` crate). [DeepWiki: sandboxing implementation](https://deepwiki.com/openai/codex/5.6-sandboxing-implementation)
- **Windows**: restricted process token via `windows-sandbox-rs`.

**Dangerous-command detection** lives in `execpolicy`: a Starlark policy engine of
`prefix_rule(pattern=…, decision=allow|prompt|forbidden, justification=…)` with
`match`/`not_match` self-tests and `host_executable` path pinning, so risky commands get
`prompt`/`forbidden` with a rationale.
[execpolicy README](https://github.com/openai/codex/blob/main/codex-rs/execpolicy/README.md)
When a sandboxed command fails on a permission boundary, Codex can escalate and re-request
approval (`shell-escalation`, `network_approval.rs`) rather than silently failing.

## Loop and stuck detection

Codex leans on a **goals** mechanism (a `gpt-5.x-codex-max` long-horizon feature) rather than
a hard iteration cap. A thread goal persists across turns and each continuation re-injects
`goals/continuation.md`, which carries a live **token budget**
(`tokens_used`/`token_budget`/`remaining_tokens`) and a strict **blocked audit**: the model may
only call `update_goal(status="blocked")` after *"the same blocking condition has repeated for
at least three consecutive goal turns"* — an explicit anti-thrash / stuck heuristic — and
*"Never use status 'blocked' merely because the work is hard, slow, uncertain, incomplete."*
[goals continuation template](https://github.com/openai/codex/blob/main/codex-rs/prompts/templates/goals/continuation.md)

When the budget is hit, `goals/budget_limit.md` tells it to stop starting new work and wrap up
with a clear next step. Thread-spawn recursion is bounded
(`exceeds_thread_spawn_depth_limit`), and `awaiter`-style polling uses **exponential timeout
backoff** to avoid busy-looping.

## Long-running tasks and background processes

Shell commands can run as **background terminals**; a builtin `awaiter` sub-agent
(`agent/builtins/awaiter.toml`, `model_reasoning_effort = "low"`,
`background_terminal_max_timeout = 3600000` ms = 1 h) is dedicated to polling a long task to a
terminal state and reporting status without modifying it, increasing yield times exponentially
across waits.
[`awaiter.toml`](https://github.com/openai/codex/blob/main/codex-rs/core/src/agent/builtins/awaiter.toml)

Interactive processes are fed via `write_stdin`; `sleep` and `wait_for_environment` handlers
coordinate timing. Real **subagent delegation** exists through the multi-agent tools
(`spawn_agent`/`wait_agent`/`send_input`/`resume_agent`/`close_agent`) plus **fan-out** via
`spawn_agents_on_csv` + `report_agent_job_result` — one agent spawns a batch of workers over
CSV rows and collects results (`codex_delegate.rs`, `agent/registry.rs`). Cloud/async execution
is handled by the `cloud-tasks` crate (Codex Cloud) and `codex exec` in CI.

## State tracking and checkpoints

The `update_plan` tool (`PlanHandler`, `spec_plan.rs`) gives the model a live, ordered
checklist that the TUI renders; the system prompt says to use it for meaningful multi-step
work, skip it for trivial tasks, and keep it current as steps complete. Sessions are persisted
as **rollout** files (`rollout`/`rollout-trace`/`thread-store` crates) under `~/.codex/`,
enabling resume and replay; `--ephemeral` opts out of persistence.
[non-interactive mode documentation](https://learn.chatgpt.com/docs/non-interactive-mode)

Codex is **Git-aware** rather than checkpoint-based: it requires a Git repo by default
(overridable with `--skip-git-repo-check`), and its guardrail is *not* undoing user work — it
refuses `git reset --hard`/`git checkout --` and preserves unrelated diffs, deferring "undo"
to normal VCS. `apply_patch` edits are diffs the user can review/revert. The `/status` slash
command reports token usage, model, approval mode, and skills.

## Self-verification

Verification is driven mostly through the prompt, and Codex's is notably rigorous. The
`goals/continuation.md` **completion audit** treats completion as *"unproven"* and requires
requirement-by-requirement evidence:

> For every explicit requirement, numbered item, named artifact, command, test, gate,
> invariant, and deliverable, identify the authoritative evidence that would prove it, then
> inspect … files, command output, test results, PR state, rendered artifacts, runtime
> behavior.

It explicitly forbids substituting a *"narrower, safer, smaller, merely compatible, or
easier-to-test solution because it is more likely to pass current tests,"* and treats
uncertain/indirect evidence as not-done.
[goals continuation template](https://github.com/openai/codex/blob/main/codex-rs/prompts/templates/goals/continuation.md)

The base prompt separately instructs the agent to run project tests/lints after edits and to
pause if the worktree shows unexpected changes. There is also a `/review` flow
(`prompts/templates/review`, `review_request.rs`) that runs the model as a code reviewer
producing severity-ranked findings — a structured self-review pass.

## Ideas worth stealing

1. **Two-axis sandbox × approval separation with native enforcement.** Codex cleanly splits *what's technically possible* (OS sandbox: Seatbelt / Landlock+seccomp+bubblewrap / restricted token) from *when to ask* (approval policy), and escalates to an approval prompt on sandbox denial instead of failing. BioRouter's permission modes could gain real OS-level enforcement plus this graceful escalation, so autonomy is bounded by the kernel, not just by prompt compliance.

2. **The `goals` long-horizon loop with a token budget and a "3 consecutive turns before blocked" rule.** A persistent objective re-injected every continuation, an explicit completion audit, and an anti-thrash blocked-threshold is a concrete, promptable answer to stuck-detection and premature "done" — cheap to adopt and directly relevant to long research runs.

3. **Self-maintained cross-session memory (`~/.codex/memories/`).** Distilling finished sessions into ranked, cited memories (`usage_count`/`last_usage`) that inject as developer instructions goes beyond static `.biorouterhints`/AGENTS.md and would let BioRouter accumulate lab-specific know-how across sessions automatically.

4. **`execpolicy` as a declarative, testable command allow/deny layer.** A Starlark `prefix_rule` engine with `match`/`not_match` self-tests, `justification` strings (surfaced in prompts), and host-path pinning is a far more auditable dangerous-command gate than ad-hoc regexes — a strong fit for BioRouter's `.biorouterignore`/security module.

5. **Hooks that can block, inject context, *and* rewrite tool input.** The `PreToolUse` outcome (`should_block` / `additional_contexts` / `updated_input`) plus Pre/Post-Compact hooks give deterministic, user-owned control points around every tool call and compaction — a clean extensibility surface BioRouter could expose for policy, redaction, and telemetry.

6. **Compaction as a first-class, hookable lifecycle with dual strategies.** Modeling both LLM-summarization handoff *and* token-budget "fresh window" reset through one lifecycle (with mid-turn vs pre-turn injection nuance and a model-callable `get_context_remaining`/`new_context_window`) is a mature template for BioRouter's `context_mgmt` pruning.

7. **Fan-out subagents over CSV (`spawn_agents_on_csv` + `report_agent_job_result`).** A built-in batch-delegation primitive that maps a worker agent across rows and aggregates results is a natural fit for BioRouter's biomedical workloads (per-gene, per-cohort, per-paper sweeps).

## Sources

Official documentation at `learn.chatgpt.com` (formerly `developers.openai.com/codex`) plus
primary source in the GitHub repository. Every non-obvious claim is cited inline; where docs
were thin the Rust source was read directly.

| Kind | Source |
|---|---|
| Docs — sandboxing | [learn.chatgpt.com/docs/sandboxing](https://learn.chatgpt.com/docs/sandboxing) |
| Docs — approvals and security | [agent-approvals-security](https://learn.chatgpt.com/docs/agent-approvals-security) |
| Docs — AGENTS.md | [agent-configuration/agents-md](https://learn.chatgpt.com/docs/agent-configuration/agents-md) |
| Docs — non-interactive / exec | [non-interactive-mode](https://learn.chatgpt.com/docs/non-interactive-mode) |
| Docs — full dump | [llms-full.txt](https://learn.chatgpt.com/docs/llms-full.txt) |
| Source — system prompts | `codex-rs/core/gpt-5.2-codex_prompt.md` |
| Source — compaction | `core/src/compact.rs`, `compact_token_budget.rs`, `prompts/templates/compact/prompt.md` |
| Source — goals | `prompts/templates/goals/{continuation,budget_limit}.md` |
| Source — hooks | `hooks/src/events/{mod,pre_tool_use}.rs` |
| Source — tools and parallelism | `core/src/tools/{parallel,spec_plan}.rs` |
| Source — subagents | `core/src/agent/builtins/awaiter.toml`, `codex_delegate.rs` |
| Source — sandbox | `core/src/landlock.rs`, crates `linux-sandbox` / `bwrap` / `windows-sandbox-rs` |
| Source — policy | `execpolicy/README.md` |
| Source — memory | `memories/README.md` |
| Third-party — sandbox analysis | [Simon Willison](https://simonwillison.net/2025/Nov/9/codex-sandbox-investigation/) |
| Third-party — sandboxing internals | [DeepWiki](https://deepwiki.com/openai/codex/5.6-sandboxing-implementation) |

> **Note.** All source paths are relative to [github.com/openai/codex](https://github.com/openai/codex)
> on branch `main`, not a pinned commit. Re-verify before relying on line-level details.

## Related documentation

- [Gemini CLI report](gemini-cli.md) — the other declarative policy engine in this corpus, for comparison against `execpolicy`.
- [Claude Code report](claude-code.md) — the hook event model Codex's `hooks` crate follows.
- [Goose report](goose.md) — Goose, the other Rust agent in this folder, whose workspace is the closest match to Codex's layout of one crate per concern.
- [Command policy engine design](../../agent-loop/designs/command-policy-engine.md) — BR-21, the BioRouter design this report fed into.
- [macOS Seatbelt sandbox design](../../history/agent-loop-campaign/cross-platform/macos-seatbelt-sandbox.md) — the BioRouter counterpart to Codex's Seatbelt profile.
- [Improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — the `BR-NN` index, including BR-2, BR-3, BR-19 and BR-21.
