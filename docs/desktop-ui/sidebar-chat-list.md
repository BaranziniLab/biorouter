# The sidebar chat list

> **What this is.** How the app sidebar lists chats: the Chats header and its View options menu,
> Group by and Sort by (nine combinations), how every chat is read when a view needs them all,
> folder groups, the row menu, renaming a chat in place, the keyboard model, and the tests that
> hold each part.
> **Status:** Current. Built in the Codex simplicity redesign (WS-SIDEBAR, October 2026).
> **Audience:** developers working on the sidebar, the chat row menu, History or the tab strip.

The sidebar used to list the newest chats under one caps label with a header per day. It now reads
like Codex's: a quiet "Chats" header, 28px rows on a 30px pitch, coarse date buckets, and a View
options menu that groups and sorts the same list in other ways. The geometry (28px rows, the icon
at x=16, the label at x=40, a 44px band) is authored in `BioRouterSidebar/sidebar.css` and measured
in Chromium by `BioRouterSidebar/sidebarGeometry.browser.test.ts`.

## The header

"Chats" is a button: clicking the label folds the list (`data-testid="recents-disclosure"`, the
fold is remembered in `biorouter:sidebar-recents-expanded`). Beside it are two 24px icon buttons.
**View options** opens the menu below, and **All chats** opens History (`view-all-chat-history`).
While the list is folded, All chats stays reachable.

## View options

The menu has two submenus, each a set of radio items:

| Submenu | Choices | Default |
|---|---|---|
| Group by | Date, Folder, None | Date |
| Sort by | Last activity, Created, Name | Last activity |

`BioRouterSidebar/sidebarChatView.ts` arranges the chats as pure functions of the list, the view and
the clock, so every combination is tested without a DOM:

| Group by | Groups | Order of groups | Order of rows |
|---|---|---|---|
| Date | Today, Yesterday, Previous 7 days, Previous 30 days, then a month ("September", with the year outside this one) | Newest first | The sort |
| Folder | The chat's working directory | Latest activity or creation first, or by name | The sort |
| None | One list, no header | None | The sort |

Date buckets come from `utils/chatDateBuckets.ts`, which History also uses. Name sort is
`localeCompare` with numeric ordering, case ignored, and default names ("New chat") last. Ties break
on the chat id, so a re-render never reorders equal rows.

The view is stored in `biorouter:sidebar-chat-view` as `{"v":1,"groupBy":"date","sortBy":"activity"}`.
It is clamped field by field on read, every storage access is wrapped, and a change in one window
reaches the others through the `storage` event.

### Reading every chat

The default view keeps the lazy paging of 10 rows as the list scrolls. Any other view needs the whole
list before it can arrange it, because the server pages in last-activity order only. So
`useSidebarSessions({ loadAll: true })` reads pages one after another at the server's maximum of 50,
up to 2,000 chats, and the list foot says "Loading chats…" meanwhile. **Every page request carries
`userActionHeaders()`**: without the proof the daemon silently omits private chats, which would read
as chats vanishing when the person changed the view.

### Folder groups

A folder header is a 28px row: a chevron, a folder glyph and the folder's name. The name is the last
folder of the path, `~` for the home folder, `Crew` for the Crew task folder, and `parent/name` when
two folders share a name. The full path is in the header's tooltip, which opens clear of the
buttons below.

- On hover or focus a header shows `+` and `⋯`. `+` opens a new chat window in that folder, because
  there is no API that starts an in-window chat in another folder (the Crew folder has none).
  `⋯` offers Copy folder path.
- A folder shows its first five chats, then a quiet "Show more" row on the label column.
- Collapsing is remembered in `biorouter:sidebar-collapsed-folders` (at most 200). A folder that
  holds the current chat is drawn open without touching what is stored. Clicking its header still
  closes it, for as long as that chat stays current; the next chat that lives in the folder opens
  it again.

### Rows hold still under the pointer

Under Last activity a running chat moves to the top. While the pointer is over the list, or a row is
being renamed, the list keeps the order it had (`applyHeldArrangement`), and the live order applies
when the pointer leaves. A row that was removed goes, a rename shows, and rows that arrive below
every held row come in; a row that would land above or between held rows waits.

