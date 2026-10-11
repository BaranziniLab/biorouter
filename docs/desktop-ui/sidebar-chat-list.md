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
`localeCompare` with numeric ordering, case ignored, and default names ("New chat") last. It sorts
the title the row shows (see "A row"), so a scheduled run files under its job id. Ties break
on the chat id, so a re-render never reorders equal rows. History breaks its ties the same way:
`GET /sessions` orders by `updated_at` alone, so chats sharing a second came back in scan order and
the two lists drew them differently (vision V1-R2-8). `sortByActivity` in
`components/sessions/sessionGrouping.ts` sorts History by last activity, then id ascending, and a
test pins it to `sortSidebarChats(…, 'activity')`.

History groups its rows through `groupSessionsByDate` in `utils/dateUtils.ts`, which asks
`chatDateBucket` once per local calendar day instead of once per chat. A day older than 30 builds an
`Intl.DateTimeFormat` for its month name (about 25 µs), and a store of 5,500 chats has about 4,700
such rows: 115 ms of the 310 ms History took to draw its first row after the list landed, measured
in Chromium over the real component (vision V1-R3-2). A test holds the result equal to
`groupByChatDate` over random instants and counts the formatters built.

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

**The view is not drawn until that first read lands** (vision V1-R3-3). Drawn from the part already
read, a name sort put the wrong 50 chats in name order and then reordered all of them 0.2 to 0.4 s
later, under a resting pointer. So after a switch from the default view the list keeps drawing the
default arrangement, with "Loading chats…" at its foot, and changes once, to the complete list. On a
reload with such a view stored there is nothing to keep, so the list draws no rows, only "Loading
chats…", and the complete list is its first draw. The view menu shows the choice at once either
way. `isLoadingAll` means the list still holds only some of the chats. Once a read has reached the
end or the cap the list stays whole, and the tail re-read a head refresh starts (the daemon has more
rows than the head page) is not reported, so it never hides a view the person already had. A failed
read does not count as reaching the end.

### Keeping the list current

The list re-reads its head (the first 10 rows, or 50 in a non-default view) when a turn finishes,
when a chat is created, diverged, deleted or imported, when the person opens a different chat
(`openSessionId`, read 250 ms later and again after 2 s), when the window comes back to the front,
and when the session-meta channel reports a row rewritten by something else. The last three were
added because opening a chat used to raise its `updated_at` on the daemon and announce nothing: the
sidebar kept a chat under September while History, fetched fresh, listed it under Today (round 1,
V1-M8). Opening no longer moves a chat (see below), so after an open these re-reads find the same
order, and the list moves no row for the open itself; they still pick up a turn, or a rename from
the terminal, made somewhere else. A burst of these is one request (the 250 ms debounce), and the
rows apply under the pointer hold, so a refresh never moves a row beneath a hovering pointer.

A rename made in this app does not wait for any of them. Once the daemon has saved the name,
`renameSession` announces it on the name channel with `activityAt`, and every window moves the chat
at once: the sidebar raises the row's `updated_at`, so it sorts to the top under Last activity, and
the shared list cache behind History and Home recents lifts the row the same way. A chat the sidebar
has not read yet is brought in by a head re-read, and a page already in flight when the rename
lands does not put the row back (`applyNamesDuringLoad`). The rename route answers with no body, so
the time is the renderer's clock until the next re-read carries the daemon's own.

### What moves a chat

Last activity is the daemon's `updated_at`. Since R-30 it moves when something is done to the
chat, and coordinator ruling 16 lists what counts: a message is stored or a turn runs (its token
accounting stamps as it goes), the conversation is compacted, edited or rewound, or the person
renames the chat, switches it to a model it does not already record, toggles its extensions or
skills from its composer, aims `workspace_set_tools` at it, or grants Crew access on it. Picking a
folder for an empty chat in the desktop is a change to that chat as well, and stamps it.

Opening a chat is not activity, and neither is any side effect of opening it. Resuming it,
restoring the model it records on a cold agent, reloading and saving its extensions, and ACP
applying the client's directory to an empty chat while it loads leave `updated_at` alone, from the
desktop, a browser, `biorouter session --resume` and ACP. Those doors run their work inside
`biorouter::session::activity::opening`, and only the setup writes read that scope (the provider
bind, the extension-data writes, the empty-chat working-directory write), so a rename or a message
inside it still moves the chat. An app's page is opened the same way: its connect binds the app's
model and re-installs its goal inside the scope. Closing or reloading the page saves the app's
shared state only when that state holds a change not yet saved (`save_ui_state` in
`routes/apps.rs`). The page's own state writes and a turn's are activity and move the chat when they
are saved; the declared initial state is never saved on its own, and a close after a saved change
does not move the chat again.

