# Crew UI redesign specification

> **What this is.** The final design for the BioRouter Crew desktop GUI: a clean, Slack-like, layered interface built only from BioRouter's design system. It covers the layout, every screen and control mapped from the current UI, the component and file architecture, progressive disclosure, the full copy deck, identity rules, revoke, privacy, motion, accessibility, theming, the regression-test migration and the acceptance criteria novice reviewers will apply.
> **Status:** Current. Approved design, 2026-09-23. Nothing in it is built yet; the implementation workplan builds it in packages, and [implementation status](implementation-status.md) records what lands.
> **Audience:** Implementers of the Crew GUI (`ui/desktop/src/components/crew/`), the daemon, broker and CLI implementers whose data it shows, reviewers who judge it with a novice walkthrough, and whoever migrates the Crew regression tests.

Crew today is one 2,175-line component (`CrewView.tsx`) that renders a form wearing a page header: a native
`<select>` for the workspace, a rail of paragraphs, flat rows of equal-weight buttons, raw machine IDs, a
connection form with thirteen inputs and a hand-rolled modal. This spec replaces it. It was chosen from three
competing proposals (a Slack information architecture, novice-first guided flows and a design-system-purist
"quiet elegance" proposal) after three independent judgements (novice walkthrough, design fidelity,
feasibility) and two reviews of the naming design. The Slack-architecture proposal is the base because it
scored best on test preservation and decomposition; its known gaps are closed with grafts from the other two,
and every problem the judges found is fixed below. The identity and join protocol it displays is specified in
the companion [naming design](naming-design.md).

## How to read this spec

- **Path abbreviations.** `crew/` is `ui/desktop/src/components/crew/`, `ui/` is `ui/desktop/src/components/ui/`,
  `CV` is `crew/CrewView.tsx`, `CVT` is `crew/CrewView.regression.test.tsx`, `CAT` is
  `crew/CrewAuthentication.regression.test.tsx`, `CFT` is `crew/CrewFiles.regression.test.tsx`. `CV:123` is a line
  of `CrewView.tsx` at the design's base commit `76b88555`.