## A row

A chat row is a 16px kind glyph (`ChatKindIcon`, which carries the lock badge of a private chat), the
title, and a 16px trailing slot that holds the running ring and gives way to `⋯` on hover or focus.
The hover card opens after 700ms with the title (only when it is cut off) and
`~/folder · 3h ago · 12 messages`. The active chat shows the 2px accent rail. A row whose menu is
open keeps its hover ground.

### The row menu

Right-click, `⋯` and Shift+F10 (or the ContextMenu key) open one menu, text only, in this order:

1. Rename (`F2`)
2. Open in new tab, Open in new window
3. Diverge, Export…, Copy chat ID
4. Delete chat… (the dialog reads "Delete chat?", `Delete "X"? This can't be undone.`)

History and the tab strip draw the same menu (`chats/ChatRowContextMenu.tsx`), with each surface
passing only the actions its chat allows. Diverge follows History's rule: not a subagent run, and at
least one message. Every action keeps the request it always made, with `userActionHeaders()`, and a
daemon refusal is shown as the daemon's own sentence.

A choice made with the pointer does not hand focus back to the row's `⋯`, because the browser would
draw a focus ring on a button that was clicked and `:focus-within` would pin the row's actions open.
A close by keyboard does hand it back (`chats/menuFocusReturn.ts`).

### Renaming in place

Rename starts from the menu, from `F2` on a focused row, or from `⌘⌥R` (`Ctrl+Alt+R`) for the active
chat. Double-click is two opens, never a rename. The row keeps its 28px box and holds a 24px input
with the title selected.

| Key | Effect |
|---|---|
| Enter, Tab, blur | Commit. An empty or unchanged name makes no request. |
| Escape | Cancel. It stops the event, because the overlay sidebar closes on any Escape nothing answered. |

A commit announces the name at once, then calls `renameSession` with the user-action proof through
`renameSessionOptimistically` (`utils/sessionNameSync.ts`), the one function every rename surface
calls. On failure it puts the old name back and shows "Couldn't rename chat" with the daemon's own
sentence. There is no success toast: the row is the confirmation. Focus returns to the row.

`⌘⌥R` also needs the Electron main process to let the key through: its `before-input-event` handler
reloads the window on any `⌘R`, so the handler has to skip the reload when Alt is held.

## Keyboard

The list is one Tab stop. Arrow Up and Arrow Down move between rows (folder headers included), Home
and End jump to the ends, and Tab leaves the list. The `⋯` and the folder buttons share their row's
tab stop, so Tab reaches them from the row.

## Tests

| File | Holds |
|---|---|
| `BioRouterSidebar/sidebarChatView.test.ts` | All nine views, folder labels and collisions, the stored view, the held order |
| `BioRouterSidebar/useSidebarSessions.test.ts` | Paging, and `loadAll` reading every page with the proof |
| `BioRouterSidebar/RecentChats.test.tsx`, `.folders.test.tsx` | The header, the buckets, the rows, Group by Folder |
| `BioRouterSidebar/RecentChats.rename.test.tsx` | Rename in place: entry points, commit, cancel, rollback, the edge cases |
| `BioRouterSidebar/RecentChats.contextMenu.test.tsx` | The menu's order and each action, delete, and where focus goes |
| `chats/ChatRowContextMenu.test.tsx`, `chats/menuFocusReturn.test.ts` | The shared menu and the focus rule |
| `BioRouterSidebar/sidebarGeometry.browser.test.ts` | In Chromium: 28px rows, the 30px pitch, x=16 and x=40 at 216, 288 and 360px, rename in place, folder headers, the rail |

## Related documentation

- [Quote selected text into a conversation](quoted-text-actions.md): the Delete chat… and Copy chat
  ID paths the row menu shares with History.
- [Session metadata contract](../agent-loop/session-metadata-contract.md): the fields the row's glyph
  and History's nesting read.
- [Implementation spec, section 3.4](../design/codex-simplicity-redesign/implementation-spec.md): the
  design this records.
