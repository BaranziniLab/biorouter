# When the app "stops scaling with the window"

> **What this is.** The recurring regression where resizing the Biorouter window
> stops changing the layout, the one product cause behind it, and the four ways
> the same symptom appears when nothing is wrong with the app at all.
> **Status:** Current.
> **Audience:** anyone who has just been told "the app doesn't rescale", and
> agents driving the dev GUI.

This has been reported more than once, and each time the first hour went into
reproducing it rather than fixing it — because four unrelated things produce the
identical symptom, and most of them are not bugs in the app. Read the
triage below before changing any CSS.

## Triage: two minutes, in this order

**Read the renderer's console first.** In a development build the app diagnoses
the commonest of these itself: a line beginning `Viewport pinned at …` means the
tooling pinned the viewport and no reading below can be trusted — skip straight
to *Viewport emulation*, and do not open the CSS. Then run these against the dev
GUI over CDP (see
[Debugging the dev GUI with agent-browser](agent-browser-debugging.md)).

```js
// 1. Is the app even rendering?
JSON.stringify({ text: (document.body.innerText || '').slice(0, 60), hash: location.hash })

// 2. Does the renderer's viewport match the real window?
JSON.stringify({ inner: [innerWidth, innerHeight], outer: [outerWidth, outerHeight] })

// 3. Is the measure fluid?
getComputedStyle(document.documentElement).getPropertyValue('--measure-chat')
```

| Reading | Diagnosis |
|---------|-----------|
| `text` is `Loading BioRouter…` and the console is EMPTY | **Not a layout bug.** The renderer's assets 404'd — see *`--base ./`* below. |
| `text` empty, or `hash` is `#/pair` | **Not a layout bug.** The app is not rendering — see *A dead daemon* below. |
| `inner` ≠ `outer`, or the console carries `Viewport pinned at …` | **Not a layout bug.** Your tooling pinned the viewport — confirm with `npm run cdp:viewport-check -- <port>` and see *Viewport emulation* below. |
| `inner` = `outer`, and neither changes when you resize | **Not a layout bug.** Your resize command silently did nothing — see *AppleScript* below. |
| Everything tracks, but the view is **Settings**, **Home**, **Chat history** or a saved/shared transcript, and its column stops at 760px | **Not a layout bug.** All of them read the chat measure by decision — see *Settings and Chat history, on the chat measure* below. |
| Everything tracks, but content stays the same width | **The real one.** A fixed pixel cap — see below. |

## The real product cause: a fixed pixel cap

The layout is built around reading measures. When those are flat pixel values,
a wider window buys **margin**, not content: at 1800px the chat column sat at
760px with roughly 400px of dead band on each side, which is exactly what
"doesn't scale with the window" looks like to someone dragging the edge.

The fix was to make the **page** measure a `clamp()` whose middle term is a
**percentage**:

```css
--measure-page: clamp(1120px, 88%, 1720px);
```

⚠ **The chat measure is NOT one of them, and this block listed it as
`clamp(760px, 78%, 1180px)` until 2026-09-07.** It was briefly widened that way
on the reasoning above, and reverted: a 1180px composer is not a more capable
composer, it is a line of prose the eye has to track back across. It is a flat
`--measure-chat: 760px`, and `styles/measures.test.ts` asserts that literal
string precisely so a re-widening cannot land quietly. Read main.css before
quoting either value.

Two invariants, both learned the hard way:

- **Percentage, never `vw`.** A percentage resolves against the containing
  block — the content pane. `vw` is the whole viewport, so it over-counts by the
  sidebar's width and widens the column at the exact moment the sidebar opens and
  takes the room away.
- **The floor is the value it replaced.** `max-width` can never force a box wider
  than its parent, so below the floor the column is simply pane-wide and nothing
  narrow changes. The clamp only ever raises the ceiling.

### Why no rendered test catches this

jsdom has no layout engine and never runs Tailwind, so **no component test in
this repo can measure a column's width.** A regression to
`--measure-chat: 760px` renders identically in all 264 frontend files and ships
green.

The guard is therefore a source assertion, `src/styles/measures.test.ts`. It
asserts the declaration is a `clamp`, that the middle term is a percentage
rather than three static pixel values, that it is not `vw`-keyed, and that the
floors still equal the old shipped numbers so a narrow window can never end up
*narrower* than before. It caught a stale doc comment on its first run.

**If you add a new measure, add it to that test.** A `max-w-[1400px]` introduced
anywhere in a layout container reintroduces this bug with nothing to stop it.

### Not this: the window's own minimum size

