# The sidebar chat list

> **What this is.** How the app sidebar lists chats: the Chats header and its View options menu,
> Group by and Sort by (nine combinations), how every chat is read when a view needs them all and
> how many rows are drawn at once, folder groups, the row menu, renaming a chat in place, its fills,
> the keyboard model, and the tests that hold each part.
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
`useSidebarSessions({ loadAll: true })` reads the first page and then the rest in one request of up
to 2,000 chats, which is also the daemon's page maximum (`MAX_SIDEBAR_SESSION_LIMIT` in
`routes/session.rs`), and the list foot says "Loading chats…" meanwhile. A daemon from before that
change answers at most 50 a page, and the same loop then reads it page by page. **Every page request
carries `userActionHeaders()`**: without the proof the daemon silently omits private chats, which
would read as chats vanishing when the person changed the view.

The pages go into a buffer and the list changes **once**, when the last one has landed. Shown page by
page, a list sorted by name reshuffled its top rows with every arrival, and each arrival re-rendered
every row already held. A rename, a removal or a privacy change that lands during the read is applied
to the buffer too, and a refresh asked for during the read runs after it. A page asks for no more
rows than the cap leaves room for, so the read stops at 2,000 exactly.

### Keeping the list current

The list re-reads its head (the first 10 rows, or 50 in a non-default view) when a turn finishes,
when a chat is created, diverged, deleted or imported, when the person opens a different chat
(`openSessionId`, read 250 ms later and again after 2 s), when the window comes back to the front,
and when the session-meta channel reports a row rewritten by something else. The last three exist
because opening or resuming a chat raises its `updated_at` on the daemon and announces nothing: the
sidebar kept a chat under September while History, fetched fresh, listed it under Today (round 1,
V1-M8). A burst of these is one request (the 250 ms debounce), and the rows apply under the pointer
hold, so a refresh never moves a row beneath a hovering pointer. Whether opening a chat should raise
`updated_at` at all is the daemon's call and is not changed here.

### Drawing the list

The list draws the first 60 lines of the arranged list (a header, a row and a "Show more" are one
line each) and 120 more each time it is scrolled near its end; it asks the daemon for another page
only when everything it holds is drawn. The whole list is still held and arranged, so the order is
right; only drawing waits. Drawing every row of a 2,000-chat view took 2 to 4 s per change of Group
by or Sort by in the development build (R-28), because a row is a context menu, a tooltip and a
rename editor. The arithmetic is `windowSidebarRows` in `sidebarChatView.ts`, free of React.

A view change starts again from the first 60 lines. A chat past the drawn lines (the current chat,
far down a name-sorted list) has no row until the list is scrolled to it, so the single Tab stop
falls back to the first row, as it does when the current chat is on a page not yet read. `⌘⌥R` is
the one entry point that cannot wait for a scroll: it draws the list up to the current chat's row
first (`linesToDrawRow`), because the editor mounts in that row.

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
when the pointer leaves. The same holds when a full read lands under the pointer. A row that was removed goes, a rename shows, and rows that arrive below
every held row come in; a row that would land above or between held rows waits.

## A row

A chat row is a 16px kind glyph (`ChatKindIcon`, which carries the lock badge of a private chat), the
title, and a 16px trailing slot that holds the running ring and gives way to `⋯` on hover or focus.
The hover card opens after 700ms with the title (only when it is cut off) and
`~/folder · 3h ago · 12 messages`, sized to its text up to 320px. It opens 16px past the row's end,
which puts it 8px clear of the pane divider rather than on it (a folder's path tooltip lands on the
same x). It closes when its row can no
longer be seen: Chats collapsed, the sidebar closed, or the row scrolled out of the list. The active
chat shows the 2px accent rail. A row whose menu is open keeps its hover ground, and its `⋯` stays
on screen until the menu closes.

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

## Fills, not lines

The sidebar follows the app's fill rule (owner message 13, R-27; design.md 9.11). It is a recessed
ground, so every fill on it is the strong step. `ui/sidebar.tsx` puts the shared `.br-ground-recessed`
on the column (`data-slot="sidebar-inner"`), which sets the control, hover and track grounds for
everything in it ([shared-controls.md](shared-controls.md), "Fills before lines"). Nothing rests on
it today, so it draws no new pixel: rows are clear, and the daemon notice is a `Note`, which sets its
own grounds and so keeps its "Try again" button on the page ground. The class is there so the next
filled control placed in the column is right without a rule of its own.