Two more things leave a chat in place. Re-binding the model a chat already records is not a change
to it, whoever asks: the provider bind stamps only when the provider or the `model_name` differs
from what the row records (`bind_provider_if_allowed`). So picking the same model again, a
`--shared-daemon` resume that names the chat's own `--provider` and `--model`, and a local CLI
resume that types the recorded model all keep its place, while a different model moves it. And
housekeeping on the machine that touches many chats at once is not activity in any of them:
uninstalling an extension prunes it from every saved roster inside
`biorouter::session::activity::housekeeping`, and no chat moves.

The tests: the store's `opening_is_not_activity_tests` (`session/session_manager.rs`, including the
same-model rebind, a switch, the housekeeping save, and the writes behind the composer toggles,
`workspace_set_tools` and a Crew grant), the door census
`only_the_opening_and_housekeeping_doors_enter_a_scope` (`session/activity.rs`, which fails if any
other door enters a scope), the cold restore in `agents/agent.rs`,
`opening_is_not_activity_route_tests` (`routes/agent.rs`: the desktop's own calls on a public and a
private chat, then a rename; a same-model `update_provider`, a model switch, and the composer's
extension toggle on and off), `opening_is_not_activity_app_tests` (`routes/apps.rs`, the connect,
and pages loaded and closed over the real socket), `pruning_an_uninstalled_extension_moves_no_chat`
(`agents/extension_manager_extension.rs`), the resume cases in
`biorouter-cli/tests/non_interactive.rs`, `test_acp_load_session_is_not_activity` in
`biorouter-acp/tests/server_test.rs`, and in the renderer
`useSidebarSessions.renameActivity.test.tsx` and `SessionListView.renameActivity.test.tsx` (a rename
moves the chat at once; opening a chat moves no row).

### Drawing the list

The list draws as many lines as it shows plus 12 (`sidebarRenderWindow`, at most 60, and 60 before
the list has been measured; a header, a row and a "Show more" are one line each) and 120 more each
time it is scrolled near its end; it asks the daemon for another page only when everything it holds
is drawn. The whole list is still held and arranged, so the order is right; only drawing waits.
Drawing every row of a 2,000-chat view took 2 to 4 s per change of Group by or Sort by in the
development build (R-28), because a row is a context menu, a tooltip and a rename editor. The
arithmetic is `windowSidebarRows` in `sidebarChatView.ts`, free of React.

A view switch costs what it mounts, so it mounts little (vision V1-R2-2: 300 to 370 ms in the
development app). The switch is decided while the new view renders, so each drawn row mounts once.
When it was set from an effect, the new order mounted under the old key first and then again under
the new one. A row builds its hover card and its ⋯ label the first time each opens, and a folder
header its tooltips; Radix mounts nothing for a closed one, but their elements were created on every
render of every row.

No row mounts a menu root (vision V1-R2-2, round 3). A mounted Radix menu root adds a capture
`keydown` listener to the document, and every key press makes each of those add two more, so a
context menu and a ⋯ menu in every drawn row made each key press and each view switch pay for every
row: with 4,233 rows drawn after a scroll to the end, a production build took 0.59 s to switch views
and 0.28 s to open the view menu. The list has one right-click menu, whose trigger is the list of
groups; it opens on the chat row the event came from (`data-row`), a right-click anywhere else in
the list opens nothing, and the rename editor keeps the app's own text menu. A ⋯ is a plain button
that says it opens a menu (`LazyMenuButton`) until it is focused, pressed or opened from the
keyboard; then its menu root takes it over, already open when the press asked for that, with focus
kept on it, and lets go when it loses focus with the menu shut. A folder header's ⋯ is the same
button. In a production build a switch takes 8 to 30 ms, and 85 ms with 4,233 rows drawn.

