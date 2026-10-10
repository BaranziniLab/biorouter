# Codex simplicity redesign: progress

> **What this is.** The live progress tracker for the redesign described in
> [requirements-and-plan.md](requirements-and-plan.md): phase status, the status of every
> requirement, and a dated log.
> **Status:** Current. Updated as each step lands; work is in progress.
> **Audience:** developers and agents continuing or reviewing this work.

Requirement IDs (`R-nn`) and workstream names (`WS-...`) are defined in the
[requirements and plan](requirements-and-plan.md). Status values: **Not started**,
**In progress**, **Done** (implemented and verified), **Blocked** (with the reason).

## Phases

| Phase | Status | Notes |
|-------|--------|-------|
| 1. Research and audit | Done | 20 agents: Codex, Claude usage, motion; shell, chat interface, summary, typography, primitives, preview, usage, two view groups, two settings halves, Crew reference, website, icons, baseline screenshots. |
| 2. Specification | Done | [implementation-spec.md](implementation-spec.md): synthesis, critic pass, coordinator rulings (section 8). |
| 3. Foundations | In progress | Wave 0 contracts (spec section 4). |
| 4. Surfaces | In progress | Wave 1, in parallel with wave 0 where no contract is needed. |
| 5. Verification loops | Not started | |
| 6. Review and pull request | Not started | |

## Workstreams and file ownership

Copied from the [implementation spec](implementation-spec.md) sections 3.17 to 3.19 when wave 0
started. Paths are under `ui/desktop/src/` unless noted. Each file has exactly one owner; the
spec's tables are authoritative where this summary is shorter.

| Workstream | Owns (summary) | Wave 0 contract | Status |
|------------|----------------|-----------------|--------|
| WS-TOKENS | `styles/main.css` token blocks, `styles/motion.ts`, `ui/desktop/themes/**`, token scripts, `design.md` | type roles, motion tokens and classes | In progress |
| WS-PRIMITIVES | `components/ui/**` (not the sidebar files), `ModalShell.tsx`, Crew fix list | `InfoTip`, `SegmentedControl`, `SettingRow`, `RowActions`, `Field`, `FilterInput`, `Spinner`, `PopoverAnchor` | In progress |
| WS-ICONS | `components/icons/**`, `chats/{chatKind.ts,ChatKindIcon.tsx}`, `utils/{toolIconMapping.tsx,toolGlyph.ts}`, `ItemIcon.tsx` | `toolGlyph.ts`, `chatKind.ts` with the crew kind | In progress |
| WS-SIDEBAR | `BioRouterSidebar/**`, `ui/{sidebar.tsx,sidebarWidth.ts}`, `TitlebarControls.tsx`, `chats/**` (rest), session name, delete and export helpers | `renameSessionOptimistically`, `chatDateBuckets`, `exportConversation`, row menu props | In progress |
| WS-HISTORY | `sessions/**` (not the usage files), `utils/dateUtils.ts` | none | In progress |
| WS-SUMMARY | `BaseChat.tsx`, `ChatSummary.tsx`, `components/summary/**`, rail hooks, `Layout/yieldLadder.ts` | none | In progress |
| WS-TRANSCRIPT | messages, markdown, activity, `subagent/**` | none | In progress |
| WS-TOOLS | tool call rows and cards, `tool-call.css` | `TranscriptRow` | In progress |
| WS-COMPOSER | `ChatInput.tsx`, `MessageQueue.tsx`, `MentionPopover.tsx`, `Hub.tsx` | none | In progress |
| WS-PICKERS | `bottom_menu/**`, `ContextWindowIndicator.tsx`, `ModelsBottomBar.tsx`, `CopilotControl.tsx` | `ToolsChip`, `ModelEffortChip`, `ComposerFooter` | In progress |
| WS-PREVIEW | `artifacts/**`, `.artifact-harness/**` | `sideWidth` on the panel controller | In progress |
| WS-USAGE | `sessions/{SessionsInsights,UsageHeatmap,HomeUsageCard}.tsx`, `components/usage/**`, `settings/usage/**` | none | In progress |
| WS-VIEWS-A | `Layout/{PageHeader,MainPanelLayout,ReadableContent,AppLayout}.tsx`, `workflows/**`, `schedule/**`, `applications/**` | `PageHeader` band API | In progress |
| WS-EXTENSIONS | `extensions/**`, `settings/extensions/**`, install modals | none | In progress |
| WS-SKILLS | `skills/**`, `baam/**` | `MarketplaceDialog` | In progress |
| WS-KNOWLEDGE | `knowledge/**` (tier confirmation logic frozen) | none | In progress |
| WS-SETTINGS-A | `SettingsView.tsx`, `settings/{app,chat,mode,permission,response_styles}/**`, theme selectors | section ids for deep links | In progress |
| WS-SETTINGS-M | `settings/{models,providers,reset_provider}/**`, `onboarding/**`, `ProviderGuard.tsx` | none | In progress |
| WS-SETTINGS-B | `settings/{capabilities,config,contexts,brsdk,memory}/**`, Copilot setup | none | In progress |
| WS-MOTION | `chatGroups/**`, find in chat, `App.tsx`, `toasts.tsx`, `alerts/**`, `main.ts` window regions, terminal dock | none | In progress |
| WS-LANDING | `landing/**`, `docs/website/**` | none | In progress |
| Coordinator | `docs/design/codex-simplicity-redesign/**`, `CLAUDE.md`, `scripts/check-crew-manual.mjs`, `docs/crew/**`, the shared dev app instance | ownership copy | Done |

