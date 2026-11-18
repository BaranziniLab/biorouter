# Diverge behavior checklist

> **What this is.** A catalog of 68 concrete user actions for **Diverge** — the feature
> that branches a conversation into a new session — paired with the behavior BioRouter
> must exhibit for each. It doubles as a manual QA script and as the spec the automated
> tests encode.
> **Status:** Current. Updated 2026-11-17 for the browser's existing-tab delivery; the desktop window behavior is unchanged.
> **Audience:** maintainers of the desktop chat UI, and anyone running a Diverge
> regression pass.

Three labelling schemes are used below, and every one of them is cited elsewhere in the
repo, so keep the identifiers stable when editing:

- **Items 1–68** run in one continuous sequence across the lettered sections. Use the
  section index to find which section an item number lives in.
- **Sections A–I** group the items by surface or concern.
- **Invariants I1–I4** are the four core guarantees stated below; the rest of the
  document is a consequence of them, and section I checks I2 directly.

Each item is tagged with how it is verified: **[T]** is covered by an automated test
(see the coverage map at the end); **[UI]** requires driving the real app.

Window-specific checks below describe the desktop app. In a `biorouter serve` browser,
Diverge opens the new branch as the active chat tab in the existing page. Its canonical
name and retained transcript appear, the source tab remains available, and no browser
popup is needed. `App.routing.test.tsx` covers this path through the browser bridge,
route and tab provider.

Two window-model terms appear throughout. **Hub** is the landing screen where a user
starts a new conversation (`ui/desktop/src/components/Hub.tsx`). **Pair** is the tabbed
chat route at `/pair` that a conversation opens into.

> **Note.** 2026-07-18 — dashboard mode removed. This checklist previously carried ~25
> items about diverging on the free-floating dashboard canvas, including a whole section
> on "Chat ⇄ Dashboard isolation". Dashboard mode has been removed in favour of tabs,
> chat groups and split panes, so diverge no longer branches on canvas-vs-chat — it
> opens a new Electron window on the desktop. Those items were vacuous and have been deleted;
> everything below still describes real behaviour. See the
> [dashboard mode history](../history/dashboard-mode/README.md) for what was removed.
> Release notes and the dated dashboard plans and specs are historical records and were
> left untouched.

## Core invariants

- **I1.** Diverge branches the conversation into a *new session* that inherits the full
  history; the **original session is never mutated**.
- **I2.** Diverge only ever *spins up a new chat surface*. It leaves the agent and other
  conversations unchanged; the desktop's current window stays where it was.
- **I3.** On the desktop, Diverge **always opens a new, focused Electron window**. There is a single
  diverge surface — it never navigates the current window and never re-uses a tab or
  split pane.
- **I4.** Closing a chat surface **never deletes a conversation.** A branch, once
  created, stays in History until the user deletes it explicitly.

## Section index

| Section | Topic | Items |
|---------|-------|-------|
| A | Button presence and affordance | 1–8 |
| B | Diverge in a chat window | 9–20 |
| C | Diverge via the `/diverge` slash command (GUI) | 21–27 |
| D | Diverge via `/diverge` in the CLI and TUI | 28–36 |
| E | Closing surfaces and data preservation | 37–39 |
| F | Naming and lineage | 40–46 |
| G | Edge cases and failure modes | 47–56 |
| H | Persistence and reload | 57–58 |
| I | "Changes nothing else" guarantees (I2) | 59–68 |

## A. Button presence and affordance

1. **[UI]** Finish an assistant reply in a chat → a **Diverge** action appears next to **Copy** on hover.
2. **[UI]** While the assistant is still streaming → Diverge is **not** shown.
3. **[UI]** On a message containing a tool call/result → Diverge is **not** shown (text-only messages only).
4. **[UI]** On a user message → no Diverge (only assistant messages).
5. **[T]** Button is disabled while a diverge is in flight (no double-trigger).
6. **[UI]** Hover tooltip reads "Branch this conversation into a new window (keeps full history)" — one wording, on every chat surface.
7. **[UI]** Diverge button has an accessible label ("Diverge conversation into a new chat").
8. **[T]** Rapid triple-click triggers exactly **one** diverge.

## B. Diverge in a chat window (Hub / Pair / tab / split pane)

9. **[UI]** Diverge from a Pair window → a **second** Electron window opens.
10. **[UI]** The new window is **focused / in front**; the original stays where it was.
11. **[UI]** The new window is **offset** (~40px) from the original so both are visible.
12. **[UI]** The new window shows the **full prior history** of the original.
13. **[UI]** The original window is **unchanged** — same session id, same scroll, still interactive.
14. **[T]** The code path calls `window.electron.createDivergedChatWindow(workingDir, newSessionId)` — a new window, never an in-place navigation.
15. **[UI]** Continue typing in the new window → it appends to the **branch**, not the original.
16. **[UI]** Continue typing in the original window → unaffected by the branch.
17. **[UI]** Diverge twice from the same message → two independent new windows/sessions.
18. **[UI]** Close the new branch window → the original window and session remain intact.
19. **[UI]** Diverge from the Hub landing chat behaves the same as from Pair.
20. **[UI]** Model / mode / extensions in the new window match the branch's inherited config; the original's are untouched.

## C. Diverge via the `/diverge` slash command (GUI)