- **Test constraints `C1`–`C16`** are the rules the current regression tests impose, and **latent defects
  `L1`–`L19`** are bugs found while inventorying the current UI. Both lists are reproduced in
  [Test constraints and latent defects](#test-constraints-and-latent-defects) so this document stands alone.
- **Naming slices `S0`–`S4`** and **decisions `D1`–`D17`** are defined in the [naming design](naming-design.md).
  Revoke daemon items **`RV-D1`–`RV-D5`** and renderer items **`RV-R1`–`RV-R3`** are defined in
  [Revoke](#revoke).
- A string marked **(pinned)** is asserted by a test or cited by acceptance evidence. Change it only together with
  its test, as listed in [Test migration plan](#test-migration-plan).

## Decisions at a glance

1. **Two Crew columns and an optional pane, beside the app sidebar.** A 240px Crew sidebar, then the channel
   column (timeline and composer at the 760px chat measure), then a 360px non-modal details pane. No page
   header, no "Crew" title, no icon rail.
2. **The workspace name is a menu.** `lab ▾` at the top of the Crew sidebar replaces the workspace `<select>`,
   the connection card, the Reconnect/Authenticate/Edit link row and the top-bar "Add workspace" button.
3. **Security state has one resting home: a 32px status row** under the workspace name. Left: a status dot and a
   word ("Connected", "Sign-in needed", "Offline"). Right: the privacy chip (`🔒 Private · ucsf`). It is visible in
   every state once a connection is selected, including setup, trust and join screens.
4. **Teams are collapsible sidebar sections; channels are rows.** Bold means unread. Sidebar sections for
   Invitations, Waiting to join (host) and Agents (running tasks and connected chats) appear only when non-empty.
5. **The channel name is the channel menu.** `# methods ▾` opens a real `DropdownMenu`; a separate details toggle
   opens the pane. Only real menus carry a chevron.
6. **The details pane is non-modal.** Channel details (About, Members, Files, Access), Ask my agent and Chat access
   (grant and revoke) live in one `<aside>` beside the conversation, never behind a scrim.
7. **The composer is BioRouter's chat composer card**: `Message #methods`, an Attach menu, Ask my agent and a
   round Send. The "Posting as…" footer and every disclaimer paragraph are gone.
8. **The timeline reads like Slack**: grouped messages, day dividers, an accent "New" line, automatic older-history
   loading, a Jump to latest pill, and agent tasks as one-line status rows at the point they were requested.
9. **Joining needs no machine strings.** The host invites `@bob` and copies one invitation message. Bob pastes it,
   confirms the workspace's privacy, signs in, and sends back a 16-character code his own computer computed. The
   host pastes the code. This is naming slice S3a ("fix A"); discovery and 8-digit codes are deferred (S4).
10. **Connection failures are classified, never guessed.** The daemon returns a typed code for each SSH failure.
    Sign-in opens by itself when the server wants a password or MFA; a new or changed host key gets its own
    screen with no accept button; a missing per-user `biorouter-crew` gets a plain sentence and a message to send.
11. **Everything a person hands to someone else is a `CopyField`** with a one-click Copy: invitation messages,
    device codes, fingerprints, commands, legacy tokens, server paths and the known-hosts path.
12. **Exposing privacy changes ask first, and the reverse is one click.** Private→Public (connection) and "Allow
    Public" (workspace) use `DangerousConfirmDialog` with the workspace name as the typed phrase, from every path
    that can make the change. The irreversible institution label has its own confirmation.
13. **People appear by display name with `@username`.** At every authority point both are shown in full. Machine
    IDs appear only behind "Copy … ID".
14. **Revoke has four homes** (the Chat access pane, the channel's Access tab, the Agents sidebar section and the
    workspace-wide Agent access tab), all backed by the daemon's existing proven routes. A revoke that the daemon
    did not complete never reads as success.
15. **Every error renders exactly once, at the surface that caused it.** Observation errors live in the connection
    bar; send errors above the composer; agent-start errors in the pane; dialog errors in the dialog.
16. **React authorizes nothing.** The daemon and broker decide every action; the UI shows state, sends proven
    requests and renders refusals in words.

## Principles

- **Layered, not flat.** A place is a row or a menu; an action on the current object lives in that object's menu
  or row `⋯`; a fact you may want to check lives in a popover or the pane; a consent tied to a channel lives in the
  pane; a failure that needs an answer is a `Note` in place with one action. Exactly one accent button per view.
- **Silence has a budget, security state does not.** Remove disclaimers and prose with no immediate consequence;
  put learn-later material behind a disclosure or a tooltip. Never hide connection state, privacy mode, who you act
  as, who owns a run, or where a post goes.
- **Only the necessary fields by default.** Every optional field sits behind one `Advanced` disclosure whose closed
  summary states the defaults. Nothing required ever hides in Advanced; a submit with an invalid hidden field opens
  the disclosure and focuses the field.
- **A dropdown looks like a dropdown; a list looks like a list.** Chevrons only on real menus and pickers. Lists
  are hairline-separated rows with hover tint, `aria-current` on the selection and keyboard navigation.
- **Names, never IDs.** People by display name and `@username`, workspaces, teams and channels by name.
- **Motion explains where something came from or confirms a change.** It never decorates, never animates
  security state, and always yields to reduced motion.
- **The design system is the skin.** Every value is a `main.css` token, every control a `ui/` primitive, every
  pattern one the chat UI already uses. Anything missing becomes a shared primitive, never a Crew one-off.

## What was chosen, and why

| Area | Taken from | Why |
|---|---|---|
| Overall structure, file lanes, test preservation, non-modal pane, exactly-once error resolver, presentation-only re-verification view, margin-based titlebar reserve, instant team-section collapse | Slack-architecture proposal (base) | Highest feasibility and design scores; keeps every behavioral test assertion |
| Persistent status row with connection and privacy | Novice-flow and quiet-elegance proposals | The base showed privacy only in the channel header |
| Invitation message, privacy confirmed at join, sign-in that opens on the real SSH failure signal, Disconnect, Task prefilled from the draft, `deriveCrewScreen()` | Novice-flow proposal | Only proposal whose novices finished without help |
| Channel name as the menu, details toggle, run rows as lines with words, accent New line, `CopyField` on the well ground, shared `Avatar`, consolidated Workspace settings dialog, auto-prepared hosting identity, visible Stop, labelled agent-access chip, fixed-width pane content during its width animation, delayed skeleton | Quiet-elegance proposal | Purest design-system use; P3-correct colour in all theme families |
| Real yield-ladder geometry, `crew` stylesheet source guard, the Advanced-field rule | Novice-flow proposal | The other two assumed a 288px sidebar at 1048px |
| Join by invitation plus desktop-computed device code; discovery deferred | Naming reviews | The broker-computed 8-digit code is unsound (a same-UID process substitutes its own key) |
| Typed SSH failure codes, "Crew isn't set up for your account", "Add Bob to Analysis Lab", joiner told which host to ask | New in this spec | Each closes a dead end every proposal shared |

**Rejected on purpose.** Coaching a novice to type `yes` at an unverified host-key prompt, and any removal command
for a changed key (both soften out-of-band trust). Modal dialogs for Ask my agent and chat access (they hide the
destination and break test constraint C1). Changing `refresh()` semantics or adding per-channel drafts inside the
refactor (behavior changes smuggled into a refactor). A second "Ask my agent" button in the channel intro (it
makes the pinned query ambiguous). `Esc` disposing the SSH session. "Signed in until …" (the daemon's SSH master
uses an idle timeout, `ControlPersist=600`, so any time shown would be a guess).

## Layout

### Placement, the 44px band and the drag region

Crew is one route (`/crew`) inside `SidebarInset`. The app sidebar (216–360px, default 288px, auto-collapsed below a
1120px window by `SIDEBAR_COMPACT_WIDTH`) is unchanged, with **Crew** highlighted.

- **One continuous top edge.** The Crew sidebar header (workspace switcher), the channel header and the pane header
  are `h-chrome` (44px) bands on `bg-sidebar` with a bottom `border-border-subtle` hairline that meets the app
  sidebar's titlebar hairline at y=44, exactly as the chat header does.
- **Drag region (issue #74).** No Crew band declares `-webkit-app-region: drag`. Every band control carries
  `no-drag`. `AppLayout.tsx`'s `isChatRoute` predicate gains `/crew`, so the 32px drag strip stops taking pointer
  events on this route. When the app sidebar is collapsed (the default at a 1048px window), the workspace switcher
  reserves `var(--biorouter-titlebar-control-reserve)` as a **margin-left**, never padding: a padded reserve stays
  inside the band's box and would re-cover the floating titlebar controls. Verify in the real app with the
  sidebar open and collapsed; jsdom cannot see drag rects.
- **Grounds.** Crew sidebar `bg-sidebar` with `border-r border-sidebar-border`. Channel column and pane
  `bg-background-canvas`. Cards, dialogs and menus `bg-background-default`. Copy boxes `bg-background-well`.
  `--background-muted` is never a column ground (in dark mode it is the lightest step).
- **Headings.** The channel name is the page `<h1>` (visually `text-label`). Non-channel states render their title
  as the `<h1>`. The document title is prefixed with "Crew" for screen readers.

### Widths, the pane and the yield ladder

Crew-local geometry is declared once on the new layout root, `.crew-app`, in the new stylesheet `crew/crew-app.css`:

```css
.crew-app {
  --crew-sidebar-width: 240px;  /* design.md §4.11 */
  --crew-pane-width: 360px;     /* = PREVIEW_MIN_WIDTH in Layout/yieldLadder.ts */
  --crew-push-min: 800px;       /* = PREVIEW_SIDE_WIDTH = 360 + READABLE_CHAT_WIDTH 440 */
  --crew-avatar: 32px;
  --crew-avatar-dense: 20px;
  --crew-setup-width: var(--dialog-md);
  container: crew-main / inline-size; /* on the channel+pane region */
}
```

A source-guard test asserts that the two numbers equal the `yieldLadder.ts` constants, because CSS cannot import
them.

| Window and app sidebar | Crew area | Channel column | Pane opens as |
|---|---|---|---|
| 1048px, sidebar auto-collapsed (default) | 1048 | 808 | **Push** (channel keeps 448px) |
| 1120–1279px, sidebar open at 288px | 832–991 | 592–751 | **Cover** |
| 1280px, sidebar open | 992 | 752 | **Cover** |
| 1280px, sidebar collapsed by the user | 1280 | 1040 | Push (680) |
| 1440px, sidebar open | 1152 | 912 | Push (552) |

- **Push** (channel region ≥ 800px): the pane is a grid column beside the channel; the conversation stays usable.
- **Cover** (below 800px): the pane covers the channel body *below the channel header*, so the channel name and the
  status row stay visible. The pane header gains **← Back to #methods**. The covered timeline and composer get
  `visibility: hidden` so they leave the tab order while unseen; the draft is kept. This deliberately differs from
  the artifact panel's "stack" rung: the pane holds forms, and stacking one above the conversation would push the
  composer below the fold.
- The switch is a container query in authored CSS. JavaScript never measures layout. jsdom does not evaluate
  container queries, so tests exercise the push behavior.
- The timeline and composer use `max-w-measure-chat mx-auto` (760px), exactly as chat does.

### Wireframes

Glyphs are placeholders for app-registry icons. `🔒` is `PrivacyBadge` (it ships as its padlock, never an emoji),
`(AC)` an initials avatar (circle for people), `[▣]` an agent avatar (square), `●` a `StatusDot`, `◌` the spinner,
`▌` the 2px accent bar, `[Copy]` the CopyField button.

**Channel view, 1048px window, app sidebar auto-collapsed, pane closed.**

```text
x=0                        240                                                            1048
┌───────────────────────────┬───────────────────────────────────────────────────────────────┐ y=0
│ ⊞ ⧉ │ lab              ▾  │ # methods ▾  Restricted              [▣] 2 chats  (A)(B) 5  ⧉  │ 44 band
├───────────────────────────┼───────────────────────────────────────────────────────────────┤ y=44
│ ● Connected  🔒 Private·ucsf│ [connection bar slot: empty when healthy]                    │ 32 status row
├───────────────────────────┤                     ── Today ──                               │
│ Invitations               │  (AC) Alice Chen  @alice  10:02 AM                            │
│   Imaging Core   [Accept] │       Counts are in. Plot next?                               │
│ ▾ Analysis Lab        ⋯ + │  ───────────────────────────────────────────── New ──        │
│   # general               │  [▣] Your agent · Working…            Open   Stop   ⋯         │
│ ▌ # methods               │       Plot counts by sample                                   │
│   # raw-data         (b)  │                                                               │
│   + Add channel           │      ┌──────────────────────────────────────────────┐         │
│ ▸ Single-cell         ⋯ + │      │ Message #methods                             │         │
│ + Add team                │      │ 📎  ⎔ Ask my agent                        (↑) │         │
│ Agents                    │      └──────────────────────────────────────────────┘         │
│   ◌ #methods · Working…   │                                                               │
│   ▣ Plot review · #methods│                                                               │
│ (AC) Alice Chen        ▾  │                                                               │
│      alice@hpc.ucsf.edu   │                                                               │
└───────────────────────────┴───────────────────────────────────────────────────────────────┘
```

`⊞ ⧉` are the floating titlebar controls; the switcher starts after them by margin. `(b)` marks a bold unread row.
`⧉` at the far right of the band is the details toggle (`PanelRight`).

**1048px, pane pushed on the Members tab.**

```text
240                                          688                          1048
┌─────────────────────────────────────────────┬──────────────────────────────┐
│ # methods ▾  Restricted     (A)(B) 5   ⧉ ✓  │ # methods                  × │ 44
├─────────────────────────────────────────────┼──────────────────────────────┤
│  messages (448px)                           │ About  Members  Files  Access│ tabs
│                                             │ ──────── (2px bar)           │
│                                             │ [Add people…]                │
│                                             │ (AC) Alice Chen   Owner   ⋯  │
│                                             │      @alice                  │
│                                             │ (BL) Bob Lee      you        │
│  ┌ composer ┐                               │      @bob                    │
└─────────────────────────────────────────────┴──────────────────────────────┘
```

**1280px, app sidebar open, Ask my agent covering the channel body.**

```text
288                  528                                                   1280
┌────────────────────┬──────────────────────────────────────────────────────┐
│ lab             ▾  │ # methods ▾  Restricted                (A)(B) 5  ⧉ ✓ │ 44 (stays)
├────────────────────┼──────────────────────────────────────────────────────┤
│ ● Connected 🔒 …   │ ← Back to #methods      Ask my agent               × │ 44 pane header
│ …                  │ Posts to #methods in Analysis Lab · lab              │
│                    │ Task                                                 │
│                    │ ┌──────────────────────────────────────────────────┐ │
│                    │ │ Plot counts by sample and post the figure.       │ │
│                    │ └──────────────────────────────────────────────────┘ │
│                    │ Model   GPT-5.5 · Versa 🔒              [Change]     │
│                    │ ▸ Advanced · Also reads nothing else                 │
│                    │ ──────────────────────────────────────────────────── │
│                    │ Your agent can read #methods and post there,         │
│                    │ for this task only.                                  │
│                    │                [Start my agent and allow posting here]│
└────────────────────┴──────────────────────────────────────────────────────┘
```

**First run (no saved workspace).** No Crew sidebar; the empty state is centred in the Crew area.

```text
┌ app sidebar ┬───────────────────────── Crew area ──────────────────────────┐
│             │                         (Users 24)                           │
│             │                    Work together in Crew                     │
│             │    Chat, share files and run agents with your lab.           │
│             │                    [ Join a workspace ]                      │
│             │                     Host a new workspace                     │
└─────────────┴──────────────────────────────────────────────────────────────┘
```

**Join dialog after pasting an invitation** (`ModalShell size="md" purpose="form"`).

```text
╭ Join a workspace ───────────────────────────────────────────── × ╮
│ Invitation from your host                                        │
│ ┌──────────────────────────────────────────────────────────────┐ │
│ │ Join lab on Crew. … brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOi…  │ │
│ └──────────────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────────────┐ │
│ │ lab                                                          │ │
│ │ Hosted by Alice Chen (@alice) on hpc.ucsf.edu                │ │
│ │ 🔒 Private · ucsf                                             │ │
│ │ Fingerprint [ 3F2A 9C1E 77B0 D4E1                 [Copy] ]   │ │
│ └──────────────────────────────────────────────────────────────┘ │
│ Your username on hpc.ucsf.edu                                    │
│ [ bob                                                         ]  │
│ You'll join as 🔒 Private · ucsf.  Change                         │
│ ▸ Advanced · Port 22 · your SSH settings                         │
├──────────────────────────────────────────────────────────────────┤
│                                          Cancel   [ Join lab ]   │
╰──────────────────────────────────────────────────────────────────╯
```

**Join states in the channel column** (one centred setup card at a time, `--crew-setup-width`).

```text
  Signing in                      Invited (code shown)                 Not invited
╭────────────────────────────╮  ╭───────────────────────────────────╮  ╭──────────────────────────────────╮
│ ◌ Connecting to            │  │ Alice Chen (@alice) invited you   │  │ You're not in lab yet            │
│   hpc.ucsf.edu…            │  │ to lab.                           │  │ Ask Alice Chen (@alice) to       │
╰────────────────────────────╯  │ Send Alice this code:             │  │ invite @bob. This page updates   │
                                │ [ 7QK2-M9XA-3JTP-WZ4D     [Copy] ]│  │ by itself.                       │
                                │ ▭▭▭▭ Waiting for Alice to let     │  │ [ Hi Alice, please invite …[Copy]]│
                                │      you in…                      │  ╰──────────────────────────────────╯
                                ╰───────────────────────────────────╯
```

**Host dialog, step 2** (`ModalShell size="lg"`, stepper `Name · Start · Create`).

```text
╭ Host a new workspace ────────────────────────────── Step 2 of 3 · Start × ╮
│ Start Crew on hpc.ucsf.edu                                                 │
│ Run this in a terminal signed in to hpc.ucsf.edu as alice:                 │
│ ┌──────────────────────────────────────────────────────────── [Copy] ┐   │
│ │ ~/.local/bin/biorouter-crew start --name lab --bootstrap-key 9f3a…   │   │
│ └──────────────────────────────────────────────────────────────────────┘  │
│ Paste what it printed                                                      │
│ [                                                                      ]  │
│ ▸ Not signed in to the server in a terminal yet?                          │
│ ▸ biorouter-crew isn't installed yet?                                      │
│ Anyone who can sign in to this server can see the workspace name.         │
├────────────────────────────────────────────────────────────────────────────┤
│                                                     Back   [ Continue ]    │
╰────────────────────────────────────────────────────────────────────────────╯
```

**Sign in** (`ModalShell size="lg" purpose="required"`; no ×, no Escape, no backdrop dismissal).

```text
╭ Sign in to hpc.ucsf.edu ──────────────────────────────────────────╮
│ Type your password or verification code in the box below.         │
│ Nothing you type is saved.                                        │
│ ┌───────────────────────────────────────────────────────────────┐ │
│ │ bob@hpc.ucsf.edu's password: ▌                                 │ │  xterm, family palette
│ └───────────────────────────────────────────────────────────────┘ │
│ ▸ Trouble signing in?                                             │
│                                                          [Close]  │
╰───────────────────────────────────────────────────────────────────╯
```

**New host key (tier 3, replaces the channel area; no accept button anywhere).**

```text
╭──────────────────────────────────────────────────────────────╮
│ Can't verify hpc.ucsf.edu yet                                │
│ Crew only connects to servers you've already verified.       │
│ Fingerprint the server offered                               │
│ [ SHA256:Wm9vZm9vZm9vZm9vZm9vZm9vZm9v…           [Copy] ]    │
│ ▸ How do I verify it?                                        │
│                                            [ Try again ]     │
╰──────────────────────────────────────────────────────────────╯
```

**Workspace menu** (`DropdownMenu`, `align="start"`, `w-72`).

```text
┌──────────────────────────────────────────────┐
│ lab                                           │ text-label, not focusable
│ Hosted by Alice Chen (@alice)                 │
│ Signed in as alice@hpc.ucsf.edu               │
│ ● Connected · identity verified               │ status line (+ last error, 2 lines)
├──────────────────────────────────────────────┤
│ Invite people to lab…                  (host) │
│ People…                                       │
│ Privacy…                                      │
│ Chats with access…                            │
│ Create team…                                  │
├──────────────────────────────────────────────┤
│ Reconnect                                     │
│ Sign in…                                      │
│ Disconnect                                    │
│ Connection settings…                          │
├──────────────────────────────────────────────┤
│ Switch workspace                  (≥2 saved)  │ radio group, each with its dot
│   ● lab                                     ✓ │
│   ● imaging-core                              │
│ Add a workspace                            ▸  │ Join a workspace… · Host a new workspace…
└──────────────────────────────────────────────┘
```

**Privacy popover** (from the status-row chip).

```text
┌──────────────────────────────────────────┐
│ 🔒 Private · ucsf                          │
│ Only private and ucsf-approved models     │
│ can read lab.                             │
│ ──────────────────────────────────────── │
│ Your connection   Private                 │
│ Workspace         Private for everyone    │
│ Institution       ucsf                    │
│ ──────────────────────────────────────── │
│ Private because the workspace is Private  │  the "why" line, always shown
│ for everyone.                             │
│ [Make public…]              [Privacy…]    │
└──────────────────────────────────────────┘
```

**Channel menu** (from `# methods ▾`).

```text
┌─────────────────────────────┐
│ Channel details             │
│ Members                     │
│ Files                       │
│ Chats and agents with access│
├─────────────────────────────┤
│ Add people…         (owner) │
│ Mark as read                │
│ Refresh channel             │
│ Copy channel name           │
│ Copy channel ID             │
├─────────────────────────────┤
│ Rename…       (owner, S2)   │
│ Transfer ownership… (owner) │
│ Archive channel…    (owner) │ destructive, last
└─────────────────────────────┘
```

**Chat access pane, active grant.**

```text
┌ Chat access ───────────────────────────────── × ┐
│ “Plot review”                         Active    │
│   Reads #methods                                │
│   Posts in #methods as Alice Chen (@alice)      │
│ Access ends when you revoke it, or after an     │
│ hour.                                           │
│ [Open chat]                     Revoke access   │  quiet destructive
└─────────────────────────────────────────────────┘
```

## Information architecture

### The Crew sidebar

`<nav aria-label="Crew">`, 240px, `bg-sidebar`, scrolls independently between the fixed header, status row and
footer.

| Region | Contents | Shown when |
|---|---|---|
| **Switcher band** (`h-chrome`) | `WorkspaceSwitcher`: a `subject`-look trigger (the `KBSelectorTrigger` recipe) holding the workspace name (truncating) and a `ChevronDown` that rotates 180° when open. Opens the workspace menu. | A connection is selected |
| **Status row** (`h-row-rail`, 32px, `px-3`) | Left: `StatusDot` + status word (`text-supporting`), a `role="status"` region. When healthy the word is "Connected" and an `sr-only` span carries **Connected · identity verified** (pinned). A "Sign-in needed" word is a button that opens Sign in. Right: `PrivacyChip`, a ghost `sm` button holding `PrivacyBadge` (full) and ` · ucsf`; accessible name **Privacy: Private · ucsf** / **Privacy: Public**. Reads **Checking privacy…** (no padlock) whenever observed privacy is not verified. Opens the privacy popover. | A connection is selected, in every state |
| **Invitations** | Label "Invitations" (`text-caps`). One row per pending invitation to me: target name, muted "from {inviter}", a small secondary **Accept**. | Non-empty |
| **Waiting to join** (host, S3a) | Label "Waiting to join". One row per pending join: `@bob` (mono) then "Bob Lee" muted, and **Let in…**. A warning line under the row when a device with a different code tried. | Non-empty |
| **Team sections** | Header row: `ChevronRight` (rotates 90°) + team name as typed (`text-secondary font-medium text-text-muted`, never uppercased), hover/focus-revealed ghost `xs` `+` (Create channel in {team}) and `⋯` (team menu). Rows: 32px `h-row-rail`, `Hash` icon, name, `text-text-muted` at rest; unread `font-semibold text-text-default` plus a neutral count `Badge` (`tabular-nums`, `99+` cap); active `--sidebar-active` plus the 2px `--accent-bar`. A trailing collapsed "Archived (n)" sub-row and a quiet **+ Add channel** row. Collapse state is per viewer (`localStorage`, try/catch). | A verified (or last verified) snapshot exists |
| **+ Add team** | Quiet row after the last team. | Same |
| **Agents** | Label "Agents". Owned tasks that are running, waiting for approval or unconfirmed (`#methods · Working…`, a warning `Badge` "Needs you" for approval) and chats connected to this workspace (chat title, muted `#destination`). A task row jumps to and highlights its timeline row; a chat row opens its Chat access pane. Header `⋯`: Show revoked and finished. | At least one row |
| **You row** (pinned footer, 48px, `border-t border-sidebar-border`) | 20px avatar, display name (`text-label`), and on a second line the SSH login as its own text node (`font-mono text-supporting text-text-muted`, e.g. `alice@hpc.ucsf.edu`). In a dev profile, a neutral `Badge` "Profile: alice". Opens the You menu. With no verified snapshot yet: placeholder avatar and the SSH login only. | A connection is selected |

The SSH login in the You row is the standalone text node CVT finds (`fixture`, `alice@new-host`); it also answers
"who am I acting as, and where?" at rest.

### Connection status

`deriveConnectionStatus()` (pure, in `crew/state/crewStatus.ts`, unit-tested row by row) checks in this order:

| Condition | Dot | Word |
|---|---|---|
| The last user-initiated connect failed with a trust code (`crew_ssh_host_key_unknown`, `crew_ssh_host_key_changed`, `crew_workspace_identity_mismatch`) | danger | Can't verify server |
| A connect or sign-in is in flight | neutral + spinner | Connecting… |
| Verified snapshot and observed privacy exist for this connection | success | Connected (`sr-only`: **Connected · identity verified**, pinned) |
| Saved status `connected` and an observation error | warning | **Updates unavailable** (pinned) |
| Saved status `connected`, no snapshot yet | neutral | **Checking connection** (pinned) |
| The last connect failed with `crew_ssh_auth_required` | warning | Sign-in needed (a button) |
| The last connect failed with `crew_bridge_missing` or sign-in ended with `crew_handoff_failed` | danger | Not set up on this server |
| Connected but not a member yet (observation refused as an unknown device, or join status is not `joined`) | neutral | Not joined yet |
| Saved status `disconnected` | idle (`--background-strong`) | Offline |
| Anything else that failed | danger | Can't connect |

The daemon only ever writes `connected` or `disconnected` (`crates/biorouter/src/crew/mod.rs:347,927,974,1000`). The
"needs sign-in", "not set up" and trust states therefore come from the typed code on the most recent connect or
sign-in result, which the controller keeps in `lastConnectFailure`. After an app restart the row reads "Offline"
until the first Reconnect re-derives the code.

### SSH failure classification

The daemon (connect route) returns a typed `code` with the unchanged message text and a bounded, redacted
`detail` (OpenSSH's own stderr, at most 2 KiB) for "Copy details". The UI maps each code to one surface:

| Code | Cause the daemon detected | Surface | One action |
|---|---|---|---|
| `crew_ssh_auth_required` | `Permission denied`, keyboard-interactive needed, or exit 255 with no live master connection | Sign in opens by itself, once per user-initiated Connect, Join or Reconnect | (the sign-in dialog) |
| `crew_ssh_host_key_unknown` | `Host key verification failed` for a host absent from known_hosts | Tier-3 pane "Can't verify {host} yet" with the offered fingerprint | Try again |
| `crew_ssh_host_key_changed` | `REMOTE HOST IDENTIFICATION HAS CHANGED` | Tier-3 danger pane "{host}'s identity changed", old and new fingerprints when known | Copy details for IT (no accept, no removal command) |
| `crew_ssh_unreachable` | Could not resolve, refused, timed out, no route | Connection bar "Can't reach {host}." | Try again (plus Connection settings… in the note) |
| `crew_bridge_missing` | Remote command exit 127, or "No such file" for `~/.local/bin/biorouter-crew` | Tier-3 pane "Crew isn't set up for your account on {host}" | Copy a message for the host |
| `crew_handoff_failed` | Sign-in succeeded but the bridge did not start | Same pane as `crew_bridge_missing` | Same |
| `crew_workspace_identity_mismatch` | `hello` signature or workspace ID does not match the pin | Tier-3 danger pane "This isn't the workspace you joined" | Copy details; Connection settings… |
| `crew_ssh_failed` or no code | Anything else | Connection bar with the daemon's text | Try again |

**Fallback for a daemon without codes.** If the message contains `ssh_eof` or `exit_255`, treat it as
`crew_ssh_auth_required` (the sign-in terminal then shows OpenSSH's own words, including any host-key refusal).
If it contains `exit_127`, treat it as `crew_bridge_missing`. Nothing else is ever labelled a host-key problem; the
old `/host|SSH|key|authentication/i` match is deleted because it sent every SSH failure to the trust screen.

### The workspace menu and the You menu

- **Workspace menu** (wireframe above). Items that open a dialog end in "…". Reconnect, Sign in… and Disconnect
  are always listed as manual tools; when the status needs one it is *also* the one action in the main area.
  People…, Privacy… and Chats with access… open the one **Workspace settings** dialog on the matching tab.
  Switching workspaces is a `menuitemradio` that does exactly what the old `<select>` did. With one saved
  connection, the Switch section is omitted but **Add a workspace** stays. The menu header's status line is the
  only other place "Connected · identity verified" is visible, and it mounts only while the menu is open.
- **You menu** (`side="top"`): the display name and `@username · alice@hpc.ucsf.edu`, then **Edit profile…**,
  **Keys and security…** (credential storage and the vault), **Copy my username**.

### The channel header and channel menu

`<header>` in the 44px band, left inset 16px, every control `no-drag`:

```text
[h1: # methods ▾] [Restricted] [Archived]      ···      [▣ 2 chats] [(A)(B)(C) 5] [⧉]
```

| Element | Primitive | Behavior |
|---|---|---|
| `# methods ▾` | `<h1>` wrapping the `subject`-look `DropdownMenuTrigger` (`Hash`, name, `ChevronDown`), `aria-haspopup="menu"`, accessible name "methods channel menu" | Opens the channel menu |
| `Restricted` / `Public-safe` | `Badge tone="neutral"` with a tooltip ("Public models can't read it." / "Public models may read it when the workspace allows.") | Static; no padlock (the padlock means privacy tier only) |
| `Archived` | `Badge tone="neutral"` | Archived channels only |
| Agent-access chip | ghost `sm` button, `Bot` icon and "{n} chats" (or "{n} tasks" / "{n} agents"); accessible name "{n} chats or agents can post here" | Shown when at least one active grant or running task posts here; opens the Access tab |
| Member stack | up to three 20px avatars with a 2px ring and −4px overlap, then the count (`tabular-nums`); accessible name "{n} members" | Opens the Members tab |
| Details toggle | ghost round `PanelRight`, `aria-pressed`, accessible name "Channel details" | Opens or closes the pane on About |

There is no privacy chip in the header: privacy has one home, the status row, directly above-left.

**Channel menu** items and who sees them are in the wireframe above. **Mark as read** sends `channel.read` to the
latest sequence without `refresh()` (fixes L12). **Refresh channel** (pinned) runs `refresh()`. **Archive channel…**
opens a `ConfirmationModal`. **Team menu** (the `⋯` on a team header, accessible name "{team} options"): Create
channel… · Add people to {team}… (team creator) · Rename team… (creator, S2) · Copy team ID. **Channel row context
menu** (right-click, or Shift+F10 on the focused row): Channel details · Mark as read · Copy channel name.

### The timeline

`ScrollArea` (the chat transcript's primitive, `autoScroll` and `anchorBottomOnResize`) with an inner
`div role="log" aria-live="polite" aria-label="{name} messages"` in the 760px column, and
`.biorouter-scroll-fade-top`.

| Element | Spec |
|---|---|
| **Top of history** | With a full page (≥ 200 messages): a sentinel row with a ghost `sm` **Older messages** (pinned). It loads automatically when the sentinel scrolls into view (`IntersectionObserver`, guarded when absent, as in jsdom); the button is the keyboard and test path. `aria-busy="true"` on the log while a page loads. |
| **Channel intro** | When the start is loaded: `Hash` at 20px, **Welcome to #methods** (pinned, `text-subheading`), one line "Alice Chen (@alice) created this channel.", and for the owner one secondary **Add people**. No second "Ask my agent" here: the composer's button must stay the only control with that name. |
| **Day divider** | A hairline with a centred `text-supporting` pill on `bg-background-canvas`, `rounded-inner`: Today, Yesterday, Monday, September 22, or September 22, 2025. `position: sticky; top: 8px`. |
| **New line** | A 1px `--accent-bar` rule with a right-aligned `New` in `text-chip text-text-accent`, drawn before the first message after `snapshot.read_positions[channelId]` (or before the last `unread` messages when no position exists). Computed when the channel opens; fixed until the channel changes. Accent, not danger: unread is live state, not a failure. |
| **Message group** | Head row: 32px avatar (people: circle with initials; agent posts: square with the Bot glyph), author (`PersonName context="header"`), badges, time (`text-supporting tabular-nums`, "10:02 AM"; full date and time in a `Tooltip`). Continuation rows show only the body; the time appears in the 44px gutter on hover and focus. A group breaks on a new author, a gap over 5 minutes, a day divider, the New line, or human versus agent. |
| **Agent author** | "Alice Chen's agent" (or "Your agent" for mine), `@alice`, `Badge` "Agent". |
| **Restricted marker** | Shown only when a message's restriction differs from the channel's: a muted "Restricted" after the time with the tooltip "Only private models can read this message." |
| **Body** | `text-body whitespace-pre-wrap [overflow-wrap:anywhere]`, plain text as today; long bodies fold with `utils/messageClamp.ts`. |
| **Row actions** | A floating cluster at top-right (28px ghost buttons on the popover surface): **Copy text**, then `⋯` → Copy message ID. Revealed on `:hover` and `:focus-within`; always visible under `@media (hover: none)`. |
| **Task status row** | A line, not a card (design.md D-17): a 32px agent tile, "Your agent · {status word}" (`text-label`), the task's first line muted beneath, the inline action, a visible **Stop** (`ghost sm text-text-danger`, accessible name "Stop task") only while cancellable, and `⋯` (Copy task ID, Open chat history, Copy error). Anchored after the first message carrying the run's `run_id` (the agent's "Task: …" post), else at the end of the log. Only the owner's runs appear (`state.runs` is owner-scoped). |
| **Viewing history** | While a history page is shown, a pill 12px above the composer: **Viewing earlier messages** (pinned) and **Jump to latest** (clears the page, then `refresh()`). Observer message frames are still ignored while paging. |
| **Scrolled up, live** | The same pill slot shows **↓ Jump to latest** when new messages arrive below the viewport. |

**Run status words** (`crewStatus.ts`):

| `status` | Word | Tone | Inline action | Stop shown |
|---|---|---|---|---|
| `starting` | Starting… | running pulse | — | yes |
| `running` | Working… | running pulse | Open | yes |
| `waiting_for_approval` | Waiting for your approval | warning `Badge` | **Review** (secondary `sm`) | yes |
| `cancellation_pending` | Stopping… | running pulse | — | yes (retry) |
| `cancellation_unconfirmed` | Stop not confirmed | warning `Badge` | Try stopping again | yes |
| `interrupted` | Interrupted | muted | Open | yes |
| `outcome_not_durable` | Outcome unknown | muted | Open | yes |
| `completed` | Done | muted | Open | no |
| `failed` | Couldn't finish | danger text | Open | no |
| `cancelled` | Stopped | muted | Open | no |
| anything else | the value with `_` → space, sentence case | muted | Open | no |

**Open** and **Review** navigate to `/pair?resumeSessionId=…`. Crew approves nothing; approval stays in the agent
session. **Stop** opens a `ConfirmationModal` ("Stop your agent? It stops working on this task. Anything it already
did stays done.") and calls the existing cancel route.

### The composer

`Composer.tsx` follows the chat composer card class for class: `.biorouter-composer-card` (its authored
`:has(textarea:focus)` accent edge), `rounded-container`, `bg-background-default border border-border-subtle/60`, no
shadow, in a `px-4 pb-6 pt-3 bg-background-canvas` bar and the 760px column.

```text
┌──────────────────────────────────────────────────────────┐
│ [counts.csv 42% ⏸]  [↗ Remote results ×]                 │ chips row, only when non-empty
│ Message #methods                                          │ textarea, 1 row → 40vh
│ 📎   ⎔ Ask my agent                                  (↑)  │ Attach menu · Ask my agent · Send
└──────────────────────────────────────────────────────────┘
```

| Part | Spec |
|---|---|
| Textarea | `aria-label="Message #methods"` (pinned), placeholder `Message #methods`, `rows={1}`, auto-grows to 40vh, `bg-transparent border-none px-0 py-1.5`. The keydown handler moves verbatim: Enter sends; Shift+Enter, `isComposing`, `keyCode 229` and key repeat do not. |
| Attach | Ghost round `Paperclip` (accessible name "Attach", `aria-haspopup="menu"`) opening a `DropdownMenu`: **Upload a file…** (the secure native picker) and **Share a server path…** (dialog). No drag-and-drop: Crew's file capability comes only from the main-process picker. |
| Ask my agent | `Button variant="ghost" size="sm"`, `Bot` icon, **Ask my agent** (pinned). Opens the pane in agent mode. Disabled while archived or unverified. |
| Send | `Button shape="round"`, accessible name **Send message** (pinned), `ArrowUp`. `secondary` until there is content, then `default` (accent). While posting it shows the spinner, stays the same DOM node and ignores clicks (single flight). |
| Chips | `Badge size="chip"` with a 14px `XIcon`. Remove names kept: "Remove counts.csv", "Remove remote reference Remote results" (pinned). Upload chips show a 16px progress ring and a Pause glyph while active. |
| Note slot | One `Note` directly above the card, in priority order: the send error ("Couldn't send." then the error in its own text node), the chat-connect note ([Revoke](#revoke)), an ownership offer, the host's institution note, the first-join name suggestion. |
| Archived | The card is replaced by a 44px bar: "This channel is archived." |
| Verifying | Without a verified snapshot the card is replaced by a same-height bar "Verifying access…" with a spinner. The textarea is not mounted (C13); the draft stays in state and returns with the card. |

**The send stays non-optimistic.** The idempotency key is reused only while the composer payload is unchanged (C15),
so the draft stays in the composer until the broker answers. A failed send keeps the draft and shows the error
above the card; pressing Send again reuses the same key. Success clears only what was sent.

**Files.** Attachment cards are 40px rows (`File` icon, name, human size in 1024 units such as "55 KB", ghost round
**Save attachment** and, for images, **Preview image**, then `⋯` → Copy file ID, Copy SHA-256), with a thin
`Progress` along the bottom edge while downloading and Pause/Resume in the `⋯`. A server path is a row with the
`Link` icon, the label and the path in a compact `CopyField`, plus a muted "Not uploaded" with the tooltip "Crew
shares the path only. It doesn't check that the file exists or grant access to it." One transfers poller serves
the whole view, every 2s only while a transfer is active (fixes L13).

### The details pane

`<aside>` 360px with a left hairline, non-modal: no scrim, no focus trap, nothing `aria-hidden`. The header is a
44px band (in push mode) or a 40px row (in cover mode) with a title, an optional back control, and a 32px ghost
`×` ("Close details"). `Escape` closes it when focus is inside. One mode at a time:

| Mode | Title | Opened by |
|---|---|---|
| `details` (tabs: About · Members · Files · Access) | `# methods` | Details toggle, channel menu items, member stack, agent-access chip |
| `agent` | Ask my agent | Composer **Ask my agent**, an Agents row's "Start another" |
| `chat-access` | Chat access | The chat-connect note, an Agents chat row, an Access-tab row |

Opening another mode replaces the content in place; the pane node stays mounted, so the pinned CVT flow that clicks
**Ask my agent** with the grant pane open simply switches modes. `refresh()` does not close the pane. It closes only
on an explicit close, a channel or connection switch, or loss of access to the channel, so an action error shown in
the pane survives a manual refresh (C1, CVT:437-466).

- **About** (`.biorouter-settings-list` rows): Name `#methods` (Rename… for the owner, S2); Content `Restricted` or
  `Public-safe` with one consequence line; Owner `PersonName context="authority"` with Transfer ownership… for the
  owner, and "Offered to Bob Lee (@bob) · waiting" while an offer is pending; Created by; Team; a danger zone
  (owner): Archive channel…; footer Copy channel ID.
- **Members**: Add people… (owner, `secondary sm`), count, one row per member (`PersonName context="authority"`)
  with Owner, you and former-member markers; the owner's row `⋯`: Make owner… · Remove from #methods… (confirm).
  Pending channel invitations you created appear as muted "invited" rows.
- **Files**: In progress (this channel's transfers, Pause/Resume); Uploaded, not sent (completed uploads not in the
  composer, with **Attach**, formerly "Restore to composer"); In this channel (attachments and server paths in the
  loaded messages).
- **Access**: [Revoke](#revoke).

### Ask my agent

- **Destination** (always shown, it is the consent): "Posts to #methods in Analysis Lab · lab".
- **Task** (pinned label): its own state, **seeded from the composer draft** when the pane opens (fixes L9). On
  success the composer draft is cleared only if it still equals the seed; the Task is cleared.
- **Model** (pinned accessible name "Model"): when `BIOROUTER_PROVIDER` and `BIOROUTER_MODEL` resolve to a
  configured provider, a one-line summary "GPT-5.5 · Versa 🔒" with **Change**; otherwise, or after Change,
  `CrewModelPicker`: a field-look `Popover` + `Command` trigger whose value reads "Choose a model" when empty, grouped
  by configured provider with `PrivacyBadge dense` and `AffiliationBadge` on each group heading, and a free-text row
  "Use “{text}” with {provider}". With no configured provider: "No models are set up." and a link button
  **Open Settings**. Crew bypasses provider onboarding, so a Crew-first user can arrive here with none.
- **Advanced** (closed summary "Also reads nothing else" / "Also reads 2 channels"): Also read, a checkbox list of
  other non-archived channels across all teams labelled `Team / #channel` (unmounted while closed, which keeps C9),
  and, only when the connection has a remote folder, "Can run commands in /home/alice/crew-work" or "Can read
  /home/alice/crew-work".
- **Consequence hint** (warning `Note`, only when it applies): "This model is Public, so it can't read Restricted
  channels." The daemon still decides.
- **Consent line**: "Your agent can read #methods and post there, for this task only."
- **Start my agent and allow posting here** (pinned) in a sticky footer. The button is rendered unconditionally
  with a stable key; errors render in a fixed slot above it, so the node never remounts (C11).
- **Unknown outcome.** After `crew_start_outcome_unknown`, a warning `Note role="alert"` at the top: title
  **Inspect the previous task before starting again** (pinned), "The request to #methods in Analysis Lab was received,
  but Crew can't tell whether it started. Repeating it could duplicate its effects.", buttons **Show task in channel**
  (scrolls to and highlights the task row) and **Open chat history**, one `Checkbox` in a `<label>` "I checked the
  previous task and its effects." (the only checkbox in the document, C9), and **Start a new task** (pinned; enabled
  once checked; runs `form.reportValidity()` first). The main Start stays rendered and disabled (C12). The lock
  stays module-scoped (C10).

### Main-area states outside a channel

`deriveCrewScreen()` (pure, table-tested) selects exactly one:

| Screen | Condition | Main area | One action |
|---|---|---|---|
| `loading` | First `GET /crew/connections` in flight | After a 150ms delay: sidebar skeleton (2 headers, 6 rows) and 4 message blocks | — |
| `welcome` | No saved connection | First-run empty state (wireframe) | **Join a workspace**; link Host a new workspace |
| `connecting` | Connect or sign-in in flight, no snapshot | Setup card "Connecting to {host}…" | — |
| `offline` | Saved `disconnected`, no failure code | `EmptyState` "{workspace} is offline" / "Connect to see your channels." | **Connect to {workspace}** |
| `sign-in` | Last failure `crew_ssh_auth_required` and the dialog was closed | "Sign in to {host}" / "The server needs your password or a verification code." | **Sign in** |
| `trust` | A trust code | The tier-3 panes above; the unknown-key pane's help offers **Open a terminal here** (the embedded terminal dock) for comparing and adding the verified key. Crew itself never accepts a key | per pane |
| `not-set-up` | `crew_bridge_missing` or `crew_handoff_failed` | "Crew isn't set up for your account on {host}" / "It's installed once per account, usually by your host or IT team." A `CopyField` holding a message for the host, and a disclosure **Install it yourself** with the install commands | **Try again** |
| `join` | Connected, not a member | The join state machine ([Onboarding](#onboarding-join-host-and-admit)) | per state |
| `updates-paused` | Observation error, protected state cleared, no last verified view | Connection bar plus "Messages are hidden until Crew reconnects." | **Retry** (pinned name) |
| `no-team` | Verified snapshot, no teams | Host: the setup checklist. Member: "You're in {workspace}" / "Ask {host} to add you to a team." with secondary link Create a team. With pending invitations: "You're invited to {team}" with **Join {team}** | per variant |
| `no-channel` | A team with no open channels | "No open channels in {team}" | **Create channel** (fixes L15) |
| `channel` | Verified snapshot and channel | The channel view | — |

**Re-verification never flashes.** `refresh()` keeps its order (abort, clear, `GET /connections`, re-observe; pinned by
CVT:652) and still clears the verified snapshot. The layout renders from `lastVerified`, a presentation-only copy
of the last verified snapshot for the same connection: the sidebar and header stay, the timeline dims to 60%
opacity with `inert`, the status row reads "Checking connection" and the chip "Checking privacy…", and the composer
is replaced by the same-height "Verifying access…" bar. `clearProtectedState()` (any observation failure or scope
change) drops `lastVerified` too, so nothing stale survives a failure. No action is enabled from the last verified
view.

**The host setup checklist** (compact card, host only, while any item is open): "Get {workspace} ready" with three
rows — Confirm the institution (**Set institution to {id}…**), Create a team (**Create team**), Invite people
(**Invite people…**) — each ticking when done, and **Hide**.

### Onboarding: join, host and admit

**Join (joiner, S3a).**

1. First run → **Join a workspace**, or Workspace menu → Add a workspace → Join a workspace….
2. The Join dialog has one visible field, **Invitation from your host** (paste the whole message; the daemon extracts
   the `brcrew1:` line and parses it with `POST /crew/connections/from-invitation` in preview mode). Once parsed, a
   summary card shows the workspace name, "Hosted by {host person} on {server}", the workspace's privacy
   (`🔒 Private · ucsf` or `Public`) and the fingerprint in a `CopyField`. "Your username on {server}" is prefilled
   from the invitation. The privacy line reads "You'll join as 🔒 Private · ucsf." with **Change** (revealing the
   radio rows and the institution field); a mismatch line appears only when the choice differs from the workspace's.
   The institution is known from the invitation, so a Private save never dead-ends for lack of one.
3. **Join lab** saves the connection (pinned exactly as the invitation says), then connects. If the server wants a
   password or MFA, Sign in opens by itself and returns here when it closes.
4. The channel column shows the join state machine, polling `GET …/join` every 5s while visible:

| Join status | Card | Action |
|---|---|---|
| `invited` | "Alice Chen (@alice) invited you to lab." "Send Alice this code:" a large `CopyField` holding the 16-character device code (`7QK2-M9XA-3JTP-WZ4D`), then an indeterminate `Progress` and "Waiting for Alice to let you in…" | Copy |
| `approved` | "Joining lab…" (the daemon sends `auth.join` by itself; pressing Join lab was the consent) | — |
| `code_mismatch` | "The code Alice entered doesn't match this computer. Send it again:" and the same `CopyField` | Copy |
| `not_invited` | "You're not in lab yet." "Ask Alice Chen (@alice) to invite @bob. This page updates by itself." A `CopyField` holding "Hi Alice, please invite @bob to lab in Crew." | Copy |
| `expired` | "This invitation expired. Ask Alice Chen (@alice) to invite you again." | — |
| `joined` | Transitions into the workspace; the first-join note offers "Use “Bob Lee” as your name in lab?" **Use** / **Edit…** (the name on the server account is offered, never applied silently) | — |
| Legacy broker (no `join_by_name_v1`), or **Other ways to join** opened | "Join with an invitation token". "Send this join request to your host:" a `CopyField` holding the join request (username and this device's key). Then a `SecretInput` (accessible name **Enrollment invitation**, placeholder "Invitation token") and **Join workspace** | `auth.enroll` |

**Other ways to join** is a `Disclosure` under the invited and not-invited cards, so the legacy path is reachable
without appearing on the default path. "Enter workspace details manually" (socket path, workspace ID, host user ID,
workspace key with `pattern="[a-fA-F0-9]{64}"`, fixing L8) lives in the Join dialog's Advanced.

**Host (S2 and S3a).**

1. First run → **Host a new workspace** (link), or the menu. The Host dialog (`lg`, stepper Name · Start · Create):
   - **Name:** Workspace name (slug, live preview "Your workspace: lab", S2 rules), Your server login, Privacy radio
     rows (Private default) and Institution (required iff Private; prefilled when exactly one configured provider
     affiliation exists). Advanced: port, identity file, jump hosts, connection name, remote work folder and the agent
     execution `Switch`. **Continue** prepares or recovers the hosting identity automatically
     (`POST /crew/devices/prepare`, idempotent); the "Prepare my hosting identity" button disappears.
   - **Start:** one multi-line `CopyField` with `~/.local/bin/biorouter-crew start --name lab --bootstrap-key <key>`
     (before S2, today's four setup lines). "Paste what it printed" takes the one-line output (`brcrew1:…`, printed
     by `start` from S2) or, from an older broker, the `biorouter-crew status` JSON; the daemon parses both. Two
     disclosures: **Not signed in to the server in a terminal yet?** (a `CopyField` with `ssh alice@hpc.ucsf.edu`
     and "If your terminal asks you to confirm the server, compare the fingerprint with the one from your IT team.
     Type yes only if they match.") and **biorouter-crew isn't installed yet?** (the install commands). Beside the
     start command, **Open a terminal here** embeds the app's `InAppTerminalDock` below the box, exactly as the
     coding-agent setup card does; the command stays copyable, and nothing is typed or run for the person. One
     consequence line (S2): "Anyone who can sign in to this server can see the workspace name."
   - **Create:** "lab on hpc.ucsf.edu", the fingerprint `CopyField`, "Creating lab makes this computer its first admin
     device." **Create workspace** saves (with the `preparation_id`), connects, and runs `auth.bootstrap`. This is the
     confirmation "Initialize as workspace host" never had (fixes L3).
2. If the workspace is Private and unlabelled, the flow asks once: "Label lab as ucsf? This can't be changed later."
   **Set ucsf permanently** / **Not now**. Not now leaves the checklist row open.
3. The host lands on the setup checklist.

**Invite and admit (host, S3a).**

1. **Invite people to lab…** (menu, checklist or the People tab): one field, **Username** with an `@` adornment.
   **Invite** sends `enrollment.invite {username}` (no per-keystroke lookup). The result view renders the broker's
   answer, not a preview: "@bob · Bob Lee (name on the server account) · invited". Then "Send Bob this invitation:"
   and a multi-line `CopyField` holding the invite message:

   ```text
   Join lab on Crew.
   In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
   brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ
   ```

   A disclosure **Is Crew installed for @bob on hpc.ucsf.edu?** holds the per-account install commands, and one
   line: "When Bob sends you a code, choose Let in… next to his name in the sidebar." **Done**.
2. **Let in…** (Waiting to join row): the Let in dialog (`sm`) titled "Let @bob into lab" with "Bob Lee (name on the
   server account)" beneath. **Code from Bob** (`DeviceCodeInput`: one field, accepts pasted `7QK2-M9XA-3JTP-WZ4D`,
   `7qk2m9xa3jtpwz4d` or with spaces; Crockford normalization; `autoComplete="one-time-code"`), helper "Paste the
   code Bob sends you directly." When a device with a different code already tried, a warning line appears above the
   field before anything is entered. **Let Bob in** sends `enrollment.approve`. The host UI never renders a code it
   did not receive from the person.
3. Success: "Approved. Bob joins as soon as his Crew checks in." and one secondary button per team the host created:
   **Add Bob to Analysis Lab** (sends `invitation.create`). When Bob's daemon joins, a toast: "Bob Lee (@bob) joined
   lab".
4. Legacy broker: the Invite dialog's Advanced **Invite someone using an older version of Biorouter**: paste their
   join request, enter their user ID on the server (helper `CopyField`: `id -u bob`, run by the host), then
   **Create invitation**; the token appears in a secret `CopyField` with "Send this to Bob. It works once and expires in
   an hour." (fixes L2's generic Save).

### Sign in

`ModalShell size="lg" purpose="required"` titled **Sign in to {host}**, one line "Type your password or verification
code in the box below. Nothing you type is saved."

- The body is `CrewAuthentication` with its contract unchanged (C16): terminal (per-family palette from
  `GENERATED_THEMES[family][mode].terminal` and `TERMINAL_FONT` at 13/20, replacing the hard-coded `#17191c`),
  exactly one `role="alert"`, and a button whose visible text is **Close** and whose accessible name stays
  **Close authentication connection** (pinned).
- Beneath the terminal, a `Disclosure` **Trouble signing in?** holds the host-trust help (`CrewHostTrust`): use the
  same username and password as for this server; add a jump host in Connection settings if IT gave you one; the
  known-hosts path in a `CopyField`.
- No ×, Escape or backdrop dismissal, so a stray key never orphans an SSH session. Exit 0 closes the dialog and
  refreshes without a POST connect (pinned by CVT:468-486).

### Dialog inventory

All use `ModalShell` (`purpose="form"` unless noted). Dialogs opened from a menu render outside the menu; focus
returns to the trigger on close.

| Dialog | Size | Opened from | Footer |
|---|---|---|---|
| Join a workspace | md | First run, Add a workspace ▸ | Cancel · **Join {workspace}** |
| Host a new workspace (3 steps) | lg | First run link, Add a workspace ▸ | Back/Cancel · **Continue** · **Create workspace** |
| Connection settings | md, `scrollBody` | Workspace menu, notes | danger-zone link · Cancel · **Save connection** (pinned) |
| Workspace settings (General · People · Privacy · Agent access) | lg, `info` | Workspace menu People…/Privacy…/Chats with access… | Done |
| Invite people to {workspace} (host) | md | Workspace menu, checklist, People tab | Cancel · **Invite** → Done |
| Let {person} in (host) | sm | Waiting to join row | Cancel · **Let {first} in** |
| Create team | sm | Menu, + Add team, empty state | Cancel · **Create team** → optional "Add people to {team}" step: Skip for now · **Add** |
| Create channel | sm | Team `+`, + Add channel, team menu | Cancel · **Create channel** |
| Add people to #{channel} / {team} | md | Channel menu, Members tab, team menu | Cancel · **Add** |
| Transfer ownership of #{channel} | sm | About tab, Members row | Cancel · **Offer ownership** |
| Rename team / channel (S2) | sm | Team menu, About tab | Cancel · **Rename** |
| Edit profile | sm | You menu, first-join note | Cancel · **Save profile** |
| Keys and security | md, `info` | You menu, vault-locked note | Done |
| Share a server path | sm | Attach menu | Cancel · **Add to message** |
| Sign in | lg, `required` | Automatic, menu, status word, sign-in state | (the component's **Close**) |
| Confirmations | sm | per action | `DangerousConfirmDialog` or `ConfirmationModal` ([Privacy and institution](#privacy-and-institution), [Copy deck](#copy-deck)) |

Radix modal dialogs `aria-hide` their siblings, which is correct here: no dialog is part of a flow in which a person
or a test must act on the background. The flows that need that (Ask my agent and chat access) are the non-modal pane.

### Where errors render: exactly once

- `act(source, key, fn)` tags every action error with its source: `composer`, `pane:agent`, `pane:chat-access`,
  `pane:details`, `dialog:<kind>`, `observer` or `global`. One resolver, `errorSlotFor(source)`, picks exactly one
  slot: the initiating surface if it is still mounted, otherwise the connection bar. Nothing else reads the error.
- Observation errors render only in the connection bar, as `Note tone="warning" role="alert"` with **Retry**
  (accessible name **Retry Crew updates**, pinned). The modal-footer duplicate is deleted.
- The error message is its own text node inside any prefix, so exact-string queries (`send failed`, `start failed`)
  and the observation-error regexes each match exactly one element.
- A single global `busy` becomes per-action pending keys (`send`, `run.start`, `grant`, `connect`,
  `mutate:team.create`, …). A control disables only while its own action, or one that conflicts, runs; the composer
  stays typeable while a team is created. The single-flight refs are unchanged.
- Field validation stays on the field (`aria-invalid`, helper turns `text-text-danger`), native constraint
  validation included (C8).
- Toasts only for results that happen off-screen: "Invitation sent to Bob Lee (@bob)", "Bob Lee (@bob) joined lab",
  "Ownership offered to Bob Lee (@bob)", "lab is now Private for everyone. Agents with access need permission again."
  Copy never toasts.

**Connection bar** (top of the channel column, under the header): at most one `Note` of each kind, in this order:
the observation error (Retry); an observer or global action error (Dismiss, a 20px ×); the one highest-priority
need: vault locked (**Unlock**), unreachable (**Try again**), reconnecting for over 1s (spinner, no action); and
the new-device notice when `actor.devices` gained a device since this computer last saw the list ("A new device
was added to your account on {date}." with **Review** opening Keys and security).

### Security-relevant state visible at rest

| State | Where a person sees it without clicking |
|---|---|
| Connection and sign-in state | The status row's dot and word; the connection bar when action is needed |
| Privacy mode (effective) and institution | The status row's chip, in every state once a connection is selected; "Checking privacy…" while unverified |
| Workspace privacy before joining | The Join dialog's summary card and the "You'll join as…" line |
| Who I act as, and where | The You row: display name and SSH login |
| Where a post goes | The channel header name, the composer placeholder `Message #methods`, the agent pane's "Posts to #methods in Analysis Lab · lab", the chat-access summary "Posts in #methods as Alice Chen (@alice)" |
| Who owns a run | "Your agent" rows; "Alice Chen's agent @alice" on agent posts |
| Which chats and tasks can post | The Agents sidebar section, the header's agent-access chip, the Access tab |
| Archived | The header badge and the archived bar |
| Trust failures | The tier-3 panes |
| Pending admission (host) | The Waiting to join section, including the different-code warning |

## Every existing screen, control and string: old to new

"Same call" means the wire call, its parameters and its ordering are unchanged.

### Entry points and routing

| Old | New | Notes |
|---|---|---|
| Route `/crew` (`App.tsx:738`), sidebar item **Crew** | Unchanged; the route element renders the new `CrewApp` | Tooltip "Collaborate with your team over SSH" → "Work with your team" |
| `/crew?sessionId=<chat>` from the `/crew` slash command | Unchanged route; the session ID drives the chat-connect note instead of a permanent banner | `ChatInput.tsx` unchanged, including the pinned "Draft kept" toast |
| Slash menu description "Open Crew workspaces, channels, files, and your agents. Press Enter" | "Open Crew. Press Enter" | `MentionPopover.tsx` |
| ProviderGuard bypass and **Open Crew →** | Unchanged | |
| Back into chat (`/sessions`, `/pair?resumeSessionId=`) | **Open chat history**, **Open**, **Review**, **Back to chat**, **Open chat** | Same navigate targets |

### Screens and states

| Old | New home | Call |
|---|---|---|
| S0 loading, no distinct UI | `loading` skeletons after 150ms | same `GET /crew/connections` |
| S1 "A shared place for your lab", **Add your first workspace**, "Private by default…" | `welcome`: **Join a workspace** + link **Host a new workspace**; the "Private by default" line is deleted (privacy is stated where it is chosen) | — |
| S1 picker on "Choose a connection" | Gone: the last used connection (per viewer) or the first is selected | — |
| S2 sub-status text | Status row dot and word | — |
| S2 **Connect to {name}** | `offline` primary **Connect to {workspace}** | same `POST …/connect` → `GET /connections` → `refresh()` |
| S2a connecting, no spinner | "Connecting…" word with spinner; setup card | same |
| S2b **Authenticate** and the inline auth panel | Sign in dialog: automatic on `crew_ssh_auth_required`, workspace menu **Sign in…**, the status word, the `sign-in` screen | same IPC; completion refreshes without POST connect (pinned) |
| S2b exit ≠ 0 alert | Unchanged inside the component (one `role="alert"`, C16) | — |
| S2b **Close authentication connection** | The component's **Close** (accessible name unchanged) | same dispose |
| S2c `CrewHostTrust` appended to any error mentioning SSH | Classified tier-3 panes; the steps behind **How do I verify it?** and **Trouble signing in?** | Try again = Reconnect |
| S2d token input, **Join workspace**, **Initialize as workspace host**, "Enrollment public key: …" | The join state machine; the token path under **Other ways to join** with the join request in a `CopyField`; Initialize becomes **Create workspace** in the Host dialog | `auth.enroll` / `auth.bootstrap` unchanged; S3a adds `…/join` |
| S3 "Choose a channel" / "Start your first team", **Create team** | `no-team` and `no-channel` variants | `team.create` / `channel.create` |
| S4 channel view | The channel view | — |
| S4 live versus history | Viewing earlier messages pill + Jump to latest | same `messages.history {before, limit:200, latest:true}` |
| S4 archived (disabled composer + footer) | Header badge + archived bar | — |
| S4 owner: header **Invite**, **Channel settings** | Channel menu Add people… and the pane's About and Members tabs | `invitation.create` etc. |
| S4 ownership offer banner + **Accept ownership** | Note above the composer, same button | same `transfer.accept` |
| S4 without `sessionId`: the permanent `/crew` hint banner + **Open Chat history** | Deleted from the channel; the instruction lives in the Access tab's empty state | — |
| S4 with `sessionId`: "Connect your current agent conversation to #X" + **Review access and posting permission** | The state-aware chat-connect note ([Revoke](#revoke)); the button's accessible name is kept | same grant POST; new list and revoke calls |
| Pending invitations at the bottom of the main pane with raw IDs | Sidebar Invitations rows with names (S1 enrichment; before S1, "Invitation from Alice Chen (@alice)", never an ID) | same `invitation.accept` |
| Errors `error` and `refreshError` (rendered twice with a modal open) | [Where errors render](#where-errors-render-exactly-once) | — |

### Observation and refresh

| Old | New | Behavior kept |
|---|---|---|
| Observer loop (CV:302-430) | Moved unchanged into `state/useCrewObservation.ts` | Frame validation, the generation guard, `verifiedScope`, the draft-clear codes, the three-fast-reconnects throw |
| `refresh()` blanks the page | Same order and clearing; presentation from `lastVerified` | C13 verbatim |
| `refresh()` closes the panel | Closes only snapshot-bound dialogs (People, Privacy, Add people, Transfer, Rename, Create team or channel, Edit profile, Share path); keeps Join, Host, Connection settings, Sign in and Keys; keeps the pane | CVT:463 still sees "start failed" |
| `mutate()` = request → `refresh()` → close panel | Unchanged for dialog mutations, except `channel.read`, which no longer refreshes (L12) | CVT:352 no refresh after a send |
| Observer `error` text | Connection bar, once | C2 |
| `refreshError` + **Retry Crew updates** | Connection bar, **Retry** (accessible name pinned) | C2 |
| History failure | Unchanged; shown in the connection bar | Draft retention (CVT:488-524) |

### Rail controls

| Old | New |
|---|---|
| `SSH workspace` `<select>` | Workspace switcher and the menu's Switch workspace radio group |
| `{ssh_target}` + status text | You row (SSH login, pinned text node); status row word |
| **Reconnect** / **Authenticate** / **Edit** (underlined) | Menu **Reconnect** / **Sign in…** / **Connection settings…**, plus the one action in the main area when the status needs it |
| `Connection privacy` `<select>` (PATCHes immediately) | Privacy chip popover **Make public…** (confirm) / **Make private** (one click); Privacy tab radio rows; Connection settings radio rows with the same confirm on save. The full-body `PATCH` is kept (L18) |
| Institution paragraph | Chip and popover rows |
| `Shared workspace policy` `<select>` (host) | Workspace settings Privacy tab: **Allow Public…** (typed confirm) / **Make Private for everyone…** (confirm) | 
| **Confirm workspace institution: {id}** | Host setup note and checklist → **Set institution to {id}…** → confirm **Set {id} permanently**; also the Privacy tab |
| "Effective: …" line | The chip is the effective mode; the popover's "why" line explains it |
| `Teams` + `+` + Team `<select>` | Team sections, **+ Add team**, menu **Create team…** |
| `Channels` + `+` + channel buttons with `<small>` counts | Channel rows (bold, neutral count badge), team `+`, **+ Add channel** |
| `People · N` list with **Remove access** per row | Workspace settings People tab; **Remove from {workspace}…** in the row `⋯` |
| **Edit my profile** | You menu **Edit profile…** |
| **Enroll a colleague** (host) | Workspace menu **Invite people to {workspace}…** |
| **Invite to team** | Team menu **Add people to {team}…** |
| `CrewCredentials` block (status, init, unlock, lock, refresh) | You menu **Keys and security…**; a locked vault also raises the connection-bar note with **Unlock**. "Refresh credential status" is removed (status loads when the dialog opens) |
| Top bar "Crew" + subtitle + dev profile + **Add workspace** | Deleted; the dev profile badge moves to the You row; Add a workspace moves to the menu |

### Channel view, composer and files

| Old | New | Call |
|---|---|---|
| Breadcrumb `{connection} / {team}` | Removed (the sidebar shows both; the agent pane and unknown-outcome text keep a destination) | — |
| `<h2># name</h2>` + Archived pill | Band title menu + badges | — |
| `{n} members · {classification} · SSH @user` | Member stack; classification badge; identity in the You row | — |
| **Mark read** | Automatic when the newest message has been visible for 1s in a focused window and `unread > 0`, at most once per 5s per channel, no refresh; plus the menu item | `channel.read` |
| **Refresh channel** icon | Channel menu **Refresh channel** (pinned) | `refresh()` |
| History row (**Older messages**, "Viewing earlier messages", **Latest messages**) | Top sentinel **Older messages**, history pill, **Jump to latest** | same |
| Empty timeline "Welcome to #name" + two sentences | Channel intro (title pinned) | — |
| Message article with full timestamp and pills | Grouped rows, short time, badges only when informative | — |
| Unknown author → raw ID | "Unknown member" or "{name} · former member", never an ID | — |
| Run cards after all messages, raw statuses, **Open agent session**, **Cancel** | Task status rows in time order, status words, **Open**/**Review**, visible **Stop** while cancellable | same navigate and cancel route |
| Visible `<label>`, `Message your teammates in #x…` | `aria-label` kept, placeholder `Message #x` | — |
| `resize: vertical` textarea | Auto-grow | — |
| **Send message** text button | Round Send, same accessible name | single flight, key reuse |
| "Message posted. Local transfer metadata could not be cleared…" | Warning note: "Message sent, but its upload record couldn't be cleared. Remove it from Files." | — |
| "Posting as @user · conn / team / channel" footer | Deleted | — |
| **Choose file to upload**, **Share remote reference**, disclaimer | Attach menu **Upload a file…** / **Share a server path…**; the 1 GiB limit appears only as an error | same picker IPC and `beginTransfer` payload |
| Chips with a text `×`; "Remote reference: {label}" | `Badge` chips with `XIcon`; `Link` icon + label; remove names kept | — |
| `CrewAttachment` **Save attachment…**, **Preview image**, `{n} bytes`, per-card 2s polling | Card glyphs, human sizes, one poller | same transfer routes |
| `TransferRows` text, **Pause**, **Select file and resume**, **Forget receipt** + four sentences, **Restore to composer** | `Progress` + state word; Pause; `⋯` **Resume…**; **Remove from list** with a `(?)` tooltip; Files tab **Attach** | same `pause`, `resume`, `DELETE`, `blob.status` |
| `CrewRemoteReference` bold label + disclaimer | Server path row + `CopyField` + tooltip | same `reference.get` |
| Native dialogs (Choose a file, Save, Replace, Remove incomplete download, vault prompts) | Unchanged (main process) | — |

### The eleven panels

| Old panel (title) | New home | Primary action |
|---|---|---|
| `connection` add ("Add SSH workspace") | Join a workspace or Host a new workspace | **Join {workspace}** / **Create workspace** |
| `connection` edit (also titled "Add SSH workspace", L1) | Connection settings | **Save connection** |
| Hosting `<details>`: Prepare/Recover, key input, `<pre>` commands | Host dialog steps 1–2 (automatic preparation; the key inside the command `CopyField`) | same `POST /crew/devices/prepare` |
| **Remove saved connection** (no confirm, L3) | Connection settings danger zone → `ConfirmationModal` | same `DELETE` |
| `team` | Create team (+ optional Add people step) | `team.create` |
| `channel` (classification not reset, L14) | Create channel (state resets on open) | `channel.create` |
| `profile` | Edit profile | `profile.update` |
| `invite` (lists existing members, L4) | Add people, filtered to non-members (and to team members for a channel) | `invitation.create` |
| `settings` "Channel ownership" | About + Members tabs; Transfer ownership dialog; Archive and Remove through confirmations | `channel.transfer`, `channel.archive`, `membership.revoke` |
| `agent` | Pane, agent mode | runs POST (pinned) |
| `reference` | Share a server path | `reference.create` |
| `grant` | Pane, chat-access mode, with revoke | grant POST; new list and revoke |
| `enroll` (UID + key → token, generic Save, L2) | Invite people (S3a by name) with the legacy form under Advanced | `enrollment.invite` (new or legacy form) |
| `offboard` (hand-rolled typed confirm, black button) | People tab row `⋯` **Remove from {workspace}…** → `DangerousConfirmDialog` with the username phrase, case-sensitive | `enrollment.revoke` |
| Hand-rolled modal shell, custom Tab loop, ✕ glyph, focus stolen by ✕ | `ModalShell`; first field focused | — |

### Capabilities the UI never had

| Capability | Decision |
|---|---|
| Revoke a chat's Crew grant, list grants | **New** ([Revoke](#revoke)) |
| Disconnect (`POST …/disconnect`) | **New**: workspace menu **Disconnect** |
| Rename team, channel, workspace (S2) | **New**: menus and About tab, when the broker advertises `unique_names_v1` |
| Join by invitation and device code (S3a) | **New**, when the broker advertises `join_by_name_v1` |
| Grant context (`GET …/sessions/{s}/context`), `auth-plan`, message search, decline or cancel invitations, leave, team delete | Out of scope; the menus have room for them |

### Latent defects

| # | Disposition |
|---|---|
| L1 | Fixed: Connection settings has its own title |
| L2 | Fixed: explicit verbs; the token appears in a success view |
| L3 | Fixed: Remove connection, Archive, Remove from channel and Create workspace all confirm |
| L4 | Fixed: pickers hide members and pending invitees; channel pickers offer only team members |
| L5 | Fixed: invitations show names |
| L6 | Fixed: `last_error` appears in the workspace menu status line and the failure surfaces |
| L7 | Fixed: only the unknown-outcome gate and the Access empty state carry Open chat history |
| L8 | Fixed: the manual workspace key has `pattern="[a-fA-F0-9]{64}"` |
| L9 | Fixed: Task has its own state, seeded from the draft |
| L10 | Fixed: a successful grant shows the Active state and Back to chat |
| L11 | Removed: `refreshChannel` calls `refresh()` directly |
| L12 | Fixed: mark-read is automatic and never refreshes |
| L13 | Fixed: one transfers poller |
| L14 | Fixed: dialog state is local to each dialog |
| L15 | Fixed: `no-channel` offers Create channel |
| L16 | Fixed: one avatar fallback rule |
| L17 | Deleted with the legacy files after cutover (`.crew-credentials`, `.crew-file`) |
| L18 | Kept deliberately: every privacy change still PATCHes the full connection record |
| L19 | Revoke, grant listing and Disconnect added; search deferred |

### Strings removed or moved behind disclosure

Deleted outright: "Your people, projects, and agents"; "Private by default. Your SSH identity determines your
permissions."; "Human conversations do not need a model."; every "Existing content keeps its restrictions."; the
connection form's intro and four-sentence disclaimer; "Membership and connection privacy still apply."; "Your verified
SSH username stays visible alongside your nickname."; "Only the current owner can archive or transfer this
channel…"; "Current channel is included…"; "policy checked by server"; the upload disclaimer; "Preparing again after
an app restart recovers the same unused identity."; "Start a conversation with your team."; "Action needs
attention".

Moved behind a disclosure, tooltip or the moment of consequence: the three-step host-key verification (How do I
verify it?, Trouble signing in?); "Local models may work across institutions…" (the popover's one-line explanation,
and the daemon's refusal text when it applies); the vault initialization note (Keys and security → Advanced); the
transfer receipts paragraph (a tooltip); "Reference only · not uploaded · existence and access not verified" (a
tooltip); the `/crew` hint banner (the Access tab's empty state); the auth disclaimer (one line).

## Component architecture and file layout

### The ownership rule that makes parallel work possible

The work lands in packages that never edit the same file. That forces one shape:

- **The controller is extracted first** (`crew/state/`), with the current markup moved unchanged into
  `crew/legacy/`. `CrewView` becomes a thin root that creates the controller, provides it through context, and
  renders an injected layout that defaults to the legacy layout. Every existing test passes unchanged at that point.
- **Area packages add new files only**, each in its own directory, each with its own tests, and **none imports
  another area**. An area reads the controller through `useCrew()` and receives cross-area content through slot
  props (for example the sidebar takes an `agentsSection` slot; the pane takes tab contents).
- **The integration package composes** the areas into `crew/layout/CrewLayout.tsx`, adds the new entry
  `crew/CrewApp.tsx` (`<CrewView layout={CrewLayout} controllerOptions={…}/>`), points the route at it, and
  migrates the regression tests.
- **Legacy files are deleted after the cutover soaks**, in a follow-up commit (they remain compiled but unreachable
  until then).

### The controller contract

`crew/state/useCrewController.ts` returns one object, provided by `CrewControllerContext` and read with `useCrew()`.
It keeps every current behavior and adds only seams that the legacy layout ignores:

```ts
export interface CrewControllerOptions {
  /** Open Sign in automatically after a user-initiated connect fails with crew_ssh_auth_required. Legacy: false. */
  autoOpenSignIn?: boolean;
  /** Keep a presentation-only copy of the last verified view during refresh. Legacy: false. */
  keepLastVerifiedView?: boolean;
}

export interface CrewController {
  // Connections
  connections: CrewConnection[];
  connectionId: string;
  connection: CrewConnection | null;          // saved merged with observed privacy
  selectConnection(id: string): void;
  saveConnection(input: SaveConnectionInput): Promise<CrewConnection>;
  updateConnection(id: string, input: SaveConnectionInput): Promise<CrewConnection>; // full-body PATCH (L18)
  removeConnection(id: string): Promise<void>;
  connect(opts?: { userInitiated?: boolean }): Promise<void>;
  disconnect(): Promise<void>;
  lastConnectFailure: { kind: ConnectFailureKind; message: string; detail?: string } | null;

  // Observation
  snapshot: Snapshot | null;                   // verified only
  lastVerified: VerifiedView | null;           // presentation only; cleared by clearProtectedState
  observedPrivacy: ObservedPrivacy | null;
  runs: ObservedRun[];
  messages: CrewMessage[];
  historyBefore: number | null;
  refreshError: string | null;
  refresh(): Promise<void>;                    // unchanged order: connections before observe
  loadOlder(): void;
  jumpToLatest(): void;

  // Selection
  teamId: string; channelId: string;
  selectTeam(id: string): void; selectChannel(id: string): void;

  // Actions, errors, pending
  act<T>(source: ErrorSource, key: ActionKey, fn: () => Promise<T>): Promise<T | undefined>;
  error: { message: string; code?: string; source: ErrorSource } | null;
  errorSlotFor(source: ErrorSource): boolean;
  dismissError(): void;
  isPending(key: ActionKey): boolean;
  busy: boolean;                               // legacy: any action pending
  request(method: string, params: object, opts?: { mutation?: boolean }): Promise<unknown>;
  mutate(method: string, params: object, opts?: { refresh?: boolean }): Promise<unknown>;
  markRead(channelId: string, sequence: number): Promise<void>;   // never refreshes (L12)

  // Composer (single flight and idempotency unchanged)
  draft: { body: string; attachments: DraftFile[]; references: DraftReference[] };
  setBody(body: string): void;
  addAttachment(f: DraftFile): void; removeAttachment(id: string): void;
  addReference(r: DraftReference): void; removeReference(id: string): void;
  send(): Promise<void>;
  clearBodyIfEquals(seed: string): void;

  // Owned runs (module-scoped unknown-outcome lock, C10)
  startOwnedRun(input: { prompt: string; provider: string; model: string;
    contextChannels: string[]; deliberateRestart?: boolean }): Promise<boolean>;
  unknownRunDestination: string | null;
  inspectedPriorRun: boolean; setInspectedPriorRun(v: boolean): void;
  cancelRun(runId: string): Promise<void>;

  // Chat grants (from ?sessionId)
  grantSessionId: string | null;
  grantSession(input: { contextChannels: string[] }): Promise<void>;

  // Sign in
  signIn: { open: boolean; reason: 'user' | 'auto' | null };
  openSignIn(): void; closeSignIn(): void; onSignedIn(): void;   // no POST connect

  // UI intents, so areas open each other's surfaces without importing each other
  ui: { dialog: DialogIntent | null; pane: PaneIntent | null };
  openDialog(intent: DialogIntent): void; closeDialog(): void;
  openPane(intent: PaneIntent): void; closePane(): void;

  // Derived (pure functions in crewStatus.ts, exported for tests)
  status: ConnectionStatusKey;
  screen: CrewScreen;
  effectivePrivacy: 'private' | 'public' | null;
  isHost: boolean;                             // host_principal_id (S1), else actor.uid === host_uid
}
```

`DialogIntent` and `PaneIntent` are discriminated unions listing every dialog of the [Dialog
inventory](#dialog-inventory) and every pane mode. `ConnectFailureKind` mirrors the SSH failure codes plus
`unknown`. `classifyConnectFailure()` (pure) implements the code mapping and the fallback for older daemons.

### File layout

```text
ui/desktop/src/components/
  ui/
    copy-field.tsx     status-dot.tsx     disclosure.tsx     avatar.tsx      (+ tests)
  icons/app-icons.tsx                    + Hash UserPlus Paperclip KeyRound Server LogOut PanelRight Fingerprint
  Layout/AppLayout.tsx                   isChatRoute also matches /crew
  crew/
    CrewView.tsx          thin root: controller + context + injected layout (default: legacy)
    CrewApp.tsx           new entry: <CrewView layout={CrewLayout} controllerOptions=…/>
    crew-app.css          new stylesheet for the new layout (tokens only; the old crew.css stays with legacy)
    crewApi.ts            wire helpers (types enriched for S1/S3)
    api/                  grants.ts names.ts join.ts errors.ts — each imports crewHttp/crewRequest from ../crewApi
    state/                useCrewController.ts CrewControllerContext.tsx useCrewConnections.ts
                          useCrewObservation.ts crewActions.ts crewSend.ts crewRunStart.ts
                          observationFailure.ts connectFailure.ts crewStatus.ts copy.ts
    legacy/               the current markup, moved unchanged (deleted after cutover)
    identity/             PersonName.tsx personLabel.ts nameKey.ts usePeopleDirectory.ts institution.ts
    sidebar/              CrewSidebar WorkspaceSwitcher WorkspaceMenu StatusRow PrivacyChip PrivacyPopover
                          AttentionSections TeamSection ChannelRow YouRow YouMenu copy.ts
    channel/              ChannelHeader ChannelMenu AgentAccessChip MemberStack ConnectionBar copy.ts
    pane/                 DetailsPane AboutTab MembersTab AgentTaskPane CrewModelPicker copy.ts
    timeline/             Timeline groupMessages.ts MessageGroup MessageRow DayDivider NewDivider
                          TaskStatusRow ChannelIntro JumpPill HistorySentinel copy.ts
    composer/             Composer ComposerChips AttachMenu copy.ts
    files/                useCrewUpload useCrewTransfers AttachmentCard ServerPathRow UploadChip FilesTab copy.ts
    onboarding/           Welcome JoinDialog JoinStatusCard HostDialog SetupChecklist TrustPane
                          NotSetUpPane EmptyStates copy.ts
    auth/                 SignInDialog.tsx
    CrewAuthentication.tsx   restyled body (default export and props unchanged)
    CrewHostTrust.tsx        becomes the "Trouble signing in?" body (default export kept)
    dialogs/              ConnectionSettingsDialog WorkspaceSettingsDialog InvitePeopleDialog LetInDialog
                          CreateTeamDialog CreateChannelDialog AddPeopleDialog TransferOwnershipDialog
                          RenameDialog EditProfileDialog KeysDialog SharePathDialog PersonPicker
                          DeviceCodeInput confirmations.tsx copy.ts
    access/               useCrewGrants ChatConnectNote ChatAccessPane AccessTab AccessList
                          AgentsSection WorkspaceAgentAccess copy.ts
    layout/               CrewLayout.tsx (composition, grid, pane push/cover)
    integration/          cross-area tests (no machine IDs, exactly-once errors, pane, privacy, verifying)
    test/                 crewTestUtils.ts
```

**Why the API helpers live in `crew/api/`.** CVT mocks `./crewApi` by spreading the actual module and replacing
`crewHttp`, `crewRequest` and `observeCrew`. A helper defined inside `crewApi.ts` would call the module-internal,
unmocked `crewHttp`. Helpers in `crew/api/*.ts` import `crewHttp` from `../crewApi`, so the mock intercepts them and
no un-mocked request reaches `src/test/networkGuard.ts`.

**Copy.** Each area keeps its strings in its own `copy.ts`; pinned strings shared across areas (status words,
observation-error suffixes) live in `state/copy.ts`. Tests import pinned strings instead of retyping them.

### Shared primitives

**`CopyField`** (`ui/copy-field.tsx`): the one box for everything a person hands to someone else.

```ts
export interface CopyFieldProps {
  value: string;                         // exactly what is copied, never the display form
  label: string;                         // Copy button accessible name "Copy {label}"
  display?: string;                      // e.g. '7QK2-M9XA-3JTP-WZ4D', '3F2A 9C1E 77B0 D4E1'
  multiline?: boolean;                   // wraps, keeps newlines; Copy sits top-right
  truncate?: 'end' | 'middle';           // single-line overflow; the full value is still copied
  secret?: boolean;                      // masked with a reveal toggle; Copy works unrevealed
  size?: 'default' | 'code';             // 'code' = 20/28 mono for device codes
  onCopied?: () => void;
  className?: string;                    // layout only
}
```

Authored `.biorouter-copy-field` in `main.css`: `rounded-element border border-border-subtle`, ground
`var(--background-well)` for both the single-line and multi-line forms (`--background-code` equals the page in
dark mode, so it vanishes inside dialogs), value `font-mono text-code select-all`, trailing ghost `sm` Copy that
swaps to `Check` + "Copied" for 2s with `.biorouter-check-settled`, an `aria-live="polite"` region announcing
"Copied", no toast. On clipboard failure the button reads "Copy failed" for 2s and the value is selected so ⌘C
works. `onboarding/codingAgentControls.tsx`'s off-spec `CommandBlock` can migrate onto it in a later change; this
campaign does not touch that surface.

**`StatusDot`** (`ui/status-dot.tsx`): design.md §4.16's 8px dot. Tones `success | warning | danger | neutral | idle`
(fills `bg-background-success`/`-warning`/`-danger`, neutral `--text-subtle`, idle `--background-strong`). `live`
adds the §4.16 2px halo on a 2s period, static under reduced motion. `label` gives `role="img"` with a name when the
dot stands alone; otherwise it is `aria-hidden`. It is a new file; `Dot.tsx` and its callers are untouched.

**`Disclosure`** (`ui/disclosure.tsx`): the one Advanced (and "How do I verify it?", "Trouble signing in?")
control. Props `label` (default "Advanced"), `summary` (muted closed-state line such as "Port 22 · your SSH
settings"), `open`/`defaultOpen`/`onOpenChange`. Trigger: `Button variant="ghost" size="sm"` in `text-text-muted`
with a `ChevronRight` that rotates 90°. Content: Radix `Collapsible`, unmounted when closed, animated by authored
`.biorouter-disclosure-panel` keyframes in `main.css`. One level only. `defaultOpen` when a saved record already
uses a field inside.

**`Avatar`** (`ui/avatar.tsx`): on the existing `@radix-ui/react-avatar` dependency. Props `fallback`,
`size` (20 | 24 | 32), `shape` (`circle` for people, `square` for agents and objects), `icon`, `ring`. Ground
`bg-background-medium`, ink `text-text-muted`. Initials: first letters of up to two words of the display name,
else the first two letters of the username.

### Crew-local components

| Component | API sketch | Notes |
|---|---|---|
| `PersonName` + `personLabel()` | `<PersonName person context="header" \| "inline" \| "authority" \| "chip" you? />`, `personLabel(p, context, dir): string` | Implements [Identity and naming display rules](#identity-and-naming-display-rules); never returns an ID |
| `usePeopleDirectory` | `byId`, `collides`, `isFormer` | Uses daemon-projected labels when present; falls back to a local `nameKey` |
| `WorkspaceSwitcher` / `WorkspaceMenu` | reads `useCrew()` | `DropdownMenu` + `DropdownMenuRadioGroup` |
| `StatusRow`, `PrivacyChip`, `PrivacyPopover` | reads `useCrew()` | Opens privacy confirmations via `openDialog` |
| `ConnectionBar` | `<ConnectionBar />` | Renders the ordered notes |
| `DetailsPane` | `<DetailsPane mode tabs={{about, members, files, access}} />` | Slots from other areas |
| `CrewModelPicker` | `<CrewModelPicker provider model onChange />` | `Popover` + `Command`, field look |
| `PersonPicker` | `<PersonPicker candidates value onChange label />` | Rows show `@username` before selection |
| `DeviceCodeInput` | `<DeviceCodeInput value onChange />` | 16 Crockford characters, grouped display, paste-friendly |
| `JoinStatusCard` | reads `api/join.ts` | Polls while visible |
| `useCrewGrants` | `list(connectionIds)`, `revoke(connectionId, sessionId)` | Refetch on open and after every action; no polling |

## Progressive disclosure, form by form

Visible means required, or needed to understand the consequence. Every hidden field has a stated default shown in
the Advanced summary. **Req** means native `required`, a `type="submit"` button and no `noValidate` (C8).

| Form | Visible by default | Behind Advanced (default) |
|---|---|---|
| **Join a workspace** | Invitation from your host (req) · then the summary card, Your username on {server} (req, prefilled) and the privacy line with Change | Server login override (an SSH alias from your SSH config instead of `{username}@{server}`) · Port (22 or the invitation's) · Identity file (none: your SSH config) · Jump host (the invitation's hint, else none) · Connection name (the workspace name; `name — server` when two collide) · Remote work folder (none) · Let my agent run commands in this folder (off, a `Switch`, disabled until a folder is set) · Enter workspace details manually (socket path, workspace ID, host user ID, workspace key) |
| Join, Change privacy | Private / Public radio rows (req) · Institution (req iff Private, `pattern="[a-z0-9][a-z0-9_-]{0,63}"`, placeholder "For example, ucsf or sdsc" (pinned)) | — |
| Join, legacy token state | The join request `CopyField` · Invitation token (req, `SecretInput`) | — |
| **Host a workspace**, Name | Workspace name (req) · Your server login (req) · Privacy (Private) · Institution (req iff Private) | Port · Identity file · Jump hosts · Connection name · Remote work folder + agent execution `Switch` (off) |
| Host, Start | The start command `CopyField` · Paste what it printed (req) | the two help disclosures |
| Host, Create | Summary + **Create workspace** | — |
| **Connection settings** | Connection name (req) · Your server login (req) · Privacy radios · Institution (req iff Private, placeholder pinned) | Port · Identity file · Jump hosts · Remote work folder + agent execution `Switch` · Workspace details (read-only `CopyField`s: workspace ID, fingerprint, socket path, host user ID, device ID, cluster ID; editable only for a manually entered connection). Opens by default when any value is non-default. Danger zone: Remove {workspace} from this computer… |
| **Invite people** (host, S3a) | Username `@` (req) | Invite someone using an older version of Biorouter (join request + user ID → token) · Add another device for an existing member (a `Switch`, shown only when the broker answers "already a member") |
| **Let someone in** | Code from {person} (req) | — |
| **Create team** | Name (req; S2 helper "Team names are unique in {workspace}.") | — |
| **Create channel** | Name (req, `#` prefix, live slug preview "Will be created as #{slug}") · Content: Restricted / Public-safe radio rows (visible: it cannot be changed later; default Restricted) | — |
| **Add people** | Person (req) | — |
| **Transfer ownership** | New owner (req); helper "They'll need to accept." | — |
| **Rename** (S2) | Name (req) | — |
| **Edit profile** | Display name (req) · Initials (optional, max 12) · a read-only "Your username: @bob" | — |
| **Ask my agent** | Task (req, seeded) · Model (summary with Change, or the picker; req) | Also read other channels (none) · the remote folder summary (only when configured) |
| **Chat access** (grant) | The consent summary only | Also read other channels (none) |
| **Share a server path** | Path on {host} (req, `pattern="/.*"`) | Label (the file name) |
| **Remove from workspace** | Type {username} to confirm (req, exact, case-sensitive) | — |
| **Keys and security** | Storage status line; Unlock/Lock for a vault; This device's key (`CopyField`) | Use an encrypted vault instead (fresh profiles only) |

Every submit with an invalid field inside a closed Advanced opens it and focuses that field.

## Copy deck

American English, sentence case, second person, typographic apostrophes. Buttons are verb plus object; items that
open a dialog end in "…". No "please", no "successfully", no internal words ("broker", "daemon", "principal",
"enrollment", "observation", "epoch") on a default path: they survive only inside daemon-authored text and under
Advanced. `{person}` means `personLabel` output; `{first}` is the display name's first word, else `@username`.

### Sidebar and menus

| Key | String |
|---|---|
| `nav.label` | Crew *(nav accessible name)* |
| `switcher.name` | {workspace} *(S2 name, else the connection name)* |
| `status.connected` | Connected *(visible; `sr-only` **Connected · identity verified**, pinned)* |
| `status.checking` | **Checking connection** (pinned) |
| `status.updatesUnavailable` | **Updates unavailable** (pinned) |
| `status.connecting` / `status.signIn` / `status.notSetUp` / `status.notJoined` / `status.offline` / `status.error` / `status.trust` | Connecting… · Sign-in needed · Not set up on this server · Not joined yet · Offline · Can't connect · Can't verify server |
| `chip.name` | Privacy: Private · {institution} / Privacy: Public *(accessible name; new test anchor)* |
| `chip.checking` | Checking privacy… |
| `section.invitations` / `section.waiting` / `section.agents` | Invitations · Waiting to join · Agents *(authored in sentence case; `text-caps` uppercases them)* |
| `invitation.row` | {target} · from {inviter first} |
| `invitation.accept` | Accept *(accessible name: Accept invitation to {target})* |
| `waiting.row` | @{username} · {name on the server account} |
| `waiting.letIn` | Let in… *(accessible name: Let @{username} in)* |
| `waiting.otherDevice` | A device with a different code tried to join as @{username}. |
| `team.toggle` | {team} *(accessible name: {team}, {n} channels)* |
| `team.options` / `team.addChannel` | {team} options · Create channel in {team} |
| `channel.row` | {name} *(accessible name: {name}, {n} unread)* |
| `channel.add` / `team.add` / `channel.archivedGroup` | Add channel · Add team · Archived ({n}) |
| `agents.task` | #{channel} · {status word} |
| `agents.chat` | {chat title} · #{channel} *(fallback title: Untitled chat)* |
| `agents.showAll` | Show revoked and finished |
| `you.login` | {ssh_target} (pinned text node) |
| `you.devProfile` | Profile: {name} |
| `wm.hostedBy` / `wm.signedInAs` | Hosted by {person} · Signed in as {ssh_target} |
| `wm.invite` / `wm.people` / `wm.privacy` / `wm.access` / `wm.createTeam` | Invite people to {workspace}… · People… · Privacy… · Chats with access… · Create team… |
| `wm.reconnect` / `wm.signIn` / `wm.disconnect` / `wm.settings` | **Reconnect** (pinned) · Sign in… · Disconnect · Connection settings… |
| `wm.switch` / `wm.add` / `wm.add.join` / `wm.add.host` | Switch workspace · Add a workspace · Join a workspace… · Host a new workspace… |
| `you.editProfile` / `you.keys` / `you.copyUsername` | Edit profile… · Keys and security… · Copy my username |
| `tm.createChannel` / `tm.addPeople` / `tm.rename` / `tm.copyId` | Create channel… · Add people to {team}… · Rename team… · Copy team ID |

### Privacy popover

| Key | String |
|---|---|
| `pop.private` | Only private and {institution}-approved models can read {workspace}. *(without an institution: Only private models can read {workspace}.)* |
| `pop.public` | Public models can read public-safe channels in {workspace}. Restricted channels stay private. |
| `pop.rows` | Your connection · Workspace · Institution |
| `pop.values` | Private · Public · Private for everyone · Allows Public · Not set |
| `pop.why.workspace` | Private because the workspace is Private for everyone. |
| `pop.why.connection` | Private because your connection is Private. |
| `pop.why.both` | Private because your connection and the workspace are both Private. |
| `pop.why.public` | Public because your connection is Public and the workspace allows it. |
| `pop.makePublic` / `pop.makePrivate` / `pop.more` | Make public… · Make private · Privacy… |
| `pop.hostOnly` | Only the host can change the workspace setting. |

### Channel header, menu and pane

| Key | String |
|---|---|
| `header.channel` | # {name} *(accessible name: {name} channel menu)* |
| `header.restricted` / `header.publicSafe` / `header.archived` | Restricted · Public-safe · Archived |
| `header.accessChip` | {n} chats / {n} tasks / {n} agents *(accessible name: {n} chats or agents can post here)* |
| `header.members` | {n} members *(accessible name)* |
| `header.details` | Channel details |
| `cm.*` | Channel details · Members · Files · Chats and agents with access · Add people… · Mark as read · **Refresh channel** (pinned) · Copy channel name · Copy channel ID · Rename… · Transfer ownership… · Archive channel… |
| `pane.close` / `pane.back` | Close details · Back to #{name} |
| `tabs` | About · Members · Files · Access |
| `about.*` | Name · Content · Owner · Created by · Team · Restricted · Public models can't read it. · Public-safe · Public models may read it when the workspace allows. · Offered to {person} · waiting · Rename… · Transfer ownership… · Danger zone · Archive channel… · Copy channel ID |
| `members.*` | Add people… · {n} members · Owner · you · invited · former member · Make owner… · Remove from #{name}… |
| `files.*` | In progress · Uploaded, not sent · In this channel · Attach · No files shared yet. |

### Timeline

| Key | String |
|---|---|
| `log.label` | {name} messages |
| `log.older` / `log.loadingOlder` | **Older messages** (pinned) · Loading earlier messages… |
| `intro.title` / `intro.createdBy` / `intro.addPeople` | **Welcome to #{name}** (pinned) · {person} created this channel. · Add people |
| `day.*` | Today · Yesterday · {Weekday, Month D} · {Month D, YYYY} |
| `log.new` | New |
| `msg.agentAuthor` / `msg.yourAgent` / `msg.agentBadge` | {display name}'s agent · Your agent · Agent |
| `msg.restricted` | Restricted *(tooltip: Only private models can read this message.)* |
| `msg.copyText` / `msg.copyId` / `msg.more` | Copy text · Copy message ID · More actions |
| `msg.unknown` / `msg.former` | Unknown member · former member |
| `history.viewing` / `history.jump` | **Viewing earlier messages** (pinned) · Jump to latest |
| `task.status.*` | Starting… · Working… · Waiting for your approval · Stopping… · Stop not confirmed · Interrupted · Outcome unknown · Done · Couldn't finish · Stopped |
| `task.*` | Open *(accessible name: Open agent conversation)* · Review · Stop *(accessible name: Stop task)* · Try stopping again · Copy task ID · Open chat history · Copy error |
| `task.stopConfirm` | Stop your agent? · It stops working on this task. Anything it already did stays done. · Keep running · Stop task |

### Composer and files

| Key | String |
|---|---|
| `composer.label` / `composer.placeholder` | **Message #{name}** (pinned `aria-label`) · Message #{name} |
| `composer.attach` / `composer.upload` / `composer.sharePath` | Attach · Upload a file… · Share a server path… |
| `composer.askAgent` / `composer.send` | **Ask my agent** (pinned) · **Send message** (pinned `aria-label`) |
| `composer.removeFile` / `composer.removeRef` | **Remove {name}** · **Remove remote reference {label}** (pinned) |
| `composer.archived` / `composer.verifying` | This channel is archived. · Verifying access… |
| `composer.sendError` | Couldn't send. {error} |
| `composer.postedMetadata` | Message sent, but its upload record couldn't be cleared. Remove it from Files. |
| `file.*` | **Save attachment** (tooltip Save {name}) · **Preview image** · Attachment · Copy file ID · Copy SHA-256 |
| `file.tooLarge` | {name} is larger than 1 GB. Crew can share files up to 1 GB. |
| `transfer.state.*` | Starting… · Uploading {p}% · Downloading {p}% · Finishing… · Pausing… · Paused · Ready · Saved · Failed · Not confirmed |
| `transfer.*` | Pause · Resume… · Remove from list *(tooltip: Removes the record on this computer. Shared files and saved downloads stay.)* |
| `ref.notUploaded` | Not uploaded *(tooltip: Crew shares the path only. It doesn't check that the file exists or grant access to it.)* |
| `upload.privacyPending` | **Refresh the workspace to verify connection privacy before uploading.** (pinned) |

### Ask my agent

| Key | String |
|---|---|
| `agent.title` | **Ask my agent** (pinned) |
| `agent.destination` | Posts to #{channel} in {team} · {workspace} |
| `agent.task` / `agent.taskPlaceholder` | **Task** (pinned) · What should your agent do? |
| `agent.model` / `agent.modelEmpty` / `agent.modelChange` / `agent.modelUse` | **Model** (pinned) · Choose a model · Change · Use “{text}” with {provider} |
| `agent.noModels` / `agent.openSettings` | No models are set up. · Open Settings |
| `agent.advancedSummary` | Also reads nothing else / Also reads {n} channels |
| `agent.alsoRead` / `agent.folderExec` / `agent.folderRead` | Also read · Can run commands in {path} · Can read {path} |
| `agent.publicHint` | This model is Public, so it can't read Restricted channels. |
| `agent.scope` | Your agent can read #{channel} and post there, for this task only. |
| `agent.start` | **Start my agent and allow posting here** (pinned) |
| `unknown.title` | **Inspect the previous task before starting again** (pinned) |
| `unknown.body` | The request to #{channel} in {team} was received, but Crew can't tell whether it started. Repeating it could duplicate its effects. |
| `unknown.*` | Show task in channel · Open chat history · I checked the previous task and its effects. · **Start a new task** (pinned) |

### Chat access and revoke

| Key | String |
|---|---|
| `note.checking` | Checking this chat's access… |
| `note.none` | Connect this chat to #{channel}? |
| `note.review` | Review access *(accessible name: **Review access and posting permission**, pinned)* |
| `note.active` / `note.activeElsewhere` | This chat can read and post in #{channel}. · This chat already uses #{other}. |
| `note.manage` | Manage access |
| `note.revoked` / `note.expired` / `note.grantAgain` | This chat's Crew access was revoked. · This chat's access expired. · Grant again |
| `access.paneTitle` | Chat access |
| `access.willBeAble` / `access.read` / `access.post` / `access.expiry` | “{chat}” will be able to · Read #{channel} · Post in #{channel} as {person, authority} · Access ends when you revoke it, or after an hour. |
| `access.allow` | **Allow this conversation to read and post here** (pinned) |
| `access.connected` / `access.backToChat` / `access.openChat` | Connected. · Back to chat · Open chat |
| `access.revokeButton` | Revoke access |
| `access.confirm` / `access.confirmRevoke` / `access.confirmKeep` | Stop “{chat}” reading and posting in #{channel}? · Revoke · Keep access |
| `access.revoked` | Access revoked. “{chat}” can't use Crew until you grant access again, or start a new chat. |
| `access.unconfirmed` | Stopped on this device. Reconnect to confirm with the workspace. |
| `access.notRevoked` | Not revoked. This chat can still read and post. {daemon message} |
| `access.retry` / `access.done` | Retry · Done |
| `access.tabTitle` / `access.status.*` | Chats and agents with access · Active · Expires {time} · Expired · Revoked · Stopped on this device |
| `access.showOld` | Show revoked and expired ({n}) |
| `access.empty` / `access.emptyHow` | No chats or agents can post in #{channel}. · To connect a chat, type /crew in it. |
| `access.untitled` | Untitled chat |

### Onboarding, join, host and admit

| Key | String |
|---|---|
| `welcome.title` / `welcome.body` | Work together in Crew · Chat, share files and run agents with your lab. |
| `welcome.join` / `welcome.host` | Join a workspace · Host a new workspace |
| `join.title` / `join.invitation` / `join.invitationPlaceholder` | Join a workspace · Invitation from your host · Paste the whole message your host sent you |
| `join.parsed.hostedBy` / `join.fingerprint` | Hosted by {person} on {server} · Fingerprint |
| `join.invalid` | This doesn't look like a Crew invitation. Ask your host to copy it again. |
| `join.username` | Your username on {server} |
| `join.privacyLine` / `join.change` | You'll join as {badge}. · Change |
| `join.mismatch` | {workspace} is Private for ucsf. Your connection will be {choice}. |
| `join.submit` / `join.connecting` | Join {workspace} · Connecting to {server}… |
| `join.invited` / `join.sendCode` / `join.waiting` | {person} invited you to {workspace}. · Send {first} this code: · Waiting for {first} to let you in… |
| `join.approved` | Joining {workspace}… |
| `join.mismatchCode` | The code {first} entered doesn't match this computer. Send it again: |
| `join.notInvited.title` / `.body` / `.message` | You're not in {workspace} yet · Ask {person} to invite @{username}. This page updates by itself. · Hi {first}, please invite @{username} to {workspace} in Crew. |
| `join.expired` | This invitation expired. Ask {person} to invite you again. |
| `join.other` | Other ways to join |
| `join.legacy.*` | Join with an invitation token · Send this join request to your host: · **Enrollment invitation** (accessible name) · Invitation token · Join workspace |
| `join.nameSuggestion` | Use “{full name}” as your name in {workspace}? · Use · Edit… |
| `noTeam.member` | You're in {workspace} · Ask {host person} to add you to a team. · Create a team |
| `noTeam.invited` | You're invited to {team} · {person} invited you. · Join {team} |
| `noChannel` | No open channels in {team} · Create channel |
| `host.title` / `host.steps` | Host a new workspace · Name · Start · Create |
| `host.name.*` | Name your workspace · Workspace name · Lowercase letters, numbers and dashes. · Your workspace: {slug} · Your server login · Continue |
| `host.start.*` | Start Crew on {server} · Run this in a terminal signed in to {server} as {user}: · Paste what it printed · Not signed in to the server in a terminal yet? · If your terminal asks you to confirm the server, compare the fingerprint with the one from your IT team. Type yes only if they match. · biorouter-crew isn't installed yet? · Anyone who can sign in to this server can see the workspace name. |
| `host.start.bad` | That isn't what Crew prints. Copy everything after the command ran and paste again. |
| `host.create.*` | {workspace} on {server} · Creating {workspace} makes this computer its first admin device. · Create workspace · Creating… |
| `host.label.*` | Label {workspace} as {id}? · This can't be changed later. · Set {id} permanently · Not now |
| `checklist.*` | Get {workspace} ready · Confirm the institution · Create a team · Invite people · Hide |
| `invite.*` | Invite people to {workspace} · Username · bob · Invite · @{username} · {full name} (name on the server account) · invited · Send {first} this invitation: · Is Crew installed for @{username} on {server}? · When {first} sends you a code, choose Let in… next to their name in the sidebar. · Done |
| `invite.message` | Join {workspace} on Crew.↵In Biorouter, open Crew, choose Join a workspace, and paste this whole message.↵brcrew1:… |
| `invite.refusal.*` | No account named @{text} on this server. · @{username} is already in {workspace}. · Invite @{canonical} instead: that's the account's exact name. |
| `invite.legacy.*` | Invite someone using an older version of Biorouter · Their join request · Their user ID on {server} · Run on the server: id -u {username} · Create invitation · Send this to {first}. It works once and expires in an hour. |
| `letIn.*` | Let @{username} into {workspace} · Code from {first} · Paste the code {first} sends you directly. · Let {first} in · That code doesn't match. Check it with {first}. · Approved. {first} joins as soon as their Crew checks in. · Add {first} to {team} |
| `notSetUp.*` | Crew isn't set up for your account on {server} · It's installed once per account, usually by your host or IT team. · Hi {host first}, Crew isn't set up for my account (@{username}) on {server} yet. Could you or IT install biorouter-crew in ~/.local/bin for me? · Install it yourself · Try again |

### Connection problems and sign in

| Key | String |
|---|---|
| `signIn.title` / `signIn.lead` | Sign in to {host} · Type your password or verification code in the box below. Nothing you type is saved. |
| `signIn.close` | Close *(accessible name: **Close authentication connection**, pinned)* |
| `signIn.ended` | **SSH authentication ended (exit {code})** (pinned fragment). Choose Reconnect to check the connection. |
| `signIn.inputLost` / `signIn.needsDesktop` | Your input couldn't reach the server. Close this sign-in and try again. · Signing in needs the Biorouter desktop app. |
| `signIn.help` | Trouble signing in? · Use the same username and password you use for this server. · If your IT team gave you a jump host, add it in Connection settings. · Crew checks servers against this file: |
| `trust.unknown.*` | Can't verify {host} yet · Crew only connects to servers you've already verified. · Fingerprint the server offered · How do I verify it? · 1. Get {host}'s fingerprint from your IT team or your institution's directory. Check jump hosts too. 2. Compare it using your usual SSH setup, then add the full key to your known-hosts file. A fingerprint alone isn't enough. 3. Come back and choose Try again. · Try again |
| `trust.changed.*` | {host}'s identity changed · Don't connect until your IT team confirms this change. Crew won't connect while the old key is in your known-hosts file. · Copy details for IT |
| `trust.workspace.*` | This isn't the workspace you joined · The server answered with a different workspace key. Don't continue until {host person} confirms what changed. · Copy details · Connection settings… |
| `bar.retry` | Retry *(accessible name: **Retry Crew updates**, pinned)* |
| `bar.unreachable` | Can't reach {host}. · Try again |
| `bar.vaultLocked` | Your Crew vault is locked. · Unlock |
| `bar.reconnecting` | Reconnecting to {workspace}… |
| `bar.newDevice` | A new device was added to your account on {date}. · Review |
| `offline.*` | {workspace} is offline · Connect to see your channels. · Connect to {workspace} |
| `signInNeeded.*` | Sign in to {host} · The server needs your password or a verification code. · Sign in |

### Dialogs and confirmations

| Dialog | Strings |
|---|---|
| Connection settings | Connection settings · Connection name · Your server login · Privacy · Private — Only private and institution-approved models · Public — Public models allowed for public-safe work · Institution · **For example, ucsf or sdsc** (pinned) · Your organization's short ID, as your host uses it. *(helper, only when empty)* · Workspace details · **Save connection** (pinned) · Remove {workspace} from this computer… |
| Workspace settings | {workspace} settings · General · People · Privacy · Agent access · Hosted by · Server · Rename… (S2 host) · Waiting to join · Members · Invite people… · Host · you · Copy username · Copy person ID · Remove from {workspace}… · Your connection · Workspace · Private for everyone · Allow Public… · Make Private for everyone… · Institution · Set institution to {id}… · Only the host can change this. · Done |
| Create team | Create team · Name · e.g. Analysis Lab · Team names are unique in {workspace}. · Create team · Add people to {team} · Skip for now · Add |
| Create channel | Create channel · in {team} · Name · e.g. methods · Will be created as #{slug} · Content · Restricted — For unpublished or sensitive work · Public-safe — Public models may read it · Create channel |
| Name refusal (S2) | A team with this name, or one that looks like it, already exists in this workspace. Choose a different name. *(channels: …in this team…)* |
| Name consequence (S2) | Everyone in this team can tell whether a name is taken. Keep identifiers out of names. |
| Add people | Add people to #{channel} / {team} · Person · Search by name or @username · Everyone in {team} is already here. · No one else has joined {workspace} yet. · Add |
| Transfer ownership | Transfer ownership of #{channel} · New owner · They'll need to accept. · Offer ownership |
| Rename | Rename team / Rename channel · Name · Rename |
| Edit profile | Edit profile · Display name · Initials (optional) · Your username: @{username} · Save profile |
| Keys and security | Keys and security · Stored in your system keychain. / Stored in an encrypted vault · Locked / … · Unlocked · Unlock · Lock · This device's key · Use an encrypted vault instead · Only for a new Crew profile. Existing identities aren't moved. · Set up vault… · Done |
| Share a server path | Share a path on {host} · Path · /home/you/project/data.h5ad · Label (optional) · Add to message |

| Action | Primitive | Title | Description | Confirm |
|---|---|---|---|---|
| Connection Private → Public (popover, Privacy tab, or saving Connection settings) | `DangerousConfirmDialog`, phrase = workspace name | Make your {workspace} connection public? | Public models will be able to read public-safe work you can see here. Restricted content stays private. Your unsent draft will be cleared. | Make public *(field: Type {workspace} to confirm)* |
| Workspace → Allow Public (host) | `DangerousConfirmDialog`, phrase = workspace name | Allow Public in {workspace}? | Members will be able to choose Public. Agents with access will need permission again. | Allow Public |
| Workspace → Private for everyone (host) | `ConfirmationModal` | Make {workspace} Private for everyone? | Agents with access will need permission again. | Make Private for everyone |
| Institution label (host, irreversible) | `DangerousConfirmDialog`, no phrase (no key confirms) | Set {workspace}'s institution to {id}? | This can't be changed later. Private data can then be used only with models approved for {id}. | Set {id} permanently |
| Remove from workspace (host) | `DangerousConfirmDialog`, phrase = username, plus a case-sensitive check | Remove {person, authority} from {workspace}? | Removes all of their devices and agent access. Their messages stay in history. | Remove from {workspace} |
| Archive channel (owner) | `ConfirmationModal` | Archive #{name} for everyone? | Nobody can post in it after this. Its history stays readable. | Archive channel |
| Remove channel member (owner) | `ConfirmationModal` | Remove {person, authority} from #{name}? | They'll lose access to its messages and files. You can invite them again. | Remove |
| Remove saved connection | `ConfirmationModal` | Remove {workspace} from this computer? | Chats connected to it lose access. Your messages stay on the server, and you can add it again. | Remove |
| Stop task | `ConfirmationModal` | Stop your agent? | It stops working on this task. Anything it already did stays done. | Stop task |
| Cancel a pending invitation (host) | inline two-step | Cancel @{username}'s invitation? | — | Cancel invitation / Keep |
| Revoke chat access | inline two-step | [Chat access and revoke](#chat-access-and-revoke) | — | Revoke / Keep access |

### Error strings

Daemon-authored messages are shown verbatim in their own text node; a sentence is added only where the current copy
already added one.

| Source | New string |
|---|---|
| Observation failure, draft kept | {message} Your **unsent draft is retained** (pinned fragment). Retry to check access before sending. |
| Observation failure, draft cleared | {message} Access or privacy changed, so we cleared your unsent draft and attachments. Retry to check access. |
| Scope change on reconnect | Workspace **privacy or selected channel access changed** (pinned fragment) while reconnecting. We cleared your unsent draft and attachments. |
| Channel access lost | Your access to #{name} changed, so its messages and your draft were cleared. Choose another channel. |
| Connections failed to load | Saved Crew connections couldn't be loaded. |
| Action fallback | Crew couldn't complete that action. |
| Wrong connection | Biorouter returned a different Crew connection. Retry to check the workspace. |
| Repeated reconnects / stopped | Crew updates keep stopping. Retry after checking Biorouter. · Crew updates stopped. |
| History failure | Earlier messages couldn't be loaded. |
| Privacy not yet verified | Crew is still checking this workspace's privacy. Try again in a moment. *(send variant: …Your message wasn't sent.)* |
| Unknown-outcome gates | Check the previous task first, then confirm below to start another. · Confirm that you checked the previous task. |
| Offboard mismatch | Type the exact username to remove access. |
| HTTP fallback | Crew request failed ({status}). |
| Broker JSON envelope | The workspace refused that request. *(raw text under Copy details)* |
| Revoke 503 / other | Stopped on this device. Reconnect to confirm with the workspace. · Not revoked. This chat can still read and post. {message} |
| Grants list failed | Couldn't load which chats have access. · Retry |
| Stale shared daemon (404 on a new route) | This feature needs a newer Biorouter background service. Quit and reopen Biorouter. |

### Outside `components/crew/`

| Where | Old | New |
|---|---|---|
| `AppSidebar.tsx` tooltip | Collaborate with your team over SSH | Work with your team |
| `MentionPopover.tsx` `/crew` | Open Crew workspaces, channels, files, and your agents. Press Enter | Open Crew. Press Enter |
| `ProviderGuard.tsx` | Open Crew → | unchanged |
| `ChatInput.tsx` | Draft kept / … | unchanged (pinned) |
| `main.ts` native dialogs | vault prompts, pickers, confirms | unchanged in this campaign |

## Identity and naming display rules

These implement the [naming design](naming-design.md)'s display rule in one component (`PersonName`) and one
function (`personLabel`). No other code formats a person.

| Context | Rendering | Used in |
|---|---|---|
| `header` | **Display name** (`text-label`) then `@username` (`text-supporting text-text-muted`) as a separate element; `@username` alone when the two are equal case-insensitively | Message heads, member rows, the You row |
| `inline` | `Display name (@username)`, or `@username` when equal | Notes, toasts, invitation rows, "Hosted by", intro lines |
| `authority` | Always `Display name (@username)`, plus ` · former member` when inactive | Pickers (every row), remove member, ownership offer and accept, offboard, grant consent, approval context |
| `chip` | Display name with `@username` in a tooltip; `Display name (@username)` on a collision | Member stack, avatar tooltips |
| Joiner at a host decision | `@username` first in mono, then "{full name} (name on the server account)" | Waiting to join rows, Let in dialog, invite result |

Rules:

1. **Never an ID.** An unknown principal renders "Unknown member" with **Copy person ID** in its `⋯`. The old
   `actorName` fallback to the raw ID (CV:151-154) is deleted.
2. **Former members** (from S1's projected former principals) render muted with ` · former member`. Before S1 they
   are "Unknown member", never an ID.
3. **Collisions** use the daemon-projected labels (`collides`) when present; otherwise the client builds a
   directory keyed by the confusable skeleton of the display name. Any collision renders `Display name (@username)`
   everywhere, chips included.
4. **Isolation.** Every display name is wrapped in `<bdi>` so a right-to-left or mixed-direction name cannot reorder
   the surrounding text, and `@username` is always its own element, never concatenated.
5. **Agents** are "{display name}'s agent" (or "Your agent"); at authority points "{Display name (@username)}'s
   agent".
6. **Host detection** uses `workspace.host_principal_id` (S1), else `actor.uid === workspace.host_uid`. No numeric UID
   is ever rendered, including in "add another device".
7. **Teams** render as typed, never uppercased. **Channels** render `#slug`; in a list that spans teams (Agents,
   Also read) they render `{team} / #slug` when two teams share a slug. **Workspaces** render their S2 name, else
   "{host display name}'s workspace" for a legacy unnamed one; the local label is `name — server` only when two
   saved connections share a name.
8. **Copy ID lives only in `⋯` menus** (person, team, channel, message, attachment and SHA-256, task, invitation) and
   in Connection settings → Advanced → Workspace details. Those are the only places a UUID, 64-hex value or numeric
   UID appears.
9. **Typed input addressing a person takes `@username`**, resolved by the daemon; pickers search display names but
   show `@username` on every row before selection, and send `expected_username` with the principal ID so the broker
   refuses a mismatch.
10. **The name on the server account is offered, never applied silently.** It appears as a suggestion after joining
    and prefilled in Edit profile; until the person accepts it, the display name is the username.

| Situation | Render |
|---|---|
| Message from Bob | **Bob Lee** `@bob` · 10:02 AM |
| Bob's display name equals his username | **@bob** · 10:02 AM |
| Two people named "Sam Park" | Sam Park (@spark) and Sam Park (@sampark), in chips too |
| Offboarded author | **Bob Lee** `@bob` · former member *(muted)* |
| Remove confirmation | Remove Bob Lee (@bob) from #methods? |
| Agent post | **Alice Chen's agent** `@alice` [Agent] · 10:04 AM |
| Pending join | `@bob` · Bob Lee (name on the server account) |

## Revoke

The daemon already exposes the list and revoke routes, gated by proof of a person; the renderer never called them.
This campaign adds the renderer and fixes the daemon so a failed revoke fails closed.

**Daemon items (prerequisites).**

- **RV-D1, fail closed locally, then confirm remotely.** `revoke_scope` marks the local grant expired and persists
  *before* sending `run.revoke`. The route answers 200 `{revoked: true, remote_revocation_confirmed: true}`, 503
  `crew_revocation_unconfirmed` when the local stop landed but the workspace did not confirm, 404
  `crew_grant_not_found`, 409 `crew_grant_other_connection`, and 403 `crew_user_action_required`.
- **RV-D2, a usable list.** Grants gain `kind` (`chat` or `task`), `session_name`, and `expires_at` recorded at grant
  time, so the UI can show Expired without a network call.
- **RV-D3, task revocation stays consistent.** Revoking a task session delegates to the owned-run cancel path, so
  the run ledger never shows a stale `running`.
- **RV-D5, the model points to the control.** The Crew tool instructions add one sentence: the model cannot grant or
  revoke; tell the person to type `/crew` in this chat and choose Revoke, or run `biorouter crew grants revoke`.

**Renderer items.**

- **RV-R1** API helpers in `crew/api/grants.ts`: `listSessionGrants`, `revokeSessionGrant` (no body, no
  `Content-Type`), `findSessionGrant` (fan-out over saved connections; first match).
- **RV-R2** the chat-connect note and the Chat access pane are state-aware.
- **RV-R3** the inventories: the Access tab (this channel), the Agents sidebar section (this workspace, at rest), the
  header's agent-access chip, and Workspace settings → Agent access (the whole workspace).

**The chat-connect note** appears above the composer only when the route carries `?sessionId=` (the person typed
`/crew` in a chat):

| Grant state | Note | Action |
|---|---|---|
| Loading | Checking this chat's access… | — |
| None | Connect this chat to #methods? | **Review access** (accessible name pinned) |
| Active, destination here | This chat can read and post in #methods. | **Manage access** |
| Active elsewhere | This chat already uses #raw-data. | **Manage access** |
| Revoked | This chat's Crew access was revoked. | **Grant again** |
| Expired | This chat's access expired. | **Grant again** |

**The Chat access pane.**

- **No grant:** the consent summary ("“Plot review” will be able to · Read #methods · Post in #methods as Alice Chen
  (@alice)", "Access ends when you revoke it, or after an hour."), Advanced Also read, and **Allow this conversation
  to read and post here** (pinned).
- **After Allow:** the pane switches to Active with "Connected.", a primary **Back to chat** and a quiet destructive
  **Revoke access**. It no longer navigates away by itself, so the person sees where Revoke lives (fixes L10).
- **Active:** the summary with an Active (or "Expires 4:40 PM") badge, the context channels, **Open chat** and
  **Revoke access**. Revoke uses an inline two-step confirm (no modal over the pane): "Stop “Plot review” reading
  and posting in #methods?" **Revoke** · Keep access.
- **Results.** 200: "Access revoked. “Plot review” can't use Crew until you grant access again, or start a new chat."
  with Open chat and Done. 503 `crew_revocation_unconfirmed`: warning `Note` "Stopped on this device. Reconnect to
  confirm with the workspace." with **Retry**. **Any other non-200**: danger `Note` "Not revoked. This chat can still
  read and post." followed by the daemon's text and **Retry**. Success is shown only on 200.
- **Until RV-D1 lands**, a transport failure returns a plain 400 and the grant stays active locally; the "Not
  revoked" rendering is therefore mandatory, not optional, and the revoke UI must not ship without it.
- React gates nothing: Revoke renders whenever the daemon lists a grant; the daemon decides the outcome.

**Access rows** (Access tab, Agent access tab, Agents section): chat title (or "Your task"), `#destination (+n)`, a
status badge (Active, Expires {time}, Expired, Revoked, Stopped on this device), **Open**, a visible **Revoke** on
active chat rows and **Stop** on task rows, and `⋯` → Copy session ID. Revoked and expired rows collapse under "Show
revoked and expired (n)". The list refetches when opened and after every action; it never polls. A revoked chat stays
Crew-scoped for good, so the copy always offers "grant again, or start a new chat".

## Privacy and institution

| Change | Surface | Friction |
|---|---|---|
| Connection Private → Public | Popover **Make public…**, Privacy tab, or saving Connection settings with the mode changed | `DangerousConfirmDialog` with the workspace name as the phrase, from every path; then the full-body PATCH (L18) |
| Connection Public → Private | Popover **Make private**, Privacy tab | One click. Without an institution, a small dialog asks for it (req) first |
| Workspace → Allow Public (host) | Privacy tab | `DangerousConfirmDialog` with the workspace name |
| Workspace → Private for everyone (host) | Privacy tab | `ConfirmationModal`, then a toast |
| Institution label (host, irreversible) | Setup note, checklist, Host flow, Privacy tab | `DangerousConfirmDialog` without a phrase; confirm **Set {id} permanently**; disabled with "Add your institution in Connection settings first." when the connection has none |
| Join (joiner) | Join dialog privacy line | The workspace's mode and institution are shown from the invitation and confirmed; Change reveals the choice |

- The typed phrase names the object being exposed, following the knowledge-base tier precedent
  (`knowledge/SourcesRail.tsx`): the person checks *which* workspace, not just the word "public".
- `PrivacyBadge` is passed `enforcementOff={false}`: the broker enforces Crew mode independently of this machine's
  privacy master switch, so the "(enforcement off)" suffix would be false here.
- An unverified mode never looks verified: the chip reads "Checking privacy…" with no padlock until observed
  privacy for this connection arrives.
- The popover always shows the "why" line, so a joiner who sets Public in a Private-for-everyone workspace sees why
  nothing changed.
- Privacy state changes never animate.

## Motion

Every value comes from the `--dur-*` ladder and the single `--ease-out` curve. No spring, no bounce, no stagger, no
framer-motion. Exits are shorter than entrances. Crew keyframes are prefixed `crew-` (in `crew-app.css`) or live in
the shared `main.css` blocks, and every infinite loop declares its static rest state under
`@media (prefers-reduced-motion: reduce)` in addition to the global reset.

| Element | Property | Enter | Exit | Reduced motion |
|---|---|---|---|---|
| Row, member, message hover; gutter time reveal | background, opacity | `--dur-fast-min` (95ms) | same | instant |
| Buttons, chips, switch, radio, Copy → Copied label | color, background-image | `--dur-fast` (125ms) | same | instant |
| Copied check | `.biorouter-check-settled` (2px settle) | `--dur-med-min` (250ms) | — | static |
| Switcher and channel-menu chevrons (180°), disclosure and team chevrons (90°) | transform | `--dur-fast-max` (175ms) | `--dur-fast` | instant |
| Menus, popovers, pickers, tooltips | the primitives' fade + zoom-95 from the Radix origin | `--dur-fast-max` | `--dur-fast` | fade only |
| Advanced disclosure body | height (`--radix-collapsible-content-height`) + opacity | `--dur-med` (300ms) | `--dur-fast` | instant |
| Team-section body | none: rows appear and disappear instantly; only the chevron rotates (a frequent action) | — | — | — |
| Details pane, push | the grid column width `0 → var(--crew-pane-width)`; the inner content is fixed at the pane width and fades in over `--dur-fast-max`, so its text never reflows mid-tween | `--dur-med` | `--dur-fast-max` | instant |
| Details pane, cover | opacity + `translateX(8px → 0)` | `--dur-fast-max` | `--dur-fast` | fade only |
| Pane mode change | `animate-fade-slide-up` (8px) on the content | `--dur-fast-max` | none (replaced) | instant |
| Dialogs | `ModalShell` as built | primitive | primitive | primitive |
| Host dialog step change, setup-card state change | incoming: opacity + `translateY(8px → 0)`; outgoing: opacity | `--dur-fast-max` | `--dur-fast` | fade only |
| Live message arrival while following the bottom (including your own echo, the only confirmation of a send) | `biorouter-tool-enter` (4px rise + fade) | 160ms | — | none |
| Jump pill, history pill | opacity + 4px rise | `--dur-fast-max` | `--dur-fast` | fade only |
| Highlight of a task row (new task, Show task in channel, Agents row jump) | `--overlay-selected` wash fading to transparent | `calc(var(--dur-slow) * 3)` on `--ease-out` | — | shown, then removed with no transition |
| Verifying dim | timeline opacity 1 → 0.6 | `--dur-fast` | `--dur-fast` | instant |
| Spinner (Connecting…, Uploading, waiting for host) | the one spinner, 700ms linear rotation | loop | — | static arc |
| `StatusDot live` | 2px halo, 2s period (design.md §4.16) | loop | — | static dot |
| Running task word | the tool-call running text pulse | loop | — | static |
| Skeletons | the `Skeleton` pulse, shown only after 150ms | loop | — | flat fill |

**What must not animate:** channel and workspace switching (instant swap, keyboard-driven or not); security state
(the privacy chip, status words, classification badges, the destination line, grant state, the unknown-outcome gate)
so no frame ever shows stale security state mid-tween; unread bold and counts; history pages prepended (scroll
position anchored); error and warning notes (they appear in place; `role="alert"` announces them); composer
auto-grow; the terminal; the member stack. Scroll moves only for Jump to latest and Show task in channel
(`scrollIntoView({behavior: 'smooth'})`, `auto` under reduced motion).

## Accessibility

**Landmarks and names.**

| Region | Element | Name |
|---|---|---|
| Crew sidebar | `<nav>` | Crew |
| Status row | `role="status"` on the word | Connection status |
| Team section | header `button[aria-expanded][aria-controls]` + `<ul role="list">` | {team}, {n} channels |
| Channel row | `<button aria-current="page">` in an `<li>` | {name}, {n} unread |
| Channel | `<section aria-labelledby>` the `<h1>` | #{name} |
| Timeline | `role="log" aria-live="polite"`, `aria-busy` while paging | {name} messages |
| Message group | `<article aria-labelledby>` author and time | — |
| Composer | textarea `aria-label` | Message #{name} |
| Details pane | `<aside>` | #{name} details / Ask my agent / Chat access |
| Connection bar | `Note role="alert"` (errors) or `role="status"` (standing conditions) | its text |
| Glyph-only buttons | `aria-label` + `Tooltip` with the same words; menus `aria-haspopup="menu"` | per copy deck |
| Status dot | `aria-hidden` beside a word; `role="img"` + label when alone | the status |

Visible labels are always contained in accessible names (WCAG 2.5.3): Accept → "Accept invitation to Imaging Core",
Let in… → "Let @bob in".

**Keyboard.**

- Tab order: sidebar → channel header → connection bar → timeline → notes → composer → pane.
- Menus are Radix: Enter or Space opens, arrows move, typeahead, Escape closes and returns focus. A channel row's
  context menu opens with Shift+F10 or the Menu key.
- Sidebar: ↑/↓ move between rows (roving focus), Enter opens, ←/→ collapse and expand a team when its header is
  focused.
- The timeline log is focusable (`tabIndex=0`); ↑/↓ move between message rows (roving `tabIndex`); row actions appear
  on `:focus-within`.
- Composer: Enter sends; Shift+Enter inserts a newline; IME composition is respected.
- Optional shortcuts (ship only if a repo-wide check finds no conflict): ⌘. toggles the pane; ⌥⇧↑/↓ jump to the
  previous or next unread channel; Esc in the timeline marks the channel read.

**Focus management.**

| Event | Focus goes to |
|---|---|
| Pane opens in agent mode | The Task textarea |
| Pane opens in chat-access mode | The pane heading (`tabIndex=-1`), so the consent summary is read first |
| Pane opens in details mode | The active tab |
| Pane closes | The control that opened it, else the composer |
| A dialog opens | Its first field (the old effect that focused ✕ and overrode `autoFocus` is deleted) |
| A dialog closes | Its trigger |
| Destructive confirmations | Cancel |
| Sign in opens | The terminal |
| A setup card changes because of a user action | Its primary action; a background change is announced by the status row instead |
| Join or accept completes | The landing channel's composer |
| After a send | Stays in the composer |
| After Create team or Create channel | The new channel's composer |

**Announcements.** One polite region per surface and never a duplicate of visible error text (C2): the connection
bar's `Note` is the alert; `CopyField` announces "Copied"; the status word's own `role="status"` announces status
changes; the log announces new messages.

## Theme behavior

- **Tokens only.** No hex, rgb, pixel font sizes or off-ladder radii in `crew-app.css` or TSX. The legacy hex values
  (`#b86b46`, `#0006`, `#0003`, `#17191c`) disappear with the legacy files.
- **The new layout never inherits the old stylesheet.** The new root is `.crew-app`, not `.crew-view`, so the old
  unlayered element selectors (`.crew-view input`, `.crew-view button:focus-visible`) cannot restyle primitives or
  beat the global focus surface (D-15).
- **Accent** appears only on the view's primary button, the sidebar active bar, the selected tab bar, the composer's
  focus edge, Send once there is content, and the New line. Alma Mater (teal) and Roche Limit (orange) re-skin Crew
  with zero Crew edits.
- **Status hues** appear only on status dots, warning and danger notes and badges, trust panes and destructive
  actions. Unread counts are neutral badges.
- **Grounds hold the dark ladder**: canvas darkest, the pane and cards one step up, the sidebar matching the app
  sidebar, the copy well reading as an inset.
- **The terminal** takes `GENERATED_THEMES[useThemeFamily()][useResolvedTheme()].terminal` and re-themes on family or
  mode change without recreating the session; both hooks are non-throwing outside a provider, so CAT needs none.
- **The padlock** means privacy tier only. Channel classification is a neutral badge.
- **Contrast** is covered by `check:contrast`, because Crew adds no colour tokens.
- **Source guard** (`crew/crewCss.sourceGuard.test.ts`): every `crew/**/*.css` file except the legacy `crew.css` has no
  element selectors, no hex or rgb literals, no pixel font sizes, a reduced-motion rule for every `@keyframes crew-*`,
  and the two geometry numbers equal to `yieldLadder.ts`. A second guard rejects new arbitrary-value Tailwind classes
  (`-[` followed by `var(` or a unit) in `crew/**/*.tsx`, because under `BIOROUTER_NO_HMR` a newly written utility
  silently never generates; load-bearing styles are authored CSS.

## Test migration plan

The tests pin the accessibility tree and module seams, not the CSS. Every behavioral assertion is kept. Queries
move only where a control deliberately leaves the resting screen for a menu (C5) or changes kind (C3, C4), and
each move is listed here so none can pass for a silent weakening.

### When each test changes

- **Extraction (controller first):** CVT, CAT and CFT pass with **zero edits**; the legacy layout renders from the
  extracted controller.
- **Area work:** new components come with their own test files; CVT keeps testing the legacy layout and stays green.
- **Integration:** CVT switches to rendering `CrewApp` and every row below changes in the same commit. CAT changes only
  where the restyle requires it (no pinned string changes). CFT keeps testing the legacy `CrewUpload` until the legacy
  files are deleted; the same two assertions are duplicated against the new `files/useCrewUpload` first.

### Shared helpers (`crew/test/crewTestUtils.ts`)

```ts
const user = userEvent.setup();

export async function workspaceAction(item: string, workspace: RegExp = /^Fixture/) {
  await user.click(screen.getByRole('button', { name: workspace }));   // the switcher trigger
  const entry = await screen.findByRole('menuitem', { name: item });
  await waitFor(() => expect(entry).not.toHaveAttribute('aria-disabled', 'true'));
  await user.click(entry);
}
export async function channelAction(item: string, channel = 'general') {
  await user.click(screen.getByRole('button', { name: `${channel} channel menu` }));
  await user.click(await screen.findByRole('menuitem', { name: item }));
}
export async function chooseModel(model: string) {
  await user.click(screen.getByRole('button', { name: /^Model/ }));
  await user.click(await screen.findByRole('option', { name: new RegExp(`^${model}`) }));
}
```

The Radix `DropdownMenu` opens on pointer events that `userEvent` produces (`SessionNamePill.test.tsx` already does
this in this jsdom setup), and its default `modal={false}` leaves the page in the accessibility tree.

### CVT, query by query

| Line(s) | Today | After | Why |
|---|---|---|---|
| 191, 210, 650 | `findByText('fixture')`, `getByText('alice@new-host')` | **Unchanged** (You row SSH login, its own node, rendered with or without a snapshot) | — |
| 192, 211 | `'Checking connection'`, `/^(Checking connection\|Updates unavailable)$/` | **Unchanged** (status row word) | — |
| 193, 212, 217, 293, 340, 352, 387, 425 | `'Connected · identity verified'` | **Unchanged** (the status row's `sr-only` span; the menu copy mounts only when open) | — |
| 221 | `getByRole('button', {name:'Edit'})` | `await workspaceAction('Connection settings…')` | Moved into the menu (C5) |
| 222 | `findByPlaceholderText('For example, ucsf or sdsc')` | **Unchanged** | — |
| 225, 229 | `getAllByLabelText('Connection privacy')[1]` + change | `fireEvent.click(getByRole('radio', {name: /^Public/}))` / `/^Private/` | Radio rows (C3, deliberate) |
| 227 | institution `not.toBeRequired()` when Public | `expect(queryByPlaceholderText('For example, ucsf or sdsc')).toBeNull()` | The field is hidden when Public; stricter |
| 231-233 | same element `toBeRequired()`, `toHaveValue('')` | Re-query with `findByPlaceholderText`, same two assertions | The field remounts; form state keeps its value |
| *(add)* | — | Type `ucsf`, toggle Public then Private, expect `ucsf` retained | The test's name promised this; nothing checked it |
| 234-240 | **Save connection**, `toBeInvalid()`, no PATCH | **Unchanged** (native validation, C8) | — |
| 249 | `findByText('Not specified')` | `findByText(crewCopy.hostSetup.title('Fixture'))` | The institution is no longer a rail paragraph |
| 250-262 | click **Confirm workspace institution: ucsf** → `policy.set` | click **Set institution to ucsf…**, then **Set ucsf permanently** → the same `policy.set` assertion | The irreversible action gained a confirmation (L3) |
| 283, 287, 294, 341, 388, 404, 426, 514, 542, 550, 554, 560, 596, 675, 687, 706, 754 | `'Message #general'` | **Unchanged** | — |
| 286, 553, 586 | `getByRole('button',{name:'Reconnect'})` | `await workspaceAction('Reconnect')` (the helper's enabled wait replaces 552's `toBeEnabled`) | Menu (C5) |
| 288, 395, 519, 588, 647 | observation error texts | **Unchanged**, exactly one element (connection bar only) | C2 |
| 296-327 | composer key events | **Unchanged** | — |
| 397, 550 | `queryByLabelText('Message #general')` null | **Unchanged** (the Verifying bar replaces the card) | C13 |
| 401-403, 520, 593, 648 | **Retry Crew updates** | **Unchanged** (accessible name) | — |
| 406, 431, 709 | **Send message** | **Unchanged** | — |
| 429, 460, 465, 783 | `findAllByText('send failed' \| 'start failed' \| …)` | **Unchanged**; each now renders once | — |
| 439, 470, 774, 808, 830 | `'Welcome to #general'` | **Unchanged** | — |
| 451, 718, 721, 775, 809, 831 | **Ask my agent** | **Unchanged** (the only control with that name) | — |
| 454, 722, 776, 810, 824, 835 | `'Task'` | **Unchanged** (seeding changes the initial value only when a draft exists; tests type into it) | — |
| 455-458, 725-730, 777-780, 811-814, 836-839 | `getByLabelText('Configured provider')` change + `getByLabelText('Model')` change | `await chooseModel('fixture-model')` | One picker; the POST assertions on `provider` and `model` are unchanged |
| 459, 731, 781, 815, 820, 822 | **Start my agent and allow posting here** | **Unchanged** | — |
| 463, 546 | `getByRole('button',{name:'Refresh channel'})` | `await channelAction('Refresh channel')` | Channel menu (C5); at 463 the pane is still open and the menu reachable (C1) |
| 474, 679, 746 | **Authenticate** | `await workspaceAction('Sign in…')` | Renamed and moved (C5) |
| 475-477, 681, 748 | **Simulate authenticated completion** | **Unchanged** (the mock renders inside the Sign in dialog) | — |
| 516 | **Older messages** | **Unchanged** (no `IntersectionObserver` in jsdom, so nothing auto-loads) | — |
| 518 | `'Viewing earlier messages'` | **Unchanged** | — |
| 651 | `getByRole('option',{name:'Renamed workspace'})` | `getByRole('button', {name: /^Renamed workspace/})` | The switcher trigger shows the selected name (C4, deliberate) |
| 684, 751 | `getByRole('combobox',{name:'Connection privacy'})` `toHaveValue` | `getByRole('button', {name: /^Privacy: Public/})` / `/^Privacy: Private/` | The status-row chip (C3, deliberate); `expected_mode` and `personal_mode` payload assertions still prove the wire |
| 686, 753 | `getByText(/Effective: public/)` | Folded into the chip assertion | The chip is the effective mode |
| 689 | `getByRole('button',{name:'Review access and posting permission'})` | `await findByRole(...)`, same name | The note waits for the grants lookup; the mocked `crewHttp` returns `{}`, read as "no grant" |
| 691 | **Allow this conversation to read and post here** | **Unchanged** | — |
| 840 | `getByRole('checkbox')` | **Unchanged** (Also read is unmounted under a closed Advanced; other toggles are switches) | C9 |
| 841 | `/Start a new task/` | **Unchanged** | — |

**Module seams (C14).** `./crewApi` (`crewHttp`, `crewRequest`, `observeCrew`, actual `CrewHttpError`),
`../ConfigContext`, `react-router-dom` and the `./CrewAuthentication` default export
(`{connectionId, onConnected, onClose}`) keep their names and signatures. `./CrewHostTrust` stays a default export.
The `./CrewFiles` mock stays until the legacy files are deleted; the new layout does not import `CrewFiles`, so the
integration test additionally mocks `./files/useCrewUpload` where a picker would open. `defaultHttp()` gains
`GET /connections/{id}/grants → {grants: []}` and reads a missing `grants` as `[]`.

### Behavioral assertions that must survive

| Assertion | Tests | How it is preserved |
|---|---|---|
| Single-flight send | CVT:330-353 | `state/crewSend.ts` keeps the `sendingMessage` ref; Enter and Send call the same `send()`; the Send node never remounts |
| No refresh after a send | CVT:350-352 | `send()` never calls `refresh()`; automatic mark-read uses `channel.read` without refresh |
| Idempotency key reuse on retry, rotation on change | CVT:355-435, 757-787 | `pendingMessage` / `pendingRun` refs and fingerprints move unchanged; Task having its own state only makes reuse stricter |
| Exactly-once observation errors | CVT:288, 395, 519, 588, 647 | One resolver, one slot; a new test asserts `getAllByText(msg).length === 1` with the pane and a dialog open |
| An action error survives a refresh | CVT:437-466 | `refresh()` neither clears `error` nor closes the pane; if the pane were unmounted the resolver would fall back to the connection bar |
| Same DOM node for Start across a failure | CVT:781-784 (C11) | Footer button rendered unconditionally with a stable key; the error slot is a fixed sibling above it |
| Unknown-outcome gate | CVT:789-847 (C10, C12) | Module-scoped lock in `state/crewRunStart.ts`; Start disabled, not hidden; Start a new task separate and gated by the one checkbox |
| Native required validation | CVT:215-241 (C8) | Connection settings is a real `<form>`, `type="submit"`, no `noValidate`; `required={mode === 'private'}` |
| Auth completion refreshes without a second connect | CVT:468-486 | `onSignedIn` = close, `loadConnections()`, `refresh()` |
| `refresh()` order | CVT:600-653 | Connections before observe, unchanged |
| Draft clear and retain rules | CVT:268-289, 488-598, 655-755 | Moved unchanged; one deliberate change: a task start clears the composer only when the draft still equals the Task seed |
| `expected_*` epochs on run and grant | CVT:655-755 | Payload builders moved unchanged |
| The composer never renders unverified | CVT:397, 550 (C13) | The Verifying bar replaces the card; the textarea is not mounted |

### CAT and CFT

- **CAT**: unchanged IPC seams, early-event replay, `resizeTerminalSession(id, 80, 12)` after creation and never after
  an early exit, no manual-complete button, dispose only on an explicit close, exactly one `role="alert"`. The visible
  button text becomes "Close" while the accessible name stays **Close authentication connection**; the exit text keeps
  the fragment **SSH authentication ended (exit 255)** (the word "Daemon" is dropped). The title and disclaimer move
  to `SignInDialog`, outside the component.
- **CFT**: unchanged while the legacy `CrewUpload` exists. The new `files/useCrewUpload.test.tsx` repeats both
  assertions against the Attach menu (`Attach` → menuitem **Upload a file…**): exactly one alert with **Refresh the
  workspace to verify connection privacy before uploading.** when `expectedMode` is undefined, and `beginTransfer`
  called with exactly `{expected_mode, connection_id, channel_id, direction: 'upload'}`.

### Other Crew tests

`crewApi.observation.test.ts` and `crewTransfers.test.ts` unchanged. `crewApi.requestSerialization.test.ts` gains:
`revokeSessionGrant` sends `X-User-Action`, no body and no `Content-Type`. `ChatInput.crewCommand.test.tsx` and
`ProviderGuard.test.tsx` unchanged. `AppLayout.test.tsx` gains a case: `/crew` toggles
`biorouter-chat-route-active`. `MentionPopover.test.tsx` and `AppSidebar.test.tsx` update only if they assert the old
copy. Global guards: `dialogDescriptionSourceGuard` is satisfied through `ModalShell` and the confirm primitives;
`uiCopySpelling` passes on American English.

### New tests

| Test | Asserts |
|---|---|
| `state/crewStatus.test.ts` | Every row of the connection-status table, `deriveCrewScreen()` rows, run and transfer words for every enum value including an unknown one |
| `state/connectFailure.test.ts` | Each daemon code maps to its surface; the fallback maps `ssh_eof`/`exit_255` to sign-in and `exit_127` to not-set-up; nothing else is labelled host-key |
| `state/useCrewController.test.tsx` | `act(source)` routing, per-action pending, `lastVerified` kept and then cleared by `clearProtectedState`, sign-in auto-open only with the option and only on a user-initiated connect |
| `identity/PersonName.test.tsx` | The four contexts, collisions (`Sam Park` twice), former members, "Unknown member", `<bdi>` isolation, never an ID |
| `integration/noMachineIds.test.tsx` | A rich fixture (former-member author, invitation, people, About tab, task row, pending join): no UUID, no 64-hex string, no fixture UID in the default DOM; each appears after the matching Copy … ID |
| `integration/errors.test.tsx` | Every error source renders exactly once with the pane and a dialog open |
| `integration/pane.test.tsx` | With the pane open the composer has no `aria-hidden` ancestor; Escape closes and returns focus; `refresh()` keeps the pane open |
| `integration/privacy.test.tsx` | Private→Public opens the typed confirm from the popover, the Privacy tab and Connection settings save; Cancel sends no PATCH; confirm sends the full body; Public→Private is one click; the institution confirm precedes `policy.set` |
| `access/*.test.tsx` | Active grant shows Manage access and Revoke (not Allow); a confirmed revoke refetches to Revoked + Grant again; 503 shows "Stopped on this device" + Retry and never success; any other failure shows "Not revoked. This chat can still read and post."; the Access tab lists chat rows with Revoke and task rows with Stop; revoked rows collapse |
| `sidebar/*.test.tsx` | Bold unread with neutral counts; Agents section rows; team sections collapse instantly; roving keyboard navigation; the switcher and channel title have `aria-haspopup="menu"` and a chevron |
| `onboarding/*.test.tsx` (join states behind a mocked capability) | Invitation preview fills the summary; privacy line and Change; invited shows the locally computed code; not invited, code mismatch, expired, joined; legacy token state |
| `dialogs/LetInDialog.test.tsx` | `DeviceCodeInput` accepts hyphens, spaces and lower case and normalizes Crockford; the host UI never renders a code; the different-code warning shows before input |
| `integration/verifying.test.tsx` | During `refresh()` the sidebar and header persist from `lastVerified`, the composer is absent, the Verifying bar is present, and the view never falls back to the welcome state |
| `ui/copy-field.test.tsx`, `status-dot.test.tsx`, `disclosure.test.tsx`, `avatar.test.tsx` | Copies `value` not `display`; the Copied swap and live region; failure selects the value; dot label rules; Disclosure unmounts when closed |
| `crew/crewCss.sourceGuard.test.ts` | The stylesheet rules in [Theme behavior](#theme-behavior) |

### Gates at each step

`npx vitest run src/components/crew src/components/ui`, `npm run typecheck`,
`npx eslint "src/components/crew/**/*.{ts,tsx}" --max-warnings 0`, `npm run lint:check`, and
`npx prettier --check "src/components/crew/**/*.{ts,tsx,css}"` (Prettier is not enforced by CI, so run it). After the
integration step, a live pass in the real app with the three-profile fixture: 1048 and 1280 widths, the sidebar open
and collapsed, the drag region, light and dark in all three theme families, reduced motion, sign-in against a
password and MFA host, and the three novice walkthroughs below.

## Backend dependencies and how the UI degrades

| Backend change | Enables | Until it lands |
|---|---|---|
| Revoke RV-D1 (fail closed) | "Stopped on this device" | Revoke still works; a transport failure shows "Not revoked. This chat can still read and post." |
| Revoke RV-D2 (`kind`, `session_name`, `expires_at`) | Chat titles, Expired and Expires badges, task versus chat rows | "Untitled chat"; Active or Revoked only; rows classified by matching `state.runs` session IDs |
| Typed SSH failure codes | Every connection-problem surface | The `ssh_eof`/`exit_255` → sign-in and `exit_127` → not-set-up fallback; everything else in the connection bar |
| Naming S1a (former principals, invitation names, `host_principal_id`, people map, projected labels) | Names everywhere, host detection | "Unknown member"; "Invitation from {inviter}"; `uid === host_uid` |
| Naming S1b (daemon resolver) | Typed `@bob` fields | Pickers only |
| Naming S2 (unique names, renames, workspace name, `start --name` printing the invitation line) | Slug preview enforced, Rename items, the workspace name in the switcher, the one-line host paste | Client slug preview; Rename hidden; the connection name; the status-JSON paste |
| Naming S3a (invitation route, join status and claim, invite by name, approve) | The default join and admit flows | Invitation route missing (stale daemon): manual workspace details under Advanced; broker without `join_by_name_v1`: the legacy token path |

A new renderer talking to a stale shared daemon gets 404 from new routes; it hides name-only affordances and shows
"This feature needs a newer Biorouter background service. Quit and reopen Biorouter." where the person tried to use
one.

## Acceptance criteria for novice critics

Three personas walk the whole flow on a clean profile: **Priya**, a biologist who has never used SSH (password and Duo,
an empty `known_hosts`, the per-user binary installed by IT); **Marcus**, a lab manager hosting for the first time who
will paste a terminal command if told exactly what to paste; and **Dana**, a returning user whose connection reads
Offline after an app restart. The design passes when all of these hold in the real app:

1. **Zero hard dead ends** for all three personas, counted as in the novice judgement: an action that loops, or a
   state with no in-app way forward. Leaving the app is allowed only for (a) the host running the start command in a
   terminal and (b) verifying an unknown host key with IT.
2. **Zero machine strings typed.** No persona types or reads a UUID, 64-hex key, token, socket path or numeric UID on
   the default path. The only strings handed between people are the invitation message and the device code, both
   copied with one click.
3. **The joiner's inputs:** one paste (the invitation) and, only if the server asks, the password and MFA code in the
   sign-in terminal. The joiner never chooses privacy or an institution unless they press Change.
4. **Sign-in finds the person.** A server that wants a password opens Sign in by itself after Join, Connect or
   Reconnect; Dana reaches her channels in at most two clicks plus her credentials.
5. **Every connection failure names its cause in one sentence** with one action: sign-in needed, unknown host key,
   changed host key, unreachable, Crew not set up for this account, wrong workspace. No generic SSH failure is ever
   shown as a host-key problem.
6. **The host always has something to send** after Invite (the invitation message) and after Let in (nothing further
   is needed), and is offered **Add {first} to {team}** in one click.
7. **Privacy is visible at every decision point**: the status row in every state, the join summary and privacy line,
   every confirm for an exposing change. Making a connection Public needs the typed workspace name from every path;
   going back is one click.
8. **Revoke is findable at rest** from Crew (Agents section, agent-access chip, Access tab) in at most three clicks,
   and from the chat by typing `/crew` in at most three; a failed revoke never reads as success.
9. **Ask my agent** starts with the Task prefilled from the draft, shows the destination and a model (or "No models
   are set up." with a way to Settings), and a waiting approval is marked in the Agents section and the task row.
10. **Nothing without a consequence is on screen**: no disclaimer paragraph on any default path; help sits behind a
    named disclosure.
11. **Every dropdown opens a menu, every list behaves as a list**, and every copyable value has a one-click Copy that
    confirms itself without a toast.
12. **Motion and theme**: the checks in [Motion](#motion) and [Theme behavior](#theme-behavior) pass, including
    reduced motion and all six family-and-mode combinations.

## Test constraints and latent defects

**Constraints the current tests impose (C1–C16).**

| # | Constraint |
|---|---|
| C1 | The background stays in the accessibility tree while the agent or grant surface is open (tests click Refresh, the composer and Ask my agent with it open) |
| C2 | Observation errors render once (exact-text queries throw on duplicates) |
| C3 | `Connection privacy` was a native select in two places (deliberately changed) |
| C4 | The workspace picker exposed `<option>`s (deliberately changed) |
| C5 | Edit, Reconnect, Authenticate, Refresh channel and others were reachable without a menu (deliberately changed for the first four) |
| C6 | Exact accessible names and labels (kept, except the listed deliberate changes) |
| C7 | Exact standalone text nodes (`fixture`, `Checking connection`, `Connected · identity verified`, `Welcome to #general`, …) |
| C8 | Native constraint validation on the connection form |
| C9 | Exactly one checkbox while the unknown-outcome gate is open |
| C10 | The unknown-outcome lock is module-scoped and survives a remount |
| C11 | The Start button node is stable across a failed attempt |
| C12 | Start is disabled, not removed, while the outcome is unknown |
| C13 | The composer never renders without a verified snapshot |
| C14 | The mocked module seams keep their names, exports and signatures |
| C15 | Enter/Shift+Enter/IME, single flight, no refresh after send, refresh order, auth completion without POST connect, draft rules, key reuse and rotation, `expected_*` epochs |
| C16 | `CrewAuthentication` resizes after creation only, offers no manual complete, disposes only on explicit close, renders one alert |

**Latent defects (L1–L19)** are listed with their dispositions in [Latent defects](#latent-defects). In short: wrong
edit title (L1), generic Save that creates a token (L2), four unconfirmed destructive actions (L3), pickers listing
members (L4), IDs in invitations (L5), `last_error` never shown (L6), duplicate Open Chat history (L7), no key pattern
(L8), Task sharing the draft (L9), grant success relying on navigation (L10), dead catch (L11), mark-read refreshing
(L12), per-card polling (L13), shared dialog state (L14), no Create channel in an empty team (L15), inconsistent avatar
fallbacks (L16), dead CSS (L17), full-body privacy PATCH (L18, kept) and missing revoke, grant list, disconnect and
search (L19).

## Risks and open questions

1. **The drag region on `/crew`** needs the real-app check with the sidebar open and collapsed; jsdom cannot see it.
2. **Cover below 800px** is a deliberate deviation from the artifact panel's stack rung. If product testing prefers
   stacking, only `crew-app.css` changes.
3. **"after an hour"** mirrors `expires_in: 3600`; once RV-D2 reports `expires_at`, the copy reads the real time.
4. **Automatic mark-read** writes a read position on view; it is gated (focused window, bottom visible for 1s, once per
   5s per channel) and the menu item remains.
5. **The last verified view** re-shows messages the person was already looking at, from the same verified scope, while
   no action is enabled. If a security reviewer prefers, the timeline can show skeletons instead with no other change.
6. **Dead legacy code** remains compiled after the cutover until the follow-up deletion; nothing routes to it.
7. **⌘K and other shortcuts** ship only if a repo-wide check finds no existing binding.

## Related documentation

- [Naming design](naming-design.md) — the identity rules, invitation format, device code and slices this UI displays
- [Implementation plan](implementation-plan.md) — Crew architecture, §16 GUI and naming requirements
- [Protocol contract](protocol-contract.md) — transport, identity and enrollment contract the join flow follows
- [Native CLI guide](cli-guide.md) — the CLI parity surface for grants, revoke and joining
- [Crew UI acceptance report](crew-ui-acceptance-report.md) — earlier three-client evidence whose cited strings this spec keeps or retires
- [Implementation status](implementation-status.md) — where progress on this design is recorded
- [SSH hop policy](ssh-hop-policy.md) — the host-trust rules the trust screens enforce
- [Design system](../../../design.md) — principles P1–P8 and element specs §4 used throughout
- [Settings visual vocabulary](../../desktop-ui/settings-visual-vocabulary.md) — row, section, note and button rules applied to Crew's lists and dialogs