## Requirements

| ID | Summary | Workstream | Status |
|----|---------|------------|--------|
| R-01 | Simplicity, Codex inspiration, Biorouter identity kept | all | In progress |
| R-02 | Remove unnecessary text and instructions | all | In progress |
| R-03 | Hover and focus help instead of paragraphs | WS-PRIMITIVES, all | In progress |
| R-04 | Website font family, few sizes and weights | WS-TOKENS | In progress |
| R-05 | Compact sidebar | WS-SIDEBAR | In progress |
| R-06 | Sidebar group by and sort by | WS-SIDEBAR | In progress |
| R-07 | Right click rename and actions | WS-SIDEBAR | In progress |
| R-08 | Docked chat summary rail | WS-SUMMARY | In progress |
| R-09 | Motion polish | WS-TOKENS, WS-MOTION | In progress |
| R-10 | Preview panel polish | WS-PREVIEW | In progress |
| R-11 | Usage dashboard on Home | WS-USAGE | In progress |
| R-12 | Chat interface redesign | WS-TRANSCRIPT, WS-TOOLS | In progress |
| R-13 | Composer redesign | WS-COMPOSER, WS-PICKERS | In progress |
| R-14 | Crew kept as the reference | none | In progress |
| R-15 | Component views in Crew's language | WS-VIEWS-A, WS-EXTENSIONS, WS-SKILLS | In progress |
| R-16 | Knowledge simplified, same logic | WS-KNOWLEDGE | In progress |
| R-17 | Settings redesign | WS-SETTINGS-A, WS-SETTINGS-M, WS-SETTINGS-B | In progress |
| R-18 | One control style | WS-PRIMITIVES | In progress |
| R-19 | Icon audit and refresh | WS-ICONS | In progress |
| R-20 | Conversation kind icons | WS-ICONS | In progress |
| R-21 | Tool call icons | WS-ICONS | In progress |
| R-22 | Website revision | WS-LANDING | In progress |
| R-23 | Vision verification loops | coordinator (vision loop) | Not started |
| R-24 | Requirements and progress documents | coordinator | Done |

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

## Related documentation

- [Requirements and plan](requirements-and-plan.md): what is being built and why.
- [Design folder index](../README.md): the other design records this work sits beside.
