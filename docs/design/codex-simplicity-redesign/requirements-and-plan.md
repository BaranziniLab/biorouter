# Codex simplicity redesign: requirements and plan

> **What this is.** The owner's requirements for the October 2026 redesign of the desktop app
> and the website, restated as numbered items, and the plan that turns them into work.
> **Status:** Current. Work in progress; the live state of every item is in
> [progress.md](progress.md).
> **Audience:** developers and agents working on the desktop renderer or the landing site.

In October 2026 the owner asked for a redesign of the whole desktop app and a careful revision of
the website, with one governing rule: simplicity. The inspiration is OpenAI Codex (the
[openai/codex](https://github.com/openai/codex) repository and the Codex desktop app). The
requirements arrived as a series of messages with screenshots; this page restates them as items
`R-01` to `R-24` so the plan and the progress tracker can refer to each one. Workstream names use
the prefix `WS-`. Both schemes are local to this folder.

## Requirements

### Direction

| ID | Requirement |
|----|-------------|
| R-01 | Simplicity governs every decision. Take inspiration from Codex for structure and restraint, and keep Biorouter's personality: the coral accent (each family's accent), warm ink, UCSF navy, the BR mark, the three theme families, light and dark, and a few deliberate moments of color so the app never reads as grey. |
| R-02 | Remove unnecessary interface text, and remove instructions the user no longer needs to read. |
| R-03 | Long explanations become small help that appears on hover and on keyboard focus, instead of paragraphs shown up front. Help stays reachable by keyboard and by screen readers. |
| R-04 | Use the website's font family everywhere (Arial first interface stack; Inter only for the wordmark; one monospace stack for code). Use only a few font sizes and weights. |

### Shell and navigation

| ID | Requirement |
|----|-------------|
| R-05 | Make the sidebar compact and clean, like the Codex and Claude Code sidebars. |
| R-06 | Let the user organize the sidebar's conversations. The default stays grouped by date, newest first. Offer other groupings (such as project folder) and other orders (such as last activity), and remember the choice. |
| R-07 | Right clicking a conversation in the sidebar offers Rename, done in place, together with the other actions a conversation supports. |
| R-08 | The chat summary becomes a narrow rail docked to the right of the conversation. When a turn starts it appears and pushes the 760px conversation column a little to the left. It updates live as tasks and to do items change, and it shrinks, hides and returns as the window or the preview panel changes size. |
| R-09 | Polish the motion for searching tabs, opening tabs, changing elements, resizing the window, opening and closing windows, and opening and closing previews. Respect the reduced motion setting. |

### Surfaces

| ID | Requirement |
|----|-------------|
| R-10 | The preview panel shows code and every other content type cleanly, with smooth transitions. |
| R-11 | The token page on Home becomes a usage dashboard in the style of the Claude and Codex usage pages, keeping the current colors. |
| R-12 | The chat interface (messages, tool calls, cards) is redesigned, not only Home. |
| R-13 | The composer follows the same principles: monospace only for code, consistent icons, a compact footer. |
| R-14 | Crew is the reference design. It is not redesigned; its design language is applied to the rest of the app. |
| R-15 | Workflows, Scheduler, Extensions, Skills, Knowledge and Built apps follow Crew's language. |
| R-16 | Knowledge keeps its view structure and interaction logic, with a much simpler look. |
| R-17 | Settings is redesigned to be minimal and clean. |
| R-18 | Toggles and other controls share one consistent style across the app. |

### Icons

| ID | Requirement |
|----|-------------|
| R-19 | Audit every icon. Redraw or replace icons so each is simple, clear and representative, and use the same icon for the same concept everywhere. |
| R-20 | Conversations of different kinds have different icons: private and public, Crew, scheduled, sub agent, branch, app, terminal. Private and public differ by shape, not only by color. |
| R-21 | Each tool call in the transcript shows an icon that represents that tool: running a command, reading a file, using the knowledge base, using Crew, using the other components. The wrench remains only as the last fallback. |

### Website, verification and records

| ID | Requirement |
|----|-------------|
| R-22 | Revise the landing website carefully toward the same simplicity, keeping all of its content and the same font family as the app. |
| R-23 | Verify with real screenshots read by a vision model: every surface, light and dark, several window sizes, and the interactions above. Repeat until no problems remain. |
| R-24 | Keep this requirements document and the [progress tracker](progress.md) in the repository and update them until the work is finished. |

## Owner decisions

| Decision | Choice |
|----------|--------|
| Interface font | The website's family: Arial first stack for interface text, Inter for the wordmark only. Sizes and weights are consolidated; the face does not change. |
| Crew | Reference only. Shared token changes reach it; its layout and components are not redesigned. |
| Knowledge | Same structure and flows; simpler visuals. |
| Website publication | The site deploys when `main` changes, so nothing is merged without the owner's approval. |

## Constraints

- Privacy and security behavior is not weakened. Session writes such as rename keep sending the
  user action proof (`userActionHeaders()`).
- The shell geometry invariants in `CLAUDE.md` hold unless a requirement changes them on purpose:
  sidebar range 216 to 360px with a 288px default, `--chrome-height` of 44px, `--measure-chat` of
  760px, drag regions above toasts.
- Theme files are generated: edit `ui/desktop/themes/*.theme.mjs` and run `npm run themes`, never
  the generated regions.
- User facing text: the brand is spelled Biorouter; no dashes as punctuation; plain words.
- Frontend checks before every commit: `npm run lint:check`, `npm run format:check`, and the
  affected Vitest files.

## Plan

### Phases

1. **Research and audit.** Parallel agents study Codex and Claude, audit every surface of the
   app, Crew (as the reference), the website and every icon, and capture baseline screenshots of
   the running app.
2. **Specification.** One synthesis turns the reports into a single specification with
   workstreams and disjoint file ownership; a critic checks it against every requirement above.
3. **Foundations.** Type and motion tokens, shared control primitives (switch, checkbox,
   segmented control, menus, help tooltip) and the icon set land first, because every other
   workstream builds on them.
4. **Surfaces.** The remaining workstreams run in parallel, each owning its own files.
5. **Verification loops.** Screenshot every surface at several sizes in light and dark, have a
   vision model review each one against the requirements, fix what it finds, and repeat until a
   round finds nothing.
6. **Review and pull request.** Lint, format, tests, a code review pass, then a pull request. The
   website change is not merged without the owner.

### Workstreams

| Workstream | Requirements | Scope |
|------------|--------------|-------|
| WS-TOKENS | R-04, R-09 | Type scale, weights, motion tokens in `styles/main.css` and the theme sources. |
| WS-PRIMITIVES | R-03, R-18 | Switch, checkbox, segmented control, menus, popovers, the help tooltip in `components/ui`. |
| WS-ICONS | R-19, R-20, R-21 | Icon registry, conversation kind icons, the tool call icon map. |
| WS-SIDEBAR | R-05, R-06, R-07 | Compact sidebar, the organize menu, inline rename and the full context menu. |
| WS-SUMMARY | R-08 | The docked chat summary rail and its width rules. |
| WS-CHAT | R-12, R-13 | Messages, tool call rows, cards, the composer on Home and in chats. |
| WS-PREVIEW | R-10 | The artifact preview panel and its motion. |
| WS-USAGE | R-11 | The usage dashboard on Home. |
| WS-VIEWS-A | R-15 | Workflows, Scheduler, Built apps, shared page chrome. |
| WS-VIEWS-B | R-15, R-16 | Extensions and the marketplace, Skills, Knowledge. |
| WS-SETTINGS | R-17, R-03 | Every Settings section. |
| WS-MOTION | R-09 | Tabs, search, dialogs, window open and close, outside the files above. |
| WS-LANDING | R-22 | The landing website. |
| WS-VERIFY | R-23 | Screenshot review loops across all of the above. |

The workstream list is provisional until the specification phase finishes; the final file
ownership is recorded in [progress.md](progress.md).

## Related documentation

- [Progress tracker](progress.md): the live status of every requirement and workstream.
- [Biorouter Design System](../../../design.md): the decision records this work builds on.
- [Theme system architecture](../theming/theme-system-architecture.md): how theme tokens are generated.
- [UI overhaul](../ui-overhaul/README.md): the previous whole interface revision.
- [Settings visual vocabulary](../../desktop-ui/settings-visual-vocabulary.md): the rules Settings is built to today.