Only the lines near the viewport are rows (vision V1-R2-2, round 4). The list still reaches as far
down as it has been scrolled, but every line outside the lines in view plus 12 on each side is
folded into a placeholder exactly its height (`.br-chat-row-gap`: N lines of a 28px row and the
2px the list puts between rows), and a folder header outside them is an empty box of its height; a
date header is drawn anyway (one element). The lines in view are read from the drawn groups' boxes
(`sidebarLinesInView`: a group's rows and gaps sit on one pitch below its first row, the pitch read
off a drawn row by `measuredLinePitch`, so no header, row or gap height is assumed), on every
scroll, committed before the frame paints, and again after anything that moves lines without a
scroll (a view drawn, a folder opened, rows arriving, the box growing). The pitch is read, not
assumed, because the Text size preference scales the row and its 2px gap (34.5px a line under
Larger text): read at a fixed 30px, the middle of a long list showed only placeholder. For the same
reason the placeholder scales its own 2px. Some lines are kept drawn whatever the range: the focused
line (a row, a folder header or a Show more), the current chat, so the single Tab stop stays on it, a
row being renamed or just renamed, and a line whose right-click or ⋯ menu is open (a ⋯ opened with
the pointer never takes focus, and the list scrolls under it). So neither focus, an open menu nor an
editor is ever unmounted under the person. Home and End scroll the list to its end before they move
focus, so that end's row is drawn; ArrowUp and ArrowDown on a line the list was scrolled away from
bring the list back to it first, so they reach the line next to it, not the first row drawn far down
the list. Before this the drawn rows only grew: with 5,542 drawn after a scroll to the
end, a right-click took about 2.2 s before its menu showed (mostly restyling every row) and a
switch 0.5 s. The arithmetic is `sliceSidebarLines`, `sidebarLinesInView` and
`nextSidebarDrawRange` in `sidebarChatView.ts`. Measured in a production build with 2,100 lines in
reach, scrolled to the middle: 45 rows drawn instead of 2,100, a right-click 16 to 26 ms (was 171
to 214), the view menu 3 to 16 ms (was 67 to 78), a switch 21 to 35 ms (was 41 to 50).

The list's right-click menu is not modal (round 4). A modal Radix menu sets `pointer-events: none`
on `<body>`, an inherited property, so opening it and closing it each restyled every element in the
window (about 50 ms each way with 2,100 rows drawn, more with a long chat open), and it locked the
page's scroll, which reads the body's computed style. A row's ⋯ menu, the same menu, was never
modal; now both behave alike: a click outside the list closes the menu and lands where it was aimed.

The list is modal itself while either menu is open (vision V1-R5-2, round 5). Left free, it
answered under the menu: the wheel scrolled it 599px and left the menu beside other chats, whose
Delete still named the chat that had gone off screen; another row lit and drew its hover card over
the menu, so the first Escape closed the card; and the press that closed the menu opened the chat
it landed on. Now, while `listMenuOpen` (the right-click menu, or a row's or folder header's `⋯`):

- the wheel and a touch drag scroll nothing (a non-passive listener on the list, not
  `overflow: hidden`, which takes an always-shown scroll bar away and widens every row);
- `data-row-menu-open` on `.br-sidebar-chats-scroll` takes the pointer from the list's rows
  (`sidebar.css`), so no other row lights or opens its card, and the press that closes the menu
  lands on the list box, so the click it makes opens nothing, as in History, whose right-click
  menu is modal;
- the row's own hover card stays shut, and its row stays lit by `data-menu-open`;
- the rows hold their order wherever the pointer is (`holding`).

That restyles only the list's own subtree, about 40 drawn rows: a right-click measured 5 to 7 ms
of commit, style and layout with 2,000 chats held (4 to 6 ms before), in a production build. A
programmatic scroll still moves the list, and the menu's row is still kept drawn for it.

Rows are memoized with stable handlers, so the pointer
entering the list or a keystroke in a rename re-renders no other row. Each view's arrangement is
kept for as long as the list of chats is the same array (`arrangeSidebarChatsCached`, keyed by the
view, the home folder and the calendar day), so switching back is a lookup.

A view change starts again from the first lines. A chat past the drawn lines (the current chat,
far down a name-sorted list) has no row until the list is scrolled to it, so the single Tab stop
falls back to the first row drawn, as it does when the current chat is on a page not yet read. `⌘⌥R` is
the one entry point that cannot wait for a scroll: it draws the list up to the current chat's row
first (`linesToDrawRow`), because the editor mounts in that row.

