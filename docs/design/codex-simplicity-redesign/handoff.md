# Codex simplicity redesign: handoff

> **What this is.** Everything the next agent needs to finish the October 2026 redesign: where the
> work stands, what is left in order, how to run it, and the traps the first run hit.
> **Status:** Current. Written 2026-10-10 when the first coordinator stopped to save tokens.
> **Audience:** the agent or developer who takes over the redesign.

The redesign is defined by three documents beside this one: the owner's requirements
([requirements-and-plan.md](requirements-and-plan.md), R-01 to R-28), the implementation spec
([implementation-spec.md](implementation-spec.md), sections 0 to 9) and the tracker
([progress.md](progress.md)). Read section 9 of the spec before anything else: it holds the
owner's latest rules (Tools switcher at the bottom, Effort slider, fills over lines).

## Where everything is

| Thing | Location |
|-------|----------|
| Branch | `claude/crew-landing-redesign-91b46e`, pushed, about 490 commits ahead of `main` (base `30e1f0622`). No pull request yet. |
| Worktree | `/Users/wgu/Desktop/BioRouter/.claude/worktrees/crew-landing-redesign-91b46e` |
| Run directory (outside the repo) | `~/biorouter-runs/redesign-2026-10-09/` |
| Owner's verbatim messages 1 to 14 and every coordinator ruling | `REQUIREMENTS.md` in the run directory. The single most important input. |
| Research and audits | `research/*.md`, `audit/*.md`, `icons/` (icon proposals and contact sheets) |
| Old app screenshots | `baseline/*.png` and `audit/baseline-screens.md` |
| Per workstream notes (commits, decisions, deviations) | `impl/WS-*.md`; WS-PERF's measurements in `impl/WS-PERF.md` and `impl/WS-PERF-tools/` |
| Vision round 1 reports and screenshots | `vision/round-1/V1.md` to `V5.md`, `vision/round-1/violations.json` |
| Requests between workstreams | `requests/<OWNER>/*.md` (pending ones have no `.done`, `.rejected` or `.withdrawn` suffix) |
| Open questions for the owner | `requests/coordinator/RULINGS-NEEDED.md`; rulings already given in `RULINGS-GIVEN.md` |
| Workflow scripts used last | `workflows/redesign-m13-fill-perf-vision-*.js` (phases A to C, vision rounds, gates) |
| Dev app launcher | `~/biorouter-runs/launch-dev-gui.sh` (see the memory note `dev-gui-launch-script.md`) |

## State at handoff

### Done and committed

