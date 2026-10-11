# Codex simplicity redesign: progress

> **What this is.** The live progress tracker for the redesign described in
> [requirements-and-plan.md](requirements-and-plan.md): phase status, the status of every
> requirement, and a dated log.
> **Status:** Current. Updated 2026-10-11: implemented, vision rounds 1 to 4 done, rounds 5 and 6
> running, final gate green at `0cb5ab465`.
> **Audience:** developers and agents continuing or reviewing this work.

Requirement IDs (`R-nn`) and workstream names (`WS-...`) are defined in the
[requirements and plan](requirements-and-plan.md). Status values: **Not started**,
**In progress**, **Done** (implemented and verified), **Blocked** (with the reason).

## Phases

| Phase | Status | Notes |
|-------|--------|-------|
| 1. Research and audit | Done | 20 agents: Codex, Claude usage, motion; shell, chat interface, summary, typography, primitives, preview, usage, two view groups, two settings halves, Crew reference, website, icons, baseline screenshots. |
| 2. Specification | Done | [implementation-spec.md](implementation-spec.md): synthesis, critic pass, coordinator rulings (section 8). |
| 3. Foundations | Done | All ten wave 0 contracts landed. |
| 4. Surfaces | Done | All 21 workstreams implemented and reviewed; integration gate green; wave 2 sweeps done. The R-27 fill sweep reached every workstream by 2026-10-11, and owner messages 14 to 16 added WS-PERF, WS-ACTIVITY and WS-BRAND. |
| 5. Verification loops | In progress | Rounds 1 to 4 done, with fixes after each: round 1 found 107 problems (45 high or medium), round 2 40 (12), round 3 28 (6), round 4 19 (6, none high). Rounds 5 and 6 running. See [handoff.md](handoff.md). |
| 6. Review and pull request | In progress | Final gate green at `0cb5ab465`. The pull request description is drafted outside the repository with the human review list; no pull request is open yet. |

## Workstreams and file ownership

Copied from the [implementation spec](implementation-spec.md) sections 3.17 to 3.19 when wave 0
started. Paths are under `ui/desktop/src/` unless noted. Each file has exactly one owner; the
spec's tables are authoritative where this summary is shorter.