A list with room to spare and nothing to scroll gets no scroll event, so the list also checks after
each page, and whenever its own box changes height (a `ResizeObserver` on the scroll box), whether
its rows fill it, and draws or reads more until they do. The box check is vision V1-R2-1: ten rows
filled the short list of the overlay sidebar, and once the window widened into the docked sidebar
(or Components collapsed) the same ten rows sat in a box twice their height, with no scrollbar,
and every other chat was out of reach of the sidebar.

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
when the pointer leaves. A row that was removed goes, a new name shows, and rows that arrive below
every held row come in; a row that would land above or between held rows waits.

Two moves are not held:

- **A full read landing.** A non-default view is not drawn until its first full read lands (see
  "Reading every chat"), so the list changes once, from the view it drew before (or from nothing)
  to the complete arrangement, which is what the pointer holds from then on. A hold is of the view
  drawn: when the read lands the drawn view changes, the old hold stops applying, and the list is
  held again from the complete list (vision V1-R2-3, V1-R3-3). Nothing is held while the list is
  empty either, so a first load is held from the rows it draws.
- **The person's own rename** (coordinator ruling 16; vision V1-R2-4). The hold is there so a row
  never moves under a click the person is about to make. A rename they just committed is the move
  they asked for, and the usual way to rename (row menu, type, Enter) leaves the pointer on the
  list. So a commit lets go of the hold when the new name is announced (a name sort moves the row
  then) and again when the daemon has saved the rename (its `activityAt` raises the row under Last
  activity) or refused it, and the list is held again from the new order. The row is drawn,
  scrolled into view with its date bucket (`scrollRowIntoView`, measured against the list, never
  `scrollIntoView`), and focused again if the move took focus with it: a row that changes bucket
  remounts. A move that is not the person's own, such as a turn in another chat, still waits.

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

The title says the chat's kind only once, in the glyph (vision V1-R3-6). It goes through History's
resolver, `historyRowTitle` in `components/sessions/historyRow.ts`, by way of `sidebarRowTitle`: a
scheduled run, stored as `Scheduled job: <job id>`, reads as the job id beside the clock, and an
app's chat, stored as `app:<app id>`, reads as the app's name. It is keyed on the kind, so a chat a
person titled "Scheduled job: x" keeps its title. The row's accessible name, its `⋯` label, the
rename field's label and the delete sentence use the shown title. It is display only: the rename
field starts from the stored name, an export is titled with it, a click hands it to the new tab, and
the Meditation job still finds scheduled runs by the stored prefix.

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
sentence. There is no success toast: the row is the confirmation. Focus returns to the row, and the
row moves to where the new name or the rename's activity puts it at once, under the pointer too
(see "Rows hold still under the pointer").

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
  surface). Closing, the overlay keeps its shadow until it has slid out, so its edge never goes
  flat while it still covers the page (vision V1-R2-9).
- View options stays pressed on `--background-fill-strong` while its menu is open.
- Crew's unread badge is the neutral badge, whose fill is the strong step. A lit row (hovered,
  current or focused) is that same fill, so there the badge steps back to the page ground; a fill
  never sits on a fill.
- The rename editor rests on the page ground (its row is already the strong fill) and is edged by a
  real 1px `--border-focus` border, not a box-shadow ring, so forced colours keep its outline.
