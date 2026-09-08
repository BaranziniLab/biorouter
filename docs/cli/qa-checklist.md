# BioRouter CLI QA checklist

> **What this is.** The end-to-end verification script for the `biorouter` CLI and its terminal UI: a per-surface checklist of things to exercise, the log of bugs found and fixed during the pass that produced it, and the list of GUI-parity gaps that pass identified.
> **Status:** Current — it describes the shipped CLI surface (a full-screen terminal user interface, or TUI, by default on a TTY, `BIOROUTER_CLI_CLASSIC` for the classic read-eval-print loop (REPL), and the `knowledge` / `extension` / `skill` / `workflow` / `schedule` subcommands). The checklist itself records no date, version, or commit for the pass, so re-confirm the **[verified]** marks against whatever build you are testing.
> **Audience:** maintainers running CLI QA

This file holds three related things about one QA pass, in order: the checklist proper, grouped by CLI surface; the defects that pass found and fixed; and the features the GUI has that the CLI does not. Read the checklist to run a pass; read the other two sections to know what has already been dealt with and what is known-missing by design. For what each command and flag is supposed to do, see the [CLI command reference](command-reference.md) — this page verifies that surface, it does not define it.

## Status legend

| Tag | Meaning |
|---|---|
| `[verified]` | Exercised headlessly in this pass and confirmed working. |
| `[manual]` | Requires a real interactive terminal (raw-mode TUI keystrokes) or a live provider. Steps are given; run them by hand. |
| `[audited]` | Verified by code review plus automated `TestBackend`/unit tests. Cannot be driven headlessly. |
| `[verified-by-code]` | Used once, on the TUI `/clear` item, to the same standard as `[audited]`: confirmed by reading the code rather than by driving the UI. |
| `[gap]` | A known GUI-parity gap — the behaviour is not in the CLI yet. |

## Before you start

Run from the repo root after `source bin/activate-hermit && cargo build -p biorouter-cli`.
Binary: `./target/debug/biorouter`. Force the classic readline UI with
`BIOROUTER_CLI_CLASSIC=1`; the full-screen TUI is the default on a TTY.

## Launch modes and output formats

- [verified] `biorouter --version`, `biorouter --help`, every `<cmd> --help`.
- [audited] `biorouter session` → full-screen TUI (default on a TTY).
- [manual] `BIOROUTER_CLI_CLASSIC=1 biorouter session` → classic readline REPL.
- [verified] `biorouter run -t "<prompt>"` → headless prompting (one-shot).
- [manual] `echo "<prompt>" | biorouter run -i -` → instructions from stdin.
- [manual] `biorouter run -t "..." --output-format json` and `stream-json`.
- [verified] Non-TTY / piped invocation auto-falls back to the classic path.

## Interactive TUI

Run `biorouter session`, then work through the four groups below.

### Input and editing

- [audited] Type text; **blinking bar (I-beam) cursor** sits at the insertion point.
- [audited] **CJK / wide chars** (e.g. `你好世界`) keep the caret at the true end.
- [audited] `Ctrl+J` inserts a newline; multi-line compose grows the input box (≤6 rows).
- [verified] **Paste** of multi-line text inserts as one chunk and does **not**
  submit prematurely (bracketed paste; unit-tested).
- [audited] `←/→/Home/End` move by character; `Backspace` deletes by character.
- [audited] `↑/↓` recall input history (single-line buffer).
- [audited] Slash **ghost autofill**: typing `/co` dims in `mpact`; `Tab` accepts.
- [audited] `Enter` submits a non-empty buffer; empty Enter is a no-op.

### Layout and chrome

- [audited] Greeting shows the coral-brown `BIOROUTER` wordmark, tagline, and
  **working directory** line.
- [audited] **Two-line status**: line 1 = model · provider; line 2 = N skills ·
  N extensions · N knowledge bases · context meter (right-aligned).