| Workstream | Owns (summary) | Wave 0 contract | Status |
|------------|----------------|-----------------|--------|
| WS-TOKENS | `styles/main.css` token blocks, `styles/motion.ts`, `ui/desktop/themes/**`, token scripts, `design.md` | type roles, motion tokens and classes | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-PRIMITIVES | `components/ui/**` (not the sidebar files), `ModalShell.tsx`, Crew fix list | `InfoTip`, `SegmentedControl`, `SettingRow`, `RowActions`, `Field`, `FilterInput`, `Spinner`, `PopoverAnchor` | Implemented and reviewed; a round 4 medium fix awaits vision round 5 |
| WS-ICONS | `components/icons/**`, `chats/{chatKind.ts,ChatKindIcon.tsx}`, `utils/{toolIconMapping.tsx,toolGlyph.ts}`, `ItemIcon.tsx` | `toolGlyph.ts`, `chatKind.ts` with the crew kind | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-SIDEBAR | `BioRouterSidebar/**`, `ui/{sidebar.tsx,sidebarWidth.ts}`, `TitlebarControls.tsx`, `chats/**` (rest), session name, delete and export helpers | `renameSessionOptimistically`, `chatDateBuckets`, `exportConversation`, row menu props | Implemented and reviewed; a round 4 medium fix awaits vision round 5 |
| WS-HISTORY | `sessions/**` (not the usage files), `utils/dateUtils.ts` | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-SUMMARY | `BaseChat.tsx`, `ChatSummary.tsx`, `components/summary/**`, rail hooks, `Layout/yieldLadder.ts` | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-TRANSCRIPT | messages, markdown, activity, `subagent/**` | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-TOOLS | tool call rows and cards, `tool-call.css` | `TranscriptRow` | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-COMPOSER | `ChatInput.tsx`, `MessageQueue.tsx`, `MentionPopover.tsx`, `Hub.tsx` | none | Implemented and reviewed; a round 4 medium fix awaits vision round 5 |
| WS-PICKERS | `bottom_menu/**`, `ContextWindowIndicator.tsx`, `ModelsBottomBar.tsx`, `CopilotControl.tsx` | `ToolsChip`, `ModelEffortChip`, `ComposerFooter` | Implemented and reviewed; a round 4 medium fix awaits vision round 5 |
| WS-PREVIEW | `artifacts/**`, `.artifact-harness/**` | `sideWidth` on the panel controller | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-USAGE | `sessions/{SessionsInsights,UsageHeatmap,HomeUsageCard}.tsx`, `components/usage/**`, `settings/usage/**` | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-VIEWS-A | `Layout/{PageHeader,MainPanelLayout,ReadableContent,AppLayout}.tsx`, `workflows/**`, `schedule/**`, `applications/**` | `PageHeader` band API | Implemented and reviewed; a round 4 medium fix awaits vision round 5 |
| WS-EXTENSIONS | `extensions/**`, `settings/extensions/**`, install modals | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-SKILLS | `skills/**`, `baam/**` | `MarketplaceDialog` | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-KNOWLEDGE | `knowledge/**` (tier confirmation logic frozen) | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-SETTINGS-A | `SettingsView.tsx`, `settings/{app,chat,mode,permission,response_styles}/**`, theme selectors | section ids for deep links | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-SETTINGS-M | `settings/{models,providers,reset_provider}/**`, `onboarding/**`, `ProviderGuard.tsx` | none | Implemented and reviewed; a round 4 medium fix awaits vision round 5 |
| WS-SETTINGS-B | `settings/{capabilities,config,contexts,brsdk,memory}/**`, Copilot setup | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-MOTION | `chatGroups/**`, find in chat, `App.tsx`, `toasts.tsx`, `alerts/**`, `main.ts` window regions, terminal dock | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-LANDING | `landing/**`, `docs/website/**` | none | Implemented and reviewed; no open high or medium after vision round 3 (round 4 did not review the site) |
| WS-PERF | the sidebar's listing path: `BioRouterSidebar/{useSidebarSessions.ts,sidebarChatView.ts}` helpers, the session listing route in `crates/biorouter-server/src/routes/session.rs` and its queries in `crates/biorouter/src/session/session_manager.rs` | none | Implemented and reviewed; a round 4 medium fix awaits vision round 5 |
| WS-ACTIVITY | `crates/biorouter/src/session/activity.rs` and the opening doors that enter its scope (agent routes, apps route, CLI session builder, ACP server) | none | Implemented and reviewed; no open high or medium after vision round 4 |
| WS-BRAND | the R-29 and R-31 removals across the tree, `scripts/check-no-goose-block.sh`, `crates/biorouter-mcp/src/app_folder/`, `utils/appFolder.ts` | none | Implemented and reviewed; no open high or medium after vision round 4 on macOS; the Windows folder move has not run on Windows |
| Coordinator | `docs/design/codex-simplicity-redesign/**`, `CLAUDE.md`, `scripts/check-crew-manual.mjs`, `docs/crew/**`, the shared dev app instance; a coordinator delegate applies requests addressed to it | ownership copy | Done |

## Requirements