- The titlebar controls hover on `--background-fill-strong`, the band's step.
- A nav row is one ink (vision V1-R2-5). Its icon or chevron wears `--sidebar-icon`, which every
  family points at its muted label ink, at rest, and the label's own ink when the row is hovered,
  focused or current; the folder and Chats headers do the same. The Update row's download glyph
  keeps the accent.
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
| `BioRouterSidebar/sidebarChatView.test.ts` | All nine views, folder labels and collisions, the stored view, the held order, `windowSidebarRows`; `sidebarRowTitle` and the name sort by the shown title; the drawn range: `sliceSidebarLines` (gaps of exact line counts, kept rows, a kept folder header or Show more, headers outside the range), `sidebarLinesInView` (lines from group boxes by binary search) and `nextSidebarDrawRange` (a range that holds still while it covers the view) |
| `BioRouterSidebar/useSidebarSessions.test.ts` | Paging, and `loadAll` reading the rest in one request with the proof (or page by page from an older daemon), into a buffer shown once, stopping at the cap; `isLoadingAll` reported for the first full read only (not a later tail re-read, but again after a failed read); the head re-read on a new open chat, on focus and on a meta change |
| `BioRouterSidebar/RecentChats.test.tsx`, `.folders.test.tsx` | The header, the buckets, the rows, Group by Folder; a scheduled run's row showing its job id (its names and delete sentence too) with the rename starting from the stored name; a stored view drawn only once its read lands, a switch keeping the default arrangement until then, the complete list held from then on, and a first load held from its own rows |
| `BioRouterSidebar/RecentChats.window.test.tsx` | The first 60 lines of the whole arranged list, growth on scroll before another page is asked for, a restart on a view change, a list whose box grew asking for the next page, and `⌘⌥R` drawing the current chat's row |
| `BioRouterSidebar/RecentChats.switchCost.test.tsx` | A view switch mounts each drawn row once, also after a scroll; a row builds a menu only when it opens; a list re-render (pointer, rename keystroke) re-renders no other row; a measured list draws its height plus the overscan |
| `BioRouterSidebar/RecentChats.drawRange.test.tsx` | The right-click menu leaving `<body>` alone (not modal), a list that cannot be measured drawing every line it reaches with no placeholder, and `scrollRowIntoView` showing a header only for its group's first line |
| `BioRouterSidebar/RecentChats.menuRoots.test.tsx` | No menu root per drawn row: rendering and a key press add a handful of document listeners, not one per row; a `⋯` armed by focus keeps focus and opens from Enter; the one right-click menu opens on the row it came from and on nothing else |
| `BioRouterSidebar/RecentChats.rename.test.tsx` | Rename in place: entry points, commit, cancel, rollback, the edge cases; a rename moving its chat at once under the pointer with focus kept, while another chat's move waits; `scrollRowIntoView` |
| `BioRouterSidebar/RecentChats.contextMenu.test.tsx` | The menu's order and each action, delete, and where focus goes; the list under an open right-click or `⋯` menu marking itself, keeping the wheel from scrolling it and holding its rows' order with the pointer gone |
| `chats/ChatRowContextMenu.test.tsx`, `chats/menuFocusReturn.test.ts` | The shared menu and the focus rule |
| `BioRouterSidebar/sidebarGeometry.browser.test.ts` | In Chromium: 28px rows, the 30px pitch, x=16 and x=40 at 216, 288 and 360px, rename in place, folder headers, the rail |
| `BioRouterSidebar/sidebarFill.browser.test.tsx` | In Chromium, the real sidebar under the whole stylesheet, light and dark: only the structural lines, rows clear at rest, the `⋯` and View options with their menus open, the rename editor's border (kept in forced colours), the badge on a lit row, the hover card clear of the divider, the titlebar hover, the column as a recessed ground (a probe rests on the strong step, the notice's button on the page ground), and a pointer-opened menu with no ring until a key; a nav row's icon in its label's ink at rest, hovered and current; the list itself under a real layout: the next page read when the list's box grows, a rename moving its chat to the top with the pointer resting on the list, only the rows near the viewport drawn in a list scrolled to its middle (each visible row on its own line, the list exactly as tall as its lines, no placeholder in view, the right-click menu leaving `<body>` alone, Home reaching the first chat), the same under Larger text (the stylesheet served through the font-scale plugin), and with the list scrolled away under them: ArrowUp and ArrowDown reaching the next line, Tab landing on the current chat, a pointer-opened ⋯ menu staying open, a focused folder header or Show more keeping focus; with a right-click or `⋯` menu open, the wheel scrolling nothing, no other row lit and no hover card, the first Escape closing the menu, a press on another row only closing it, and the list live again after; and every order painted (frame by frame) on a reload with None / Name stored and on Sort by Name through the menu being the complete name order, or the default before the read lands. `SIDEBAR_SHOTS_DIR` saves screenshots |
| `ui/sidebarMotion.browser.test.ts` | In Chromium: the panel slides with the gap (175ms out, 300ms in), snaps during a window resize, and wears the overlay shadow only while open; closing, it keeps the shadow until the slide ends, and drops it with the panel when a resize or reduced motion snaps it |

## Related documentation

- [Quote selected text into a conversation](quoted-text-actions.md): the Delete chat… and Copy chat
  ID paths the row menu shares with History.
- [Session metadata contract](../agent-loop/session-metadata-contract.md): the fields the row's glyph
  and History's nesting read.
- [Implementation spec, section 3.4](../design/codex-simplicity-redesign/implementation-spec.md): the
  design this records.
