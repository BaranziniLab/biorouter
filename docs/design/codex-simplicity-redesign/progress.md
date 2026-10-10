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
| 1. Research and audit | In progress | 19 agents: Codex, Claude usage, motion; shell, chat, summary, typography, primitives, preview, usage, two view groups, two settings halves, Crew reference, website, icons, baseline screenshots. |
| 2. Specification | Not started | |
| 3. Foundations | Not started | |
| 4. Surfaces | Not started | |
| 5. Verification loops | Not started | |
| 6. Review and pull request | Not started | |

## Requirements

| ID | Summary | Workstream | Status |
|----|---------|------------|--------|
| R-01 | Simplicity, Codex inspiration, Biorouter identity kept | all | In progress |
| R-02 | Remove unnecessary text and instructions | all | Not started |
| R-03 | Hover and focus help instead of paragraphs | WS-PRIMITIVES, all | Not started |
| R-04 | Website font family, few sizes and weights | WS-TOKENS | Not started |
| R-05 | Compact sidebar | WS-SIDEBAR | Not started |
| R-06 | Sidebar group by and sort by | WS-SIDEBAR | Not started |
| R-07 | Right click rename and actions | WS-SIDEBAR | Not started |
| R-08 | Docked chat summary rail | WS-SUMMARY | Not started |
| R-09 | Motion polish | WS-TOKENS, WS-MOTION | Not started |
| R-10 | Preview panel polish | WS-PREVIEW | Not started |
| R-11 | Usage dashboard on Home | WS-USAGE | Not started |
| R-12 | Chat interface redesign | WS-CHAT | Not started |
| R-13 | Composer redesign | WS-CHAT | Not started |
| R-14 | Crew kept as the reference | none | In progress |
| R-15 | Component views in Crew's language | WS-VIEWS-A, WS-VIEWS-B | Not started |
| R-16 | Knowledge simplified, same logic | WS-VIEWS-B | Not started |
| R-17 | Settings redesign | WS-SETTINGS | Not started |
| R-18 | One control style | WS-PRIMITIVES | Not started |
| R-19 | Icon audit and refresh | WS-ICONS | In progress |
| R-20 | Conversation kind icons | WS-ICONS | In progress |
| R-21 | Tool call icons | WS-ICONS | In progress |
| R-22 | Website revision | WS-LANDING | Not started |
| R-23 | Vision verification loops | WS-VERIFY | Not started |
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

## Related documentation

- [Requirements and plan](requirements-and-plan.md): what is being built and why.
- [Design folder index](../README.md): the other design records this work sits beside.