| ID | Summary | Workstream | Status |
|----|---------|------------|--------|
| R-01 | Simplicity, Codex inspiration, Biorouter identity kept | all | Implemented; vision round 5 pending (it spans every surface; round 4 left six medium findings, all fixed since) |
| R-02 | Remove unnecessary text and instructions | all | Done |
| R-03 | Hover and focus help instead of paragraphs | WS-PRIMITIVES, all | Done |
| R-04 | Website font family, few sizes and weights | WS-TOKENS | Done (round 4 type probes: sans 12, 13 and 14, mono 12 and 13) |
| R-05 | Compact sidebar | WS-SIDEBAR | Done (round 4: row density exact at 216, 288 and 360px) |
| R-06 | Sidebar group by and sort by | WS-SIDEBAR | Implemented; vision round 5 pending (V1-R2-2: view switch speed) |
| R-07 | Right click rename and actions | WS-SIDEBAR | Implemented; vision round 5 pending (V1-R2-2: the row menu on a long drawn list) |
| R-08 | Docked chat summary rail | WS-SUMMARY | Done |
| R-09 | Motion polish | WS-TOKENS, WS-MOTION | Done |
| R-10 | Preview panel polish | WS-PREVIEW | Done |
| R-11 | Usage dashboard on Home | WS-USAGE | Done |
| R-12 | Chat interface redesign | WS-TRANSCRIPT, WS-TOOLS | Done |
| R-13 | Composer redesign | WS-COMPOSER, WS-PICKERS | Implemented; vision round 5 pending (V2-R4-1: the context ring's first reading) |
| R-14 | Crew kept as the reference | none | Done (round 4: no Crew regressions) |
| R-15 | Component views in Crew's language | WS-VIEWS-A, WS-EXTENSIONS, WS-SKILLS | Implemented; vision round 5 pending (V3-R4-1: New schedule's empty name error) |
| R-16 | Knowledge simplified, same logic | WS-KNOWLEDGE | Done |
| R-17 | Settings redesign | WS-SETTINGS-A, WS-SETTINGS-M, WS-SETTINGS-B | Implemented; vision round 5 pending (V4-R4-1, V4-R4-3) |
| R-18 | One control style | WS-PRIMITIVES | Implemented; vision round 5 pending (V4-R4-2, V4-R4-3: the last raw dialog and native select) |
| R-19 | Icon audit and refresh | WS-ICONS | Done |
| R-20 | Conversation kind icons | WS-ICONS | Done |
| R-21 | Tool call icons | WS-ICONS | Done |
| R-22 | Website revision | WS-LANDING | Done (rounds 2 and 3 found no high or medium on the site) |
| R-23 | Vision verification loops | coordinator (vision loop) | In progress: rounds 1 to 4 done, rounds 5 and 6 running |
| R-24 | Requirements and progress documents | coordinator | Done (kept current) |
| R-25 | Tools popover switcher at the bottom | WS-PICKERS | Done |
| R-26 | Effort slider | WS-PRIMITIVES, WS-PICKERS | Done |
| R-27 | Fill over lines, plus a polish review | WS-TOKENS, WS-PRIMITIVES, all | Done |
| R-28 | Fast sidebar reordering (listing path) | WS-PERF | Implemented; vision round 5 pending (V1-R2-2: the list now draws only the rows near the viewport) |
| R-29 | The wordmark is the logo; unreferenced logo images deleted | WS-BRAND | Done |
| R-30 | Opening a chat is not activity | WS-ACTIVITY, WS-SIDEBAR | Done (rounds 2 to 4: opening moved nothing; a rename and a sent message moved the chat at once) |
| R-31 | The retired upstream and its publisher named nowhere but NOTICE | WS-BRAND | Done on macOS (the repo guard passes; rounds 2 to 4 found no mention); the Windows folder move has not run on Windows |

## Log

### 2026-10-09

- Branch `claude/crew-landing-redesign-91b46e` created from `main` at `30e1f0622`.
- Built a daemon matching this tree (`cargo build -p biorouter-server --bin biorouterd --features
  biorouter/privacy-test-auth`) so the dev app can be launched in a sandbox for screenshots.
- Owner decisions recorded: website font family kept; Crew is the reference; Knowledge keeps its
  structure.
- Phase 1 started: research and audit agents running in parallel; reports are written outside the
  repository and summarized here when the specification lands.
- Requirements and this tracker added to the repository.
- Phase 1 done: 20 research and audit reports, baseline screenshots of every surface in light and
  dark at 1440 and 1100 wide, and an icon audit with contact sheets.
- Phase 2 done: [implementation-spec.md](implementation-spec.md) added. Coordinator rulings: the
  section 6 defaults stand, except that the website keeps "Join Slack" in the header (the owner's
  September decision) and the sidebar keeps the coral active row rail.
- Owner messages 7 to 12 folded in: icons per chat kind and per tool (built in only), keep the
  accent and personality, sidebar density measured against Codex and Claude Code.
- Waves 0 and 1 started: 21 workstream agents in one worktree, each owning a disjoint file set.

### 2026-10-10