- [audited] **Context meter** updates after each turn; warns red ≥85%.
- [audited] ~2-row **gap** separates the response from the input box.
- [audited] Window starts compact and **grows downward** as the chat fills, then scrolls.
- [manual] Mouse wheel / PageUp / PageDown scroll the history; latest output stays visible.
- [manual] Terminal **resize** reflows without corruption.

### Turns, tools, and control

- [verified] Send a prompt → streaming response renders; **thinking spinner** on the input border.
- [verified] Tool calls render as a distinct **`▸ tool call`** badge (tool + namespace);
  arguments in plain text (no green).
- [manual] **Permission modal**: a tool needing approval shows a modal; `↑/↓` + `Enter`
  pick Allow / Always allow / Deny / Cancel; `Esc`/`Ctrl+C` cancels.
- [manual] **Ctrl-C mid-stream** cancels the in-flight response promptly (event-channel fix).
- [audited] Markdown rendering: headings, **bold**, `inline code`, bullets, code fences.

### Slash commands in the TUI

- [audited] `/help`, `/?` → in-TUI command + navigation help (now lists `/compact`).
- [verified-by-code] `/clear` clears the **persisted** conversation + token counts +
  scrollback (no desync; shared `clear_conversation`).
- [manual] `/compact` → condenses the conversation via a normal turn.
- [audited] `/exit`, `/quit` → leave; `Ctrl+C` on empty input quits.
- [audited] Other slash commands show a "use BIOROUTER_CLI_CLASSIC=1" note.
- [audited] **Panic safety**: a render panic restores raw mode / alt screen / cursor
  (panic hook) — terminal is never left corrupted; `Tui::Drop` covers the error path.

## Classic REPL

Run `BIOROUTER_CLI_CLASSIC=1 biorouter session`.

- [manual] `/help`, `/t [light|dark|ansi]`, `/r`, `/mode <m>`, `/plan` … `/endplan`,
  `/compact`, `/clear`, `/workflow`, `/extension`, `/builtin`.
- [audited] Interrupt handling no longer panics on an empty last message.

## Models and provider config

Parity target: GUI settings → models/providers.

- [verified] `biorouter models current` — shows configured provider+model.
- [verified] `biorouter models providers` — lists providers + defaults.
- [verified] `biorouter models list <provider>` — known models; unknown → clear error.
- [manual] `biorouter models set --provider <p> --model <m>` — writes shared config.yaml.
- [manual] `biorouter configure` — interactive provider/key wizard.

## Knowledge bases

Parity target: GUI Knowledge view.

- [verified] `knowledge list` — visible (●) vs hidden (○) model, with a hint line.
- [verified] `knowledge active`, `knowledge active --set <id>`, `--clear`.
- [manual] `knowledge create <id> --name "<n>"` — creates + sets active when none.
- [verified] `knowledge hide <id>` / `knowledge unhide <id>` — controls agent visibility;
  unknown id → clear error.
- [verified] `knowledge query "<q>"` — live LLM answer over the active base (verified end-to-end).
- [manual] `knowledge ingest --url <u>` / `--file <p>` / `--text "<t>" [--focus ...]`.
- [manual] `knowledge lint [--fix]` — deterministic scan (verified clean) + optional autofix.
- [gap] No CLI for knowledge **graph**, **history/restore**, or **`.brkb` export/import**
  (GUI/server-only).

## Extensions

Parity target: GUI Extensions plus `.brxt` install.

- [verified] `extension list` — configured extensions with enabled/disabled dots.
- [manual] `extension install <file.brxt> [--env K=V] [--secret K=V] [--no-enable]`
  — extracts to `~/.config/biorouter/extensions/<name>`, runs **`uv sync`**, registers a
  stdio extension; missing `uv` → actionable error; missing file → clear error (verified).
- [manual] `extension remove <name> [--purge]`.
- [manual] In a session: `/extension <ENV=v cmd args>`, `/builtin <names>`.

## Skills

Parity target: GUI Skills plus `.zip` install.

- [verified] `skill list` — installed skills (incl. bundles).
- [verified] `skill install <file.zip> [--force]` — single skill **and** bundle layouts
  (verified install + remove round-trip); bad/missing zip → clear error.