- All 21 workstreams implemented and reviewed against the spec (R-01 to R-24): compact sidebar with
  Group by and Sort by, right click Rename in place (also F2 and Cmd+Opt+R) and the full row menu,
  the docked chat summary rail with its width rules, the usage dashboard on Home, one font family
  (the website's Arial stack) on a five step scale with a 14px body, InfoTips in place of
  paragraphs, one switch, segmented, menu and dialog style, icons per chat kind (lock badge for
  private, Crew bubble) and per built in tool, the redesigned transcript, tool rows, composer,
  pickers, preview panel, Workflows, Scheduler, Built apps, Extensions, Skills, Knowledge (same
  flows, simpler skin), Settings, motion polish, and the website revision.
- The integration gate was green after implementation: `npm run lint:check` (944 contrast
  assertions then), `npm run format:check`, all 851 test files, the website and Crew manual checks.
- Vision round 1 ran on four sandboxed app instances plus the website: 107 findings, 45 high or
  medium. Every owner's round 1 fixes have since landed except the few listed under "Open items".
- Owner message 13 and 14, first changes:
  - R-25: the Tools popover's Extensions, Skills and Knowledge switcher sits at the bottom, fixed
    above the Tools button (WS-PICKERS, browser geometry test).
  - R-26: the Effort choice is a three stop slider built on the new `components/ui/slider.tsx`
    (contract commit `2cce15237`).
  - R-27 tokens and primitives: `--background-fill` and `--background-fill-strong` with contrast
    assertions (WS-TOKENS), and the shared controls converted to fills (contract commit
    `859f90b37`, polish `0708c3494`).
  - R-28: sidebar listing sped up end to end (commits `aceb5d6e5`, `47bbb9a72`, `b38147479`,
    `ce9140365`): one 50 row page 118 to 10 ms, a complete reordered list of 2,000 chats 3.1 s to
    about 0.2 s, view switches 70 to 140 ms in the dev build. The dev daemon in
    `target/debug/biorouterd` was rebuilt with this code.
- R-27 fill sweep finished for WS-SUMMARY, WS-USAGE, WS-TRANSCRIPT, WS-COMPOSER, WS-PICKERS and
  WS-PREVIEW. WS-TOOLS' sweep was stopped mid-run and committed as is by the coordinator
  (`7dc4d168a`): it type-checks and its 13 test files pass, but it had no review.

### Not done yet, in the order to do it

1. **Finish the R-27 fill sweep** (spec section 9, phase C of the last workflow) for the eleven
   workstreams that have no `R-27` commit: WS-VIEWS-A, WS-EXTENSIONS, WS-SKILLS, WS-KNOWLEDGE,
   WS-SETTINGS-A, WS-SETTINGS-M, WS-SETTINGS-B, WS-HISTORY, WS-SIDEBAR, WS-MOTION (the tab strip's
   tabs are still outlined) and WS-LANDING. Then review WS-TOOLS' unreviewed sweep. Check with
   `git log --oneline --grep='(WS-NAME): R-27'`. Each sweep includes the owner's "careful polish
   review" of its own surfaces.
2. **Process the pending requests** in `requests/*/` (about a dozen, mostly to WS-PRIMITIVES and
   WS-PICKERS, plus two `.patch` files from WS-SETTINGS-A).
3. **Run the full gate** (the `gateLoop` in the saved workflow script): about 150 commits have
   landed since the last full green gate, and the Rust listing change has not had the full gate.
   Include `cargo fmt --check`, `./scripts/clippy-lint.sh`, the session route tests and
   `cargo test -p biorouter --test privacy_guard_wiring`.
4. **Vision rounds 2 to 4** (the saved script's loop): restart the four instances one at a time
   (they share the worktree's `.vite` build), five reviewers (shell and Home, chat, component
   views, Settings with dialogs and a Crew regression check, website), owner routed fixes, until
   each group is clean twice. The reviewers' prompts already check R-25 to R-28 and the fill rule.
5. **Coordinator documentation:** update `CLAUDE.md` (28px sidebar rows and the view menu, the new
   composer layout and Tools popover, the icons registry and lock badge, the fill rule and the
   Slider primitive; rung 0 and the PreviewBar bullet are already in), mark the requirements in
   [progress.md](progress.md) Done as they pass the vision loop, and answer or forward the owner
   questions in `RULINGS-NEEDED.md`.
6. **Pull request** against `main` with the human review list below. Do not merge: the landing
   site deploys when `main` changes, and the owner approves publication.

### Open items carried from vision round 1

- V3-19 and V4-30: one rule for the dialog footer hairline (RULINGS-NEEDED item 7).
- V4-19: the dark switch knob reads inverted (RULINGS-NEEDED item 8).
- V4-27 is half fixed and V4-13 follows a different reading of spec 3.14 (see `impl/WS-SETTINGS-B.md`).
- RULINGS-NEEDED also asks about a second Crew test flake (`Timeline.test.tsx` tab order), spec
  wording for the preview panel, and records that opening or renaming a chat counts as activity.
- History's own `GET /sessions` call still uses the old full listing query
  (`list_sessions_by_types_maybe_empty`); only the sidebar path was sped up.

## Needs human review in the pull request

- The session listing SQL changed shape (`aceb5d6e5`), and `MAX_SIDEBAR_SESSION_LIMIT` went from
  50 to 2,000. The privacy filters are still SQL predicates and unchanged in meaning, but it is
  the listing every private chat passes through.
- Privacy UI presentation moved: the Knowledge tier trigger is a badge in the band (its
  confirmation logic is unchanged), the model chip's privacy lines and the DR-17 disclosure moved
  into the model menu, the extension provenance strings stay visible.
- Text fields lost their resting border under the fill rule (recorded in `design.md` Part 9);
  forced colours still draw borders.

## How to resume cheaply

- Run at most six to eight agents at once. Fourteen at once hit the account's usage limit within
  35 minutes three times; each hit kills every agent in flight.
- Every agent must commit small steps and append notes to `impl/<WS>.md`; a resume prompt tells
  the next agent to read its notes, its commits and its uncommitted edits.
- Give agents spec line ranges instead of the whole 3,270 line spec.
- Sonnet is enough for inbox processing, gates and most fixes; keep Opus for the vision reviewers
  and for the summary rail, sidebar, primitives, composer and motion work.

## Traps the first run hit

- **`git commit -a` in a shared worktree** sweeps in other agents' unfinished files. Always add
  explicit paths. A plain `git commit` also takes whatever another agent has staged (it happened
  twice with staged deletions).
- **`main.css` partial staging corrupted the file twice** (fixed in `09dcc0213` and `f3b7a8116`).
  Stage `main.css` whole; it may carry other workstreams' blocks, which is fine.
- **Dev app instances:** start them one at a time, `chmod 700` the run's `electron` directory,
  dismiss the dependency and CLI update modals without installing anything, and know that
  `BIOROUTER_NO_HMR=1` means code changes need a restart (and a newly written Tailwind class only
  appears after one). The app runs `target/debug/biorouterd`, so rebuild it after any Rust change
  with `cargo build -p biorouter-server --bin biorouterd --features biorouter/privacy-test-auth`.
- **Never close all agent-browser sessions**; close only your own by name.
- `vitest` can exit 1 with zero failures (the `artifactCdnAssets` browser teardown); read the
  summary line.

## Related documentation

- [Requirements and plan](requirements-and-plan.md): R-01 to R-28 and the owner decisions.
- [Implementation spec](implementation-spec.md): the rules, ownership and verification; section 9
  holds the latest owner rulings.
- [Progress](progress.md): the live tracker and the dated log.