- The first implementation run (14 agents at once) hit the account's usage limit after about 35
  minutes. Before it stopped, 74 commits had landed, including all ten wave 0 contracts (tokens,
  primitives, icons, page band, sidebar helpers, transcript row, pickers, marketplace dialog,
  settings section ids, preview width). Seven workstreams had not started.
- Coordinator requests applied: Settings and Skills doc wording, the Skills e2e locator, and
  `--row-height-nav` scaling with Text size.
- Implementation resumed through a pool of eight agents at a time: each resumed workstream picks
  up its predecessor's notes, commits and uncommitted edits; a reviewer follows each one; a gate
  then runs the full lint, format and test suites.
- The resumed run finished 37 of 42 agents before the usage limit hit again: 219 more commits.
  Twenty workstreams are implemented, and seventeen of those are also reviewed. Still open: the
  website workstream, three reviews (primitives, models and providers, extensions), the pending
  requests between workstreams, and about 30 coordinator requests (doc wording, e2e locators, the
  Cmd+Opt+R rename shortcut in `main.ts`, approved).
- Started the finishing run: a coordinator delegate applies or routes the coordinator requests,
  then the website resumes, the three reviews and every inbox run, then the full gate loop, then
  the wave 2 sweeps (alias deletions, the switch variant removal, stricter guards) and a final gate.
- Finishing run done (37 agents, none lost): the coordinator delegate applied or routed 29
  requests (doc wording for the moved Settings, Skills, Extensions and Knowledge surfaces, e2e
  locators, the Crew manual checker, and the Cmd+Opt+R rename shortcut, which no longer reloads
  the window); the website workstream finished; the last three reviews ran; every inbox emptied.
- Integration gate green: `lint:check` (944 contrast assertions), `format:check`, all 851 test files
  (11,858 tests), the website consistency and Crew manual checks, clean tree. Wave 2 sweeps removed
  the old aliases and tightened the guards; the second gate was green too.
- Coordinator rulings: dialogs take Crew's top anchor and header rule app wide; the unreferenced
  logo images are deleted (R-29); the 14px body default lands before the vision loop so the loop sees the final
  sizes.
- Vision loop started: four sandboxed app instances (shell and Home, chat, component views,
  Settings with dialogs and a Crew regression check) and the website, each reviewed from real
  screenshots in light and dark at several widths.
- Vision round 1 (five reviewers on four sandboxed app instances and the website) found 107
  problems, 45 of them high or medium. Fixes for about half landed (tabs grow in, the sidebar
  slides with its gap, the rail survives StrictMode and rescales, find in chat stays pinned, the
  History list no longer re-sorts in front of the reader, dates and stats read cleanly) before the
  usage limit stopped the rest.
- Owner message 13 added R-25 (Tools popover switcher at the bottom), R-26 (Effort slider) and
  R-27 (subtle solid fills instead of outlines, plus a polish review). Spec principle 8 now reads
  "fills come before lines", and spec section 9 holds the rulings. The vision loop was stopped so
  that reviewers and fixers aim at the new rule; it resumes after the fill sweep.
- Round 1 fixes landed for every owner. Owner message 13 first changes landed: the Tools switcher
  at the bottom (R-25), the Slider primitive and the Effort slider (R-26), the fill tokens and the
  filled shared controls (R-27). The sidebar listing is fast (R-28): a 50 row page 118 to 10 ms, a
  complete reordered list of 2,000 chats 3.1 s to about 0.2 s.
- The R-27 surface sweep finished for six workstreams; WS-TOOLS' sweep was committed unreviewed when
  the run stopped. The coordinator stopped work here to save tokens and wrote
  [handoff.md](handoff.md) with the remaining steps: eleven surface sweeps, pending requests, a
  full gate, vision rounds 2 to 4, CLAUDE.md updates and the pull request.
- A second coordinator took over from the handoff in the evening. Coordinator rulings 5 to 13
  answered the open questions: the 14px body is recorded as done, the dialog footer rule stays (it
  marks a body that scrolls), the dark switch's off knob matches light, the preview panel's round 1
  details go into spec 3.8, two privacy tests take the filled Note's classes, the shared controls
  are filled in Crew too, and the command search glyph sits on the 16px edge.