- [verified] `skill remove <slug>`.

## Workflows

Parity target: GUI Workflows. The workflow builder is GUI-only.

- [verified] `workflow list` — local + GitHub workflows.
- [verified] `workflow install <file.json|yaml>` — validates + saves to the library;
  wrong extension → clear error.
- [manual] `workflow validate <name|path>`, `workflow deeplink <name>`, `workflow open <name>`.
- [manual] `biorouter run --workflow <name|path> [--explain]`.

## Scheduler

Parity target: GUI Schedules.

- [verified] `schedule list` (empty → "No scheduled jobs found"), `schedule cron-help`.
- [manual] `schedule add …`, `schedule run-now <id>`, `schedule sessions <id>`, `schedule remove <id>`.

## Sessions

- [manual] `session --resume` / `--name` / `--session-id`; `session list`, `session remove`,
  `session export`, `session diagnostics`.

## Miscellaneous commands

- [verified] `info`, `info --verbose` (product-name string corrected — see the bug log below).
- [verified] `completion <bash|zsh|fish|…>`.
- [manual] `project`, `projects` (interactive directory manager).
- [manual] `update [--canary]`, `web`, `term`, `mcp <server>`, `acp`.

## Error handling and robustness

Spot-checked:

- [verified] Unknown subcommand, missing files, unknown provider, missing `--kb`,
  wrong workflow extension → all produce clear, non-panicking errors (exit ≠ 0).

## Bugs found and fixed in this pass

1. `info` printed the wrong product name, and was corrected to read **"BioRouter"**.
   > **Note.** The incorrect string this item originally quoted was itself overwritten by a
   > later global product-name rename applied to this file, so the "before" value is no
   > longer recoverable from this document. Only the fix is on record.
2. TUI `/clear` only reset in-memory messages → now clears the **persisted** conversation
   + token counts (shared `CliSession::clear_conversation`), so the context meter and a
   reopened session stay correct.
3. TUI lost/delayed keystrokes (incl. mid-stream **Ctrl-C**) because multiple
   `EventStream::next()` futures raced across `select!` arms → a **single reader task now
   forwards events over an mpsc channel** (cancel-safe).
4. Multi-line **paste** fired line-by-line (premature submit) → **bracketed paste** enabled;
   pastes insert as one chunk.
5. No terminal restore on **panic** → installed a panic hook restoring raw mode / alt
   screen / cursor before the default hook.
6. Scroll height estimate counted chars, not display width → **wide (CJK) lines could clip**;
   now uses unicode width.
7. Tool-call permission **cancel** could desync the conversation → now drains the stream
   after cancelling.
8. Classic interrupt handler could **panic** on an empty last message → now a graceful warn.
9. `/compact` was unavailable in the TUI → now wired (sends the compaction trigger turn).

## Known GUI-parity gaps

These are future work, not defects:

- Image/file attachment (multimodal input) — GUI-only.
- Voice dictation (Whisper) — GUI-only.
- Knowledge graph view, history/restore, `.brkb` export/import — server/GUI-only.
- First-class secrets-management command (keys only via `configure` / per-extension `--secret`).
- Tabs, chat groups and split panes, interactive MCP-UI resources, session-sharing tunnel,
  response-style settings — GUI-only.

## Related documentation

- [CLI command reference](command-reference.md) — the definition of every command and flag this checklist exercises.
- [Diverge behaviour checklist](../desktop-ui/diverge-behavior-checklist.md) — the equivalent manual verification script on the desktop UI side.
- [Llama Server model catalog QA checklist](../providers/llama-server/model-catalog-qa-checklist.md) — the per-model checklist for the bundled local-model provider.
- [Managing sessions](../getting-started/managing-sessions.md) — background on the session storage the `session` items here exercise.
- [Extensions, skills, and MCP agents](../extensions/extensions-and-skills-guide.md) — what the `extension` and `skill` subcommands install and how the pieces relate.