21. **[UI]** Type `/diverge` in the chat input → it appears in the slash popover.
22. **[UI]** Select it (Enter) → inserts `/diverge`; Enter again → branches (does **not** send a chat message).
23. **[UI]** `/diverge` from a chat opens a new focused window (same as the button).
24. **[UI]** `/diverge` with leading/trailing spaces still triggers.
25. **[UI]** `/divergexyz` or `/diverge now` is treated as a normal message (no branch).
26. **[UI]** `/diverge` with no active session yet (empty Hub) is a no-op, input clears.
27. **[UI]** After `/diverge`, the input is cleared and the original conversation is unchanged.

## D. Diverge via `/diverge` in the CLI and TUI

28. **[T]** CLI parser maps `/diverge` → `InputResult::Diverge`; near-misses don't.
29. **[T]** `/diverge` is in the CLI completion registry.
30. **[T]** The deeplink is `biorouter://diverge?session_id=…&dir=…`, URL-encoded.
31. **[UI]** Running `/diverge` in the TUI prints "Diverged into a new window (session …)".
32. **[UI]** The TUI's own conversation is unchanged after `/diverge`.
33. **[UI]** The deeplink opens a **new, focused** desktop window with the branch (`main.ts` `diverge` handler).
34. **[UI]** If the desktop app can't be opened, the TUI still reports the created branch + the link.
35. **[T]** A branch session row is created in the DB with full history + lineage.
36. **[UI]** Classic CLI `/diverge` renders the success/"couldn't open" output.

## E. Closing surfaces and data preservation

37. **[UI]** Closing a branch window **never** deletes its session — it is still listed in History.
38. **[UI]** Closing the *original* window never affects the branch, and vice-versa.
39. **[UI]** Close a branch window, then reopen it from History → it still loads with the full inherited history.

## F. Naming and lineage

40. **[T]** Branch name = `"{parent} (branch 1)"`, `"(branch 2)"`, … (sibling-numbered).
41. **[T]** Diverging a branch flattens numbering (no `(branch 1) (branch 1)`).
42. **[T]** Placeholder parent name → branch name derived from the conversation.
43. **[T]** Branch records `diverged_from = parent id`.
44. **[T]** Branch token counts reset; the parent's are preserved.
45. **[UI]** History shows "⑂ branched from {parent}" on a branch; parent shows none.
46. **[T]** A custom name passed to diverge overrides the auto branch name.

## G. Edge cases and failure modes

47. **[T]** Diverge with no session id → error toast, nothing opens.
48. **[T]** Backend diverge fails → error toast, nothing opens, original intact.
49. **[T]** Diverge response missing session id → error toast, nothing opens.
50. **[T]** Nonexistent source session → backend 404.
51. **[T]** Invalid session id → backend 400.
52. **[T]** Name longer than 200 chars → backend 400.
53. **[UI]** Diverge an empty conversation → branch created (empty), original intact.
54. **[T]** Concurrent diverges from one session → unique branch ids, no collision.
55. **[UI]** Diverge while the agent is mid-stream is not offered (button hidden while streaming).
56. **[T]** LIKE-wildcard names (`100%_done`) don't break sibling counting.

## H. Persistence and reload

57. **[UI]** A branch window survives an app restart (the session is in History).
58. **[UI]** Reload the app with a branch window open → it re-loads its session with full history.

## I. "Changes nothing else" guarantees (I2)

59. **[UI]** Diverge does not change the current window's model, mode, or extensions.
60. **[UI]** Diverge does not interrupt a different conversation's in-flight stream.
61. **[UI]** Diverge does not alter scroll position or input contents of other surfaces.
62. **[UI]** Switching conversations in chat after a diverge behaves exactly as before.
63. **[UI]** Opening/closing/reordering other tabs and split panes is unaffected by a diverge.
64. **[UI]** Knowledge / workflow / scheduler views are unaffected by diverge.
65. **[UI]** Diverge works regardless of the active provider/model.
66. **[UI]** Diverge from a renamed session keeps the original's user-set name.
67. **[UI]** The original conversation can still be exported/diverged/edited after a diverge.
68. **[UI]** No extra biorouterd backends leak per diverge beyond the new window's own.

## Automated coverage map

| Area | Test file |
|------|-----------|
| Hook behaviour (desktop window, failure paths) | `ui/desktop/src/hooks/useDiverge.test.tsx` |
| Serve branch route, title, transcript and source tab | `ui/desktop/src/App.routing.test.tsx` |
| Button affordance & failure modes | `ui/desktop/src/components/MessageDivergeLink.test.tsx` |
| CLI parsing + deeplink | `crates/biorouter-cli` (`session::input::tests`, `session::tests`) |
| Backend naming/lineage/history | `crates/biorouter` (`session_manager::tests`), `crates/biorouter-server` (`routes::session::diverge_tests`) |

## Related documentation

- [Debugging the dev GUI with agent-browser](agent-browser-debugging.md) — how to drive the real app for the `[UI]` items above.
- [Dashboard mode history](../history/dashboard-mode/README.md) — the removal that deleted ~25 items from this checklist.
- [Sessions](../getting-started/managing-sessions.md) — the session and history model that diverge branches from.
- [CLI command reference](../cli/command-reference.md) — where `/diverge` sits among the other CLI and TUI commands.
- [CLI QA checklist](../cli/qa-checklist.md) — the sibling manual test script covering the terminal surfaces.