- Owner messages 15 and 16 added R-29 (the wordmark is the logo), R-30 (opening a chat is not
  activity) and R-31 (the retired upstream and its publisher are named nowhere but NOTICE). Spec
  section 10 records them.
- R-29: the unreferenced logo images and the unused logo components are deleted (`c351c5cc2`,
  `aabd540ff`).
- R-31: the retired names left the interface, the CLI, the recorded prompts, the test fixtures, the
  docs, the history records and the release notes; the private-environment filters lost the retired
  prefix (`11fb8b649`); the LICENSE appendix line went (`9bdb97464`); a repo guard keeps the count
  at zero (`5133325c6`).
- R-30: opening a chat no longer changes its activity time or its place in any list
  (`5c9d742e8`), with tests at every door (`0f1fe36d7`).
- The R-27 fill sweep landed for the eleven remaining workstreams (finished the next morning), and
  WS-TOOLS' unreviewed sweep was reviewed.

### 2026-10-11

- Windows: the app folder is now `%APPDATA%\UCSF\biorouter` (cache `%LOCALAPPDATA%\UCSF\biorouter`).
  The daemon, and only the daemon, moves the older folder once, waits while another Biorouter
  program uses it, and rewrites the paths stored inside (`eed608d6a`, `4830531f7`, `bce54c000`). It
  is tested on macOS with Windows layouts and has not run on Windows.
- Gate h, rounds 1 to 3: rounds 1 and 2 found a `.br-rows-flat` rule defined twice (it failed two
  Chromium cases), an uncommitted test edit that failed `format:check`, and a test sandbox the
  privacy census counted as production; each was fixed. Round 3 passed every check and was held red
  only by pending requests and by edits that sessions in another worktree could not commit.
- The write block: a hook refuses edits into this worktree from a session whose own worktree is
  another one, so three delegate passes queued verified patches instead of landing them. Ruling 18:
  every agent is launched from a session in this worktree, and the queued patches landed after a
  fresh `git apply --check`.
- Coordinator rulings 14 to 19: the Windows folder question is closed by the move; the two class
  only restyles in the privacy UI are applied (`6bb7daced`); ruling 16 sets what counts as activity;
  the CLI's approval mode words follow the desktop's; the write block; the LICENSE line stays
  removed.
- Gate L at `4c40d384b` green, then vision round 2 (five reviewers): 40 findings, 12 high or
  medium. Fixes landed the same morning; round 3 found eleven of the twelve fixed and re-reported
  the sidebar's view switch speed (V1-R2-2).
- Vision round 3 at `76affedac`: 28 findings, 6 high or medium, among them a terminal dock that
  could not start a shell, History taking about 2 s to show its rows, and dialogs opened from the Skills
  and Extensions Add menus that dropped keyboard focus on close. Round 4 found four of the six fixed; the view switch speed and History's
  load time (now 0.95 s, re-filed as low) were better but not done.
- Vision round 4 at `5625a0d74` (four reviewers; the site was clean in rounds 2 and 3): 19
  findings, 6 medium and none high. All six have fixes, including a sidebar that draws only the rows
  near the viewport; round 5 checks them.
- Final gate, round 1 at `2d236ebd7`: every check passed, but the tree had an untracked `public/`
  (a vite run from the repository root writes `public/pdfjs`); `0cb5ab465` ignores it. Round 2 at
  `0cb5ab465` green: `lint:check` (1,148 contrast assertions), `format:check`, 946 test files
  (13,095 tests), the site and Crew checks, 175 site tests, `cargo fmt`, clippy, 509 session tests,
  `privacy_guard_wiring`, and every `biorouter-server` target with `BIOROUTER_DISABLE_KEYRING=true`
  as CI sets it.
- Vision rounds 5 and 6 started. The docs pass put the last two spec wordings in (3.4, the moves
  that are not held; 3.6, the message actions' 26px rise) and added R-29 to R-31 to these records.

## Related documentation

- [Handoff](handoff.md): what is left and how to resume.
- [Requirements and plan](requirements-and-plan.md): what is being built and why.
- [Design folder index](../README.md): the other design records this work sits beside.