`main.ts` sets `minWidth: 1048` on the main window, and it is easy to read that
as the same mistake. It is the opposite kind of number: a **floor under a narrow
window**, which does nothing at all to a wide one. The measures above still
govern how the content uses the room.

It is derived rather than chosen — the **288px sidebar default**
(`SIDEBAR_DEFAULT_WIDTH` in `components/ui/sidebarWidth.ts`) plus the 760px
reading column (`--measure-chat`) — because Home's usage heatmap is the one
element whose size is *computed* rather than declared: it fits its cells to the
box it is given, so a window narrow enough to squeeze the reading column squeezes
the grid with it. `styles/measures.test.ts` asserts the arithmetic, so changing
either the sidebar's bounds or the chat measure fails there instead of quietly
letting the window compress the heatmap again.

⚠ **The sidebar's DEFAULT, not its minimum** — the sidebar is user-resizable
(216–360px, default 288), and this line read *"240px sidebar (`SIDEBAR_WIDTH`,
15rem)"* while that was still a single number. The floor was then briefly taken
from the *bottom* of the range, on the argument that the minimum is the only
width in it that is a property of the app rather than of a preference. That
argument runs backwards: `216 + 760 = 976` is a promise about a width no install
has until someone drags the edge, and at the width every install ships with it
leaves the column `976 − 288 = 688px`, under the very measure the floor exists to
protect. Past the default the user is trading reading room away deliberately,
with the window open and the edge under their hand; a floor cannot promise
anything about a preference it is never told.

The wide end is bounded by construction rather than by this number:
`SIDEBAR_MAX_WIDTH + 760 = 1120 = SIDEBAR_COMPACT_WIDTH`, rung 1 of the yield
ladder, below which the sidebar auto-collapses to an overlay and costs the chat
nothing. `measures.test.ts` pins that identity too, so raising the max without
moving the ladder fails loudly.

Measured against the real stylesheet **with the then-240px sidebar**, the cliff
was at 989px of content width (23px cells at 989, 22px at 988, stepping down to
16px by 800px). ⚠ That 989 is a *window* width and therefore carries the sidebar
of the day inside it: what the heatmap reacts to is its own column, so the cliff
moves with the sidebar. The measurement is really a **749px column**
(989 − 240), and against the 288px default the same cliff sits at a **1037px**
window — which 1048 clears by the same 11px the old 1000 cleared 989 by.

