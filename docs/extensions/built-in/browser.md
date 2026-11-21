# Biorouter Browser capability

> **What this is.** The user guide to the Biorouter Browser capability: the agent tools that open,
> read and operate web pages in the browser in a chat's side panel, how the person allows them,
> and how they divide work with Biorouter Copilot.
> **Status:** Current. Built on branch `br1946/browser` (phase 2 of the
> [built-in browser design](../../desktop-ui/built-in-browser.md)).
> **Audience:** end users, and developers who need the tool contract.

The Biorouter Browser is the Chromium browser in the artifact side panel. People browse with it
like any browser. With this capability on, the agent can drive it too: open a page, read it as
text or as an accessibility tree, click, fill and submit forms, and report what happened. Every
action happens in the visible panel, in the chat's own tab, and the person allows it first.

## Turning it on

The capability is on by default in **Settings > Chat > Capabilities > Biorouter Browser**. Being
on only offers the tools. The agent cannot use them until the person allows it in the chat.

It exists only in the desktop app, and only in a person's own chat. In `biorouter serve`, in the
CLI running on its own, in a subagent and in a scheduled run, the tools are not offered, because no
window there can hold a page or show the approval.

## Tools

| Tool | What it does | Permission |
| --- | --- | --- |
| `browser_open` | Open an http or https address in the chat's agent tab, or a new tab with `new_tab` | Acting |
| `browser_navigate` | Back, forward, reload or stop | Acting, except stop |
| `browser_tabs` | List the agent's tabs, switch to one, or close one it opened | Reading |
| `browser_snapshot` | The page as a compact accessibility tree with `@e` references | Reading |
| `browser_read` | The page, or one element, as Markdown | Reading |
| `browser_screenshot` | A PNG of the viewport, the full page (up to 8,000 CSS pixels tall) or one element | Reading |
| `browser_scroll` | Scroll the page or an element, or bring a reference into view | Reading |
| `browser_hover` | Hover a reference | Reading |
| `browser_wait` | Wait for text, an element's state, an address or a load state, up to 60 seconds | Reading |
| `browser_click` | Click a reference. A plain link to another page of the same site needs only reading | Reading for links within the same origin, acting otherwise |
| `browser_fill` | Clear a field and set its value | Acting |
| `browser_type` | Type text key by key | Acting |
| `browser_press` | Press a key or chord such as `Enter` or `Meta+A` | Acting, except Escape, Tab, arrows, Page Up and Down, Home and End |
| `browser_select` | Choose options in a `select` | Acting |
| `browser_dialog` | Accept or dismiss a JavaScript dialog Chromium reports | Acting |
| `browser_drag` | Drag an element and drop it on another element or a point in the page: reorder a list, move a card, fill a drop zone. Each element is an `@e` reference, a CSS selector, or `text=` and its visible words. Page events only: the person's pointer never moves, and no file from disk is dropped | Acting |

The advertised names carry the capability's prefix, `browser__browser_open` and so on. No name
matches one of Copilot's tools.

### Snapshots and references

A snapshot lists the page's interactive and landmark elements, one per line:

```text
Page: Request a sample  https://site-a.test/agent/form
Tab t1, opened by Biorouter. References last until this page navigates.
@e2 [textbox] "Full name"
@e5 [combobox] "Country" value="Choose" options="Choose|Canada|Mexico|United States"
@e15 [textbox] "Password" value="(hidden)"
@e17 [button] "Send request"
```

- A reference names one element by the node Chromium gives it, not by a CSS path.
- An element keeps its reference across later snapshots of the same page, and a number is never
  reused, so an old reference can never land on a new element.
- After the page navigates, every reference from before fails with the reason and the advice to
  take a new snapshot.
- A control with no name, or the same name as another, gets a hint from the page: `icon:`,
  `img:`, `testid:`, `name:`, `id:` or `in:` (the text of its row).
- Cross-site frames, such as a consent dialog, are listed under a `frame` line and can be used
  like the page itself.
- Something a person would drag gets a reference even when it is plain structure: an element
  with `draggable="true"`, the items jQuery UI and similar libraries mark, and an element that
  sets a `move` or `grab` cursor (or, when a list sets it, each item of the list). It shows as
  `@e9 [draggable] "Write the report"`, or `[listitem] "Item 1" [draggable]` when its role
  says more, and a grip with no words of its own is named by its row with `in:`. At most 100
  per frame, so a board with hundreds of cards does not flood the snapshot.

### Dragging

`browser_drag` takes `from` and either `to` or a point (`to_x`, `to_y`). Each end is one of:

| Form | Example | Matches |
| --- | --- | --- |
| A reference | `@e12` | The element of the latest snapshot |
| A CSS selector | `#example1 > .list-group-item:nth-child(1)` | Every element the selector matches |
| Visible words | `text=Item 1` | The smallest element whose whole text is the words (case and spacing ignored), or else the elements whose text contains them |
| Scoped words | `#example1 >> text=Item 1` | The same, inside the elements the selector before `>>` matches |

A selector or words must match exactly one visible element of the page (frames are reached by
their references). Otherwise nothing is dragged and the result says what was found, for example
`2 visible elements match the text "Item 1" (in #example1, #example2-left)`, with a scoped form
to try. A sortable list whose rows have no role and no drag cursor, such as SortableJS's demo, is
reached this way.

