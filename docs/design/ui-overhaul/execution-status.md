# UI overhaul — execution status

> **What this is.** The single status record for the UI cohesion and chat-groups
> branch: the 20-step list, every commit, the gates, what was proven by driving the
> real app, the register of what is still broken or open, and the brand rollout.
> **Status:** Current — this is the stated source of truth for
> `worktree-redesign-ui-cohesion`. All 20 steps are marked done; the branch still
> carries open items (see [Known broken and open](#known-broken-and-open)).
> **Audience:** maintainers working on the BioRouter desktop UI.
> **Identifier key.** `D-NN` are numbered design decisions. The register lives in
> [`design.md`](../../../design.md) — Part 6 (open decisions), Part 6b (this
> cohesion pass), Part 7 (the drift register). `R1`/`R4`/`R7` are the risks named in
> the chat-groups plan.

This branch had two jobs: make the desktop app look like one product rather than
several (the cohesion pass), and turn the chat area into browser-style tabs and
splits (chat groups). Both are implemented. The sections below are ordered so the
status material — steps, gates, commits, open items — comes first, and the longer
evidence and retrospective write-ups follow it.

**Branch:** `worktree-redesign-ui-cohesion` · **Worktree:** `.claude/worktrees/redesign-ui-cohesion`
**Started:** 2026-07-16 · **Last updated:** 2026-07-17

## Contents

- [Companion documents](#companion-documents)
- [Where the work stands: 20 of 20 steps](#where-the-work-stands-20-of-20-steps)
- [Gates](#gates)
- [Commits](#commits)
- [Known broken and open](#known-broken-and-open)
- [Still to verify by driving](#still-to-verify-by-driving)
- [Evidence from driving the real app](#evidence-from-driving-the-real-app)
- [Dated change log](#dated-change-log)
- [Decisions the user overrode](#decisions-the-user-overrode)
- [What is not built, and why](#what-is-not-built-and-why)

## Companion documents

| Doc | What it is |
|---|---|
| `execution-status.md` (this file) | Status, step list, what's done, what's left, what's known-broken |
| [`ui-cohesion-redesign.html`](ui-cohesion-redesign.html) | The approved visual spec. Interactive: Current ⇄ Redesigned, light/dark, Highlight |
| [`ui-cohesion-redesign.md`](ui-cohesion-redesign.md) | The written half of that spec — the forensics and the per-change specifications, each individually tagged. Readable without a browser |
| [Chat-groups design judgement and plan](../../history/chat-groups/design-judgement-and-plan.md) | The chat-groups plan (3 designs → adversarial judge). Carries the MEASURED banner for R1/R4 |
| [Nested `KnowledgeProvider` blocker](../chat-groups/knowledge-provider-nesting-blocker.md) | Proof that two `KnowledgeProvider`s clobber each other through the server |
| [`design.md`](../../../design.md) | The design system. Decisions D-01…D-37; Part 6b is this pass; Part 7 is the drift register |

## Where the work stands: 20 of 20 steps

The list grew from 15 steps to 20. The user added the browser-tab keyboard model
(⌘T / ⌘N / ⌃Tab), made **lag a first-class acceptance criterion**, and asked for
**progressive conversation loading** (paint the transcript first, let extensions
finish behind it); D-31 turned out to have a second half (the preview's chrome,
D-33). New work is added as new rows rather than folded into existing ones, so
the denominator stays honest — the fraction should get *worse* when scope grows,
not quietly better.

| # | Step | Status |
|---|---|---|
| 1 | Audit the app against the design system (sidebar / chat / preview / terminal / markdown / pop-ups) | ✅ done |
| 2 | Approved visual spec (rev 1→5) | ✅ done |
| 3 | Cohesion pass implemented (prose tokens, tables, math, headings, surfaces, tabs, sidebar, titlebar, icons) | ✅ done |
| 4 | Docs: design.md Part 6b, drift register given a status column, supersession banner | ✅ done |
| 5 | Chat-groups design: 3 approaches + adversarial judge | ✅ done |
| 6 | R1 measured — drag inside the drag region | ✅ done (PASS) |
| 7 | R4 measured — N mounted BaseChats | ✅ done (affordable) |
| 8 | Stage 0 — session-scope the broadcast events | ✅ done |
| 9 | Stages 1–2 — tabs in one group | ✅ done |
| 10 | Stage 3 — split, drag, drop zones, global dock | ✅ done |
| 11 | Theme default → auto | ✅ done |
| 12 | **User-reported bugs** (tab-per-click + Cmd+W, sidebar collapse, centering, icon parity) | ✅ done |
| 13 | **D-31** tab labels → sans — spec + code | ✅ done |
| 14 | **D-33** the other half of D-31: the preview's chrome → sans | ✅ done |
| 15 | **D-34** kill the "New Session" flash — a tab opens already named | ✅ done |
| 16 | **Browser keys** — ⌘T new tab, ⌘N new window, ⌃Tab cycle (chat + preview) | ✅ done (D-35) — 13/13 driven; preview-side ⌃Tab jsdom-only |
| 17 | **Lag** — measure first, then fix; leave a repeatable perf gate | ✅ done — **measured; no refactor shipped, because nothing measured indicted app code** |
| 18 | **Progressive load** — paint the transcript first; extensions/model finish behind it; toast on ready (naming partial failures) | ✅ done — **2667ms → 738ms to read** |
| 19 | **D-32** the yield ladder — responsive collapse at every window size | ✅ done (+ D-36, D-37) |
| 20 | Final gate + full visual QA sweep (**lag is an acceptance criterion, not a nice-to-have**) | ✅ done — 6/6 on screen, 1 real bug found + fixed |

## Gates

| Gate | State |
|---|---|
| `tsc --noEmit` | ✅ clean |
| `lint:check` | ✅ clean (tsc + eslint 0 warnings + contrast). **Use `npm run lint:check`, not a bare `npx eslint .`** — the latter lints `.vite/build/main.js`, a build artifact, and reports thousands of phantom errors |
| `check-contrast.mjs` | ✅ 140/140 (was 128 — +12 guard the code ground) |
| `vitest run` | ✅ **1266/1266 (151 files), 0 failures** — no timeout flags. **Run it on an idle machine; see [The suite is load-sensitive](#the-suite-is-load-sensitive-and-it-produced-a-false-diagnosis)** |

> **Note.** The suite count above is the figure at the close of step 20. The
> follow-on fixes of 2026-07-17 report **1290/1290**; see
> [Follow-on UI and terminal fixes](#follow-on-ui-and-terminal-fixes-2026-07-17-on-user-review).

## Commits

Every step is reversible.

```text
94125bbc  refactor      1120 had three homes; give it one (+ correct D-32's rationale)
0c329018  fix(sidebar)  the Recents hidden-count badge is sans, not mono (D-31)
0e75258f  fix(tabs)     move the titlebar reserve OUT of the strip's scroll box
4621d654  test(harness) a git-repository fixture — D-33's split was unverifiable without it
8bcfcbe5  feat(preview) rung 3 for the preview's strip, through the SHARED rule
85fe794c  feat(groups)  rung 4's trigger — the split's width watcher
c611c154  feat(groups)  rung 4's state half — merge to one, and give it back
3f71f504  feat(tabs)    rung 3 — tabs collapse into a ▾ once they scroll out of sight
6158c6ae  feat(chat)    rung 2 — the preview panel yields its column to the transcript
ea68495d  feat(layout)  the yield ladder, as pure rules (D-32)
d9028330  feat(chat)   paint the conversation before the model and extensions load
0a701721  feat(toasts) extension readiness: multi-chat aware, partials reported honestly
c5676eae  fix(build)   land closeActiveTabRegistry — HEAD imported a module not in the tree
84b0c5a4  test(tabs)   the chat side of the Ctrl+Tab arbitration
fa725c70  design       D-35 — the tab model is a browser's, and so is the keyboard
acef5969  fix(chat)    a submit fills the blank tab you typed in, not the leftmost
4fc34bb3  fix(preview) Ctrl+Tab in the panel is scoped to focus, not to "is it open"
2e5c9262  design       the other half of D-31 — the preview's chrome → sans (D-33)
c9cff940  fix(tabs)    open a history chat already named, not "New Session" (D-34)
d0257a6a  design(tabs) set tab labels in the app's own sans — the D-31 code
427ea3d4  docs         EXECUTION.md — the suite is 1133/1133
621326ac  test         fix the suite's two real root causes, no timeout crutch
c22710f2  docs         add EXECUTION.md — the single status doc
609849b2  fix(chat)    session-scope the broadcast events so N chats can coexist
751f06db  feat(chat)   tabs — one group, on the shared .br-tab classes
191fbc28  feat(chat)   split the chat area into groups — drag a tab to an edge
76882cd6  docs         R4 measured — N-mount affordable, split unblocked
68fc7a24  design       tab labels are sans, not mono (D-31) — user override
081de99e  fix(theme)   default to auto (follow the OS) instead of light
1c6df30d  feat(ui)     implement the UI cohesion pass
6ca66fe7  fix(ui)      the Biorouter logo was a solid square, not a glyph
c6f7551e  docs         record the cohesion pass; drift register status column
4e6a4121  docs         the chat-groups plan, the R7 spike, the R1 measurement
```

> **Note.** The commit subjects above name `EXECUTION.md`, the file's original
> name. This document is now `execution-status.md`; the commit messages are quoted
> as written.

## Known broken and open

The register for this branch. Resolved items stay listed, with the cause and the
fix, so the record survives.

### Sidebar collapse could not recover — resolved

✅ **Fixed.** Not the cause first guessed. The rail translated correctly at every
width and the toggle always hit-tested itself — it needed a *resize* to reproduce:
`AppLayout`'s auto-collapse effect listed `open` in its deps, so the user's own
click re-triggered it and set `open` straight back to `false` in the same tick. Now
gated on a width-bucket **crossing**. Pure `sidebarAutoCollapseAction` + 8 tests,
because the bug was **state, not geometry** — so it is genuinely provable in jsdom.

### Chat column flush-left, ~158px dead space on the right — resolved

✅ **Fixed, measured 250/250, delta 0** (was 24/158). Two causes: `BaseChat`'s root
had no `flex-1` (it used to mount only in a flex *column*, where stretch gave it
full width for free; a chat *group* mounts it in a flex **row**, where width is the
main axis — so it hugged its content and `mx-auto` centred inside *that*), and
`contentClassName` opened with `pr-1`, a right-only 4px inset. **No jsdom test**,
deliberately: jsdom computes no layout, so a centering assertion there would pass
with the bug present. Verified by measurement in the real app.

### Tab icons at 13/12px, off §3.9's 16/20/24 scale — resolved

✅ **Fixed** — both 16px. Glyphs already came from `app-icons` (stroke 1.5
confirmed *rendered*, which catches a raw `lucide-react` swap).

### History click replaced the chat instead of opening a tab — resolved

✅ **Fixed** — tab per click, deduped across groups, closable via × / ⌘W. **User
override of the VS Code preview-tab design**; the whole preview-tab concept
(`preview` field, `pinTab`, pin-on-run, double-click-to-pin) is retired, with a
test that fails if it creeps back.

### ⌘W closed the window, not the tab — resolved

✅ **Fixed, and it was a landmine.** `{ role: 'close' }` in `main.ts` silently
claimed `CmdOrCtrl+W` with no `accelerator:` line to grep for. A renderer keydown
listener could **never** have won — a menu accelerator is consumed before the web
contents sees the key, so the window would have closed regardless, taking every tab
with it. ⌘W = Close Tab (via IPC), ⇧⌘W = Close Window, per Safari/Chrome. Verified
by dumping the built menu.

### Preview panel is not window-pinned in a split — open

📌 **Known partial, deliberate.** It stays per-pane; in a left/right split it sits
inside the active group's box, not at the window edge. Hoisting it drops the
artifact tab stack on every group switch (plan Stage 5, its own change).

### `KnowledgeProvider` nesting — blocked

🚫 Blocked on the R7 prerequisite fix. The fix was written, could not be
demonstrated with a green test, and was **reverted** rather than ship an unverified
change to a server-write path. See
[Nested `KnowledgeProvider` blocker](../chat-groups/knowledge-provider-nesting-blocker.md).

### The app's own `biorouterd` cannot read this machine's provider secrets — open

📋 **Pre-existing, verified identical on old code** (by reverting).
`provider_restore_failed: XIAOMI_MIMO_` → the route returns `extension_results:
None` → extensions are never attempted → no readiness toast fires. All toast
verification therefore ran against an **external** backend (`EXTERNAL=1`). This is
not caused by progressive loading, but progressive loading is the first feature to
*depend* on that field. Worth its own investigation.

### Extensions are per-session, not global — open

📌 **The deeper cost, and bigger than this branch.** 4 splits = 4 × ~1.0s of
duplicate extension startup, spawning 4 copies of the same MCP servers. Proven, not
inferred: a 4-split window fired **4 separate
`resume(load_model_and_extensions=true)` calls**, and three cold sessions each paid
independently. **Progressive loading hides this latency; it does not remove the
work.** A shared/pooled extension manager is worth more than the reordering — and
is a backend change of real size. Recommended, not built.

### A split created at a narrow width keeps its slivers — open

📌 **Deliberate — D-37.** A 4-up dragged out at 1400px sits at 169px panes and
stays. Rung 4 merges only on a *crossing*, because a watcher that dissolved a split
the instant you made it would be fighting the drop that just happened. If slivers
should never exist, the fix belongs in the **drop** (refuse it, where the user can
see why), not in a watcher that undoes it afterwards. Needs a decision, not a patch.

### No virtualization in the transcript — open

📋 **Count-proven, left alone deliberately.** 4 tabs mount **1211** message
components at once and never unmount them (`ProgressiveMessageList` batches 20 at a
time until *all* are mounted). It does not hurt typing today — 0 long tasks at 4
groups — so fixing it now would be a refactor with no measured delta. **This is the
scaling cliff**: the number that grows is messages × tabs, and nothing currently
bounds it. File it before someone opens a 5000-message chat in four tabs.

### `ChatStreamController` is never evicted — open

📋 Pre-existing leak; tabs make it easier to hit but do not cause it. File
separately.

### `WorkflowsView.tsx:167` still routes to `/dashboard` — open

📋 Removing the titlebar control did not remove the last entry point.

## Still to verify by driving

- **The preview side of ⌃Tab is jsdom-only.** Opening the panel needs a live
  agent turn to produce an artifact. The arbitration is a pure
  `event.target`/`closest()` question, which jsdom models faithfully and two
  mutations caught — but the two listeners have not been proven to coexist in
  the real app.
- **Inherent, not a bug to fix:** with focus inside a preview's *sandboxed
  iframe*, ⌃Tab does nothing — the keydown never reaches the parent document.
  Clicking the panel's chrome restores it. Documented in `tabCycle.ts`.
- **Production-bundle timing numbers were never obtained** — every absolute
  figure in this document is dev-inflated. The ratio and long-task counts are the
  trustworthy parts.
- **4-group scroll shows a 442.9ms worst frame** (3/117 dropped). Real but
  narrow — and **unverified**: the probe's scroller selector was wrong, so this
  is a lead, not a finding.
- ✅ **Resolved.** D-31 / D-33 / D-34 are now confirmed on screen — see
  [the step-20 sweep](#step-20--the-visual-sweep-66-on-screen-and-one-real-bug).
  The caution was warranted: the sweep found a real bug (the traffic-light
  reserve) that every unit test passed straight through.
- **A live agent turn was never exercised** — no provider is configured in this
  environment (`XIAOMI_MIMO_API_KEY` missing; the composer reads "Unavailable").
  The sweep worked around it with saved transcripts, real file links and the
  harness, so **no artifact was produced by a live turn**.
- **Split / multi-group layouts were not screen-verified** — the sweep drove the
  single-group case only. The `firstLeaf` reserve aiming is unit-tested, not
  seen.
- **Widths below 800px are unreachable** (`main.ts` sets `minWidth: 800`). Not
  faked.
- macOS only.

## Evidence from driving the real app

jsdom applies no CSS and computes no layout, so none of this could be proven in
a test. Each was observed:

- the tab strip renders in the 52px header; deep-link `?resumeSessionId=` loads
- **the split works**: drag a tab to an edge → 2 groups, each with its own strip,
  transcript and composer; 2 chat columns mounted
- **the drop zone tints live, mid-flight**: `top → top → center → center → right`
  as the cursor crosses, ghost up, source dimmed at 35%
- prose colours are warm tokens (`<strong>` = `#2a2520`, not `#101828`)
- the tab-strip ground is byte-identical to the sidebar's (`rgb(243,237,225)`)
- dark mode via the real persistence path

### The "pre-existing test failures" were real, and are fixed

They were dismissed as flakes three times. They were not. Two genuine defects in
the shared test setup:

1. **`findBy*` was racing a 1s timeout.** Testing Library's async helpers use
   their own `asyncUtilTimeout` (default 1000ms), which is INDEPENDENT of
   vitest's `testTimeout` — which is why raising `--testTimeout` to 30s never
   helped. Under parallel load the list took >1s to render, so `findByText`
   gave up while the test still had 29s left. Fixed globally with
   `configure({ asyncUtilTimeout: 5000 })`.
2. **jsdom has no `matchMedia` and nothing stubbed it.** Any component reaching
   a responsive hook died. It was being re-stubbed per file — another such
   workaround was added here rather than fixing the root, which was the tell.

### The suite is load-sensitive, and it produced a false diagnosis

A run showed 17 failures, diagnosed as a global ⌃Tab keydown listener swallowing
Escape across the app. The cluster looked damning: menu dismissal, modal
dismissal, focus restore. **That diagnosis was wrong, and the way it was disproved
is the lesson.**

The failing *sets differed between runs*, every failing file *passed in
isolation*, `ChatGroupsProvider` only mounts under `/pair` so most of those
components never had the listener at all — and `uptime` showed **load average
112–166 on 8 cores**, because two heavy agents were driving Electron while a
third ran the suite. At load 7: **1181/1181**. Exactly one of the 17 was real
(the ArtifactViewer ⌃Tab contract, since fixed).

Two things follow, and they generalise:

1. **A coherent-looking story is not evidence.** The hypothesis explained the
   cluster beautifully. It was still wrong. The cheap discriminator —
   *does it fail in isolation?* — costs seconds and would have killed it
   immediately. Run that before theorising.
2. **A green suite under heavy load is not a pass, and a red one is not a
   fail.** `asyncUtilTimeout: 5000` bought headroom, not immunity; at load 166
   nothing saves you. **Check `uptime` before believing a suite result.**

### Bugs only the running app found

1. **The logo was a solid square.** Vite inlines `glyph.svg` as a `data:` URI
   containing `'` and `(`; an *unquoted* CSS `url()` cannot contain those, so
   the tokenizer emitted a bad-url-token and Chrome discarded `mask-image`
   entirely. The mark paints `currentColor` and relies on the mask to cut its
   shape → a solid accent block. Pre-existing; affects every use of the mark.
   **The old test asserted the property was SET and passed** — jsdom's CSS
   parser accepts what Chrome rejects. Green test, broken feature.
2. **The tab read "New Session"** for a loaded chat while `document.title` had
   the real name: nothing announces a session name on *load*, only on rename.
   Fixed twice over — first by listening for the load (which corrected the name
   but only *after* a visible second), then properly, by handing the name over
   at open time (D-34). The first fix treated the symptom: it made the
   correction arrive reliably instead of asking why a correction was needed for
   a string the sidebar was already rendering.

### The browser tab model — ⌘T / ⌘N / ⌘W / ⌃Tab (D-35)

**⌘T was already claimed by the menu**, exactly as ⌘W had been: "Go → New Chat"
held `CmdOrCtrl+T` and merely navigated Home. That is now the *second* time a
menu accelerator silently owned a key the renderer wanted — a renderer listener
could never have won either. Both go through menu + IPC. Worth generalising:
**before binding any key in the renderer, dump the built menu.**
`scripts/debug/menu-dump.mjs` does it, and it confirmed ⌃Tab is claimed by
nothing, so ⌃Tab is an honest DOM listener.

**Verified by driving, 13/13** (`scripts/debug/tab-shortcuts-probe.mjs`): ⌘T
0→1→2 blank tabs each with a composer; a real ⌃Tab keypress moving active 1→0;
⌃⇧Tab back; wrap; ⌃Tab from *inside* the composer (a browser switches tabs while
you type — no text-input guard, deliberately); plain Tab inert; ⌘N 1→2 windows.

**Two bugs this work found, neither of them the feature:**

1. **The preview panel was hijacking ⌃Tab app-wide.** Its listener was gated on
   `isOpen`, not focus — with a panel open it cycled *previews* from anywhere,
   including the composer. **Its test dispatched at `window`, so it passed while
   the bug was live** — theatre, and rewritten to dispatch from inside the panel.
   Both listeners now consult one predicate (`isWithinArtifactPanel`), because
   both listen on `window` in capture and capture order is merely mount order.
2. **Empty-tab adoption filled the *leftmost* blank tab.** ⌘T makes two blanks a
   keystroke away, so the first message landed in the wrong one.

### Lag — measured, and the honest outcome was "don't refactor"

**The tabs/split architecture is not a lag source.** Typing latency
(keydown→paint) with the split at full stretch, on a quiet machine, mount
verified (`groups=4 tabs=4 composers=4 messagesInDom=1211`):

| | p50 | p95 | long tasks |
|---|---|---|---|
| 1 group (355 msgs) | 25.7ms | 33.5ms | **0** |
| 4 groups (1211 msgs) | 35.4ms | 38.7ms | **0** |

**Ratio 1.16×.** That was the headline risk of this branch — mounting N chats —
and it is answered.

**No perf refactor was shipped, deliberately.** Nothing measured indicts app
code. A refactor with no measured delta is a risk, not a fix.

**Two traps invalidated the first round of numbers**, and both generalise:

1. **Machine load.** Same probe, same build: **load 93 → p95 70ms + 41 long
   tasks; load 11 → p95 25.7ms + 0 long tasks.** Contention was nearly reported
   as an app bug — the same trap that produced the phantom "17 failures" above.
   The probe now asserts load and prints it next to every number.
2. **The dev build is not the app.** A CPU profile indicted `jsxDEV` /
   `logComponentRender` / owner stacks — **~646ms of a ~1650ms typing burst is
   React dev-only work that does not exist in the packaged app.** Every
   absolute number here is dev-inflated; the *ratio* is what's trustworthy.

The probe **refuses to emit a number until it has proved the thing under test
is on screen**, and that caught four real voids (including a "production"
number that was silently measured against the dev bundle). This is the direct
answer to the earlier N-mount probe that reported "affordable" while measuring
an empty page.

**The standing gate:**

```bash
cd ui/desktop && node scripts/perf/chat-perf-probe.mjs
```

It needs vite on `:5173`, exits non-zero on breach, and is documented in
`scripts/perf/README.md`. Load-bearing budgets are the **ratio (1.6×)** and
**long tasks (2)** — both near-immune to dev overhead. Absolutes are coarse on
purpose.

### The premise behind progressive loading is confirmed, with numbers

Measured against the real backend on a 355-message session:

| `/agent/resume` | Time |
|---|---|
| `load_model_and_extensions: false` (transcript only) | **0.50s** (warm 0.067s) |
| `load_model_and_extensions: true` (what the app calls) | **5.07s** |
| a *different* session, `true` — i.e. chat #2 | **2.64s** |

The conversation is fetched **first** and then held ~4.6s behind 9 extensions
that contribute **359 bytes**. And extensions are **per-session, not global**,
so every new tab re-pays ~2.5s — a cost tabs and splits multiply.

The user called this from the outside, hedged it ("perhaps"), and was right.

### Progressive loading — built, and it was the user's call

Split `/agent/resume` in two at the **existing** `load_model_and_extensions`
seam: phase 1 (`false`) paints the transcript, phase 2 (`true`) loads the model
and extensions behind it and toasts. No route change, no OpenAPI regen — the
seam was already there; nobody had used it.

**Observed, real app + real backend, n=3, load 8–12:**

| | before | after |
|---|---|---|
| time to transcript | median **2667ms** | median **738ms** |
| time to ready (toast) | median 2667ms | median 2651ms |

**3.6× faster to read, ~1.9s earlier, and readiness arrives no later than
before.** The BEFORE column is the proof of the coupling the user suspected:
paint time **== ready time to the millisecond**. They were literally the same
event.

**The premise was confirmed but the numbers recalibrated.** Same setup (9
extensions, 373 bytes, gating 355 messages), but cold full resume measures
**1.25–1.42s here, not the 5.07s** the perf agent reported — that figure was
most likely taken under machine load. Flagged rather than repeated: **a number
you cannot reproduce is not a number.**

**Submit-before-ready: HELD, not dropped and not blocked.** The message lands in
the transcript, the chat shows Streaming with a live abortController, and Stop
works throughout — so the user sees their message and sees it working. Observed:
submitted at 962ms (2s *before* ready), sent at 3167ms, text intact. Both submit
tests fail when the hold is removed.

**Multi-tab toasts, two rules:** a clean load only toasts for the *focused* chat;
a **failure always toasts and can never be overwritten by a later success**.
Verified in a real 2×2 split: max 1 toast on screen, background successes
silent, a background *failure* still spoke up.

**Two pre-existing bugs found on the way, both fixed, both would have got worse:**

- the transcript LRU cache (process-lifetime, no TTL) could reach a submit
  having **never loaded the agent at all** — a live-looking chat over a backend
  with no extensions;
- `useToolCount` fetches on `sessionId` alone with nothing to re-trigger it. It
  already raced; progressive loading would have made it *reliably* read zero
  tools and cache that forever, silently killing the "too many tools" alert.

**Regression:** a deliberately broken extension (isolated config root) still
painted the transcript at 686ms and toasted *"8 of 9 extensions loaded — Failed:
Ucsfomopagent"* on the correct popover surface. Navigating away mid-load leaves
no unmount warnings, no orphaned stream. The artifact panel, KB chip and model
selector never depended on the old ordering (they derive from the transcript or
from global state) — checked, fine.

### The yield ladder (D-32) — measured at every width

Each rung is a **pure function of (width, state)** in `Layout/yieldLadder.ts`
(34 unit tests), following `sidebarAutoCollapseAction`'s shape on purpose:
these are **state bugs waiting to happen, not layout bugs**, and the sidebar
showed that the testable part is the rule, not the geometry.

| window | sidebar | pane | chat col | preview | tabs | rows | ▾ |
|---|---|---|---|---|---|---|---|
| 1400 | expanded | 1160 | 584 | side, 520 | 5×88 | 1 | no |
| 1120 | expanded | 880 | 464 | side, **360 floor** | 5×88 | 1 | **yes** |
| 1000 | **compact** | 1000 | 584 | side, 360 | 5×88 | 1 | yes |
| 860 | overlay | 860 | **760** | **overlay** | 5×98 | 1 | no |
| 800 | overlay | 800 | 744 | overlay | 5×89 | 1 | no |

**It reverses exactly** — the same numbers at every width on the way back up.
**`rows = 1` in every run at every width: the strip never wrapped**, which was
the one thing the spec forbade outright.

640/760 are absent because they are **unreachable** — the OS window floors at
800 (`main.ts`). Sweeping there would have been three identical rows dressed up
as data.

**Rung 4 composing with rung 1, which is the ladder actually working:** a 3-up
split (needs 1082) at **1400** → 3 groups @289px; at **1300** (shell 1060) →
**merged**, chat back to 760; at **1100** → the sidebar collapses, which hands
the shell 1100 back → **the split is restored**. Rung 1 yielding buys back the
room rung 4 needed, *while the window is still shrinking*.

**A bug rung 3 exposed:** the preview panel's strip was `overflow-hidden` — its
tabs never shrank-then-scrolled at all; past the floor they were **clipped and
unreachable**. That is squarely what the user asked about ("if certain elements
don't fit, then collapse the element"), so rung 3 covers both strips through a
**shared hook** — the spec's claim that both strips behave alike is only true
if the *rule* is shared, not merely similar. An existing test had pinned the
clipping as if it were intended; it was flipped deliberately, not quietly.

**Mutation: 9 run.** M4 (remove the crossing gate) killed 4 — including *"keeps
a split the user made BY HAND"*. **M7 initially SURVIVED**: a "leaf order" test
was decorative, because the fixture's object order accidentally matched leaf
order. Rewritten with a `left`-split fixture where the two genuinely disagree;
it kills the mutant now. That is the second decorative test found on this
branch by mutating rather than trusting a green tick.

**Perf gate: 1.19× against a 1.6× budget**, p95 33.8/40.3ms on a verified-quiet
box — a ResizeObserver-driven ladder was exactly the kind of thing that could
have regressed typing, and it didn't.

### Step 20 — the visual sweep: 6/6 on screen, and one real bug

Driven in the real Electron app (Playwright, real userData) plus the artifact
harness in Chrome. **Observed values, not impressions.** 144 screenshots.

| # | Item | Verdict | The evidence |
|---|---|---|---|
| 1 | **Fonts** (D-31/D-33) | **PASS** | `.br-tab__label`, **both** strips: `ui-sans-serif, -apple-system, "system-ui", …` @ **13px**, tracking `normal`. The D-33 split, checked item by item: chip "TypeScript" **sans**; path + `app.ts` + "1 line" **mono** (+`tabular-nums`); git ref **mono**; all 5 legend labels incl. "Modified" **sans**. App-wide: **5 of 136** visible text leaves are mono — 4 file paths and the live `$1.97`. Every one earned |
| 2 | **No "New Session" flash** (D-34) | **PASS** | 3 runs, **402 frames sampled**, tab already named at first paint. **0 placeholder frames.** The sampler was proven live by catching each transition |
| 3 | **Artifact panel / code view** | **PASS** | 15 types. The documented Prism/Tailwind collision reproduced: **26 `token table` spans, every one `display: inline`** — the `main.css` guard beats Tailwind. Long line **763ch** (added: the shipped fixtures max 70ch, so they proved nothing): `scrollWidth 6038 vs clientWidth 1179`, `white-space: pre`, line numbers aligned, **0 flex lines** |
| 4 | **Light + dark × parchment + alma-mater** | **PASS** | All four via the real persistence path + reload, 11 tabs mounted each |
| 5 | **Yield ladder 1400→800** | **PASS** | 11 tabs: never wraps (single `tabTops [9]`), nothing clipped, ▾ reachable at every rung, menu lists all 11 |
| 6 | **Collapsed sidebar + traffic lights** | **BUG → FIXED** | below |

#### The bug: the reserve was inside the thing that moves

The 172px traffic-light reserve was `padding-left` **on the scrolling element**.
Padding is scrollable content — so any scroll carried the reserve away with it.
And selecting a tab scrolls it into view, so **this was the ordinary path, not an
edge case.** Measured clip-aware at 800px, sidebar collapsed, 11 tabs,
`scrollLeft: 428` *as found*:

- **before** — the scroll box clips `0..674`: tabs visibly rendered at **x=0**,
  **40..128**, **131..219** — under the traffic lights (which end at 72) and under
  the controls (100–164), each hit-testing to itself
- **after** — clips `172..674`: 6 visible tabs, all ≥172, **zero violations**

A/B'd by stashing the fix and re-running the identical probe.
**`getBoundingClientRect` alone is not evidence here — it ignores clipping**, and
the sweep's first probe was wrong for exactly that reason.

**The unit tests asserted the padding property was SET. It was set. It just could
not hold.** This is the third time on this branch that a green test guarded a
property instead of the behaviour — the logo's `mask-image`, the preview panel's
`window`-dispatched ⌃Tab, and now this. The tests now assert the *structural*
invariant (the reserve lives on an ancestor of the scroll box; the scroll box
carries no left inset), which is the strongest claim jsdom can honestly make.

The spec called this one in advance: *"a reserve that fails silently is worse
than no reserve."*

#### The duplicated `1120` constant, and why it was worth fixing

`BaseChat` re-declared `SIDEBAR_COMPACT_TITLE_WIDTH = 1120` instead of importing
it. In fact **1120 was declared three times** — `AppLayout` (rung 1's threshold),
`TitlebarControls` (the titlebar reserve), and that third copy. All three agreed,
so nothing was broken. **That is exactly why it was worth fixing before it broke:**
rung 1 fires on this width and rungs 2–4 inherit the room it frees, so a drifted
copy would desynchronise the ladder from the chrome *with nothing failing*.

The guard is a **source-text assertion**, deliberately — the defect is textual (a
duplicate literal), and no runtime assertion can see a constant that was never
imported. Mutation-checked: re-introducing a fourth copy fails exactly that test.

The same commit corrected `CHAT_MIN_WIDTH`'s comment, which still carried the
68ch rationale D-36 records as false. **A comment repeating a justification
formally documented as wrong is worse than no comment.**

### Post-sweep: the one preview type the sweep could not see

The step-20 sweep verified the artifact panel across 15 types — but **not the
Jupyter notebook**, because the harness had no `.ipynb` fixture. That gap was
the tell: the user suspected the notebook design was "new and maybe not
implemented," and they were right in the way that matters. A fixture was added
(`preview_fixture_dump.rs` now dumps a real differential-expression notebook
exercising every branch: markdown, highlighted code, stdout, an HTML DataFrame,
text/json results, an `image/png` plot, an error traceback, a raw cell), and on
screen the notebook **had drifted**: a bespoke sticky header (`background-muted`
blur, no path) on a `background-medium` floor that was **lighter than the panel
in dark mode** — a visible seam, a different visual language from the md/csv/code
previews. Fixed to the shared status strip (`NOTEBOOK` chip · mono path · mono
cell count · sans kernel), cells on the panel ground. To stop it drifting again,
`STRIP_LABEL/IDENT/META_CLASS` + `splitPathForStrip` were promoted from
`ArtifactViewer` into the shared `artifactUtils`, imported by both. Verified on
screen in light and dark for every cell/output type. `8630796b` (fixture),
`b4aeea44` (strip). This is the third time on this branch a renderer that
"passed" was wrong until someone looked — the notebook simply had no fixture to
look at.

### Logo studios (separate deliverables, not part of the cohesion pass)

Interactive tuning tools, self-contained, published as artifacts:

- [`logo-icon-studio.html`](../branding/logo-icon-studio.html) — the square `BR`
  app-icon mark (D-38: center the union of letters + underline; size as a share of
  the plate). The user finalized it (mark 70%, offset −20, gap 2%); the assets
  shipped as `ui/desktop/src/images/br-icon-{beige,transparent}.{svg,png}`
  (`ff4635a7`) — **new files, not the live `icon.*`**, which stays the abstract
  glyph until the swap is approved. Note: `sips` flattens font-weight, so the
  rasters are browser-rendered (weight-faithful), not `sips`-rendered.
- [`logo-wordmark-studio.html`](../branding/logo-wordmark-studio.html) — the
  horizontal `BioRouter` wordmark (navy Bio + coral Router, a short split underline
  **between the o and the R**, UCSF-teal `#18A3AC` navy on dark surfaces). Awaiting
  the user's finalized export.

## Dated change log

Entries in chronological order.

### Brand rollout — the BR mark and BioRouter wordmark ship (2026-07-17)

The abstract circle glyph is retired (D-40). The new identity flies everywhere
the old glyph did.

**Assets (D-38 mark, D-39 wordmark, all committed).**

- App icon (`icon.icns`/`.ico`/`.png` + light variants) → the BR mark on the
  cream plate, **inset to the macOS icon safe-area** — the plate is 78.9% of the
  tile (~100px transparent margin), not the 97.6% full-bleed that read
  *oversized* next to native icons like Chrome (the user caught this). Menu-bar
  template → a **monochrome BR**. Rasters are built by resizing a
  **browser-rendered** PNG master, because `sips` flattens font-weight — a heavy
  800 mark comes out medium through the SVG-text path.
- In-app: `<BioRouterWordmark>` (sidebar brand row, replacing the mono glyph +
  the plain "Biorouter" text in one element) and `<BioRouterMark>` (welcome,
  provider setup, suspense loader). Both are **runtime-measured** (getBBox), so
  the underline stays right in any font, and both flip navy → UCSF teal on a dark
  surface (but stay navy on the cream plate).

**Verified on screen, not by unit test** (jsdom has no getBBox):

- the two components rendered in isolation (esbuild + real browser), light and
  dark, transparent and plated — all correct;
- the packaged **arm64 `.app`** launches and the sidebar flies the wordmark;
- the bundled `electron.icns`, extracted, is the BR mark with the safe-area
  margin.

**The bug this surfaced, and why it mattered.** The marks first measured in a
dependency-less `useLayoutEffect`, so they re-measured every render; getBBox
jitters sub-pixel, so the stringify guard never matched and setGeo looped
("Maximum update depth exceeded"). Because these marks mount at the top of the
tree (sidebar, provider guard, suspense fallback), the loop **crashed the whole
app to a blank screen** — which first looked like a dev-server problem. jsdom has
no getBBox at all, so the 1266 unit tests were blind to it; only rendering in a
real engine showed it. Fixed: measure once on mount + on `fonts.ready`. **This
is the fourth time on this branch a renderer "passed" while being wrong until
someone looked** (logo mask, ⌃Tab, traffic-light reserve, now this).

The old wrappers (`BioRouterLogo`, `WelcomeBioRouterLogo`, `BioRouterIcon`) were
unrendered by any live site and were kept as dead code for this pass. They have
since been removed as unused (R-29 of the Codex simplicity redesign). The
`BioRouter` glyph mask (`icons/BioRouter.tsx`) remains, because
`scripts/check-brand-consistency.sh` asserts it references `glyph.svg`; `glyph.svg`
is itself now the BR mark, so even an overlooked mask render shows BR, not the old
circle.

### Follow-on UI and terminal fixes (2026-07-17, on user review)

Eight review fixes, all committed, gates green (**1290/1290**, tsc + lint clean).

**Sidebar / home / tabs** (`fecf5dd3`, verified in the packaged build via CDP):

- **Recents count → past 7 days.** The badge counted the loaded buffer (a paging
  artefact — 170); it now counts chats touched in the last week (20) with a
  tooltip. Shown in both states.
- **"+" new-chat button** at the right end of each chat tab strip (the strip's
  `endSlot`), opening a fresh blank chat in that group.
- **Sidebar divider aligned.** It sat 9px low (band bottom y=60 vs the tab
  strip's y=51); `-mt-2` cancels `SidebarContent`'s 8px top padding so the band
  starts at the window's top edge — the "one continuous top edge" (D-23) it was
  meant to have. Now y=52 vs 51.
- **Default window 1280×900 → 1440×1000** so Home opens with the heatmap AND
  recents above the composer.
- **Home fold order** — recent chats fold first as the window shortens, then the
  heatmap (greeting + composer last). ResizeObserver on the scroll area; jsdom's
  0-height reads as "show all".
- **Drag ghost rides under the cursor** — it tracks the grab point (captured in
  `beginDrag`) instead of pinning its top-left to the pointer while a CSS
  translate shoved it further off. The compounding translate is gone.

**Extension toast** (`faad5c39`): the "x of y" count excludes the 7 bundled
built-ins that always load, so 5 user extensions with one failure reads "4 of
5", not "15 of 16". A *failed* built-in still surfaces. 13 tests.

**In-app terminal → per chat tab** (`db095200`, `e08a4a18`, `c247b214`,
`a5dabdb1`; 21 tests; verified in the packaged build):

- Was one window-global dock. Now `TerminalDockContext` is a map keyed by **tab
  id**; each tab has its own terminal, and it only shows for the active group's
  active tab. Background tabs' shells stay alive (hidden) until the tab closes
  (`retain` disposes the pty). CDP-confirmed: opening a new chat tab shows no
  terminal in it.
- **Opens in the session's working folder** (`session.working_dir`), frozen per
  terminal at open. CDP-confirmed: the pty spawned in `/Users/wanjun/Desktop`.
- **"+" left-aligned**, inside the strip right after the last terminal tab (like
  the chat "+"). CDP-confirmed: `plusInStrip`, `plusAfterLastTab`.
- **Cmd+T is focus-aware** — a new terminal pane when the terminal is focused
  (`isTerminalFocused()` via the dock's `data-testid`), a new chat tab when the
  chat is (unchanged). The menu-accelerator-keeps-focus path is unit-tested;
  end-to-end keystroke routing is the one item left for a manual check.

**Process note this round:** the dev server's Electron served **stale cached
modules** on reload, so a change (the chat "+") looked broken while its code was
correct — proven by rebuilding the packaged (production) bundle and driving it
over CDP, where every change rendered. When HMR lies, the packaged build is the
source of truth.

### Brand font → Inter, and a re-tune (2026-07-18)

**Why the font changed.** The mark shipped in the native UI stack
(`-apple-system` → **SF Pro** on macOS). But Apple's font license forbids SF Pro
"in app icons, logos, or any other trademark use" — legitimate as live UI text,
not as a brand mark, and doubly wrong once baked into cross-platform icon rasters
and the served landing SVG. A native stack also renders the logo in a *different*
face per OS (SF Pro / Segoe UI / Roboto), which a logo must never do.

**The font chosen.** From a 7-font study (all **SIL Open Font License**, which
explicitly permits logos + embedding + outlining), the user picked **Inter** —
the closest to SF Pro's UI face, with the "rounder" feel they asked for. OFL
resolves both problems at once: legal for a logo, and fixed across platforms.

**Re-tuned in the studios** (now set in Inter), exported by the user:

- **Wordmark** — weight 600, letter-spacing 0, underline gap 2%, thickness 10%,
  underline width 100%, vertical offset 0 (word + underline centered as one body).
- **BR icon** — mark size 60% of the icon, vertical offset −10px, underline gap
  2% (cap 262, underline 39 thick, union 306 = 60% on the 512 canvas). The
  wordmark studio gained a **Vertical position** dial it had lacked.

**How Inter ships.**

- In-app: a single latin-subset **variable** Inter `@font-face` is embedded (data
  URI, not fetched) in `main.css` — a deliberate, *logo-only* exception to D-06's
  native-stack rule, used only by `<BioRouterWordmark>`/`<BioRouterMark>`. The
  components now name `Inter` first in their font stack (native stack stays as the
  load fallback); the monogram's plate fill is pinned to the approved 60%.
- Assets: `icon.svg` / `br-icon-beige.svg` / `br-icon-transparent.svg` /
  `glyph.svg` embed a weight-800 Inter `@font-face` so they render Inter without
  it installed. The 1024 PNG masters (`br-icon-beige.png`, `br-glyph-mono.png`)
  were re-rendered through Chromium (still not `sips` — it flattens weight), and
  `prepare.sh` re-propagated every raster: `icon.icns`/`.ico`/`.png` + light
  variants, the menu-bar templates, `landing/icon.{svg,png}` (+ video copies),
  and the CLI `logo_{light,dark}.png`.

The mark is otherwise unchanged — same navy `#052049` "B/Bio", coral `#b85a32`
"R/Router", split underline, and navy → UCSF teal `#18A3AC` on dark. Only the
letterforms are now a font BioRouter is licensed to fly as its logo.

## Decisions the user overrode

Recorded, not argued.

- **D-31 — tab labels: mono → sans.** The original choice borrowed otty.sh's
  mono-for-UI-labels idea and justified it with P6. In the app it read as a
  special thin font that belonged to nothing else on screen — the exact
  incoherence this pass exists to remove. Mono keeps the jobs it earns (code,
  terminal, paths); a tab label is a name. **Mono for data, sans for chrome.**
- **Tab-per-click.** VS Code preview tabs (single click = italic, reuses the
  slot) were deliberate, to stop history browsing leaving twelve tabs behind.
  The user looked at it and wants a tab per click, deduped, closable with ×/⌘W.
- **D-32 — the yield ladder.** The active chat always wins; everything else
  yields in a fixed order: sidebar → preview panel → tab labels → the split
  itself.
- **D-33 — mono for data, sans for chrome.** D-31 was only half the answer: the
  user said the font was still wrong in the chat *and the preview*. The tab
  strips were already fixed (one line covered both — the preview shares chat's
  `.br-tabstrip`), but the preview's status strip still set *everything* in
  mono. The test, applied per-usage: **mono is a claim that the glyphs matter.**
  Paths, git refs and `tabular-nums` counts keep it; the language chip and the
  status legend don't.
- **D-34 — a tab opens already named.** The "New Session" flash was a
  round-trip to the server for a string that was already on screen. The name
  now travels with the click. The late rename stays — deep links, reloads and
  fresh chats carry no name, and only the load knows `user_set_name`.
- **The browser model.** ⌘T = new tab (a new chat), ⌘N = new window, ⌘W =
  close tab, ⌃Tab = next tab — in the preview's tab stack as well as chat's.
  ⌃Tab and not ⌘Tab because the OS owns ⌘Tab and will not give it up; this is
  also what Safari and Chrome do on macOS.

## What is not built, and why

Each has a reason, not a shrug.

- **Splitting past 4 groups** — `MAX_GROUPS=4` is the edge of the R4 evidence
  (measured with small windows and short transcripts), not a memory cliff.
- **⌥-duplicate a tab** — two tabs on one sessionId = two composers submitting
  into one turn.
- **Harvesting Dashboard's `createdHere` delete-on-close** — combined with
  preview tabs, browsing history could DELETE sessions. The judge called it the
  highest-consequence bug in the packet.
- **The artifact-panel hoist** — see
  [Preview panel is not window-pinned in a split](#preview-panel-is-not-window-pinned-in-a-split--open).

## Related documentation

- [UI cohesion redesign spec](ui-cohesion-redesign.html) — the approved visual
  spec this branch implements, with a Current ⇄ Redesigned toggle.
- [UI cohesion redesign, the written spec](ui-cohesion-redesign.md) — the
  reasoning behind each tagged change; read it when you have no browser.
- [Chat-groups design judgement and plan](../../history/chat-groups/design-judgement-and-plan.md) —
  the three candidate designs, the adversarial judge, and the shipped subset that
  steps 8–10 above execute.
- [Nested `KnowledgeProvider`: the chat-groups nesting blocker](../chat-groups/knowledge-provider-nesting-blocker.md) —
  the R7 spike behind the one blocked item in this branch's register.
- [BioRouter design system](../../../design.md) — the D-NN decision register
  (Part 6 / 6b) and the drift register (Part 7) that this document cites throughout.
- [BioRouter logo and wordmark specification](../branding/logo-and-wordmark-spec.md) —
  the normative geometry and colour for the marks whose rollout is logged here.