The height axis is deliberately **not** capped to the same standard. A short
window shrinks the heatmap's cells too, but the `minHeight` that would prevent it
lands near 700–800px, which is unusable on a 1280×800 display once the menu bar
and Dock are removed — a worse bug than the one being fixed. The heatmap keeps
its chrome locked to its own grid instead (`UsageHeatmap`'s `heatStyle` sets the
block's width from the fitted footprint), so a compressed grid stays a coherent
block rather than leaving its labels and legend pinned to the old edge.

### Not this: Settings and Chat history, on the chat measure by decision (2026-09-07)

**Settings and the chat-history surfaces stop widening at 760px, and that is the
intended behaviour** — not the fixed-cap bug above, and not a measure someone
forgot to clamp. Every one of their reading columns is
`<ReadableContent size="chat">`:

| View | Columns | File |
|---|---|---|
| Settings | header, tab strip, scrolling body | `components/settings/SettingsView.tsx` |
| Chat history | header, scrolling body | `components/sessions/SessionListView.tsx` |
| Saved transcript | one | `components/sessions/SessionHistoryView.tsx` |
| Shared transcript | one | `components/sessions/SharedSessionView.tsx` |

so Settings, Home, Chat history, a saved conversation, the live chat and the
composer share one left edge and one width at every window size. Measured in the
running app at 1440 with the default sidebar, all three of Home's greeting,
Settings' title and Chat history's title start at **x = 508.00**.

The distinction that decides which of the two you are looking at is **what the
extra width would have bought**, not whether the column moved:

- A **document-shaped** view gains real content from a wider window: more table
  columns, more cards per row. Those stay on `--measure-page` and a flat cap
  there is the regression this page is about. ⚠ **This bullet used to name
  extensions, skills, workflows and applications as the examples, and they were
  the wrong ones** — see the component-views paragraph below. There is no view
  left in the app that is document-shaped by this test; the category and the
  measure both stay because the next one might be.
- **Settings is a column of labelled rows**: a label on the left, the control it
  names on the right, one per row. Widening the column adds nothing to either
  half — it only pushes them apart, so at 1800px the Local Model Inventory's
  Install button sat about a foot from the model it installs. That is the same
  "margin, not content" failure as the fixed cap, arriving from the opposite
  direction.
- **Chat history is the same shape**: a chat's name on the left, its
  message/token/extension counts on the right. At the page measure and 1440
  those counts sat roughly 700px from the chat they count. It also has an
  argument Settings does not — clicking a row opens the live chat, and a saved
  transcript *is* a chat, so both must line up with the column a conversation is
  read in.

So a report of the form *"Chat history doesn't use my big monitor"* is expected
and closes as working-as-intended; a report of the form *"Chat history truncates
/ hides something at 760px"* is a real bug, and the fix belongs in the section
that truncates — `min-w-0`, `truncate`, `flex-wrap`, a tighter `min-w-*` on a
count box — never in the measure. (One such fix shipped with the Settings move:
the model inventory's metadata line was `truncate`, which at the 508px label
block ate the context window and the model id; it wraps now. The history row
needed none: measured at 704px of row — 760 less the `px-6` inset and the scroll
area's own padding — with a 120-character title, a 7-digit token count, a
5-digit message count and a deep path, every row stayed 55px tall, nothing
overflowed, and the stats cluster pushed from 320px to 330px while the title box
gave way from 340px to 330px, which is what the `min-w-*` floors on those counts
are for.)

**The Scheduler joined it on 2026-09-07**, which is why `schedules` left the
document-shaped list above. Both of its surfaces are columns of rows on the same
argument: the list pairs a schedule with its status and its actions, and the
detail pairs a label with the fact it names, so the extra width a wide window
hands either one lands between the two halves of every row. Both
`components/schedule/SchedulesView.tsx` and
`components/schedule/ScheduleDetailView.tsx` read `size="chat"`, and
`styles/measures.test.ts` asserts it at the source for the same jsdom reason as
the paragraph below.

**The component views joined on 2026-09-07 too**, and they are the reason the
document-shaped bullet above lost its examples: Workflows, Extensions, Skills
and Built apps were the views that bullet named, and the prediction it made
about them did not survive being looked at. None of them
grows a column or a card per 100px of window. Each is a list of rows — a
workflow's name and its seven hover actions, an extension's name and its
switch, a skill's name and its three buttons — which is Settings' shape exactly,
so the extra width landed between the two halves of every row there as well.
The operator asked for it directly: *"for those different components like
workflows, scheduler, extensions, skills, and applications or build apps, please
make sure that you're also applying the 760 pixels redesign"*.

A sixth view, MCP apps, joined at the same time and had no reading column at all
— only a `px-8` div — so at 1440 its title started at **x = 320** while its
siblings started at **x = 336** and Settings at **x = 508**: three different left
edges across one family of pages. That view was an inherited feature and was
removed in September 2026 (see the record under `docs/history/`), but the reason
it is recorded here outlives it — every remaining page is 508 now, which is also
what made a single `PageHeader` primitive possible (rule 10 of the settings
visual vocabulary).

⚠ **jsdom cannot see any of this**, exactly as with the fixed cap: there is no
layout engine and Tailwind never runs, so the component tests assert the
`data-size` attribute and the column count, and `styles/measures.test.ts` asserts
at the source that no `<ReadableContent` in any of the eleven listed files is
left on the default size — and that none of them declares a second `max-w-*`, which would
silently take precedence over the column. The widths themselves were measured in
the running app.

## The four impostors

### Viewport emulation pins `innerWidth`

`agent-browser`/CDP can apply `Emulation.setDeviceMetricsOverride`, which fixes
the renderer's viewport regardless of the real window. Resizing the OS window
then changes nothing on screen and every measurement lies.

**This repo has now hit it at least twice**, and the second time it arrived as a
bug report about the merged `main` — "weird empty space at the bottom", "the app
doesn't scale with the window", with a screenshot of Provider Configuration
ending at a fixed area inside a larger window. The three subsections below are
what that recurrence bought: a warning the app prints itself, a script that
answers the question in one command, and the measurement that settles which cure
actually works.

**Tell:** `outerWidth` moves and `innerWidth` does not. Observed as
`outer: [800, 600]` with `inner: [1400, 900]` — a round 1400×900 that nobody set
is itself the giveaway.

#### First line of defence: the app says so itself

In a **development** build the renderer checks its own viewport against its
window on load and after every (debounced) resize, and `console.warn`s once per
distinct pinned size:

> Viewport pinned at 1440×900 while the window is 1638×963: a DevTools
> device-metrics override (agent-browser set_viewport, Playwright
> setViewportSize, or DevTools device mode) is holding the renderer, so the
> layout cannot follow the window and every measurement lies — this is NOT a
> layout bug. …

The decision lives in `ui/desktop/src/utils/viewportPin.ts`
(`describeViewportPin`), a module with no React and no DOM, unit-tested in
`viewportPin.test.ts`; `renderer.tsx` installs it behind `import.meta.env.DEV`,
so it is dropped from a packaged bundle entirely. It is a console line and
nothing else — never a toast, never a throw. A user who has never opened
DevTools cannot cause this and is never shown it.

The tolerances are the tell above, made precise: macOS windows have **no side
frame**, so any width difference at all is emulation; the height may differ by
the title bar (40px allowed, ~28px real); Windows and Linux get 16px more on
both axes for their frame. The comparison is signed, so a viewport *larger* than
its window is a pin on every platform.

#### Diagnosis: one command

```bash
cd ui/desktop
npm run cdp:viewport-check -- 9333          # exit 0 = clean, 1 = PINNED, 2 = harness
npm run cdp:viewport-check -- 9333 --json
npm run cdp:viewport-check -- 9333 --clear  # attempt the clear too; read the caveat below
```

`scripts/cdp-viewport-check.mjs` prints `inner`, `outer` and `client` for the
Biorouter page and applies the same tolerances as the renderer's warning. Node
≥ 22, no dependencies — deliberately, because the thing being debugged is the
tooling.

#### Fix: restart the instance

Use a fresh session with no override, or set the viewport explicitly to the size
you mean to test. Once an override is already stuck, **restart the instance** —
or have the driver session that set it clear it before detaching (see
[Debugging the dev GUI with agent-browser](agent-browser-debugging.md), "Reset
the viewport you set").

⚠ **Clearing the override from a *different* CDP session is unreliable — it is
not the cure, even when it looks like one.** Chromium keeps emulation state per
DevTools session, so a foreign session has to apply its own override before it
can drop one, and what it hands back is not the state the page started in. Both
outcomes have been measured:

- On the operator's affected instance (2026-09-08) the clear restored
  `inner == outer` at once and left the renderer **half-frozen** — it followed
  the next OS resize once and then stopped, with `outerWidth` stale.
- On a fresh instance pinned and cleared the same way while building this guard,
  the clear recovered fully and the next three OS resizes tracked exactly.

You cannot tell from inside which one you got, and the second is the dangerous
one: it looks fixed. A **restart**, with no emulation ever applied, tracked OS
resizes exactly at 1200×800, 1900×1050 and 1440×1000 in both runs. `--clear`
exists only for the case where a restart would cost you the state you are
debugging; it is a stopgap, and the script says so.

#### The measurements

The two affected instances, over CDP, with the layout correct throughout
(2026-09-08):

| Instance | `inner` | `outer` | Verdict |
|---|---|---|---|
| A | 1440×900 | 1638×963 | pinned — width differs at all on macOS |
| B | 1440×900 | 1440×1000 | pinned — width *matched*, height off by 100 |

Instance B is the one worth remembering: a check that compares widths alone
passes it. The override came from an `agent-browser set_viewport(1440, 900)`
during a post-merge test drive, which is why the rule in
[agent-browser-debugging.md](agent-browser-debugging.md) now names resetting it
as part of the call.

**A live override and an orphaned one are not the same state**, which is most of
why this is so confusing to look at. Both were reproduced on a dev instance
while building the guard:

| State | `inner` | `outer` on an OS resize |
|---|---|---|
| Override live, setting session still attached | pinned | **follows the window** |
| Override orphaned — setting session detached | pinned | **freezes too** |

So an override survives `agent-browser close`; detaching does not undo it, it
only takes the last thing that could. In the orphaned state the window really
does move — `get size of window 1` reads back the new size — while the page
insists on both numbers it had at the moment it was orphaned. That stale
`outerWidth` is the reading that makes people doubt the OS resize itself.

⚠ **Two blind spots, stated so nobody assumes the warning's silence is an
all-clear.** The check script is the backstop for both — it measures on demand
and depends on no event.

1. **A pin at exactly the window size, then orphaned.** Warning and script both
   compare `inner` against `outer`, and an orphaned override freezes both, so
   they stop moving while still agreeing and the page has nothing to disagree
   with. While the setting session is still attached this resolves itself:
   measured, a 1440×1000 pin on a 1440×1000 window was invisible until the window
   moved, at which point `outer` followed to 1200×800 and the warning fired. The
   uncoverable combination needs a driver to pin the window's exact current size
   *and* detach, which neither recurrence did.
2. **A `resize` Chromium never emits.** The warning runs on load and on resize,
   so it is only as reliable as that event. Measured: a fresh instance pinned by
   a driver fires one and the warning appears (reproduced twice), but an override
   *stacked on an already-emulated page* did not always emit one, and the warning
   stayed silent until something else caused a resize — while
   `npm run cdp:viewport-check` reported the pin correctly throughout. **If the
   symptom is there and the console is quiet, run the script before believing the
   silence.**

### `osascript … set size of front window` silently no-ops

```applescript
-- Reports success. Frequently does nothing.
tell application "System Events" to tell (first process whose name contains "Electron") ¬
  to set size of front window to {1400, 900}

-- Works.
tell application "System Events" to tell (first process whose name contains "Electron") ¬
  to tell front window to set size to {1400, 900}
```

The first form returned no error while the window stayed at 900×800 across three
consecutive attempts, which reads exactly like an app that refuses to resize.

**Tell:** always read the size back after setting it. If the value you get is not
the value you set, the harness failed, not the app.

### A renderer built without `--base ./` never mounts under `file://`

The packaged and dev-launched app both load the renderer from
`file://…/.vite/renderer/main_window/index.html`. A bundle built with vite's
default base emits absolute asset paths (`/assets/index-….js`), which under
`file://` resolve against the **filesystem root**, 404, and leave React unmounted
behind the boot splash forever.

**Tell:** the BR splash spins indefinitely, `document.body.innerText` is
`"Loading BioRouter…"`, and — the part that misleads — the console shows **no
error at all**, because the failure is a resource that never loaded rather than
code that threw. It reads exactly like a hung backend, and it is not.

```bash
npx vite build --config vite.renderer.config.mts \
  --outDir .vite/renderer/main_window --emptyOutDir --base ./
```

⚠ Related, and the reason people reach for that command in the first place:
**`MAIN_WINDOW_VITE_DEV_SERVER_URL` is a build-time constant, not an environment
variable.** `main.ts` declares it with `declare var` and the forge vite plugin
substitutes it at build time, so exporting it before launching Electron does
nothing and the app loads the built renderer regardless. If your source edits are
not appearing, this is usually why — rebuild the renderer rather than restarting
a dev server the app is not reading.

And `BIOROUTER_NO_HMR=1` disables vite's **watcher**, so a dev server started
that way keeps serving cached transforms of the source as it stood at launch.

### A dead daemon looks like a frozen layout

If `biorouterd` is not running, the renderer sits on `#/pair` or renders nothing.
A blank page has no layout to reflow, so dragging the window appears to do
nothing at all.

**Tell:** `document.body.innerText` is empty, or `location.hash` is `#/pair`.
Confirm with `pgrep -f 'target/debug/biorouterd'` and check `/tmp/electron.log`
for `biorouterd process exited with code null`.

## The trap behind that trap: `code null` after copying a binary

`exited with code null` in the Electron log means the daemon was **killed by a
signal**, and running it by hand gives exit **137** (SIGKILL) with no output at
all — not a log line, not a panic.

On Apple Silicon, overwriting an existing code-signed Mach-O in place
invalidates the kernel's cached signature for that path, and the new file is
killed on exec. `cp` preserves the bytes perfectly, so the copy is
byte-identical to a working binary and `codesign -dv` still reports a valid
adhoc signature — which makes this very hard to see.

```bash
codesign --force --sign - target/debug/biorouterd
```

That is the whole fix. It applies to any restage of `biorouterd`, `biorouter`,
or the binaries under `ui/desktop/src/bin/` — which is why `just copy-binary`
re-signs, and why hand-copying around it breaks the app in a way that looks
like a renderer bug.

⚠ **Signature invalidation is the whole explanation — resist adding a second
one.** When this happened here the copy had been taken from `target/release/`
while a build was running, so "the binary was captured mid-link and truncated"
looked like the obvious cause and was written down as fact. It was wrong: `cmp`
later showed the copy byte-identical to the finished binary, and re-signing
alone fixed it. Two plausible causes for one symptom is how a real fix gets
attributed to the wrong action and stops being applied.

## Related documentation

- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) — the
  five other ways a working app looks broken when launched from an agent shell.
- [Debugging the dev GUI with agent-browser](agent-browser-debugging.md) — how to run
  the triage snippets above.
- [Renderer testing traps](renderer-testing-traps.md) — the wider family of
  frontend tests that pass while the code they cover is broken.
- [Theme system architecture](../design/theming/theme-system-architecture.md) — where
  the design tokens, including the measures, are defined and generated.