The drag is made the way a person makes one: a hover, the press, a short lift past the start
thresholds of drag libraries, then many small moves to the destination and a pause there. The
drop lands past the middle of the destination in the direction of travel, so a sortable puts the
item in the destination's place. The destination is measured again once the source is picked
up, because many pages reflow at that moment. A page that uses native HTML drag and drop gets
the matching drag events, ending in a drop; a page that uses mouse or pointer events gets those.

The result says how the page answered when it can tell: `the page moved it after [listitem]
"Item 3"`, `into [region] "Done"`, or that the element is still where it was (a drop zone may
copy rather than move). Take a new snapshot to check anything else.

### What every action checks

Before it touches the page, every click, fill, hover and selection resolves the reference,
scrolls the element into view, and checks that it is visible, enabled, not moving and not
covered. A failed check is reported, for example
`@e9 [button] "Under the banner" is covered by [dialog] "Cookie consent"`, and nothing happens.
The element is outlined for a moment before the action, so a person watching sees what is about
to be clicked. Input is sent to the page only: no action moves the mouse pointer, takes the
keyboard focus or raises the window.

### What the agent never does

- It never types into a password, verification code or card number field, and their values
  never appear in a snapshot or in `browser_read`. When a page needs a login, the agent asks the
  person to sign in in the panel. A drag never starts or ends on one either, since a drop types
  the dragged text into the field, and a drop at a point is refused when the point is on one or
  inside a frame.
- It never types text that looks like a credential (a private key, a cloud key, a provider key).
- It has no tool to run page JavaScript, read cookies or storage, or upload a file.

## Permission

The browser asks the person through a card above the composer, beside Biorouter Copilot's. There
are two grants:

| Grant | Covers | Lasts |
| --- | --- | --- |
| Reading | The pages the agent has open: reading them, snapshots, screenshots, scrolling, hovering, waiting, stopping a load, following a link within the same origin | The chat, until Stop |
| Acting | Opening an address, back, forward and reload, following a link to another origin, every other click, filling, typing, keys, choices, dialogs, downloads | The request, or the chat with **Allow for this chat** |

Every navigation the agent chooses is acting, because an address the agent opens is text it
wrote: a page that slipped it instructions could otherwise have it carry page content to another
site in a query string under the reading grant alone. Approving acting also allows reading.

The card names the model and where page content goes, in full. Only the person can answer it: the
route that records the answer requires the user-action proof, so neither the model nor a plain
API call can allow the browser. It applies in every permission mode, including Completely
Autonomous.

**Stop** is one control for both agent capabilities: it ends the browser's grants and Copilot's,
and cancels any browser action in flight. A model or provider change and Chat mode end the
browser's grants too. A declined or stopped grant is a stop condition: the agent does not retry
it and does not move the task to Copilot or to a script.

## Where it works

- **Only on screen.** The agent drives a tab only in the chat's own side panel, while the chat is
  the active tab of its window. If the chat is behind another tab, or not open, the tool says so
  and the agent asks the person to open the chat. A chat that is not open in any window gets that
  answer at once, without waiting for an approval nobody can see.
- **Only its own tabs.** The agent sees the tabs it opened and the tabs those pages opened, never
  a tab the person opened.
- **Local addresses.** A chat on a public model cannot open a private network or loopback
  address. A chat on a private model can, as the person can. Biorouter's own ports are refused
  to everyone.
- **Downloads.** Under the acting grant a download is saved, without a dialog, to `downloads/` in
  the chat's working folder, never opened, quarantined on macOS and capped at 500 MB. Under
  reading only it is cancelled and the result says why.

## Biorouter Browser or Biorouter Copilot

| The task | Use |
| --- | --- |
| Anything on a web page: open, search, read, compare, fill, submit, download | Biorouter Browser |
| The plain text of a public page, no login and no interaction | Web & Documents `web_scrape`, which needs no permission |
| A native app, the operating system, Finder or Explorer, system settings, an Open or Save dialog | Biorouter Copilot |
| The person's own Chrome or Safari window, its tabs or its logins | Biorouter Copilot on that app |

Both tool sets say this in their instructions. A Copilot grant does not allow browser tools and a
browser grant does not allow Copilot tools. The browser never moves the pointer or the focus, so
the two can work at the same time without interfering.

## How results reach the model

Every result passes through the same checks as any other tool: credentials in page text are
withheld, and page text arrives inside a `<browser-page origin="…" untrusted="true">` frame with
page-chosen labels made safe, followed by the generic untrusted-output frame. Page text and
screenshots are data, never instructions, and the instructions say so. Results carry the tab, its
address and title, whether the page navigated, tabs a page opened, a finished download and
whether the person used the page during the action.

## Related documentation

- [The built-in browser](../../desktop-ui/built-in-browser.md): the design (BB-01 to BB-26) and what each phase built
- [Biorouter Copilot](computer-controller.md): native app and desktop control, and the consent model the browser follows
- [Web & Documents](web-documents.md): fetching public pages without a browser
- [Privacy tiers](../../security/privacy-tiers.md): why a public chat cannot open a local address
- [Workspace control](../../agent-loop/workspace-control.md): the workspace socket the browser's operations travel over