- Rows rest clear and light with the sidebar's own steps, `--sidebar-hover` and `--sidebar-active`.
  No hairline separates rows; spacing and the hover fill do.
- The only lines are structural: the band's edge at y=44, the rule over the footer (the list
  scrolls under it), the pane divider, and the compact overlay's edge and shadow (a floating
  surface).
- View options stays pressed on `--background-fill-strong` while its menu is open.
- Crew's unread badge is the neutral badge, whose fill is the strong step. A lit row (hovered,
  current or focused) is that same fill, so there the badge steps back to the page ground; a fill
  never sits on a fill.
- The rename editor rests on the page ground (its row is already the strong fill) and is edged by a
  real 1px `--border-focus` border, not a box-shadow ring, so forced colours keep its outline.
- The titlebar controls hover on `--background-fill-strong`, the band's step.
- A menu the pointer opened rings no row (the row's `⋯`, View options and its submenus): the shared
  menu surface says how it is driven (`data-menu-input`, from `ui/inputTrail.ts`), and the D-15 ring
  comes off a row the pointer only crossed. The first key pressed in the menu brings it back on the
  focused row. The rule is the primitives' (`main.css`, after `.br-menu-row`); the sidebar only
  proves it on its own menus.

⚠ A menu trigger in the sidebar sits inside a `TooltipTrigger`, and the tooltip's `data-state`
("closed") overwrites the menu's on the same button. A rule written as `[data-state='open']` never
matches there, which is how the row's `⋯` vanished under its own open menu. The sidebar's rules
read `aria-expanded` instead.

## Keyboard

The list is one Tab stop. Arrow Up and Arrow Down move between rows (folder headers included), Home
and End jump to the ends, and Tab leaves the list. The `⋯` and the folder buttons share their row's
tab stop, so Tab reaches them from the row.

## Tests

| File | Holds |
|---|---|
| `BioRouterSidebar/sidebarChatView.test.ts` | All nine views, folder labels and collisions, the stored view, the held order, `windowSidebarRows` |
| `BioRouterSidebar/useSidebarSessions.test.ts` | Paging, and `loadAll` reading the rest in one request with the proof (or page by page from an older daemon), into a buffer shown once, stopping at the cap; the head re-read on a new open chat, on focus and on a meta change |
| `BioRouterSidebar/RecentChats.test.tsx`, `.folders.test.tsx` | The header, the buckets, the rows, Group by Folder |
| `BioRouterSidebar/RecentChats.window.test.tsx` | The first 60 lines of the whole arranged list, growth on scroll before another page is asked for, a restart on a view change, and `⌘⌥R` drawing the current chat's row |
| `BioRouterSidebar/RecentChats.rename.test.tsx` | Rename in place: entry points, commit, cancel, rollback, the edge cases |
| `BioRouterSidebar/RecentChats.contextMenu.test.tsx` | The menu's order and each action, delete, and where focus goes |
| `chats/ChatRowContextMenu.test.tsx`, `chats/menuFocusReturn.test.ts` | The shared menu and the focus rule |
| `BioRouterSidebar/sidebarGeometry.browser.test.ts` | In Chromium: 28px rows, the 30px pitch, x=16 and x=40 at 216, 288 and 360px, rename in place, folder headers, the rail |
| `BioRouterSidebar/sidebarFill.browser.test.tsx` | In Chromium, the real sidebar under the whole stylesheet, light and dark: only the structural lines, rows clear at rest, the `⋯` and View options with their menus open, the rename editor's border (kept in forced colours), the badge on a lit row, the hover card clear of the divider, the titlebar hover, the column as a recessed ground (a probe rests on the strong step, the notice's button on the page ground), and a pointer-opened menu with no ring until a key. `SIDEBAR_SHOTS_DIR` saves screenshots |
| `ui/sidebarMotion.browser.test.ts` | In Chromium: the panel slides with the gap (175ms out, 300ms in), snaps during a window resize, and wears the overlay shadow only while open |

## Related documentation

- [Quote selected text into a conversation](quoted-text-actions.md): the Delete chat… and Copy chat
  ID paths the row menu shares with History.
- [Session metadata contract](../agent-loop/session-metadata-contract.md): the fields the row's glyph
  and History's nesting read.
- [Implementation spec, section 3.4](../design/codex-simplicity-redesign/implementation-spec.md): the
  design this records.
