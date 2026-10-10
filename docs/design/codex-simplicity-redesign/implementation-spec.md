# Codex simplicity redesign: implementation spec

> **What this is.** The implementation specification for the Codex simplicity redesign: design
> principles, tokens, control specs, one section per workstream with file ownership and
> acceptance checks, sequencing, verification and the owner decisions. Synthesized on 2026-10-09
> from the owner's requirements, three research reports and fifteen audits, then checked by a
> critic pass (section 7) and by the coordinator (section 8).
> **Status:** Current. Implementation in progress; see [progress.md](progress.md).
> **Audience:** implementation agents (one per workstream), the coordinator and the vision
> reviewer.

The research reports and audits it cites (`[codex-ui]`, `[shell-sidebar]` and so on) live outside
the repository in the coordinator's run directory, `~/biorouter-runs/redesign-2026-10-09/`. The
requirement IDs are defined in [requirements-and-plan.md](requirements-and-plan.md).

Worktree: `/Users/wgu/Desktop/BioRouter/.claude/worktrees/crew-landing-redesign-91b46e`
(branch `claude/crew-landing-redesign-91b46e`, base `f2ff06132`). All code paths are relative
to `ui/desktop/src/` unless they start with `ui/desktop/`, `landing/`, `docs/` or `crates/`. The
`main.css` line numbers are the ones at `f2ff06132`. They are a guide only, so grep the
selector. Inputs live in `/Users/wgu/biorouter-runs/redesign-2026-10-09/{research,audit}/*.md`.
This spec cites them as `[codex-ui]`, `[shell-sidebar]` and so on.

**The in-repo record.** The owner's requirements are also restated in the repo as `R-01` to
`R-24` in `docs/design/codex-simplicity-redesign/requirements-and-plan.md`, with a live tracker in
`progress.md` beside it. R-24 makes keeping both current a requirement, and that plan says "the
final file ownership is recorded in progress.md". The coordinator owns
`docs/design/codex-simplicity-redesign/**`: it copies section 3.17 (with the section 3.19 splits)
into `progress.md` when wave 0 starts, and updates each R row at every checkpoint. Every commit
message names the R-IDs it serves.

### Requirement trace (owner message or R-ID → where this spec answers it)

| Requirement | Spec |
|---|---|
| M1 / R-01, R-02 simplicity, remove text | Principles 1, 2; every workstream |
| M1 / R-05, M12 compact sidebar, measured density | 3.4; acceptance measured by a browser test (5.7) |
| M1 / R-04 one family, few sizes | 2.1, 3.1, the type probe (5.3) |
| M1 / R-09 motion: searching tabs, opening tabs, changing elements, scaling, opening and closing windows, previews | 2.4, 3.16 (search, tabs, windows), 3.8 (previews), 3.5 (rail, glide); "searching tabs" mapping in 3.16 |
| M1 / R-08 docked summary rail | 3.5, decision 6.3 |
| M1 / R-10 preview panel | 3.8 |
| M1 / R-11, screenshot 4 usage page | 3.9 |
| M1 / R-23, M5 vision loops | 5.4 |
| M3 / R-06 group and sort | 3.4 view options |
| M4 / R-07 right-click Rename plus the other actions | 3.4 row menu and inline rename; History uses the same menu |
| M5 / R-12, R-14, R-15, R-17, R-18 chat interface, Crew as reference, views, Settings, one toggle style | 3.6, 3.7, 3.10 to 3.14, 2.6 |
| M5 / R-03 hover help | InfoTip (2.6), principle 2 |
| M6 / R-16 Knowledge, same logic | 3.12 |
| M7 / R-19, R-20 icons, chat kinds | 3.3 |
| M8 / R-13 composer | 3.7 |
| M9 / R-21 tool-call glyphs | 3.3 tool ladder |
| M10 / R-01 keep Biorouter's accent | Principle 7 zest list (now including the lock badge) |
| M11 / R-21 specific glyphs for built-in extensions only | 3.3 (external extensions draw the Puzzle) |
| M12 sidebar density as a hard criterion | 3.4 acceptance, 5.7 browser test |
| R-22 landing site | 3.15 |
| R-24 records | The paragraph above |

---

## 0. Decisions this spec makes where the reports disagree

| Question | Reports | Decision | Why |
|---|---|---|---|
| UI face | typography §7.5, REQUIREMENTS | **Settled by the owner on 2026-10-09: keep the website's Arial-first stack** (`--font-body` = landing `--font`). Inter is used only for the wordmark and BR mark. There is one mono stack. | The owner made this decision. One consequence: Arial renders only 400 and 700, so 500 equals 400 and 600 equals 700 [typography §5]. |
| Visible text sizes | typography (12/13/14/17/24), codex-ui (12/13/14/20 plus 28), claude-usage (12/13/14/20) | **12 · 13 · 14 · 17 · 24.** Retire 10, 11, 15, 16, 19, 20, 28 and 30 in the app. | Crew runs on 12/13/14/17. 24 is the one large step (the Home greeting, the document h1, stat values). Dropping 20 keeps the count at five. |
| Sidebar row height | crew-reference and shell-sidebar (32px Crew rail) vs codex-ui and claude-usage (28px) | **28px rows, 13px text, 2px gap**, for the app sidebar only (new `--row-height-nav`). Every other part of the row follows Crew's recipe. | The owner asked for a "compact, Codex-like" sidebar (messages 1 and 3). Crew keeps its 32px rail and is not touched. This is owner decision 6.1. |
| Section labels | codex-ui (no caps anywhere) vs Crew and settings-a/b (caps label) | **One caps style survives, for page section labels only**: `text-caps` is repointed to 12/16/500 with +0.04em tracking. Sidebar group headers, menu group labels and bucket labels are sentence case at 12px. | Crew and Settings use caps section labels. The token change reaches Crew's 7 labels as a 1px change [typography R3]. Codex-style lists stay sentence case. |
| Summary rail vs width tween | preview-panel §5.3 (port Crew's width tween) vs motion §0.2 and chat-summary §3.6 (grid snaps, FLIP) | **The grid snaps and the transcript and composer cells FLIP.** Only `transform` and `opacity` animate. Crew's width tween stays inside Crew. | The chat transcript is not virtualized, so a width tween re-lays out every frame. Codex's jitter bugs 22860 and 23245 come from this. `measures.test.ts:739-746` already pins "nothing geometric transitions". |
| Overview / Models control | claude-usage and primitives (segmented) vs usage-home (underline Tabs) | **Underline `Tabs` for Overview/Models, `SegmentedControl` for the range.** | Panel switching keeps tab semantics (D-07, Crew's pane). A range is a value, so it is a radiogroup. |
| Help primitive name | InfoTip / HelpTip / HelpHint | **`InfoTip`** (`components/ui/info-tip.tsx`) | Four reports used this name. |
| Primary buttons | codex-ui (ink-filled) vs design.md D-01 and message 10 (coral) | **Coral stays.** Only one coral-filled button per view. | The owner said: "retain the color accent". |
| Focus indicator | codex-ui (2px ring) vs design.md D-15 (surface shift plus inset edge) | **D-15 stays** unchanged. | It is a signed-off decision, and nothing in the requirements reopens it. |
| Menu row height | primitives (28px) vs current (32px) | **32px menu rows at 13px**, one recipe. The check and the value sit at the trailing edge. | Crew's menus are 32px. Changing them would re-skin Crew. |
| Extension provenance strings | views-b (move to tooltip) vs settings-b and privacy-tiers.md §13.5 (keep visible) | **Keep the three §13.5 strings visible.** The built-in sentence moves into the badge's tooltip. | A privacy mandate cannot become tooltip-only (owner decision 6.7). |
| Page headers | current (24px title, paragraph, action strip) vs views-a, views-b and crew (44px band) | **A 44px band with a 14px `<h1>`**, actions on the right and the description in an InfoTip. This reverses the 2026-09-07 "actions on their own line" decision, and the reversal is recorded. | Crew's channel header is the reference (message 5). |
| Markdown code blocks and tables (critic) | chat-interface C4/C5 (no header bar, booktabs) and preview §5 vs crew-reference §4.4 ("the code block with a head row, the r8 table") | **Copy Crew's recipes** (`crew/timeline/timeline.css:481-553` code block, `:594-647` table) as shared `br-md-*` classes. Code block: `--background-well`, 1px `--border-subtle`, radius 12, a head row (language in 12px muted, Copy and Run as 24px icon buttons) over a hairline, body 10px 12px mono 13/20. Table: 1px `--border-subtle` frame, radius 8, top rules only, header row `--background-muted` at 12px muted 600, cells 8px 12px (Crew's 11px 16px is off-token, D11), tabular numbers, no bold first column. **The same table recipe serves chat, documents, CSV and notebook DataFrames**, so the app, including Crew, has one table look. Chat headings stay 17px (2.1): a deliberate divergence from Crew's body-size headings, because chat answers are documents. | Message 5 names Crew as the reference for the chat interface; the earlier choice would have left two table and code-block looks in the app, one in Crew and one everywhere else. |
| Help, motion and control names (critic) | spec principle 9 vs motion §3.1; WS-SETTINGS-B ("keep Toggle …") vs 2.6 ("no Toggle …") | Principle 9 regains motion's named exceptions and the pointer rule; a switch is named by its visible row label everywhere (2.6). | The spec contradicted itself in both places. |

---

## 1. Design principles (binding)

Codex supplies the structure and the restraint. Crew is the in-house reference for every recipe.
Biorouter keeps its personality: the coral accent, the warm ink, the navy wordmark and the BR
mark.

1. **One reading column, quiet chrome.** Every surface has one 44px band (`--chrome-height`)
   holding a 14px title on the left and a cluster of 32px icon actions on the right. Content sits
   in the 760px measure (`--measure-chat`). Pages have no hero headers or subtitle paragraphs.
   Home (the Hub) is the one exception: it has no band, only the greeting. **Every band is a
   window-drag region** (`-webkit-app-region: drag`) and every interactive thing in it (buttons,
   inputs, triggers, tabs) is `no-drag`, because the band replaces the 32px drag strip on its
   route (3.10).
2. **Say it once, then hide the explanation.** A visible line is a label, a value or a state
   word. Explanations live in an `InfoTip` that opens on hover, on Tab focus and on click, and is
   linked by `aria-describedby`. **These always stay visible:** errors, refusals, privacy
   disclosures (DR-17, §13.5, locked Settings copy), the consequence of a destructive action at the
   moment of decision, and empty-state one-liners.
3. **Five sizes, two visible weights, two families.** The sizes are 12, 13, 14, 17 and 24. The
   weights are regular (400) and strong (600, which Arial draws as bold). The 500 weight is
   declared on `label`, `chip` and `caps` but must never be the only signal of a state. Mono is for
   machine strings only (code, paths in tooltips, IDs, versions, terminal text). Counts, dates,
   costs and percentages use the sans face with `tabular-nums`.
4. **Rows, not cards.** Anything enumerable is a hairline row list. A row has one primary line
   and at most one secondary line. A card is reserved for something that asks the person to act
   (an approval, an elicitation, a secret request) or for a standalone object (an artifact card,
   the usage card, the summary card). A box never sits inside a box.
5. **Each job has one control, and each control has one look.** Use a `Switch` when the change
   applies now, on the right of the row. Use a `Checkbox` to pick items or for an option submitted
   with a form, on the left of its label. Use a `SegmentedControl` for 2 to 4 short options that
   need no explanation. Use a select menu for 4 or more options, or for any options that each need
   a line of explanation (the line is a muted second line inside the menu item). Radio rows appear
   **only inside dialogs and forms** (the Export dialog's "Launcher" and "Full"), never on a page.
   Use a trailing check inside menus. Use underline `Tabs` for sections.
6. **Actions appear when wanted.** Row actions (a primary icon plus `⋯`) show on hover, on
   `:focus-within` and while their menu is open, and they are always shown under `(hover: none)`.
   Right-click, Shift+F10 and the ContextMenu key open the same menu as `⋯`. Only opacity changes,
   so the actions stay in the tab order.
7. **The accent marks state and adds zest, nothing else.** Coral appears only on: the 2px
   active-row rail, the focused field and composer edge, the single primary button of a view, a
   ready Send button, the on-state of switches and checkboxes, the active tab underline, the
   in-progress to-do glyph, the composer's working sweep, links, text selection, the heatmap ramp,
   "New" or unread marks, **the lock badge on private chat glyphs** (coordinator decision on the
   icon report; message 12 "keep … the coral lock badges"), the Update row's download glyph, and
   the landing site's coral rule segment. Hover is always neutral. Do not grey the app out
   (message 10).
8. **Borders come before shadows.** Use a 1px `--border-subtle` hairline plus a step of ground
   colour. Shadows are allowed only on menus, popovers, toasts, dialogs and the composer
   (design.md P1). Hairlines are straight: no `border-bottom` on a rounded row. Use Crew's inset
   hairline.
9. **Layout snaps and content glides.** Animate only `transform`, `opacity` and `clip-path`, and
   use tokens only. The named exceptions, and no others [motion §3.1]: the Disclosure's height
   (`.biorouter-disclosure-panel`, which the tool rows reuse), the Progress fill's width,
   absolutely positioned indicators that move no other box (the segmented thumb, the tab
   underline), the long-message `max-height`, the sidebar's own gap width, and Crew's pane width.
   Never animate against a resize of the window, a split, the sidebar or the preview. A surface
   that mounts because of navigation, a reload, a tab switch or a history load appears at rest,
   and so does the **first placement** of a sliding indicator. **Never move a row under the
   pointer**: a live re-sort or insert that would shift rows beneath a hovering pointer waits until
   the pointer leaves the list. Reduced motion is honoured in CSS, in JS and in the main process.
10. **Show names, not IDs.** Machine IDs and file paths go behind a "Copy …" menu item or a
    tooltip, never on an everyday row.
11. **When unsure, copy Crew, and never edit Crew to match.** Promote Crew's recipes into
    `main.css` or `components/ui/` under `br-` names. Never borrow a `crew-*` class outside
    `crew/`, and never name a `br-*` or `biorouter-*` class inside a Crew stylesheet (enforced by
    `crew/crewCss.sourceGuard.test.ts`).
12. **Simplicity never costs accessibility.** Everything that is hidden must stay reachable by
    keyboard and by screen reader. The contrast gates (`npm run check:contrast`) must pass, and so
    must the new 3:1 control assertions in section 2.6.

**Copy rules.** These apply to every workstream:
- Sentence case everywhere, and the noun is "chat" (never "conversation").
- Use "Biorouter" and "BAAM", the typographic ellipsis "…", and American spelling
  (`test/uiCopySpelling.test.ts`).
- No em or en dashes as punctuation, and no marketing or AI-sounding words.
- One-line menu descriptions and labels take no trailing period. InfoTip text is written as full
  sentences, at most two of them (about 160 characters).
- Each area keeps its strings in a `copy.ts`, and tests import those strings (Crew's pattern).
  **Three exceptions, each read by a checker as a literal:**
  - the nav rows in `BioRouterSidebar/AppSidebar.tsx` stay written as `label: 'Home'` and so on in
    that file. `landing/scripts/check-consistency.mjs:185-194` greps `label: '([^']+)'` there,
    fails below 8 matches, and requires each match in `landing/app-mockups.js`. So never move them
    into `copy.ts`, and never add any other `label: '…'` literal to `AppSidebar.tsx` (write menu
    options in `SidebarViewMenu.tsx`);
  - the labels in `tests/e2e/helpers/sidebar.ts` (`COMPONENT_LABELS`);
  - any app string that `scripts/check-crew-manual.mjs` reads byte for byte from a Crew `copy.ts`
    (for example the "dialling" sentence, `crew/onboarding/copy.ts:608`, pinned at
    `check-crew-manual.mjs:1111-1119` and quoted in `docs/crew/connections-and-troubleshooting.md:56`).
- Strings the daemon serves and the locked privacy copy are outside these rules. Product names
  (Agent Drafter, Auto Visualiser, Biorouter Copilot, Chat Recall) keep their capitals.

---

## 2. Tokens and primitives

### 2.1 Typography (owner WS-TOKENS; `styles/main.css:52-170`)

Families. These are settled and must not change:
- `--font-body`, together with its aliases `--font-heading` and `--font-sans`, is the Arial-first
  stack. It is byte-equal to `landing/site.css:50` `--font`.
- `--font-mono` (`main.css:1493`) is pinned to xterm by `styles/fontStacks.test.ts`.
- Inter is used for the mark only.
- Three stray stacks must be set to the body stack:
  - `artifacts/DocumentPreview.tsx:436`, which gets the resolved string because it renders in an
    iframe. Owner: WS-PREVIEW.
  - `dragGhostWindow.ts:133`. Owner: WS-MOTION.
  - `knowledge/graph/graphStyle.ts:47`. Owner: WS-KNOWLEDGE.

Roles after this redesign:

| Role (utility) | Size / line height | Weight | Tracking | Use |
|---|---|---|---|---|
| `text-supporting` | 12 / 16 | 400 | 0 | Metadata, times, counts, helper text, tooltip text, sidebar bucket and group labels, menu group labels, table heads, footnotes |
| `text-chip` | 12 / 16 | 500 | 0 | Badges and chips (sentence case) |
| `text-caps` | **12 / 16** (was 11) | 500 | **+0.04em** (was 0.08em), uppercase | Page section labels only (Settings sections, view group labels, Crew) |
| `text-secondary` | 13 / 18 | 400 | 0 | Sidebar rows, menu rows, tab labels, segmented labels, dense controls in strips of 36px or less, tool-call rows, table cells, composer controls |
| `text-code` | 13 / 20 mono | 400 | 0 | Code and terminal text |
| `text-body` | 14 / 20 | 400 | 0 | UI prose and values |
| `text-label` | 14 / 20 | 500 | 0 | Control text, row titles, band titles, field labels |
| `text-prose` **(new)** | 14 / **21** | 400 | 0 | Chat markdown prose, document prose and Crew messages. Repoint `.biorouter-markdown` from 1.7 to 21px. Crew's `.crew-md` is already 21px, so it does not change. |
| `text-subheading` | 17 / 24 | 600 | 0 | Dialog titles, empty-state titles, markdown h1 and h2 in chat, markdown h2 in documents |
| `text-title` | 24 / 32 | **400** (was 600; owner decision 6.2) | −0.01em | The Home greeting, the document h1, the preview front-matter title, stat values (paired with `tabular-nums`) |
| `text-heading` | aliased to `text-subheading` | | | It has 1 CSS use (`main.css:5680`). Move it, then delete the role and its entry in `utils.ts:36-48`. |
| `text-display` | aliased to `text-title` | | | It has 2 uses (`settings/app/UpdateSection.tsx`, `AppSettingsSection.tsx`). Move them, then delete the role. |

Mapping for call sites. Each workstream sweeps its own files, and about 85% of the swaps change
no pixels [typography §2.4]:

| Current | Target |
|---|---|
| `text-xs` | `text-supporting` |
| `text-xs font-medium` | `text-chip` |
| `text-sm` | `text-body` |
| `text-sm font-medium` | `text-label` |
| `text-sm font-semibold` | `text-label font-semibold` |
| `text-base`, `text-base font-medium` | `text-body` or `text-label` |
| `text-base font-semibold`, `text-xl` | `text-subheading` |
| `text-2xl font-semibold tracking-tight` | `text-title` |
| `text-[13px]` | `text-secondary` or `text-code` |
| `text-[10px]`, `text-[11px]`, `text-[12px]` | `text-supporting` (or `text-chip`) |
| A hand-rolled caps recipe (`uppercase tracking-*`) | `text-caps` |

Also delete these:
- every `tracking-*` utility except the ones that come from a role;
- `uppercase` placed beside `text-caps` (`ui/badge.tsx:47,73,89`, `ui/BuiltInBadge.tsx:20`);
- `font-bold` and CSS `700`, which become 600;
- `leading-*` where it only restates the role.

`body` defaults to 16px (`main.css:1597-1600`). Moving it to `var(--text-body)` is P2 and is the
**last** typography change. It needs a visual sweep, because KaTeX and any element without a size
inherit it [typography R5].

**Markdown headings** (`main.css:4287-4345`, WS-TRANSCRIPT):

| Context | h1 | h2 | h3 to h6 | Prose |
|---|---|---|---|---|
| Chat | 17/24 600 | 17/24 600 | 14/21 600 | 14/21 400 |
| Document (`[data-variant='document']`) | 24/32 400 | 17/24 600 | 14/21 600 | 14/21 400 (was 15) |

Remove the following from markdown:
- the 3px coral bar and indent on h1 and h2 (`main.css:4307-4321`);
- the tinted blockquote, which becomes a 2px `--border-strong` left edge with 12px left padding,
  muted ink and no fill;
- the `-0.015em` tracking.

When a front-matter title exists, demote the first `# H1` to the h2 style.

### 2.2 Spacing, rows, bands, radii (WS-TOKENS)

| Token | Value | Use |
|---|---|---|
| Spacing scale | 2, 4, 6, 8, 12, 16, 20, 24, 32 | 8 is the default gap and 16 is the default inset (Crew §1.14). Gaps and insets **between boxes** use no other values. Exempt, because they are derived rather than chosen: padding that centres content in a token height (for example `(40 − 20) / 2 = 10px` in a settings row), hairline and indicator offsets (the 7px rail inset), negative hit-box margins, the heatmap's 3px cell gap, and values copied verbatim from a Crew recipe (the code block's 10px 12px body), which the parity test keeps equal to Crew's. |
| `--chrome-height` | 44 | Every band: the sidebar titlebar band, the chat header, the artifact strip, page bands, the Knowledge band and pane headers. **The three shell bands move together or not at all** (CLAUDE.md). |
| `--dock-height` | 36 | Dock strips (terminal, Knowledge facet strip) and the usage card header row |
| `--tab-height` | 32 | Tabs in strips (unchanged) |
| `--row-height-nav` **(new)** | 28 | App sidebar rows only |
| `--row-height-rail` | 32 | Crew rail and menu rows (unchanged) |
| `--row-height` | 40 | Content and settings rows (unchanged; A-03's move to 36px is not part of this pass) |
| Controls | 24 (`compact`) / 28 (`sm`) / 32 (`md`) / 36 (`lg`) | Unchanged |
| Icons | 14 (chip and inline help glyph) / 16 (rows and controls) / 20 (banners and empty states) | Amend the `--icon-chip` comment (`main.css:371`) to allow "inline help glyph" |
| Radii | 4 inner / 8 element / 12 container / 16 surface (the preview sheet only) / full | Unchanged. A nested radius is `outer − padding` (so a segment is 6px). `rounded-2xl` is banned outside `artifacts/`. |
| `--measure-chat` | 760 flat | Unchanged (pinned by `measures.test.ts:202-222`) |
| Sidebar width | 216 to 360, default 288 | Unchanged (`ui/sidebarWidth.ts`; `minWidth` 1048 = 288 + 760) |

### 2.3 Colour and contrast additions (WS-TOKENS; light and dark in every family)

- `--border-control`: the resting edge of every input, checkbox, radio, select trigger and the
  switch knob ring. It must reach **at least 3:1** against `--background-default` and
  `--background-card` in all six scopes. Start from ink at 50% and measure. It replaces
  `--border-emphasized` (1.6:1) on Checkbox, Input and Select [primitives §3 item 2].
- `--control-track-off`: the off track of the Switch. Start from
  `color-mix(in oklab, var(--text-default) 24%, var(--background-default))`.
- Dark `--heat-0` gets one step lighter, because zero-cells are invisible on `#131312`
  [baseline §7]. Set it in each `themes/*.theme.mjs` and keep the rest of the ramp unchanged.
- Add assertions to `scripts/check-contrast.mjs`:
  - `--border-control` against the default and card grounds is at least 3:1;
  - the off knob ring against `--control-track-off` is at least 3:1;
  - the on knob against `--background-accent` is at least 3:1;
  - `--heat-0` against the card ground is at least 1.3:1 (a visible step);
  - the lock badge ink (`--text-accent`) against `--sidebar`, `--sidebar-active` and
    `--background-default` is at least 3:1 (WCAG 1.4.11; it carries the privacy state).
- **The checker reads hex only.** `resolveHex` (`scripts/lib/theme-tokens.mjs:126-129`) returns
  null for anything but a 6-digit hex, and `assert` then records `UNRESOLVED` as a failure. So
  `--border-control` and `--control-track-off` are written as **hex literals** (they are neutrals,
  so declare them once in the hand-authored base `:root` and `.dark` blocks, and confirm no family
  block needs its own). "Ink at 50%" and the `color-mix(...)` above are starting points for
  choosing the hex, not the declaration. If a translucent form is ever needed, author it as
  `color-mix(in srgb, …)` and assert it with `blend()` so the check composites the way the browser
  does (oklab mixing would not match `blend`).
- Any new colour token that a Tailwind colour utility names (`bg-…`, `border-…`, `ring-…`) needs
  a `--color-*` mirror inside `@theme inline`, or the utility silently does nothing
  (`npm run check:tokens`, `scripts/check-token-mirrors.mjs`). Prefer authored CSS for these
  controls, as the rest of this spec does.

### 2.4 Motion tokens (WS-TOKENS; `main.css:261-300`, [motion §3.2])

| Token | Value | Use |
|---|---|---|
| `--dur-fast-min` | 95ms | Row and menu-item hover, row-action reveal, tooltip exit |
| `--dur-fast` | 125ms | Control state changes; **every exit of a small surface** (menu, popover, toast, crossfade-out, preview close) |
| `--dur-fast-max` | 175ms | Entrances of small surfaces, tab enter, slide and settle, rail content, crossfade-in, chevrons |
| `--dur-med-min` | 250ms | Dialog enter, progress fill, side↔stack crossfade |
| `--dur-med` | 300ms | Panels: preview open, rail open, the conversation glide, sidebar open, disclosure open |
| `--dur-slow` | 525ms | The period unit for ambient loops only (`calc(var(--dur-slow) * n)`) |
| `--ease-out` | `cubic-bezier(0.24, 1, 0.4, 1)` | Everything |
| `--ease-spring` | `cubic-bezier(0.34, 1.56, 0.64, 1)` | Transforms of physical gestures only: tab slide and settle, drag lift, the segmented thumb's `translate` |
| **Delete** | `--dur-med-max` (0 uses), `--ease-in`, `--motion-fast/-base/-slow` | Delete them in wave 2 only, after every owner has swept its call sites. There are 29 files, listed in [motion §2.1]. |

Constants used in rules: pop-in scale 0.97 (dialogs 0.98); travel 4px (menus, row insert), 8px
(crossfade rise, cover pane, toast), 32px in and 16px out (preview body); stagger 30ms with at
most 5 items animated; the resize settles 180ms after the last event.

New files and rules owned by WS-TOKENS:
- `styles/motion.ts` exports `DUR`, `EASE_OUT`, `EASE_SPRING`, `prefersReducedMotion()` and
  `isWindowResizing()` (true while `body.biorouter-window-resizing` or
  `biorouter-sidebar-resizing` is set). Add `styles/motion.test.ts`, which pins every number to
  `main.css`. It replaces the four copies of `prefersReducedMotion` [motion I7]. Each owner swaps
  its own copy.
- The resize rule goes in the global motion area near `main.css:2000`, **not** in the rung-2
  block:
  `body.biorouter-window-resizing [data-motion-layout], body.biorouter-sidebar-resizing [data-motion-layout] { transition-duration: 0ms !important; animation: none !important; }`
- Shared motion classes go next to the existing `@keyframes appear` (around `main.css:2012`).
  Every animated rule gets a reduced-motion rest:
  - `.br-enter`: opacity plus `translateY(4px)` to none, 175ms. This is the one transcript and
    list-row entrance.
  - `.br-crossfade` (stack in one grid cell): the incoming layer rises 8px and fades in over 175ms,
    and the outgoing layer fades over 125ms.
  - `.br-highlight`: a `--overlay-selected` wash that clears over `calc(var(--dur-slow) * 3)`. This
    replaces `ExtensionsView`'s inline ring.
- Ambient loops use multiples of `--dur-slow`. Set the tool pulse to `calc(var(--dur-slow) * 4)`.

### 2.5 Interaction states (shared by every control)

- Hover is `--overlay-hover` (ink at 5%) and press is `--overlay-pressed` plus the
  `--press-scale` 0.98 transform.
- Focus follows D-15: controls get the surface shift (`.biorouter-focus-surface`), and rows get
  `--background-focus` plus an inset `0 0 0 2px var(--border-focus)`. Tabs take no fill; their
  underline firms from 2px to 3px.
- **Defect to fix (WS-PRIMITIVES):** a focus ring appears after a mouse click on the Components
  disclosure, the summary trigger and "Make workflow" [baseline §4.8]. Programmatic focus must not
  match the visible-focus treatment. Audit `onCloseAutoFocus`/focus return and apply Tooltip's
  Tab gate to `AppTooltipLayer`.

### 2.6 Control specs (WS-PRIMITIVES; `components/ui/`)

**Switch** (`ui/switch.tsx`, one style, no variants):

| Part | Spec |
|---|---|
| Track | 32×20, `--radius-full`. Off: `--control-track-off`. On: `--background-accent`. Disabled: 50% opacity, `cursor: not-allowed`. |
| Knob | 16px, inset 2px on every side in both states, travel 12px, **no growth**. Off: `--background-default` with `box-shadow: 0 0 0 1px var(--border-control), 0 1px 2px rgb(0 0 0 / .12)` (dark: `--text-muted` fill). On: `--text-on-accent`. |
| Hit target | At least 24px tall. When the switch sits in a `<label>` row, the whole row toggles it. |
| Motion | Knob `transform` and track `background-color` over 125ms `--ease-out`. **A value that loads asynchronously does not animate**: mount it with `data-motion-still`, which gives `transition-duration: 0ms` for the first commit after the value arrives. |
| Name | `aria-labelledby` points at the visible label, so **the name is the row's label** everywhere (a settings label, an extension or skill title, a capability name). Names never depend on state. No "Toggle …" and no "Enable …" labels. The sites to change, each by its owner: `settings/capabilities/CapabilitiesSection.tsx:50`, `settings/contexts/ContextsSection.tsx:57`, `settings/brsdk/BrsdkSection.tsx:80`, `settings/extensions/subcomponents/ExtensionItem.tsx:194`, the skills rows, `KBManagerDialog`; `workflows/shared/WorkflowResourcePicker.tsx:181` becomes a checkbox anyway. Update the tests that query names [primitives §6.3], **and the e2e query at `tests/e2e/app.spec.ts:182-184`** (`Toggle ${MCP_EXTENSION_NAME} extension`). Keep the element a Radix `<button role="switch">`: `tests/e2e/spokeagent-skills.spec.ts:141,277` locate `button[role="switch"]`. |
| API | `<Switch checked onCheckedChange aria-labelledby aria-describedby disabled />`. **Delete the dead `variant` prop** and remove `variant="mono"` at about 23 call sites. Each owner removes it in its own files. The primitive accepts and ignores the prop until wave 2, then removes it. |

**Checkbox** (`ui/Checkbox.tsx`):
- 16px visual inside a 24px hit target (keep the existing input-as-target construction), radius
  4px, 1.5px `--border-control` edge.
- Checked: accent fill with a 10px check (stroke 2) in `--text-on-accent`. Indeterminate: an
  8×2px bar.
- The fill transitions over 95ms. The label sits 8px to the right of the box.

**Radio** (`ui/CustomRadio.tsx`):
- A 16px ring with a 6px dot. Same edge token as the Checkbox. The dot scales from 0.6 to 1 over
  125ms.
- Fold the two inlined copies (`settings/mode/ModeSelectionItem.tsx:113-132`,
  `settings/response_styles/ResponseStyleSelectionItem.tsx:50-83`) into it. Those files are owned
  by WS-SETTINGS-A.

**SegmentedControl** (new `ui/segmented-control.tsx`):

| Part | Spec |
|---|---|
| Semantics | `role="radiogroup"` with `role="radio"` items, `aria-checked`, roving arrow keys, Home and End, and one Tab stop. Build it on `@radix-ui/react-radio-group`, which is already a direct dependency with no importers. |
| Track | 28px tall, 2px padding, `--radius-element`, `--background-muted`, no border |
| Segment | 24px tall, radius 6, padding 0 10px, `text-secondary`. Rest ink `--text-muted`; selected ink `--text-default`. Optional 14px icon or 10px swatch. `fit` (content width) or `fill` (equal columns). |
| Selected | One absolutely positioned thumb: `--background-default` plus `--shadow-raised` plus `--inset-hairline`. It slides with `translate` (175ms `--ease-spring`) and `width` (175ms `--ease-out`), a named exception in principle 9. Under reduced motion it jumps. **It never uses coral.** **Placement:** the thumb is measured from the selected segment with a `ResizeObserver` (`fit` mode) or computed as `index × 100% / n` (`fill` mode). Until the first measurement, the selected segment paints its own ground and the thumb is hidden; the first placement never animates (`data-motion-still`), so a mount does not slide in from x=0. jsdom measures 0, so position and motion are checked in a browser test (5.7), not in `segmented-control.test.tsx`. |
| Focus | An inset 2px `--border-focus` edge on the focused segment |
| API | `options=[{value, label, icon?, swatch?, testId?, ariaLabel?}]`, plus `value`, `onValueChange`, `aria-label`, `fill?` |
| Replaces | `BioRouterSidebar/ThemeSelector.tsx`, `ThemeFamilySelector.tsx`, `settings/app/FontSizeSelector.tsx`, the Preview/Raw and Table/Raw toggles in `artifacts/ArtifactViewer.tsx:2570-2590`, `onboarding/InstitutionalSetupCard.tsx:150-171` (P2), the usage range control and Settings' Tool call details control. Keep every existing `data-testid` (`dark-mode-button`, `light-mode-button` and `system-mode-button` are used by `tests/e2e/app.spec.ts:92-94`). Tests that assert `aria-pressed` on theme buttons move to `aria-checked`: `contexts/ThemeContext.browserBridge.test.tsx:83-97`, owned by WS-SETTINGS-A for this edit. |

**Menus** (DropdownMenu, ContextMenu, Command, Select options and composer popups all share one
row; export `MENU_ROW_CLASS_NAME` once):

| Part | Spec |
|---|---|
| Surface | `.biorouter-popover-surface` with `--background-default`, radius 12, 4px padding, **no row gap** (drop `space-y-0.5`), min width 180, max width 320, offset 6, z 500 |
| Row | **32px**, padding 0 12px, radius 8, gap 8, `text-secondary` 13/18. An optional leading 16px icon in `--text-muted`. Highlight `--overlay-hover` over 95ms. Disabled at 50% opacity. |
| Trailing slot | A muted, right-aligned value ("Group by  Date ›"), then a 16px chevron for a submenu **or a 16px check for a selected checkbox or radio item. The check sits on the right: drop the `pl-8` left gutter** (`dropdown-menu.tsx:358-364, 387-391`). Shortcut hints are `text-supporting` muted with no `tracking-widest`. |
| Group label | `text-supporting` 500 muted, sentence case, padding 6px 12px 2px. Replaces the `text-caps` label. |
| Separator | 1px `--border-subtle`, margin 4px −4px |
| Destructive | `--text-danger`, with a highlight of `--background-danger` at 10% (20% in dark) |
| Second line | Rows that need one use a minimum height of 32px and a `text-supporting` muted line. Never use their own padding. |
| Motion | Open: opacity plus `scale(.97)` plus 4px toward the trigger, from `--radix-*-content-transform-origin`, over 175ms. Close: opacity only, over 125ms. Always `--ease-out`. Fix `popover.tsx:32-36` (add origin, ease and side slide) and `context-menu.tsx:66` (add `MENU_EASE_CLASS_NAME`). Replace `zoom-in-95` with 97 and `slide-in-from-*-2` with `-1` in `dropdown-menu.tsx:281, 478`. |
| Chat-row and view menus | Text-only (Crew `ChannelRow` menus). Other menus keep their icons if they already have them, at 16px. |

**Tooltip** (`ui/Tooltip.tsx`, `ui/AppTooltipLayer.tsx`):
- Mount one app-level `TooltipProvider` with `delayDuration={500}` and
  `skipDelayDuration={300}`. Remove `settings/providers/subcomponents/buttons/TooltipWrapper.tsx`.
- **Do not simply delete the per-root provider (`Tooltip.tsx:127`).** Two traps:
  1. Radix throws "`Tooltip` must be used within `TooltipProvider`" with no ancestor provider, and
     most component tests (and the artifact harness) render tooltips with none. Keep a fallback:
     the app-level provider sets a small React context; `Tooltip` wraps itself in a provider only
     when that context is absent.
  2. `ui/sidebar.tsx:428` wraps the **whole app shell** (`data-slot="sidebar-wrapper"`) in
     `<TooltipProvider delayDuration={0}>`. Today each tooltip's own provider shadows it. Once the
     per-root provider goes, the nearest provider is that one, and every tooltip in the app opens
     instantly. WS-SIDEBAR deletes that provider (or sets it to the app values) in the **same
     commit** as WS-PRIMITIVES' change; WS-PRIMITIVES sends the diff.
- Per-root delays stay available as a prop: the InfoTip (200ms) and the sidebar hover card
  (700ms) set their own.
- The surface is unchanged: inverse fill, radius 12 (pinned by two tests), padding 6×8, 12/16.
  Fade in over 125ms and out over 95ms (replaces the literal `duration-[120ms]`).
- `AppTooltipLayer` opens on focus only after a Tab (copy Tooltip's gate at `:63-87`) and gains a
  focus test.
- **Help text never goes in `title=`**, because `AppTooltipLayer` copies `title` into
  `aria-label` and so renames the control [primitives defect 6].

**InfoTip** (new `ui/info-tip.tsx`; the help-tooltip primitive):

| Property | Spec |
|---|---|
| Trigger | A `<button type="button" aria-label="About {label}">` holding a 14px `Info` glyph (`icons/app-icons.tsx:168`). The hit box is 24×24, drawn with a −5px margin so it adds 14px to the line. It sits 4px after the label, centred on the baseline. Ink `--text-subtle` at rest, `--text-default` on hover, focus and open. Radius 4 for the focus fill. Styled in **authored CSS** (`.br-info-tip`), not with new Tailwind strings, because of the `BIOROUTER_NO_HMR` class-scanning trap. |
| At rest | **Visible** in `--text-subtle` (owner decision 6.8). The InfoTip is not hidden until hover. |
| Opens on | Pointer hover after **200ms**. Tab focus, immediately (reusing Tooltip's gate, so a programmatic focus restore never opens it). Click or tap, as a toggle. **Closes on** pointer leave (with a 100ms grace to cross the 8px gap), blur, Escape and scroll. |
| Description | The same text is always rendered in a visually hidden `<span id>`. The trigger carries `aria-describedby` to it, and `useInfoTipId()` and the `id` prop let the explained control point at the same id. So a screen reader hears the help when it reaches the control, and no hover is needed. |
| Surface | The tooltip surface with max width 280px, `text-supporting` at **400**, `text-left`, `whitespace-normal`, `side="top"` with a collision flip, `align="start"` and an 8px offset |
| Content | Plain text of at most two sentences. **No links, no buttons and no bold runs** (inline ink on an inverse ground disappears; see `knowledge/IngestPanel/Dropzone.tsx:179`). Anything longer, or anything with a link, uses `Disclosure` or a `Popover`. |
| `asChild` form | `<InfoTip label="…" asChild><Badge>Legacy</Badge></InfoTip>` makes an existing badge the focusable trigger. This is Crew's `ClassificationBadge` pattern (`crew/channel/ChannelHeader.tsx:41-77`), with its ring drawn as an outline outside the chip. |
| Never | `title=`. It is never the only carrier of essential information (the always-visible list in principle 2). **Never inside a `<label>`** or inside a row whose click toggles a control: a click on the glyph would also activate the label's control. It sits as the label's next sibling, and the row's click-to-toggle handler ignores events from `.br-info-tip`. |
| Tests | `ui/info-tip.test.tsx`: does not open before 200ms, opens after; opens on Tab and not on `focus()`; closes on Escape; toggles on click; the hidden node exists while the tip is closed and is referenced by `aria-describedby`; the name is "About {label}". When text moves into an InfoTip, tests use `toHaveAccessibleDescription(...)`. A sr-only copy also keeps `getByText` passing. |

**SettingRow and SettingSection** (new `ui/setting-row.tsx`, shared by every Settings
workstream and by Crew-like rows elsewhere):

```tsx
<SettingSection title="General" help?="…" action?={<Button variant="ghost" shape="round" …/>}>
  <SettingRow label="Prevent sleep while running" help="The screen can still lock."
              value?={…} status?="Restart to apply" controlId="prevent-sleep">
    <Switch id="prevent-sleep" … />
  </SettingRow>
</SettingSection>
```

- **Row:** `.biorouter-settings-row`, `min-height: var(--row-height)` (40px), padding 10px 12px,
  gap 12px.
  - Left: a real `<label htmlFor={controlId}>` in `text-label`, truncated, then the InfoTip as the
    label's **sibling** (never inside it; see InfoTip "Never").
  - An optional `status` line in `text-supporting` muted sits under the label. It is for transient
    state only: "Restart to apply", "Up to date", "Managed by your organization".
  - An optional `value` in `text-secondary` muted (mono only for versions and IDs).
  - **One** control, at the trailing edge, with `shrink-0`.
  - The row passes `aria-describedby={helpId}` to the control.
- **Section:**
  - A `text-caps` muted label, an optional InfoTip, and an optional right action inset 12px.
  - Block padding 14px; 8px under the header; no `<p>` under the header.
  - Rows sit in `.biorouter-settings-list`.
- **Row CSS** (`main.css:3624-3690` and `3144-3167`; WS-PRIMITIVES):
  - `.biorouter-list-row` and `.biorouter-settings-row` share one recipe.
  - The hairline is **straight and inset by the row radius**, drawn with `::after` (copied from
    `crew/dialogs/dialogs.css:84-115`). It replaces the curling `border-bottom` and is not drawn
    on the last row.
  - Hover is `--overlay-hover` over 95ms (delete the 42% and 38% `color-mix` fork). Padding is
    authored in the class as 8px 12px for list rows.
  - Update `styles/settingsRowHover.test.ts` in the same change.

**RowActions** (new `ui/row-actions.tsx`):

```tsx
<RowActions primary={<IconAction icon={Play} label="Run" onSelect/>} menu={items} />
```

- It renders the hover-revealed cluster: opacity goes from 0 to 1 over 95ms on row `:hover`, on
  `:focus-within` and on `[data-state=open]`, and it is always shown under `(hover: none)`.
- It renders a `⋯` button (ghost round 32, Tooltip "More actions") that opens a `DropdownMenu`.
- The same items become a `ContextMenu` on the row, opened by right-click, Shift+F10 or the
  ContextMenu key. Use a copied `ui/keyboardContextMenu.ts`; **copy it, do not move it**, so Crew
  stays untouched.
- An optional `meta` slot (for example a relative time) shows at rest and gives way to the actions
  on hover or focus.

**Other primitive changes (WS-PRIMITIVES):**
- **Fields:**
  - New `ui/field.tsx`, copying Crew's `Field`: a `text-label` label, a 6px gap, one
    `text-supporting` helper linked by `aria-describedby`, and an error that replaces the helper.
  - New `ui/textarea.tsx`, copying the Input recipe that `crew/pane/AgentTaskPane.tsx:533`
    reproduces.
  - `Input`, `Select` and `Textarea`: 32px tall (textarea at least 3 rows), radius 8, padding 8px,
    rest edge `--border-control`, focus as in `main.css`.
- **Buttons:**
  - Keep the 24, 28, 32 and 36px ladder, radius 8, 12px padding and `text-label`.
  - `secondary` (`--background-medium`) is the default secondary action; replace `outline` where it
    is used as a secondary.
  - `destructive` is a 22% danger wash with danger ink. Crew's "Danger zone" button is
    `destructive` `sm`.
  - `size="xs"` svg sizing goes to 14px (`button.tsx:62`).
- **Dialogs:**
  - `dialog.tsx`: the overlay fades over 250ms, and the panel goes from opacity 0 and scale 0.98 to
    1 over 250ms with `--ease-out`. Exit is opacity plus scale 0.98 over 125ms.
  - `ModalShell` gains an `xl` 800px size. The 34 raw `DialogContent` call sites migrate,
    each by its own owner.
  - Delete the `p-6` default so raw content cannot drift.
  - P2, decided by the coordinator after a visual check: make Crew's `anchor:'top'` and
    `headerRule` the default outside Crew too.
- **Sheet:** 300ms in, 125ms out, `--ease-out` (`sheet.tsx:78`).
- **Tabs:** unchanged geometry (36px, `text-label`, 2px `--accent-bar` underline, 20px gap). The
  underline becomes one indicator that travels (`translate` plus `width`, 175ms; a named exception
  in principle 9). Its first placement does not animate, the same rule as the segmented thumb.
- **Filter field** (new `ui/filter-input.tsx`, wave 0): the one list filter for every view band
  (Workflows, Scheduler if it ever needs one, Built apps, Extensions, Skills, History). A 28px
  `Input` with a leading 16px `Search` glyph, placeholder "Filter" (History: "Search history"),
  a Tooltip "Filter · ⌘F", and a `useFindShortcut(ref)` hook that focuses it on ⌘F/Ctrl+F while
  the view is mounted. It is `no-drag`. It replaces the views' use of `conversation/SearchView`
  (see 3.16: that bar becomes the transcript's overlay and is no longer a list filter).
- **Badges and cards:**
  - `BuiltInBadge` becomes sentence case, and its explanation moves to an `InfoTip asChild`.
  - `card.tsx` gets 16px padding on the header, content and footer [primitives defect 11].
- **Feedback primitives:**
  - `Progress` gains a `size="thin"` variant (4px track).
  - New `ui/spinner.tsx`: a 16px or 14px `Loader2` at `animation: br-spin calc(var(--dur-slow) * 2) linear infinite`, static under reduced motion. It replaces `animate-spin` across the app, each owner in its own files.
- **Scrolling:** `scroll-area.tsx` keeps the scrollbar hidden at rest and shows it on hover and
  while scrolling. The transcript thumb is permanently visible today [baseline, Chat].
- **Copied helper:** add a `ui/useRovingRows.ts` copy with generic attribute names (`data-row`,
  `data-row-item`).
- **Delete** `ui/BaseModal.tsx` and `ui/Expand.tsx` (0 importers).
- **Remove `transition-all`** (12 sites, motion J4) and `ease-linear`/`ease-in-out` wherever an
  owner touches a file; `transition-all` can animate width, height and padding by accident.

---

## 3. Workstreams

### Global rules for every workstream

- **Each file has exactly one owner.** The ownership table in section 3.17 is authoritative. To
  change a file you do not own, send the owner the exact diff through the coordinator, and the
  owner applies it in its next commit.
- **`styles/main.css`** is shared by selector family (table in section 3.18):
  - Edit only your own families.
  - Wrap every changed or added block in markers:
    `/* @ws WS-NAME begin: topic */ … /* @ws WS-NAME end: topic */`.
  - New CSS that serves **one surface** goes in a file next to the component, owned by you and
    imported by that component. This follows `chatGroups/chat-tabs.css` and
    `conversation/../styles/search.css`.
  - **Shared recipes** go in `main.css` and are written only by WS-TOKENS or WS-PRIMITIVES.
  - Author load-bearing rules as CSS, keyed on classes or `data-*`. Do not rely on newly written
    Tailwind utilities, because of the `BIOROUTER_NO_HMR` scanner trap.
- **Frozen areas.** No workstream edits these without coordinator approval:
  - privacy and security UI and logic: `components/privacy/**`,
    `sessions/DeclassifySessionDialog.tsx`, `sessions/declassifyOnBrowser.ts`,
    `utils/userAction*`, `hooks/chatStreamStore*`, the confirmation logic in
    `knowledge/KbTierControl.tsx`, and the locked blocks in `settings/privacy/PrivacyPanel.tsx`
    (§5 of settings-a);
  - generated code and other non-UI code: `api/**` (generated), `crates/**`, `preload.ts`;
  - unmounted Settings folders: `settings/{dictation,tunnel,sessions}/**`;
  - other areas outside this redesign: `context_management/**` (except the class-only pass on
    `SystemNotificationInline.tsx` below), `LauncherView.tsx`, `Pair.tsx`, `ConfigContext.tsx`,
    `ModelAndProviderContext.tsx`, and `index.html` (except the regions the generator writes).
  - **Class-only, not frozen** (message 5 asks for "all kinds of different components"): the
    user-facing dialogs `DependencySetupModal.tsx` (18 legacy size classes),
    `UpdateAvailableModal.tsx` (10), `DependencyErrorBanner.tsx` (2), `AnnouncementModal.tsx` and
    `ErrorBoundary.tsx` (4) take type roles, button variants and ModalShell sizes from
    WS-PRIMITIVES (P2). No logic, no copy except copy-rule fixes, and nothing that adds main-thread
    work at startup (`docs/desktop-ui/startup-freeze-and-main-thread-blocking.md`).
    `context_management/SystemNotificationInline.tsx` renders in the transcript, so WS-TRANSCRIPT
    gives it the transcript row recipe, class-only.
- **Keep every `data-testid` and every state attribute a test reads.** The e2e suites read, among
  others: `chat-input`, `chat-stop-button`, `user-message-bubble`, `message-container`,
  `turn-activity-indicator` with `data-phase` (`enhanced-context-management.spec.ts:184,452,491`),
  `artifact-viewer`, `panel-host`, the knowledge testids, `settings-app-tab`,
  `dark-mode-button`/`light-mode-button`/`system-mode-button`, `sidebar-components-disclosure`,
  `sidebar-<label>-button`, `extension-submit-btn`, and `data-message-meta="end"`
  (`user-message-layout.spec.ts:164`; see 3.6). Grep `tests/e2e` and `scripts/*e2e*` before
  removing any testid.
- **No backend changes in this redesign** unless the owner approves one (section 6).
- **Copy rules from section 1 apply. Strings move to `copy.ts`** per area, and tests import them.
- **Crew** has no redesign workstream. WS-PRIMITIVES owns `components/crew/**` and may make only
  the light-touch fixes listed in section 3.2.
- **The landing coupling:** do not rename, move or add any `label: '…'` literal in
  `BioRouterSidebar/AppSidebar.tsx`. `landing/app-mockups.js:90-102` must contain each one, or
  `release.sh landing` fails [landing §1.4]; `scripts/check-crew-manual.mjs:758-774` reads them
  too. The "Chats" header, the view menu options and the Update row's text are not written as
  `label: '…'` in that file.
- **Crew copy quoted by the manual.** `scripts/check-crew-manual.mjs` reads Crew strings byte for
  byte. A Crew string change runs `node scripts/check-crew-manual.mjs` before commit.
- **Sweep your own files** to `--dur-*` tokens, type roles, `secondary`, `Spinner`, no
  `variant="mono"`, no `transition-all`, and the glyphs in the icon sweep table (3.3).
- **Docs that describe what you change move with it** (CLAUDE.md: docs live in `docs/`, with the
  context header and "Related documentation"). The coordinator updates `CLAUDE.md` itself in the
  same PR: "Desktop shell geometry" describes the composer as three rows (context above, card,
  controls below), which this redesign replaces with one card and a footer line; add the summary
  rail (rung 0), the 28px sidebar rows and the PreviewBar to the relevant bullets. WS-SIDEBAR
  updates docs that say "Recents" or "See all" (for example `docs/desktop-ui/quoted-text-actions.md`
  and `docs/website/` through WS-LANDING).

---

### 3.1 WS-TOKENS: type, colour, motion and the record

**Goal:** the token layer the other workstreams build on. Land it in wave 0.

**Behaviour, in priority order:**
1. **P0:**
   - The type roles in section 2.1. Repoint `--text-caps`, `--text-title` weight,
     `--text-heading` and `--text-display`; add `text-prose`.
   - Register `text-prose` in `utils.ts:36-48`. Keep `text-heading` and `text-display` registered
     until their call sites move.
2. **P0:** the tokens in sections 2.2 and 2.3:
   - add `--row-height-nav`, `--border-control` and `--control-track-off`;
   - add the dark `--heat-0` step to the three `themes/*.theme.mjs` files, then run
     `npm run themes`;
   - add the contrast assertions.
3. **P0:** the motion work in section 2.4: `styles/motion.ts` and its test, the resize rule, and
   `.br-enter`, `.br-crossfade` and `.br-highlight`.
4. **P1:** amend the `--icon-chip` comment. Delete the dead `--knowledge-subject-height` and its
   mirror in `utils.ts:77`.
5. **Wave 2 (after every owner has landed):**
   - Delete `--motion-*`, `--ease-in` and `--dur-med-max`.
   - Delete `text-heading` and `text-display`.
   - Add `styles/typeScale.test.ts`, an app-wide source guard modelled on
     `crew/crewCss.sourceGuard.test.ts:187-252`. It fails on `text-(xs|sm|base|lg|xl|\dxl)`,
     `text-[Npx]`, a px `font-size` outside the token block, `tracking-*`, a bare `uppercase`,
     `font-bold` or weight 700. Allow-list `crew/` (which has its own guard), `artifacts/` iframe
     sandboxes, and the frozen files that no workstream may restyle: `components/privacy/**`
     (`CrossAffiliationAcceptCard`, `HostManagedModelPanel`, `FirstRunPrivacyNotice`,
     `NonPrivateModelDisclosure` and `HostManagedModelNote` carry legacy size classes today),
     `context_management/**` other than `SystemNotificationInline.tsx`, `LauncherView.tsx`,
     `Pair.tsx`, the unmounted `settings/{dictation,tunnel,sessions}/**`, and `onboarding/**` plus
     `ProviderGuard.tsx` until their P2 lands. Without the list the guard fails on arrival and
     gets disabled. Extend `styles/popupType.test.ts` to the transcript files.
     Its source list is owned by WS-COMPOSER; send the request.
6. **Wave 2:**
   - Move `body` to `var(--text-body)` behind a visual sweep.
   - Add **Part 9, "2026-10 redesign decisions"**, to the root `design.md`. Record every row of
     section 0 and of this section 2, and annotate the passages they supersede:
     - §3.2 scale; §4.9 switch; §4.11 sidebar (28px rows); §4.12 page header (band);
     - §4.17 markdown tables (booktabs); §4.18 headings (no accent bar);
     - D-12 (sidebar row 28px; content rows unchanged).
   - Do not rewrite historical passages.
   - Regenerate `.claude/commands/frontend-design.md` from Parts 1 to 3 (coordinator approval).

**Files:**
- `styles/main.css` families "tokens and global" (section 3.18). The generated region is changed
  only through `npm run themes`.
- `ui/desktop/themes/*.theme.mjs`.
- `ui/desktop/scripts/{generate-themes.mjs,check-contrast.mjs,check-token-mirrors.mjs}` and
  `scripts/lib/theme-contract.mjs`.
- `styles/themes.generated.ts` (generated), `styles/codeTheme.ts` and `utils.ts`.
- New `styles/motion.ts`, `styles/motion.test.ts` and `styles/typeScale.test.ts`.
- Tests: `styles/{fontStacks,fontScale,noteClamp,themeNeutrals,focusFallback,focusSurface,codeTheme}.test.ts`.
- `design.md`.

**Tests:**
- `npm run lint:check`, which includes themes, contrast and tokens.
- `npx vitest run src/styles`.
- `npx vitest run src/components/crew`, because the caps change reaches Crew.
- `crew/sidebar/crewSidebarGeometry.browser.test.ts:36-53` copies token values. WS-PRIMITIVES
  updates it after WS-TOKENS lands.

**Acceptance (vision reviewer):**
- Crew's section labels read 12px caps, and Crew is otherwise identical to its baseline.
- In dark mode the heatmap zero cells are visible.
- No text renders at 10, 11, 15, 16, 19, 20, 28 or 30px anywhere. This is checked by the probe in
  section 5.3.

---

### 3.2 WS-PRIMITIVES: controls, menus, tooltips, rows, dialogs (plus Crew touch-ups)

**Goal:** one look per control (sections 2.6 and 2.5). **Publish the API first**: export
`InfoTip`, `SegmentedControl`, `SettingRow`/`SettingSection`, `RowActions`, `Field`, `Textarea`,
`FilterInput` (with `useFindShortcut`), `Spinner` and `PopoverAnchor`, plus the Tooltip provider
fallback (2.6), with their final props in the first commit, even if the styling is incomplete.
Every other workstream imports them right away.

**Priorities:**
- **P0:** InfoTip, SegmentedControl, SettingRow and SettingSection, RowActions, the Switch spec,
  the menu row recipe and its motion, the row CSS (straight hairline, one hover), and the Tooltip
  provider and its Tab gate.
- **P1:** Checkbox and radio, Field and Textarea, `--border-control` on inputs, the dialog and
  sheet motion, ModalShell `xl`, Tabs indicator travel, BuiltInBadge, Card padding, the Spinner,
  `Progress thin` and the scroll-area thumb.
- **P2:**
  - Delete the dead files.
  - The ModalShell defaults.
  - The defect behind the focus ring after a mouse click (section 2.5).
  - The Crew light-touch fixes below.
  - The class-only pass on the user-facing dialogs listed under "Frozen areas" (Global rules).

**Crew light-touch fixes** (the only Crew edits in this redesign; run
`npx vitest run src/components/crew` before each commit):
- **D1:** `crew/onboarding/fields.tsx:207-219` `SwitchRow`: the label goes on the left
  (`text-label`) with its hint under it, and the switch goes on the right
  (`justify-between`). This makes the toggle side consistent.
- **D3:** `crew/sidebar/crew-sidebar.css:438-452` `.crew-sidebar-team-toggle` gets `gap: 8px`.
  Confirm first in the geometry browser test.
- **D5:** drop the 600 override on section labels (`crew-sidebar.css:293-298`), so every caps
  label is 500.
- **D12:** the copy in `crew/onboarding/copy.ts:168,176` becomes "Start with /: the full path …".
  In `crew/identity/objectNames.ts:156`, `name — server` becomes `name · server`. Use one verb for
  "Set/Add a remote work folder first." **Leave "dialling" (`onboarding/copy.ts:608`) alone in this
  round:** `scripts/check-crew-manual.mjs:1111-1119` matches `/^The connection dropped, and Crew has
  been dialling\b/` in that file and `docs/crew/connections-and-troubleshooting.md:56` quotes the
  sentence verbatim, so the fix needs the checker, the doc and the string in one coordinator
  commit. Run `node scripts/check-crew-manual.mjs` after any D12 edit.
- **Optional D2:** one shared `Spinner` used by Crew's TSX; delete the three `crew-*-spin`
  keyframes and their reduced-motion rests together.
- Update the stale token copies in `crew/sidebar/crewSidebarGeometry.browser.test.ts:48-53`.
- The 32×20 Switch and the menu rows reach Crew through the primitives: run Crew's browser tests
  too (`crew/sidebar/{crewSidebarGeometry,waitingRowGeometry}.browser.test.*`,
  `crew/dialogs/letInGeometry.browser.test.tsx`, `crew/integration/paneCover.browser.test.ts`);
  they run inside `npx vitest run src/components/crew` through Playwright's chromium.
- **Not in scope:** any Crew layout, colour or recipe change, and **no icon sweep inside Crew**
  (the icon report's Crew call sites, `AccessList.tsx:106` and the five agent marks, stay as they
  are; new glyphs reach Crew only if Crew already imports that export name).

**Crew parity guard (wave 2, crew-reference §4.2):** add `styles/crewParity.test.ts`, which parses
both stylesheets with the Crew guard's CSS reader and asserts that each promoted recipe declares
the same properties as its Crew original, except the values this spec changes on purpose (the
28px nav row height and 13px text): `.br-nav-row` against `.crew-sidebar-row`, the group header
against `.crew-sidebar-team-toggle`, the row-action reveal against `.crew-*-actions`, the straight
hairline against `crew/dialogs/dialogs.css:84-115`, `.br-crossfade` and `.br-highlight` against
`crew-app.css:356-438`. Without it "refer to Crew" drifts the day either side changes.

**Files:**
- `components/ui/**` **except** `ui/sidebar.tsx` and `ui/sidebarWidth.ts`.
- New: `ui/{info-tip,segmented-control,setting-row,row-actions,field,textarea,spinner,filter-input}.tsx`,
  `ui/keyboardContextMenu.ts` (a copy), `ui/useRovingRows.ts` (a copy), each with tests.
- `components/ModalShell.tsx` and its test.
- `components/crew/**` (fix list only).
- `main.css` families "shared primitives".
- `styles/{settingsRowHover,settingsScrollFade,tabFocus}.test.ts`.
- `settings/providers/subcomponents/buttons/TooltipWrapper.tsx` (delete it, and update its
  importers through their owners).

**Tests to add:**
- `ui/switch.test.tsx`: the 32×20 classes, a 16px knob with no size change on `data-state`, the
  off-knob token, the name taken from `aria-labelledby`, no `variant` prop, and a
  `data-motion-still` first commit.
- `ui/segmented-control.test.tsx`: the radiogroup, arrow keys, one Tab stop, `aria-checked`, the
  thumb present, and `fill` giving equal widths.
- `ui/info-tip.test.tsx`: as in section 2.6.
- `ui/menuRow.test.tsx`: the dropdown, context, command and select rows render the one row string,
  and the indicator is trailing.
- `ui/row-actions.test.tsx`: reveal classes, `hover:none`, and Shift+F10 opening the menu.
- `AppTooltipLayer.test.tsx`: a programmatic `focus()` does not open the tooltip, and Tab does.
- A source guard over `components/` outside `ui/` that fails on:
  - `type="checkbox"` or `type="radio"`, allow-listing `crew/timeline/MessageBody.tsx` task boxes;
  - `role="switch"`;
  - imports from `@radix-ui/themes`;
  - `title=` on a `DropdownMenu*Item`.
  Add it in wave 2, when the owners have swept.

**Tests to update:**
- `ui/{Checkbox,CustomRadio,badge,controlSizing,Select*,dropdown-menu,Tooltip}.test.tsx`;
- `ContextWindowIndicator.test.tsx:65-73` (the tooltip surface), handed to WS-COMPOSER.

**Acceptance:**
- In Settings, Chat, Extensions, Skills and Crew, every switch is 32×20 and sits on the right of
  its label, with a visible off state in light and in dark.
- Menus have 32px rows, a trailing check and a 12px radius. They open from their trigger and do
  not grow from their centre.
- An InfoTip on a Settings row opens on hover and on Tab, and stays shut after a mouse click
  returns focus.
- No row divider curls at its ends.
- Crew screenshots match the baseline except for the 12px caps labels, the D1 switch side and the
  D3 gap.

---

### 3.3 WS-ICONS: one representative glyph per thing (messages 7 and 9)

**Goal:**
- Chat kinds are distinct by **shape**, and private and public are distinct by shape (a lock
  badge), not only by colour.
- Every tool call draws a representative glyph, and the glyph mapping lives in one module.
- Inks are calmer: chat glyph bodies use `--text-muted`, and only the active row uses default ink.
  **The lock badge stays in the family accent** (coordinator decision on the icon report, message
  12): body muted, badge `--text-accent`, on every row including the active one.

**Inputs:** the proposals in `~/biorouter-runs/redesign-2026-10-09/icons/`:
- `proposed-chatKind.ts` adds the `crew` and `workflow` kinds and a `Private` variant for every
  kind;
- `proposed-toolGlyph.ts` resolves a tool call to a glyph through a 7-rung ladder;
- `proposed-icons.tsx` holds custom stroke-1.5 glyphs with `br-icon-<name>` classes;
- `proposed-entity-icons.ts` adds `ENTITY_ICONS` entries for crew, agent, chat, model and folder;
- `app-icons.proposed.diff`;
- `svg/*`;
- `audit/icons.md` (landed): §8 is the call-site list, distributed by owner in the sweep table
  below.

**Two corrections to the proposals before porting them:**
1. **Message 11.** `proposed-toolGlyph.ts:193-201` maps six marketplace agents to specific glyphs
   (`spokeagent`, `ucsfomopagent`, `cdwagent` → database, `playwrightagent` → web,
   `codegraphagent` → search, `biroffice` → document), and `proposed-toolGlyph.test.ts:200`
   expects `spokeagent__query_graph` → `database`. Delete those rows and flip that test to
   `extension`. Rungs 1 to 4 of the ladder apply **only to built-in extension keys**: the keys
   enumerated in icons.md §5.2 (developer, knowledge, memory, webdocuments, autovisualiser,
   computercontroller, agent_drafter, appcontrol, datasql, files, compute, evidence, todo,
   workspace, skills, extensionmanager, code_execution, chatrecall, crew, platform, workflow),
   held as one exported `BUILT_IN_EXTENSION_KEYS` set that the drift guard also checks. Any other
   key, whatever its tool is called (an external `acme__read_file` included), draws the Puzzle.
   `extensionGlyphFor(key)` follows the same rule, so an external extension's card in Extensions
   is a Puzzle too.
2. **The lock badge ink.** `withPrivateBadge` (`proposed-icons.tsx:212-247`) draws `LOCK_BADGE` in
   `currentColor`, so the badge would take the muted body ink. Wrap it in
   `<g className="br-icon-lock-badge">` and author `.br-icon-lock-badge { color: var(--text-accent) }`
   in CSS (unlayered, beside `.br-chat-kind-icon[data-privacy='unknown']`). Keep the existing
   invariants: `data-privacy="unknown"` draws the unmarked shape dimmed, `off` draws no badge.

**Behaviour:**
- **Crew detection** without a backend change:
  1. `origin === 'crew'` when the field exists;
  2. otherwise a `working_dir` that matches `/[\\/]crew[\\/]tasks[\\/]?$/`, which survives a
     rename;
  3. otherwise the title prefix `Crew · ` or `Crew task`.
  A daemon `origin` field is owner decision 6.10.
- **The `workflow` kind ships dormant.** It needs `SessionSummary.origin` (`workflow_json` is not on
  the summary), which 6.10 defaults to "no backend change". Keep the kind and its glyph in
  `chatKind.ts` so the daemon field can switch it on later; until then workflow runs read as plain
  chats, and no acceptance check expects them.
- **The tool glyph ladder:**
  1. the exact full tool name;
  2. an argument-aware tool (`text_editor` view vs write);
  3. the extension's family, using the entity glyph;
  4. a per-tool table for the toolbox families (developer, webdocuments, files, platform);
  5. coding-agent child tools (`Bash`, `Read`, `apply_patch`, …);
  6. any other `extension__tool`, which draws the Puzzle;
  7. otherwise the wrench.
- `utils/toolIconMapping.tsx` stays as a **shim** that re-exports from the new
  `utils/toolGlyph.ts`, so the consumers in `ToolCallWithResponse.tsx` (including
  `ExecutedCallRow`) and `PendingToolCallCard.tsx` keep compiling. WS-TRANSCRIPT switches the
  imports later.
- **Rule:** never change the glyph behind an **existing** export name that Crew imports. Add new
  names instead, for example `NewChat = SquarePen`. `Plus` stays `Plus`.
- `ChatKindIcon` always draws in a 16px slot (today chat rows use 14px, [shell-sidebar F-04]).

**The icon sweep (message 7, "used consistently whenever similar content is referred to").**
WS-ICONS publishes the glyphs in wave 0; each owner swaps the call sites in its own files from
icons.md §6 and §8 items 5 to 10:

| Call site (icons.md) | Change | Owner |
|---|---|---|
| `BioRouterSidebar/AppSidebar.tsx:89, 96`; `RecentChats.tsx:251-257` (16px, no `isActive` ink) | `NewChat`, `ENTITY_ICONS.crew`; kind glyph | WS-SIDEBAR |
| `sessions/SessionItem.tsx:28` (`h-3.5` → `h-4`), `SessionHistoryView.tsx:394` (Sparkles → ArrowRight), time metadata Clock → History | as listed | WS-HISTORY (3.19) |
| `chatGroups/ChatTabStrip.tsx:48-68` `tabKindSource` passes `working_dir` (and `origin`) so a renamed Crew task stays Crew in a tab | data, not glyph | WS-MOTION |
| `BaseChat.tsx:55, 2275` AlignLeft → PanelRight | as listed | WS-SUMMARY |
| `ToolCallWithResponse.tsx:1364-1368, 1688-1692`, `PendingToolCallCard.tsx:31-33`, `ToolCallStatusIndicator.tsx:29-40` (`data-tool-glyph`), `UserMessage.tsx:502-505` Send → ArrowUp | `toolGlyphFor(name, args)` | WS-TRANSCRIPT |
| `bottom_menu/*` (entity glyphs, `DirSwitcher.tsx:177,186` Folder, effort 16px), `ContextWindowIndicator.tsx:409,418` stroke 2, `ModelsBottomBar.tsx:520,563` no Brain, `MessageQueue.tsx:45` ArrowUp, `copilot/CopilotControl.tsx:249` Copilot | as listed | WS-COMPOSER |
| `artifacts/ArtifactViewer.tsx:426` ChartColumn for figures | as listed | WS-PREVIEW |
| `settings/usage/UsagePanel.tsx:416, 430, 542, 547` | Target, MessageSquareText, Brain | WS-USAGE |
| `schedule/SchedulesView.tsx:552`, `workflows/WorkflowsView.tsx:530, 569`, `applications/ApplicationsView.tsx:392, 434` | entity glyphs, SquareSlash, History, ChatBubble | WS-VIEWS-A |
| `settings/extensions/subcomponents/ExtensionList.tsx:124` `extensionGlyphFor(key)`, `baam/BrowseExtensionsModal.tsx:223` Puzzle, `skills/SkillItem.tsx:70` FolderOpen | as listed | WS-VIEWS-B |
| `knowledge/KnowledgeView.tsx:10, 351, 365` | `ENTITY_ICONS.knowledge` | WS-KNOWLEDGE |
| `settings/memory/MemorySection.tsx:253` Globe → Laptop; `settings/SettingsView.tsx:140` (tab icons are removed anyway) | as listed | WS-SETTINGS-B, WS-SETTINGS-A |
| `components/ItemIcon.tsx:40` Zap → SquareSlash, `:42` | as listed | WS-ICONS (now owns the file) |
| Edit/Edit2 → Pencil (icons.md §2.2 list) | each in its own files | every owner |

Crew's call sites are not in the table: Crew is not swept.

**Files:**
- `components/icons/{app-icons.tsx,entity-icons.ts,index.tsx}` and new glyph components in
  `components/icons/`;
- `components/chats/{chatKind.ts,ChatKindIcon.tsx}` and their tests;
- `utils/toolIconMapping.tsx` (shim) and the new `utils/toolGlyph.ts` with its test;
- `components/ItemIcon.tsx`; `components/AnimatedIcons.tsx` and the dead legacy files in
  `components/icons/` (delete after a grep confirms no importer, icons.md §8 item 11).

**Tests:**
- Port `proposed-chatKind.test.ts`, `proposed-ChatKindIcon.test.tsx` and
  `proposed-toolGlyph.test.ts`.
- Assert that the family glyphs **are** the entity glyphs.
- Assert that every kind has a glyph, a private glyph and a name.
- Assert that `spokeagent__query_graph`, `playwrightagent__browser_navigate` and an invented
  `acme__read_file` all resolve to `extension` (message 11), and that `BUILT_IN_EXTENSION_KEYS`
  matches the crates the drift guard scans.
- Assert that a private glyph renders a `.br-icon-lock-badge` group and a public one does not.

**Acceptance:**
- A sidebar with a crew chat, a scheduled run, a branch, a subagent, an app chat and a plain chat,
  each both private and public, shows 12 distinguishable glyphs: bodies in muted ink, lock badges
  in the accent.
- A SPOKEAgent tool row and the SPOKEAgent card in Extensions both draw the Puzzle.
- A transcript with shell, read, edit, search, knowledge, crew, todo, visualiser, workspace and
  unknown-MCP calls shows a distinct glyph for each family.
- The wrench appears only for an unknown tool.

---

### 3.4 WS-SIDEBAR: compact Codex-like sidebar, Group by/Sort by, inline rename, row menu, History

**Goal:** owner messages 1, 3 and 4. A compact sidebar in Crew's recipe at Codex density, with
ways to organise chats, plus right-click Rename and the other actions.

**Geometry at the default 288px width** (computed; verify in the app):
- **Container:**
  - Drop the inset variant's asymmetric `py-2 pl-2 pr-4`: override it to `p-0` in
    `ui/sidebar.tsx:682-684`. Every list then pads 8px on both sides, so rows span x 8 to 280.
    The icon sits at x=16 and the label at x=40.
  - The 44px band's hairline runs the full width from 0 to 288 and meets the chat header at x=288.
    Measure this seam in light and dark at 216, 288 and 360px.
- **Brand row:** 8px above and 4px below a 32px row, for 44px in total (it was 56). The wordmark is
  20px tall (it was 24). Delete the ten dead nav `tooltip` strings (`AppSidebar.tsx:58-153`).
- **Nav rows** (Home, New chat, Crew, Components, the Components children, Settings, Update), as
  one authored class `.br-nav-row` in the new `BioRouterSidebar/sidebar.css`:
  - 28px tall (`--row-height-nav`) with a 2px gap, padding 0 8px, radius 8 and gap 8.
  - A 16px icon in `--sidebar-icon` and `text-secondary` text.
  - **Muted ink at rest.** Hover: the `--sidebar-hover` ground and default ink, over 95ms.
  - **Current:** `--sidebar-active`, default ink, and a **2px `--accent-bar` rail** at the leading
    edge, inset 7px top and bottom (the zest moment).
  - Focus: `--background-focus` plus an inset 2px `--border-focus` edge.
  - Show the `⌘T` hint on the New chat row only on hover or focus (`text-supporting`, subtle).
- **Components** is a group header in Crew's team-toggle recipe. A 16px `ChevronRight` turns 90°
  over 175ms. **The children are not indented** (drop `pl-9`); the chevron marks the group.
  Collapse is instant. Keep every testid and label in `tests/e2e/helpers/sidebar.ts`.
- **Chats header** (28px):
  - "Chats" in `text-secondary` muted, **sentence case**. It replaces the caps `RECENTS`
    (11/600/0.08em). Clicking the label toggles collapse (keep the `recents-disclosure` aria and
    storage).
  - On the right, two 24px ghost icon buttons with 16px glyphs: `SlidersHorizontal`, with the
    tooltip and name "View options", and `History`, with the tooltip "All chats". `History`
    replaces the `See all` link and keeps `data-testid="view-all-chat-history"`.
- **Buckets** (in the Date group): labels in `text-supporting` `--text-subtle`, 12px above and 4px
  below. The buckets are Today, Yesterday, Previous 7 days, Previous 30 days, then the month name,
  with the year added outside the current year.
  - One shared function, `utils/chatDateBuckets.ts`, is used by the sidebar and by History. It
    replaces one header per day (`RecentChats.tsx:74-110`) and the third format in
    `utils/dateUtils.ts:9-60`.
- **Chat row:**
  - 28px, the same `.br-nav-row` recipe.
  - A 16px kind glyph slot (`ChatKindIcon`, WS-ICONS), then the title (truncated), then a 16px
    trailing slot.
  - The trailing slot holds the running ring (one CSS keyframe; pause with `animation-play-state`
    when it is off screen or the document is hidden). It is swapped for `⋯` on hover or focus.
    **The privacy glyph never hides.**
  - The hover card opens at `delayDuration` 700 and has two lines: the title (only when it is
    truncated), then `~/path · 3h ago · 12 messages` in 12px. Drop its icons and the absolute
    timestamp line.
- **List:**
  - One Tab stop, with ArrowUp, ArrowDown, Home and End (`ui/useRovingRows.ts`).
  - Shift+F10 and the ContextMenu key open the row menu (`ui/keyboardContextMenu.ts`) [F-01].
  - A 10px scroll-edge mask at the bottom [F-30].
- **Footer:** a full-bleed `border-block-start` hairline (it replaces `mx-3.5 my-1`). The **Update**
  row becomes a normal nav row: a `Download` icon in `--accent-bar` and the text "Update to
  {version}". It replaces the caps filled `UPDATE` bar; keep its testid and aria-label. The
  **daemon notice** keeps its `failed` line visible, and its consequence moves into an InfoTip
  [F-15].
- **Overlay below 1120px:** add `--shadow-popover` and the 1px edge to the overlay panel, and
  confirm in the app that it renders. Today it has no scrim and no edge [baseline, Home].

**View options menu** (`BioRouterSidebar/SidebarViewMenu.tsx`), drawn with the menu recipe in
section 2.6:

```
Group by          Date  ›     → radio items: Date · Folder · None
Sort by  Last activity  ›     → radio items: Last activity · Created · Name
```

- **Pure logic** lives in `BioRouterSidebar/sidebarChatView.ts`:
  `arrangeSidebarChats(sessions, view, now)` implements the table in [shell-sidebar §2.3].
  - Name sort uses `localeCompare(…, {numeric:true, sensitivity:'base'})`, with default names last.
  - Folder groups use the basename of `working_dir`, `~` for home, and `parent/basename` when two
    basenames collide. The full path goes in the header's tooltip. **Crew tasks** all live in
    `<data dir>/crew/tasks`, whose basename "tasks" says nothing: that folder is labelled "Crew"
    (the same `working_dir` test the crew kind uses, 3.3).
- **Folder group header** (28px, Crew's group recipe):
  - A chevron, a folder glyph and the name in `text-secondary`.
  - On hover, a `+` (tooltip "New chat in {folder}") and a `⋯` (Copy folder path) appear. **There is
    no API that starts an in-window chat in another folder** (a new chat takes
    `getInitialWorkingDir()`, the window's folder, `App.tsx:108-175`), so `+` opens a new chat
    window there through the existing `window.electron.createChatWindow(undefined, workingDir)`.
    If that reads wrong in the app, drop the `+` rather than inventing a folder switch. The Crew
    folder gets no `+`.
  - Collapse state is remembered in `biorouter:sidebar-collapsed-folders`, a capped array of
    200. A folder that holds the active chat opens itself without overwriting the stored choice.
  - After 5 rows, a muted "Show more" row appears.
- **Persistence:**
  - The key `biorouter:sidebar-chat-view` holds `{"v":1,"groupBy":"date","sortBy":"activity"}`.
    Clamp it on read, wrap every access in try/catch, inject the storage (copy
    `ui/sidebarWidth.ts:65-119`), and follow the `storage` event to stay in sync across windows.
- **Paging:**
  - The default view keeps today's lazy paging of 10 rows.
  - **Any other view** pages sequentially through all rows (`useSidebarSessions({loadAll})`) at
    `limit: 50` (the server maximum), capped at 2,000 rows, and shows "Loading chats…" at the list
    foot. **Every page request carries `userActionHeaders()` exactly as today's request does**:
    without the proof the daemon silently drops private rows (CLAUDE.md, privacy tiers), which would
    read as chats vanishing when the person changes the view.
  - Changing the view re-renders the list at rest, and the list fades from 0.6 to 1 over 175ms.
    Rows do not FLIP.
  - **A live re-sort waits for the pointer.** Under "Last activity" a running chat moves to the
    top. While the pointer is over the list, hold the previous order (principle 9) and apply the
    new one when it leaves, so a click never lands on a row that moved under it.

**Row menu** (right-click, `⋯`, Shift+F10). It is text-only, in this fixed order:
1. **Rename** (with an `F2` hint)
2. separator
3. Open in new tab
4. Open in new window
5. separator
6. Diverge
7. Export…
8. Copy chat ID
9. separator
10. **Delete chat…** (danger)

Notes on the items:
- Add `onRename`, `onDiverge` and `onExport` as optional props on `ChatRowContextMenuContent`, the
  way `onDelete` works. **Do not add them to `chatRowActions`**: four tests pin its three items
  (#114 order).
- Rename the label "Copy conversation ID" to "Copy chat ID" and "Delete conversation" to
  "Delete chat…", and update every surface and test that uses them [F-19].
- The delete dialog reads: title "Delete chat?", body `Delete "X"? This can't be undone.`, and the
  buttons Cancel and Delete [F-20].
- Pin, Archive, Mark as unread and Reveal in Finder are **not** added, because there is no backend
  for them. Make public is **not** added either (#56 §12.1).
- Every item keeps the request it makes today: Diverge goes through `hooks/useDiverge.ts`, Export
  through the extracted `utils/exportConversation.ts`, Delete through `utils/deleteConversation.ts`,
  each with `userActionHeaders()`. A refusal shows the daemon's own sentence as the toast message
  (refusals stay visible, principle 2). Offer only what the row's chat allows today: History's
  `⋯` menu (`SessionListView.tsx:600-692`) is the reference for which items a subagent, scheduled
  or terminal chat shows.
- **History, the tab strip and the sidebar share this one menu** (`ChatRowContextMenuContent`):
  History's right-click and `⋯` gain Rename (it opens History's rename), Diverge and Export in the
  same order; "Make this chat public" stays in History only, after Copy chat ID. The tab strip gets
  the same order when its P3 Rename lands.

**Inline rename** (`chats/ChatRowRenameInput.tsx`; the shared helper
`renameSessionOptimistically(sessionId, name, previous)` lives in `utils/sessionNameSync.ts`):

| Step | Behaviour |
|---|---|
| Enter | The menu item **Rename**, **F2** on a focused row, or `⌘⌥R` for the active chat. **Not** double-click: `RecentChats.test.tsx:174-180` pins a double-click as two opens. |
| Render | The row becomes a `div` with the same 28px geometry, holding the glyph and then an `<input>`: 24px tall, 13px, padding 0 4px, radius 4, `--background-default`, ring `box-shadow: 0 0 0 1px var(--border-focus)`, `maxLength={200}`, `aria-label="Rename chat {old title}"`. All the text is selected. The tooltip and the context menu are held shut, and the draft is kept per session id across head refreshes. |
| Commit | Enter, blur or Tab. Trim the value. When it is empty or unchanged, make no call. Otherwise announce the name optimistically (`origin:'user'`), then call `renameSession` with `userActionHeaders()`. On failure, roll back (`origin:'sync'`) and show the toast "Couldn't rename chat" **with the daemon's error text as its message** (a privacy refusal must stay readable; today's BaseChat helper passes `errorMessage(err)`). **Show no success toast.** Ignore Enter while `isComposing`. Pasted newlines become spaces. |
| Cancel | Escape calls `preventDefault()` and `stopPropagation()`, because the overlay sidebar's Escape handler returns early only on `defaultPrevented` (`ui/sidebar.tsx:566-588`). |
| After | Focus returns to the row button. |

**History** (`sessions/SessionListView.tsx`):
- Move to the band API (WS-VIEWS-A). The description becomes an InfoTip, and the shortcut moves into
  the search placeholder "Search history… ⌘F".
- Rename the modal title from "Edit chat description" to "Rename chat", with the placeholder "Chat
  name".
- Delete the "Chat updated / Description saved." toast, and use the shared helper.
- The hover tooltip becomes "Rename" and the aria-label becomes `Rename ${name}`.
- Use the shared date buckets. Use "…" instead of "...".
- Rows get the keyboard context menu.
- Stats on the right are sans `tabular-nums` in fixed-width columns. They no longer use
  mono [baseline, History].
- The "Show subagent runs" checkbox moves into the band as a ghost icon toggle with
  `aria-pressed`, or into a 36px sub-strip if the band has no room.
- The search is the band's `FilterInput` (2.6), not `SearchView`.

**Saved and shared transcripts** (`sessions/SessionHistoryView.tsx`, `SharedSessionView.tsx`): the
spec said nothing about them, and both still render a 24px `text-title` `<h1>`
(`SessionHistoryView.tsx:139`, `SharedSessionView.tsx:37`). They take the band: `PageHeader` with
`onBack`, the chat title in `text-label`, the privacy marker (`titleAdornment`) in the
`adornment` slot rendered at mount exactly as today (its docblock explains why it must not appear
late), and the existing actions (Share, Resume with the `ArrowRight` glyph, and the "Make this chat
public" trigger, whose `DeclassifySessionDialog` stays frozen) as ghost round 32px icons with
tooltips, at most one of them `secondary`. Their transcripts pick up
WS-TRANSCRIPT's components with no extra work. They get no summary rail (chat-summary §6 item 9).
Find in these transcripts keeps `SearchView` (the overlay, 3.16).

**Other sidebar details the spec had not covered:**
- `ui/sidebar.tsx:428` wraps the app shell in `<TooltipProvider delayDuration={0}>`. Delete it (or
  give it the app values) in the same commit as WS-PRIMITIVES' Tooltip change (2.6).
- Keep the Crew unread badge (`Badge tone="neutral"`, "99+" cap, `AppSidebar.crewUnread.test.tsx:78`).
- `AppSidebar.tsx:206` sets `document.title = 'Biorouter - Workflows'` with a spaced hyphen as
  punctuation; use `Biorouter · Workflows` after checking e2e title matchers [F-26].
- The `EnvironmentBadge` dot (dev and alpha builds only) is exempt from the one-accent-hue check.

**Sidebar collapse motion** (`ui/sidebar.tsx:665, 679, 845`): open over 300ms, close over 175ms.
Replace `transition-all ease-linear` (`:811`) and the shadcn leftover (`:934`) with tokens. Add
`data-motion-layout` to the gap, the shell and the inset.

**Files:**
- `BioRouterSidebar/{AppSidebar,RecentChats,useSidebarSessions,SidebarUpdateButton,EnvironmentBadge,DaemonRestartNotice,index}.ts(x)`
  and the new `{sidebarChatView.ts,SidebarViewMenu.tsx,sidebar.css}`, with all their tests.
- `components/ui/{sidebar.tsx,sidebarWidth.ts}` and their tests. The width bounds are unchanged.
- `Layout/TitlebarControls.tsx`, which is not expected to change.
- `chats/{ChatRowContextMenu.tsx,chatRowActions.ts}` and the new `chats/ChatRowRenameInput.tsx`.
- `utils/{sessionNameSync.ts,deleteConversation.ts,dateUtils.ts}` and the new
  `utils/{exportConversation.ts,chatDateBuckets.ts}`.
- `sessions/{SessionListView,SessionItem,SessionsView,SessionViewComponents,ImportSessionModal,SessionHistoryView,SharedSessionView}.tsx`
  and `sessions/sessionGrouping.ts`.
- `main.css` families "sidebar and history".
- `ui/desktop/tests/e2e/helpers/sidebar.ts` (keep the labels and testids).

**Tests:**
- **New:**
  - `BioRouterSidebar/RecentChats.rename.test.tsx`: Rename comes first; the input opens with the
    title selected; Enter calls `updateSessionName` with the user-action headers; Escape restores
    the title and does not close the overlay; blur commits; an empty or unchanged name makes no
    call; a rejected rename rolls back with a toast; F2 opens the editor; focus returns to the row.
  - `sidebarChatView.test.ts`, covering all 9 group and sort combinations, folder labels and
    collisions, and the buckets.
  - A test for the view store: clamping, a throwing storage, and the `storage` event.
- **Update:**
  - `AppSidebar.test.tsx:110-338`. The muted rest ink conflicts with the assertion at `:167,170`,
    so update it. Keep the band (`:131`) and the wordmark-then-Home order (`:156-159`).
  - `RecentChats.test.tsx`; delete the stale "past-week count" comments.
  - `RecentChats.contextMenu.test.tsx:82-87`.
  - `chats/ChatRowContextMenu.test.tsx`.
  - `chatGroups/ChatTabStrip.contextMenu.test.tsx:56-58`, which is owned by WS-MOTION. Send them
    the string diff.
  - `sessions/SessionListView*.test.tsx`.
  - `useSidebarSessions.test.ts` (paging).
  - `SidebarUpdateButton.test.tsx:53`.
  - `DaemonRestartNotice.test.tsx:43-44`.

**Acceptance:**
- At 1440×900 light and dark, rows are 28px on a 30px pitch.
- Nav icons and chat glyphs share a 16px column at x=16, and the labels align at x=40.
- The header reads "Chats" in sentence case, with no caps and no "See all".
- Bucket labels are coarse: there is no "Oct 7" header.
- The active row shows the coral rail.
- Group by Folder shows the folder headers with `+` and `⋯` on hover.
- Right-clicking a row shows the menu in the order above. Rename edits the title in place with no
  layout shift.
- With the sidebar collapsed, nothing in the band sits under x ≤ 172 on macOS.
- **Measured, not assumed (message 12):** a new `BioRouterSidebar/sidebarGeometry.browser.test.ts`
  (the repo's Playwright-in-vitest pattern, as in `crew/sidebar/crewSidebarGeometry.browser.test.ts`)
  mounts the authored `sidebar.css` with the real tokens and asserts the 28px rows, the 30px pitch,
  the icon column at x=16 and the label column at x=40 at 216, 288 and 360px. The vision reviewer
  also measures it in the running app.
- At 1440 on `/crew` with the app sidebar open, the app sidebar (28px rows, 13px) sits beside
  Crew's rail (32px rows, 14px). The vision reviewer looks at that pair specifically and reports it
  to the coordinator; it is the visible cost of decision 6.1.
- A view change with private chats present keeps every private row (the proof travels on every
  page).
- The sidebar's tooltips open after the app delay, not instantly.

---

### 3.5 WS-SUMMARY: the docked Chat summary rail, the conversation glide, BaseChat

**Goal:** message 1. When a chat starts, the rail pushes the 760px conversation slightly left. It
shows, hides and scales with the window and with the preview. This workstream **owns
`components/BaseChat.tsx`**, so it also applies the BaseChat edits the other areas need
(listed under Behaviour 5).

**Geometry (variant A, owner decision 6.3).** New "rung 0" in `Layout/yieldLadder.ts`. Use the
code in [chat-summary §5.1] verbatim:
- `CHAT_COLUMN_CHROME` is 56. It comes from the ScrollArea root `px-1` plus the viewport
  `paddingX={6}`.
- `CHAT_FULL_MEASURE_WIDTH` is 816.
- `SUMMARY_RAIL_MIN_WIDTH` is 240 and `SUMMARY_RAIL_MAX_WIDTH` is 280.
- `SUMMARY_RAIL_CHAT_FLOOR` is 816.
- `SUMMARY_RAIL_SHOW_WIDTH` is 1056, and `SUMMARY_RAIL_RETURN_BUFFER` is 12.

The rule:
- `available` is the pane width minus the side preview's width.
- The rail is hidden under a stacked preview.
- It shows when `available ≥ 1056`. After a measured hide, the threshold is 1068.
- Its width is `clamp(available − 816, 240, 280)`.

This gives the rail at a 1344px window with the 288px sidebar open, and at a 1056px window with
the sidebar collapsed. It is never shown beside a side preview below about 2272px of pane width.

**Behaviour:**
1. **The grid.**
   - New CSS lives in `components/summary/summaryRail.css` (owned here and imported by
     `ChatSummaryRail.tsx`). It is not in the rung-2 region of `main.css`.
   - Rail only: `[data-preview-split][data-summary-rail]:not([data-preview-layout])` with columns
     `minmax(0,1fr) var(--summary-rail-width)` and areas
     `'header header' 'subheader subheader' 'transcript rail' 'composer rail'`, with the column and
     body flattened by `display: contents`.
   - Side preview plus rail: `[data-preview-split][data-preview-layout='side'][data-summary-rail]`
     with columns `minmax(440px,1fr) var(--summary-rail-width) var(--preview-panel-width)`.
   - The rail cell is `[data-preview-area='rail']` with padding 12px 12px 12px 0.
   - Keep the text `{...artifactPanel.splitPaneProps}` verbatim (`measures.test.ts:759-765`).
     Merge the styles explicitly after both spreads.
2. **State** (`hooks/useSummaryRail.ts`, `Layout/summaryRailPreference.ts`):
   - `shown = preference==='open' && active && fits && !isMobile`.
   - `preference` is per viewer: `biorouter:summary-rail`, `open` or `closed`, default `open`,
     shared through `useSyncExternalStore` and the `storage` event.
   - `active = !isCleanConversation && (messages.length > 0 || isRunningState(chatState))`.
   - Measure synchronously in `useLayoutEffect` on mount, then use a `ResizeObserver` on the split
     box.
   - **Hide at once; show after 160ms of stable width.**
   - Never resize the window.
   - **Hook order:** `useSessionTodos` is called at `BaseChat.tsx:1631`, but `isCleanConversation`
     (needed for `active`) is computed at `:2227`. Move the `useSessionTodos` call below the rail
     hook, both above the early return at `:2543`, and pass `reviewOpen || railShown` as its
     `open` argument [chat-summary §3.4].
   - `ChatSummaryRail.tsx` is imported statically by `BaseChat`, so `summaryRail.css` is loaded
     before the first chat renders (a lazily imported stylesheet would leave the grid unstyled
     until the module loads, the hazard crew-reference §2.1 describes).
3. **The header button** (in the chat header's actions):
   - It is `PanelRight` and replaces `AlignLeft`.
   - When the rail fits, the button toggles the preference, with `aria-expanded`,
     `aria-controls` and the pressed fill `--background-medium`. The tooltip says "Hide summary" or
     "Show summary".
   - When the rail does not fit, the button opens today's popover at **280px** wide (it was 360).
   - Use a `PopoverAnchor`, exported from `ui/popover.tsx` (request it from WS-PRIMITIVES).
   - When the rail becomes shown while the popover is open, close the popover.
   - **When the rail is hidden and a to-do item is `in_progress`, show a 6px coral dot** on the
     button. Its name then reads "Chat summary, 2 of 5 to do items complete".
4. **Content** (`components/ChatSummary.tsx`, rendered in the rail by the new
   `components/summary/ChatSummaryRail.tsx`):
   - **The card:** radius 12, 1px `--border-subtle`, `--background-default` ground (one step up in
     dark), `max-height: 100%`, a flex column with a scrolling body and a pinned footer.
   - **To do** (shown when there are items):
     - A 12px muted label with a tabular count ("2 of 5") carrying `aria-live`, then a 4px
       `Progress thin`.
     - Rows are 28px with a 14px status glyph that differs by shape: ○ pending, a dashed circle
       while running (with an accent ink arc), ✓ done (muted text, no strike-through) and ⚠ failed.
     - The text is `text-secondary`. The **status word is sr-only**.
   - **Stats:** a `dl` of 28px rows with a muted label and a tabular value on the right: Tool calls;
     **Tokens**, with an InfoTip "Billed tokens across every model in this chat. N/A when a model
     has no certified total."; Artifacts; and Code `+N` in success ink and `−M` in danger ink, using
     U+2212.
   - **Footer:** "Make workflow" and "Diagnostics" as full-width ghost `sm` rows in
     `text-secondary`.
   - There is no "no tasks" sentence. The error is one line, `role="alert"`, "To do could not
     refresh.", with a Retry.
   - Use two sizes only: 13 and 12.
   - Move `biorouter-focus-region` and `tabIndex=0` to the scrolling body in the rail, and update
     `styles/tabFocus.test.ts:392-436`, which is owned by WS-PRIMITIVES, and
     `ChatSummary.test.tsx` together.
5. **The edits to BaseChat this workstream applies on behalf of other areas:**
   - **The conversation glide** for the rail **and** for the preview, in `hooks/useColumnGlide.ts`:
     - Capture the rects of the `[data-preview-area='transcript']` and
       `[data-preview-area='composer']` cells before the change, then FLIP them after the commit
       with WAAPI.
     - Open: `translateX(dx)` to 0 over 300ms with `EASE_OUT`.
     - Close: 175ms, after the panel's exit.
     - Skip when the window is resizing, under reduced motion, when `dx < 8px`, when the window is
       growing for a preview, or when the change came from geometry rather than an action
       (`data-rail-still`).
     - Animate the grid items, not `.biorouter-chat-column` inside the scroller.
     - **On the first turn the composer already moves**: the existing composer FLIP
       (`BaseChat.tsx:2307-2345`) carries it from the centred empty state to its final rect, which
       already includes the rail. So on that turn the glide animates the transcript cell only and
       leaves the composer to its own FLIP; two transforms on one element fight. On a toggle (no
       empty-state move) the glide animates both cells.
   - **Keep the clicked artifact card in view** when the preview opens. Today the transcript
     scrolls away [baseline, Artifact panel]. Anchor the scroll on the card's rect before the
     layout commit and restore its offset after it.
   - The Stop & send banner (`:2393`): replace the non-existent `border-border`/`bg-muted` with
     `<Note tone="neutral">`, one line per state ("Stop & send is waiting", "… is settling",
     "… is open in another window"), plus an InfoTip carrying today's sentence. Take over and
     Abandon are `sm` secondary and ghost.
   - The composer FLIP (`:2307-2345`): translate only, with `DUR.med` and `EASE_OUT` from
     `styles/motion.ts`. **Drop `scaleX`.**
   - Delete the non-coherent branch (`:2630-2632, 2783-2785`). The load-error state uses
     `EmptyState` with a `secondary sm` Retry. The workflow sticky header wrapper (`:2797-2800`)
     uses `text-secondary` on the column edge.
   - Delete the dead `biorouter-composer-view-transition` hook (`:2366`). `handleRename` uses
     `renameSessionOptimistically` from WS-SIDEBAR.
   - Header action buttons are `Button variant="ghost" shape="round"` at 32px. Drop the
     hand-synced `HEADER_ACTION_BUTTON_CLASS`.
   - **For WS-MOTION:** the find-in-chat overlay needs a positioned transcript cell
     (`position: relative` on `[data-preview-area='transcript']` or a wrapper) so the bar can sit at
     its top right without pushing the column; and the one-time 125ms fade of the transcript
     container after a tab opens or switches. WS-MOTION sends the diff.
   - **For WS-TRANSCRIPT:** the `isLive` flag reaches `ProgressiveMessageList` from here, and the
     `SessionNamePill` keeps receiving `handleRename` (now the shared helper).
6. **Motion** ([motion E1-E4], [chat-summary §3.6]):

   | Cause | Grid | Transcript and composer | Rail card |
   |---|---|---|---|
   | The person toggles it open, or the first turn starts | snaps | FLIP over 300ms | opacity 0 to 1 and `translateX(12px)` to 0, over 175ms after a 60ms delay |
   | The person toggles it closed | holds until the card exits | FLIP back over 175ms | opacity to 0 and `translateX(8px)`, over 125ms |
   | Resize, sidebar, preview, split, tab switch or reload | snaps | none | none (`data-rail-still`) |
   | Reduced motion | snaps | none | a fade over 125ms only |

   - New to-do rows use `.br-enter` with a 30ms stagger, at most 5 animated. A status change
     crossfades the glyph over 125ms. Progress uses the primitive's 250ms width transition.
   - Auto-scroll to the in-progress item with `block:'nearest'`, smooth only when reduced motion is
     off and the pointer is not over the rail.

**Files:**
- `components/BaseChat.tsx` and its tests (`BaseChat.*.test.tsx`).
- `components/ChatSummary.tsx` and its test.
- New `components/summary/{ChatSummaryRail.tsx,summaryRail.css}`.
- `hooks/useSummaryRail.ts` (new), `hooks/useColumnGlide.ts` (new) and `hooks/useSessionTodos.ts`,
  with tests.
- `Layout/yieldLadder.ts` and its test, and `Layout/summaryRailPreference.ts` (new) with its test.
- `main.css` family `.biorouter-composer-motion`.
- New `styles/summaryRail.test.ts`.
- Docs: `docs/desktop-ui/summary-and-figures-qa.md` (the contract changes to "rail or popover",
  and "explicit status text" becomes "announced to assistive technology, distinguishable by
  shape"); `docs/desktop-ui/preview-panel/narrow-panes.md` (rung 0 with 816, 1056 and 1068); the
  new `docs/desktop-ui/chat-summary-rail.md` with rows in the folder README and in
  `docs/README.md`; and `docs/troubleshooting/diagnostics-and-bug-reports.md:12`.

**Dependency:** WS-PREVIEW adds `sideWidth: number` to `ArtifactPanelController` in its first
commit (`useArtifactPanel.ts:214-236, 620-643`). It is
`mounted && previewMode==='side' ? resolvedWidth : 0`.

**Tests:** [chat-summary §4.2] verbatim:
- `summaryRailFit` checks each seam one pixel apart (1055 and 1056; 1067 and 1068), the width
  steps (1056→240, 1076→260, 1096→280), a property loop showing `available − width ≥ 816`, the
  preview subtraction, `stack` giving hidden, and an unmeasured pane returning the previous answer.
- `summaryRailVisible` and `summaryToggleMode`.
- `useSummaryRail.test.tsx` uses a fake RO: no hidden-then-shown flash on mount; a narrowing hides
  at once; a widening shows only after 160ms (fake timers); a mount does not animate.
- The preference store test.
- `styles/summaryRail.test.ts`: the exact templates; the `:not([data-preview-layout])` selector;
  unlayered rules; no `@container`, `@media`, `transition` or `animation` in the geometry rules;
  BaseChat marks `data-preview-area="rail"` exactly once; the source still contains `paddingX={6}`
  and `'px-1',`.
- A BaseChat test for the button's two modes.
- `hooks/useSessionTodos.test.tsx` (it fetches while the popover **or** the rail is shown).
- `useColumnGlide.test.ts`: no glide while resizing, under reduced motion or when `dx < 8`.

**Acceptance (vision reviewer and recordings):**
- Desktop GUI widths 1100 (sidebar overlay), 1344, 1356, 1440 and 1920.
- On the first turn the rail appears and the column glides left by about 140px, with no horizontal
  scrollbar during the glide.
- Opening a preview at 1440 hides the rail in the same frame. Closing it brings the rail back at
  rest after 125ms.
- Dragging the window across 1344px snaps with nothing animating.
- A tab switch shows no jump.
- A run with a five-item to-do list updates the rail live.
- The composer's left edge always equals the column's.
- Under 1056px of available width, the button opens the 280px popover.

---

### 3.6 WS-TRANSCRIPT: messages, markdown, tool rows, cards

**Goal:** a calm, Codex-like transcript in Crew's row language. A tool call is a line, not a card
(D-17).

**Behaviour:**
- **Rhythm** [chat-interface B1, B2, B14]:
  - 24px between turns, 8px between the blocks of an assistant turn, and 4px between consecutive
    tool rows.
  - `ProgressiveMessageList` uses a wrapper `mt-6` when the role changes and `mt-2` otherwise.
  - Delete the `.assistant:has(+ .user)` rule (`main.css:4516-4518`) and the `mt-[16px]` on the
    user root.
- **Message actions** use Crew's floating cluster (`crew/timeline/timeline.css:325-374`):
  - It is `position:absolute` at the row's top right, 12px above it, on `--background-default`
    with `--shadow-popover` and an inset hairline, radius 8, padding 2 and gap 2.
  - It holds 24px icon-only ghost buttons with tooltips. **No row is reserved for it**: delete the
    28px `MessageMeta` row.
  - The user message gets [Edit][Copy], and the assistant message gets [Copy][⋯ → Diverge from
    here]. The time is a 12px muted tabular label inside the cluster.
  - Copy confirms in its tooltip ("Copied"), so the width never jumps. Keep the accessible names
    "Copy", "Copied" and "Copy failed".
  - The cluster is shown on hover and on focus-within, and always under `hover:none`.
- **The user bubble** stays `--background-medium` with a hairline, radius 12 and right alignment
  (D-16).
- **Edit mode** stays inside the 760 column and becomes one composer-recipe card. Its buttons are
  Cancel (ghost), "Save" (default) and "Save as new chat" (secondary), with tooltips; keep the
  aria-labels the tests use. Delete the "Edit in place …" line. "Edited" folds into the time.
  Messages that were never delivered or never answered show a state word plus an InfoTip
  [B11].
- **Entrances:** `.br-enter` applies only to rows appended **live**. Messages loaded from history,
  after a tab switch or after a reload, do not play `appear` [motion B2, J6]. Pass `isLive` from
  the list.
- **Markdown** (section 2.1, `main.css:4287-4518`, and `MarkdownContent.tsx`):
  - **Code block (Crew's recipe, section 0):** `--background-well`, 1px `--border-subtle`, radius
    12, and a head row over a hairline on the same ground (no filled slab): the language in 12px
    muted on the left, Copy and Run as 24px icon-only buttons on the right with tooltips (Copy
    confirms in its tooltip, "Copied"; keep the accessible names "Copy", "Copied" and "Run"). Body
    10px 12px, mono 13/20, a 40px right fade when it scrolls. Promote it as `.br-md-code` (copied
    from `crew/timeline/timeline.css:481-553`, never a `crew-*` class).
  - **Tables (Crew's recipe, section 0):** a 1px `--border-subtle` frame at radius 8, top rules
    only, a header row on `--background-muted` at 12px muted 600, cells 8px 12px in
    `text-secondary` with tabular numbers, **no bold first column**, and the right fade plus
    hairline when it scrolls. Promote it as `.br-md-table`; WS-PREVIEW uses the same class for CSV
    and notebook DataFrames. This supersedes design.md §4.17.
  - Replace `prose-code:text-[13px]` and `prose-table:text-[13px]` with roles.
  - The chat and document variants follow the heading table in section 2.1.
- **Tool rows** (`ToolCallWithResponse.tsx`, `tool-call.css`, `PendingToolCallCard.tsx`):
  - **The row:** 28px, `text-secondary` muted. A leading 16px glyph comes from `utils/toolGlyph.ts`
    (WS-ICONS). It is followed by the verb and summary, and then a trailing chevron at 60% that
    goes to 100% on hover.
  - **The verb carries the state:** "Running {x}", "Ran {x}", "Failed {x}", "Stopped {x}".
    **There is no suffix by default.** The " · N results ready", " · Working through the tool
    call" and "Problem with" suffixes are gone. A real progress message from the tool may still
    appear.
  - **Hover and focus:** hover uses `--overlay-hover` on a −8px bleed with radius 8.
  - **Fix the focus clip:** drop `overflow-hidden rounded-md text-sm font-sans` from the wrapper
    (`:404`).
  - **The expanded body:**
    - It mounts through Radix `Collapsible` plus `.biorouter-disclosure-panel`, opening over 300ms
      and closing over 125ms.
    - It is **one well**: `ml-6` (24px), `rounded-element`, `bg-background-well`, `p-3`.
    - Sections are 8px apart with no rules. Their labels are `text-supporting` muted and their
      content is `text-code`.
  - **Failure:** the title is `text-secondary` in danger ink and the body is `text-code`.
  - **Progress:** the shared `Progress` at 4px, with the message in `text-supporting`.
  - "· not gated by Biorouter" stays, with an InfoTip in place of `title=`. "N not recorded" also
    gets an InfoTip.
  - The render-boundary fallback becomes an error row.
  - **Thinking** uses the same row: `[Brain 16] Thinking · 8s ›` while streaming, then "Thought".
  - P2: fold runs of read, list and search calls into one "Explored · 3 files, 1 search" row
    [D18].
- **Activity line** (`TurnActivityIndicator.tsx`, `LoadingBioRouter.tsx`):
  - `[Spinner 16] Thinking · 12s` in `text-secondary` muted, with the same text pulse as a running
    tool row (period `calc(var(--dur-slow)*4)`).
  - Delete the ring and glow, and delete the 45-second "Still working…" line.
  - `utils/trailingActivity.ts:134` reads "Waiting for your answer".
- **Cards** (`ToolCallConfirmation`, `ElicitationRequest`, `SecretRequestCard`):
  - **One card recipe:** radius 12, 1px `--border-subtle`, `--background-default`, `p-4`, with no
    `rounded-2xl` and no two-piece bubble. The title is `text-label`, the body `text-body`, and the
    actions sit at the bottom right at `sm`.
  - **Copy** becomes "Allow once", "Always allow" and "Deny" (ghost). **Keep the security-finding
    banner visible** and keep withholding "Always allow" on a finding.
  - **SecretRequestCard:**
    - Use `ui/secret-input.tsx`, a `text-label` label, and the description as an InfoTip only (it
      is currently rendered twice).
    - Keep one visible reassurance line, "Saved on this machine. The model never sees it.", in
      `text-supporting` with a 14px lock.
  - **Resolved states:** a resolved elicitation collapses to a transcript row with a 16px check.
- **Artifact card** (`MCPUIResourceRenderer.tsx`):
  - It spans the **760 measure** (it is 576 today). Radius 12, 1px `--border-subtle`, **no
    shadow**.
  - A 32px tile holds the kind glyph (WS-ICONS). The title is `text-label` and the subtitle is a
    human kind ("Figure", "App", "Report", "Page") in `text-supporting`, never a MIME type.
  - It opens the panel. **There is no delete control** (CLAUDE.md).
- **Errors** (`conversation/ChatTurnError.tsx`): a 16px `AlertTriangle` in danger ink, a
  `text-label` title, a `text-secondary` muted message, a `secondary sm` Retry, and a
  `Disclosure` labelled "Details".
- **Peripheral pieces:**
  - **SessionNamePill:** `text-label`; the shared `Input` for renames, with `maxLength={200}`.
  - **Subagent band** [J1-J4]: fold it into a 32px line at the top of the column rather than a
    second 44px band. The line reads "Subagent of {parent}" and holds the counts, Context (a
    `Disclosure`) and Stop. Keep the aria-label "Stop subagent".
  - **WorkflowActivities starter pills:** focusable `Button variant="secondary" size="sm"` elements
    (`outline` is retired as a secondary style, 2.6) with a `--dur-fast-max` entrance.
  - **`context_management/SystemNotificationInline.tsx`** (the inline "compacted" notice) takes the
    transcript row recipe, class-only (Global rules).
  - Keep `data-testid="turn-activity-indicator"` and its `data-phase` attribute on the new activity
    line: three e2e cases wait on `[data-phase="compacting"]`.
- **Placeholder:** "Loading messages…" becomes a single `text-supporting` line. The ⌘F tip moves
  into the search bar's InfoTip, which belongs to WS-MOTION.

**Files:**
- `components/{BioRouterMessage,UserMessage,MessageMeta,MessageCopyLink,MessageDivergeLink,MarkdownContent,ProvenanceChip,ProgressiveMessageList,ToolCallWithResponse,PendingToolCallCard,ToolCallStatusIndicator,ToolContentPreview,ToolCallArguments,ToolCallConfirmation,ToolCallPreview,ElicitationRequest,SecretRequestCard,MCPUIResourceRenderer,TurnActivityIndicator,LoadingBioRouter,SessionNamePill,WorkflowHeader,InlineImage,ImagePreview}.tsx`.
- `components/{message-meta.css,tool-call.css}`.
- `conversation/{ChatTurnError,ChatTurnStopped}.tsx` and `conversation/turnStoppedNotice.ts`.
- `subagent/**` and `workflows/WorkflowActivities.tsx`.
- `utils/trailingActivity.ts`.
- `context_management/SystemNotificationInline.tsx` (class-only).
- `ui/desktop/tests/e2e/user-message-layout.{spec.ts,fixture.tsx,fixture.html}` and
  `ui/desktop/tests/e2e/bioroffice-{install,verify}.spec.ts` (the approval labels).
- All their tests, and `test/uiCopySpelling.test.ts` assertions for these strings.
- `main.css` families "transcript and markdown".
- `styles/chatSurfaceType.test.ts`.

**Tests** ([chat-interface §5]):
- Strings to update: "Working on" (also in `uiCopySpelling`, `trailingActivity`,
  `ProgressiveMessageList`, `TurnActivityIndicator` and `copyWhileStreaming`), "Problem with",
  "result ready", "Still working", "Allow Once" and "Always Allow" (also in
  `bridgeApprovalPrompt.test.ts` and `utils/startChatFailure.test.ts`, which are owned by this
  workstream for these strings), "Information submitted", and "Technical details".
- `MessageCopyLink` text-content assertions become name assertions.
- The `h-6` and `!px-0` classes asserted at `ToolCallWithResponse.test.tsx:576-586`.
- `MarkdownContent.test.tsx:1195-1374, 1655-1777` (the `text-[13px]` classes).
- `UserMessage.clamp.test.tsx`.
- **e2e `tests/e2e/user-message-layout.spec.ts`** measures the bubble against
  `[data-message-meta="end"]` (`:164`, asserted at `:191, 199, 204, 231`) and expects the edit
  state to be wider than twice a short bubble (`:247`). Deleting the reserved `MessageMeta` row
  removes that anchor: keep `data-message-meta="end"` on the floating cluster's positioned box
  (aligned to the bubble's right edge) or rewrite the geometry assertions in the same commit.
- `bioroffice-{install,verify}.spec.ts:240,151` click `button:has-text("Always Allow")` and
  `"Allow Once"`; Playwright's `:has-text` ignores case, so they keep passing, but update the
  strings to the new sentence case.
- **Add:** a guard that bans `rounded-2xl` in `components/` outside `artifacts/` (only five files
  use it today, all in this workstream or in `conversation/SearchBar.tsx`, which WS-MOTION fixes).

**Acceptance:**
- A tool-heavy chat at 1440 light and dark shows one line per tool, each with a distinct glyph,
  4px apart, with no suffixes.
- An expanded call shows one well indented 24px.
- About 24px separates the turns (not 84px).
- Headings carry no coral bars. Code blocks have a head row over a hairline, not a filled slab.
  Markdown tables, CSV tables and Crew's tables look the same.
- Hovering a message shows the floating cluster with no layout shift.
- The artifact card spans 760px with a human kind subtitle.
- An approval card has a 12px radius and sentence-case buttons.

---

### 3.7 WS-COMPOSER: the composer, pickers and footer, Home composer (message 8)

**Goal:** one card in Crew's composer recipe that holds its own controls, plus one quiet Codex-like
footer line.

**Structure** (`ChatInput.tsx:2957-3764`):

```
┌ card: radius 12, border-border-subtle/60, bg-background-default, padding 10px 12px 10px 16px ┐
│ [chips row: references and files, only when present]                                         │
│ Ask Biorouter anything…                                       (textarea 14/20, grows to 40vh) │
│ [+] [Tools 15 ▾]                              [gpt-5.6-sol · Deep 🔒 ▾]  (↑ 32px Send)         │  controls row 32px
└───────────────────────────────────────────────────────────────────────────────────────────────┘
  ▢ Desktop                                                                ◔ 38% left   $0.42      footer 12px muted, 4px below the card
```

- **`+`** is a ghost round 28px button. Its menu holds "Attach file…", "Mention (@)" and "Commands
  (/)", each with a muted shortcut hint.
- **The Tools chip:**
  - It is a ghost `sm` chip in `text-secondary`, labelled "Tools", followed by the sum of
    extensions, skills and knowledge bases.
  - It opens **one popover** with a `SegmentedControl` (Extensions · Skills · Knowledge) above the
    existing three lists.
  - **One picker shell** serves all three lists: an `h-8` search ("Search extensions…"), then a
    `DropdownMenuLabel` with a ghost `xs` bulk action, then rows in the menu recipe with the name,
    the `BuiltInBadge` and a `Switch`. The description sits in the row tooltip, **not in `title=`**,
    because a `title` would rename the menu item [primitives defect 6].
  - The scope notes become a single InfoTip.
  - **Fallback**, if this restructure overruns the round: keep the three menus, but give them one
    shared `ComposerChip` trigger (28px, `text-supporting`, a 16px muted icon, no alpha inks).
- **Model · effort** is one picker, a ghost `sm` chip in `text-secondary`:
  - **The label:** a friendly model name with the `-YYYY-MM-DD` suffix stripped and the full id in
    the tooltip. There is **no Brain icon**.
  - **The padlock stays** (privacy signal). The affiliation glyph moves out of the chip into the
    menu and the tooltip.
  - **Truncation** uses "…".
  - **Effort** is a `DropdownMenuRadioGroup` inside this menu (Quick · Normal · Deep). Each
    description goes in an item tooltip.
  - **The menu header** holds the name in `text-label` and "Versa Azure · Private" in
    `text-supporting`. One InfoTip holds the chat-binding, new-chats and handover notes.
  - **The DR-17 privacy and disclosure line stays visible** as one `text-supporting` line.
  - The items "Change model…" and "Lead and worker…" have no trailing icons.
- **Send** is 32px, radius 8. It is grey (`secondary`) while empty and coral (`default`) when there
  is content. Stop uses the same geometry with a `Tooltip`; use a `Tooltip` for both, and drop the
  `animate-ping` acknowledgement in favour of `scale-90` only.
- **The footer line:**
  - On the left, the folder **basename** in sans, with the full path (mono) in the tooltip. It is
    still the `DirSwitcher`. Drop the mono path, the alpha inks, the lone `•` and the signal-bars
    glyph.
  - On the right, a 14px context ring. Its text "N% left" appears **only below 50%**, and its
    tooltip is one line, "72% context left · 92k of 128k".
  - Cost appears **only when it is above $0 and inside a chat**, in sans `tabular-nums`.
- **Placeholders:** "Ask Biorouter anything…" on Home and in empty chats (pinned by
  `Hub.startFailure.test.tsx`), and "Ask a follow-up" in a chat. This replaces "⌘↑/⌘↓ to navigate
  messages" (`utils/keyboardShortcuts.ts:6`).
- **Attachments** move into one chips row **above** the textarea (Crew's `ComposerChips`):
  - 48px image thumbnails and file chips (`text-chip`, a 14px icon, the name and an X).
  - An error shows as a danger-ink chip with an InfoTip.
  - **Fix the solid black overlays** (`bg-opacity-*` does not exist in Tailwind v4,
    `:3501-3558`) with `bg-[color:var(--scrim)]` and use `Spinner`.
- **Banners:**
  - "No model selected" with a `link` button "Choose a provider". This removes the em dash in
    `composerNoProvider.ts:18`.
  - The vision banner reads "This model can't read images" with an inline "Switch model" and an
    `EyeOff` icon.
- **MessageQueue:**
  - The header reads "Queued · 2" in 12px. Rows read `↳ text` in `text-secondary`, truncated.
  - Per-row actions are 24px ghost buttons with tooltips. Delete the `text-[10px]` and
    `text-[11px]` sizes and every `title=`.
- **MentionPopover:** rows in the menu recipe. The `z-[1210]` becomes `--z-modal-dropdown`.
  Replace "N items found" with nothing, and use a `Spinner` with "Searching…".
- **Hub** (`Hub.tsx`):
  - `max-w-[760px]` becomes `max-w-measure-chat`.
  - Hide the ring and the cost when `sessionId === null`.
  - Creation progress shows as a spinner in Send. Delete `LoadingBioRouter` above the composer and
    the dead view-transition hook (`:116`).
- **The notices above the composer** (`PrivacyTiersOffNote`, `PinnedModelNote`, crew access bar,
  `CopilotControl`) keep a 4px stack gap. **Their copy is frozen.** `CopilotControl.tsx` receives
  type-role swaps only. The privacy notes live in the frozen `components/privacy/**` and the crew
  access bar in `crew/access/`: they keep their look, which is why the type guard (5.3) allow-lists
  them.
- **Composer motion:** P2, only if the autosize can move to CSS. Grow multiline height with
  `field-sizing: content` plus `interpolate-size`, over `--dur-fast-max` (175ms; Codex's 220ms is
  not a token, and principle 1 of motion says tokens only). Height here is a named exception in
  principle 9 only if this lands; otherwise the height snaps as today.

**Files:**
- `components/{ChatInput,MessageQueue,MentionPopover,Hub,ContextWindowIndicator,QuotedTextChip,QuotedTextSelection,ResourceRefChip,WaveformVisualizer}.tsx`.
- `components/{composerNoProvider.ts,extensionReferenceItems.ts}`.
- `bottom_menu/**`.
- `settings/models/bottom_bar/ModelsBottomBar.tsx` and its tests.
- `store/reasoningEffort.ts` (copy only).
- `utils/keyboardShortcuts.ts`.
- `copilot/CopilotControl.tsx` (type roles only).
- `main.css` families "composer".
- `styles/{composerFocus,composerWorkingEdge,dirChipPath,popupType}.test.ts`.

**Tests:**
- `ChatInput.*.test.tsx` (noProvider, steerBrowserSurface, workingEdge, workingDir, references,
  queue*), `MessageQueue.test.tsx`, `MentionPopover*.test.tsx` and `Hub.startFailure.test.tsx`.
- `bottom_menu/*.test.tsx`: the effort glyph `size-[17px]` becomes 16, and the KB menu classes
  change.
- `ContextWindowIndicator*.test.tsx`: drop the three-line tooltip.
- `ModelsBottomBar*.test.tsx` and `ModelAndProviderContext.test.tsx`.
- `dirChipPath` asserts mono; flip it to sans.
- `workflows/shared/__tests__/WorkflowResourcePicker.test.tsx` shares the "Search …" placeholder
  strings with these menus. **Change both or neither**; coordinate with WS-VIEWS-A.
- **Keep** the `.biorouter-composer-card:has(textarea:focus)` accent edge (owner request) and the
  working sweep.

**Acceptance (Home and chat, 1440 and 1100, light and dark):**
- One card. Inside it, at most four controls and the Send button. Under it, one muted footer line.
- No mono anywhere in the composer. No "100%" and no "$0.00" on Home.
- The model chip shows "gpt-5.6-sol" with a padlock and no extra glyphs.
- The Tools chip opens one popover with segmented tabs.
- Image attachment overlays are translucent, not black.

---

### 3.8 WS-PREVIEW: the artifact panel

**Goal:** message 1. Previews look calm, consistent across kinds and in motion, and every kind
reads with one voice.

**Behaviour:**
- **P0 defects** [preview-panel §5.1]:
  1. **Re-scroll when the strip narrows.** Add the overflow state to the
     `ArtifactViewer.tsx:835-841` effect, or observe the tablist. Add
     `scroll-padding-inline: 8px` to `.br-tabstrip__scroll`. That selector is in WS-MOTION's
     family, so send them the request. Add an edge-fade mask that applies only while the strip is
     scrolled.
  2. **Drag feedback.** Remove `opacity-50` and `bg-background-medium` (`:1359-1360`). Write
     `data-dropbefore` the way `ChatTabStrip` does, so the shared `.br-tab` rules dim the tab and
     draw the insertion hairline.
  3. **Iframe pre-paint.** Hold iframes (`:1677-1689`, `:2622-2635`) at opacity 0 until `load`,
     then fade them in over 175ms. Keep the `bg-white` page ground.
  4. **Fold chevron.** `rotate` transitions over 125ms. Write this rule **outside** the rung-2
     block, after the resizing block, because `measures.test.ts:739-746` scans that block.
  5. **Header buttons** use `Button variant="ghost" shape="round"`. Drop the hand-synced constant.
- **Grounds.** The aside is `--background-default` (paper) for **every** kind, and the strip stays
  `--sidebar`. That leaves two grounds in total. Change `:1306`, `:1724`, `:2018`, `:2300` and
  `DocumentPreview.tsx:282, 368, 607`.
- **Header actions:**
  - Always shown: **Open externally** (always `ExternalLink`) and **Close**, as 32px ghost round
    icon buttons, the size of every other 44px band's actions (the chat header and Crew's pane
    header use 32). Drop to 28px only if a measured 380px panel cannot fit two tabs with them.
  - A `⋯` menu holds Quote selection, Annotate and Reveal in Finder. Quote can also appear only
    while a selection exists. **The Annotate item keeps the accessible name "Send a region to the
    chat"**: the CI job `preview-panel` (`.github/workflows/frontend.yml`, macOS) runs
    `scripts/preview-panel-e2e.mjs`, which calls `page.getByLabel('Send a region to the chat').click()`
    at `:155`. Update the script in the same commit to open `⋯` first, and update
    `ArtifactViewer.browserBridge.test.tsx:153,169`, which asserts that button's name and its
    `title`.
  - The `“` text glyph is gone. Fold (in the stack layout) stays.
  - The tab icon is 14px and the close glyph is 14px in a 20px hit area. The active tab is flat:
    drop `--shadow-raised`, coordinating with WS-MOTION on the shared `.br-tab` rule.
- **One `PreviewBar`** (new, 32px, `--tab-height`). It renders for **every** kind from the artifact
  metadata **before the content loads**, so it never disappears.
  - **Left:** the file name in 13px sans default ink, with the full path in a tooltip.
  - **Middle:** meta in `text-supporting` muted, for example `300 lines`, `500 of 620 rows`,
    `7 cells`, `800×480 · 42 KB` or `12 pages`.
  - **Right:** an icon-only Copy (Copy, then Check, with the tooltip "Copy" and failures in the
    tooltip plus `aria-live`), then the view switch (`SegmentedControl`: Preview/Raw, Table/Raw,
    Fit/1:1, or the sheet picker with **sheet names**).
  - Its horizontal padding equals the paper inset, so its text sits on the content's left edge.
  - It replaces `ArtifactViewer.tsx:2546-2594` and `NotebookPreview.tsx:231-248`, and drops the
    language label (`STRIP_LABEL_CLASS`).
- **Stale-while-loading:**
  - Cache the last `PreviewState` per tab in a `Map`, keyed by source key and revision.
  - Activating a tab renders the cache at once and refreshes it in the background. **Never return
    `{kind:'loading'}` for a tab that has been seen** (`:1159-1160`).
  - On a first load, show nothing for 200ms, then a 3-line skeleton on paper with the PreviewBar
    already rendered. This fixes the "Loading" flash and the 4.5 to 13 second blank panel
    [baseline].
- **Switch motion:**
  - Crossfade with `.br-crossfade`: the incoming layer fades and rises 8px over 175ms, and the
    outgoing layer fades over 125ms. For an iframe, start only on `load`. A live page
    (WebContentsView) uses opacity only.
  - Images fade in on `onLoad`, PDF canvases on `data-rendered="true"`, and docx/pptx on
    `rendered`.
  - Preview↔Raw is a 125ms opacity crossfade that keeps the scroll position for each view.
- **Panel motion** ([motion F1-F8]; the grid **snaps**):
  - **Open:** the body goes from opacity 0 and `translateX(32px)` to 0, over 300ms with
    `EASE_OUT` (this replaces the literal `cubic-bezier(0.2,0,0,1)` at `usePreviewMotion.ts:60`).
    The conversation glide belongs to WS-SUMMARY's hook. While the conversation slides out from
    under the panel edge, the viewer sits above it with `position:relative; z-index:1` on an
    opaque ground.
  - **Close:** opacity and `translateX(16px)` over 125ms, then unmount at
    `ARTIFACT_PANEL_EXIT_MS`=125.
  - **Stack:** travel −16px (it is −32 today).
  - **Side↔stack:** when an action causes it, crossfade the body over 250ms. When a resize causes
    it, **snap**: do not start the orientation animation while `isWindowResizing()`.
  - Expose `onPresentedChange` for the glide.
  - **Nothing pops.** Today the `<aside>`, its strip and its buttons appear in one frame and only
    the body moves [preview-panel finding 2]. The strip and header fade in with the body (opacity
    only, same timing) and fade out with it, so no part of the panel appears or vanishes in a
    single frame. Keep the motion on `previewBodyRef` (`measures.test.ts:818-821` pins
    `usePreviewMotion(previewBodyRef`); the strip's fade is authored CSS keyed on the same state.
- **Window grow.** Start the panel open in the same frame as the request: do not
  `await ensureFits()` before `setPresented` (`useArtifactPanel.ts:286-296, 331-337`). Measure
  both variants first and record the result in
  `docs/desktop-ui/window-scaling-regressions.md`. WS-MOTION gates the OS animation on Reduce
  Motion in `main.ts`. If the concurrent version looks busy, the fallback is an instant grow.
- **Content details:**
  - **Code:** padding 28/48 becomes 16/32. Keep 13/20 mono, the 70% gutter ink and the
    `wrapLines`-without-`wrapLongLines` invariant.
  - **CSV:** the bar count reads `500 of 620 rows`, and the cap sentence moves into its tooltip. The
    table uses the shared `.br-md-table` recipe (section 0); the mono row index stays.
  - **Notebook:**
    - The DataFrame sandbox CSS copies the `.br-md-table` values (it renders in a sandbox, so the
      class cannot reach it): 12/600 muted header on `--background-muted`, no bold index, hairlines.
    - Image outputs get radius 10 plus a `0 0 0 1px var(--border-subtle)` ring.
    - **Leave the iframe height floor alone:** reporting the height needs a security decision.
  - **Directory:**
    - The rail header uses the same 32px bar height.
    - The filter moves into the bar as a search icon, or stays under it.
    - Drop the `Eye` icon on the selected row and drop the git legend; the status already lives in
      the row tooltip.
    - The empty state reads "Select a file", or auto-selects the README.
  - **Image:** paper ground, the PreviewBar with dimensions and size, and a Fit/1:1 toggle.
  - **Documents:** the fidelity sentence becomes an `Info` InfoTip in the bar, status overlays
    become the skeleton, and the XLSX font stack matches the body stack.
  - **External page card:** drop the "External page" heading and the floating shadow.
  - **Binary card:** the name, `DOC · 24 KB` and one button; the reason and the suggestion go into
    an InfoTip [preview-panel §4].
  - **"Update ready" / "Retry update"** text buttons (`ArtifactViewer.tsx:1451`,
    `WebPagePreview.tsx:293-302`) become a refresh icon with a 6px accent dot and a tooltip.
  - **Front matter:** the title is 24/400 (`text-title`), the subtitle 14px muted, and the byline
    12px.
- **Harness** (`ui/desktop/.artifact-harness/`):
  - Add a "panel" mode. It mounts `useArtifactPanel` in a real `[data-preview-split]` grid with a
    fake header, transcript and composer and Open, Close and Switch buttons, so motion can be
    filmed.
  - Treat `text/html` returned for a non-.html fixture as a 404.
  - Add the missing `embeddedBrowser` and `openArtifactInBrowser` stubs.

**Files:**
- `artifacts/**` (`ArtifactViewer`, `DocumentPreview`, `MarkdownDocument`, `NotebookPreview`,
  `DelimitedTable`, `WebPagePreview`, `AnnotationOverlay`, `useArtifactPanel`, `usePreviewMotion`,
  utilities) and their tests.
- New `artifacts/PreviewBar.tsx`.
- `styles/prismGrammars.ts`.
- `ui/desktop/.artifact-harness/**`.
- `ui/desktop/scripts/preview-panel-e2e.mjs` (the CI Electron preview job).
- `main.css` families "preview".
- `styles/artifactPaper.test.ts`.

**Tests:**
- `usePreviewMotion.test.tsx:63-83` keeps the 300/250/125 durations and adds cases for the token
  easing and for skipping during a resize.
- `ArtifactViewer.test.tsx:204` and `ArtifactViewer.liveRefresh.test.tsx:146` query
  `[data-preview-loading]` instead of the word "Loading".
- `useArtifactPanel.pendingOpen.test.tsx`.
- `npm run test:preview-panel` (needs Electron; the macOS CI job runs it): the image, PDF
  (`canvas[data-rendered="true"]`), DOCX, XLSX (`iframe[name="biorouter-spreadsheet-preview"]`),
  PPTX (`.artifact-pptx-preview[data-rendered="true"]`) and annotation-crop checks must keep
  passing, so keep those selectors.
- **Keep** `measures.test.ts:739-746, 818-821` true: no motion inside the grid block, and the
  motion still goes through `usePreviewMotion(previewBodyRef`.

**Acceptance:**
- Re-shoot `audit/img/preview/01-15` at 600px and 380px, light and dark.
- The active tab is never clipped.
- There is no "Loading" flash on a tab switch.
- Every kind shows the 32px bar and paper ground.
- The strip shows two header icons and `⋯`, at the same size as the chat header's.
- On open and close, no part of the panel appears or disappears in a single frame.
- Document headings have no coral bars, and there is one table style.
- A 60fps recording of open and close shows the body sliding with no jump of the conversation (the
  WS-SUMMARY glide) and no white flash in dark mode.

---

### 3.9 WS-USAGE: the Home usage card and Settings usage (one implementation)

**Goal:** message 1 and screenshot 4. A Claude/Codex-style usage card that keeps the coral
heatmap, and one stat style shared with Settings.

**Home layout** (`Hub` content via `sessions/SessionsInsights.tsx`):
- Delete the `UCSF Biorouter` eyebrow.
- The greeting is `text-title` (24/32/400) with `text-wrap: balance`. Keep the random questions
  (personality).
- The hero text edge equals the composer's text edge (16px in from the card): `pt-16` (64) and
  `pb-5` (20).
- Then `<section aria-label="Usage">`, the **HomeUsageCard**.
- Keep the container queries that compact or hide the hero at short heights
  (`main.css:3525-3563`).
- Optional: group the hero, card and composer with `my-auto`.

**The card** (new `sessions/HomeUsageCard.tsx`):
- **Shell:** width `var(--measure-chat)`, radius 12, 1px `--border-subtle`, `--background-card`,
  no shadow.
- **Header row** (36px, `--dock-height`), with a hairline under it and `px-4`:
  - Left: **underline `Tabs`** Overview | Models (`ui/tabs.tsx`, `TabsList` with `border-b-0`).
  - Right: a `SegmentedControl` for the range, **`1y · 30d · 7d`**, with
    `aria-label="Time range"` and the item names "Last year", "Last 30 days" and "Last 7 days".
  - The default range is `1y`, persisted in `localStorage['biorouter-home-usage-range']` with
    try/catch. The tab is not persisted.
  - The `1y` label follows owner decision 6.4. Do not ship "All" over a partial window.
- **Body** (`p-4`, tiles block `min-h-[112px]` so both tabs keep one height):
  - **Overview tiles:**
    - `<dl class="grid grid-cols-3 gap-x-6 gap-y-4">`, with two columns under a 480px container.
    - Each `dt` is `text-supporting` muted. Each `dd` is `text-title` (24/32/400) with
      `tabular-nums`, truncated, with an overflow tooltip.
    - There are no tile boxes or icons.
    - **The six tiles:**
      1. Chats: the sum of `sessions` in range.
      2. Messages: the sum of `messages`. **It undercounts:** the daemon drops a day that has
         messages but no new chat and no billed tokens (`build_activity_window`, pinned by the Rust
         test `messages_alone_do_not_create_an_active_day`; usage-home §1.1). Give the tile the same
         lower-bound InfoTip as Tokens ("Counts days with a new chat or billed tokens."), since
         decision 6.4 keeps the backend unchanged.
      3. Tokens: the sum of `tokens`, compact (`87.9B`), with the exact number in the tooltip.
         When any day in range is incomplete, show an **InfoTip**: "Some days lack full token
         records, so this total is a lower bound." When the sum is 0 and incomplete, show
         "Unavailable" with the InfoTip "No trustworthy token total was recorded for this
         period."
      4. Active days, shown as `12` with a muted `/{days in range}` (`/7`, `/30`, `/365`).
      5. Streak: `currentStreak` with the InfoTip "Longest in the last year: N days" (the daemon
         computes `longestStreak` inside the fetched window only).
      6. Favorite model: the row with the most tokens in
         `/usage/report?group=model&from=rangeStart`, shown through `getModelDisplayName`. When
         that call fails (403 on `serve`), the tile becomes **Longest streak**.
  - **The Models tab:**
    - At most 4 rows of 28px: the top 3 by tokens, plus Other.
    - Columns: the model name (`text-body`, truncated, tooltip `provider/modelId`); a 4px share bar
      (track `--heat-0`, fill `--heat-3`); tokens right-aligned and tabular; and the share in
      `text-supporting` muted.
    - The tab carries an InfoTip "Includes deleted chats and subagents." because its scope differs
      from Overview.
    - The empty state is "No model usage in this period."
    - On a `biorouter serve` browser `/usage/report` answers 403 (`require_billing_user`): hide the
      Models tab trigger there rather than showing an empty or error state.
  - **The heatmap**, `mt-6`, on both tabs. It always shows the full window and does not follow the
    range.
    - The grid is fluid (Codex), with **one column per week of the fetched window**: 371 days is
      53 columns (`weeks = ceil(days / 7)`, computed, never a literal). The earlier `repeat(26, …)`
      contradicted "the full window": it would show half a year under a `1y` default. At the 760px
      card (726px inside) a column is about 10.8px with a 3px gap and the grid about 94px tall, so
      the fitting code can go. If the cells read too small in the vision pass, show the last 26
      weeks and say "Last 6 months" in the heatmap's label; do not mix the two.
      `grid-auto-flow:column; grid-template-rows:repeat(7,1fr); grid-template-columns:repeat(var(--heat-weeks),minmax(1px,1fr)); gap:3px`.
    - Cells are `aspect-square` with radius 2 and the fills `--heat-0..4`.
    - **Drop the weekday labels.** Month labels are fixed at 12px.
    - Keep the single Tab stop, the roving arrows and the cell tooltip. Drop `useFittedMetrics` and
      `chromeFor`.
  - **Footnote row:** `mt-3`, `text-supporting` muted.
    - Left: the streak, led by the 12px ring glyph: "3 day streak · longest 16 days"; or "Longest
      streak 16 days" when the current streak is 0; or nothing when both are 0.
    - Right: `Less ■■■■■ More`.
    - **Delete:** the 17px streak heading, the caps "Longest streak" line, the
      incomplete-history paragraph (it also wrongly carries `role="status"`), and every
      "busiest day" or "Highest recorded estimate" footer.
- **Heatmap tooltip:** `rounded-container` (not `rounded-surface`); rows in `text-supporting`
  with `py-0.5`; values in sans `tabular-nums font-medium`. Keep its notes, which are
  test-pinned.
- **Loading:** the same card with the triggers disabled, six tile skeletons and the grid skeleton.
  Keep `role="status" aria-label="Loading usage activity"` and no visible text. On a hard failure
  with no cache, collapse the card.
- **Motion:**
  - The tab panel crossfades over 175ms.
  - On a range change, each `dd` remounts with a 125ms fade, with no count-up.
  - On the first paint of a session only, cells reveal with a stagger of `col*8 + row*12`ms over
    `--dur-med` (300ms) on `--ease-out` (scale .85 → 1). Turn this off under reduced motion. This
    is a named exception to the "30ms stagger, at most 5 items" rule in 2.4 (one deliberate moment
    of delight, message 10); never on a range change, a tab change or a remount after navigation.
- **Data:**
  - `utils/homeInsightsCache.ts`: `HOME_ACTIVITY_DAYS` goes from 155 to **371**, and the storage
    key bumps to `biorouter-home-insights-v2`. A v1 blob must not crash.
  - The pure `utils/usageStats.ts` (React-free) provides `rangeStart`, `deriveUsageStats`,
    `formatTokensCompact`, `streakFootnote` and `favoriteModel`.
  - **No backend change** (owner decision 6.4).

**Settings usage** (`settings/usage/*`, [settings-b UP1-UP12, US1-US3]):
- **The row:**
  - The label is "This month", replacing the coral ISO chip. Keep `2026-07` in the row's InfoTip
    so `UsagePanel.test.tsx:584` still finds it.
  - The value is `33M tokens · $125.00 · 42 turns` in `text-secondary` tabular.
  - The action is `secondary sm` "Details". Keep the aria-label "Open detailed usage report".
- **The dialog:**
  - `ModalShell size="lg"` (640, down from 1040), with the title "Usage, October 2026". Keep the
    heading "Usage report" if the test is not updated.
  - There is no band, icon tile or description.
- **Stats:** the shared **StatGrid** (new `components/usage/StatGrid.tsx`, also used by Home)
  shows Tokens, Est. cost and Turns, each a label in 12px muted over a value at 24/400. The detail
  lines move into InfoTips. "Known conservative subtotal" stays visible as a 12px suffix when it is
  true.
- **Budgets:** flat rows (label plus `used / limit (pct)`) with `Progress` under them. An
  unavailable reason is a `text-supporting` line.
- **Sections:** "By day" and "By model" have caps labels and no cards.
  - The tables use `text-secondary` with hairlines and tabular figures, and a header row in 12px
    muted.
  - The token flow becomes one line, "3M in · 1M out", with the cache split in a tooltip, so the
    model table fits 640px.
  - The provider under the model id is `text-supporting` subtle (not caps).
- **"About these totals"** becomes the one-line footnote "Some totals are estimates." with an
  InfoTip, shown only when it applies. Keep its testids on sr-only list items.
- **Loading:** one row-sized skeleton (`h-10 w-full`).

**Files:**
- `sessions/{SessionsInsights,UsageHeatmap}.tsx` and their tests.
- New `sessions/HomeUsageCard.tsx` and test.
- New `components/usage/StatGrid.tsx` and test.
- New `utils/usageStats.ts` and test.
- `utils/homeInsightsCache.ts` and its test.
- `common/Greeting.tsx` and its test, and `hooks/use-text-animator.tsx` (Greeting's only
  consumer outside tests).
- `settings/usage/**` and its tests.
- `main.css` families "home and heatmap".

**Tests:** [usage-home §5]:
- `UsageHeatmap.test.tsx:95-132, 164-210, 357-365`: the window-level assertions move to the
  card's InfoTip and `streakFootnote`.
- `SessionsInsights.test.tsx` mocks `HomeUsageCard`: there is no "UCSF Biorouter" text, and the
  greeting has `text-title`.
- `homeInsightsCache.test.ts`.
- `UsagePanel.test.tsx`: `:191-198, 294-302, 375-391, 572-594`.
- `UsageSection.test.tsx`.
- `settings/settingsVocabulary.test.ts:62` names `SessionsInsights.tsx` and `UsageHeatmap.tsx`.
  A new `HomeUsageCard.tsx` must comply, or the list must be updated by WS-SETTINGS-A on request.

**Acceptance:**
- Home at 1440×900 and 1100×760, light and dark, shows the greeting, then one card (tabs, range,
  six tiles at 24px, the coral heatmap, one footnote line), then the composer.
- There are no caps, no 17px text, no 8 to 11px labels, and no paragraph.
- Range changes move the tiles but not the heatmap.
- In dark mode the zero cells are visible.
- The Settings usage dialog is 640px with no nested boxes and no icon tiles.

---

### 3.10 WS-VIEWS-A: shared page chrome, Workflows, Scheduler, Built apps

**Goal:** message 5. These pages take Crew's language: a 44px band, two-line rows, row actions
revealed on intent, one accent per view and help in InfoTips.

**Shared chrome:**
- **`Layout/PageHeader.tsx` becomes a band.**
  - The API is `title`, `info`, `adornment`, `actions`, `tabs?` and `onBack?`. `description` stays
    as a deprecated alias that renders as `info`, so the owners can migrate one file at a time.
  - Markup:
    `<header class="biorouter-page-header">` (authored: `h-chrome`, `bg-sidebar`, a bottom
    hairline `--border-subtle`, `pl-4 pr-3`, `gap-2`, `items-center`) contains:
    - `[←]` when there is `onBack` (a ghost round 32px button);
    - `<h1 class="text-label truncate">`;
    - the InfoTip;
    - `adornment` in `text-supporting` muted tabular;
    - `tabs`, when present, rendered inline and filling the band's height, with the underline at the
      band hairline;
    - `ml-auto` actions with `gap-1`: ghost round 32px icons with a `Tooltip` and `aria-label`,
      plus at most **one** primary `Button` at the default size, plus an optional `FilterInput`
      (2.6). Every control is `no-drag`.
  - **The band itself is a drag region**: `.biorouter-page-header { -webkit-app-region: drag }`,
    with `no-drag` on every interactive descendant (buttons, the filter input, the KB selector
    trigger, the tier badge-button, tabs). Once the 32px strip stops taking pointer events on these
    routes, the band is the only thing that lets the window be dragged from the top.
  - When the sidebar is collapsed, the title takes the titlebar reserve as `margin-left`
    (`getSessionTitlePadding()`, `TitlebarControls.tsx:36`).
  - The band does not sit in `ReadableContent`; only the body does.
  - Record the reversal of the 2026-09-07 "actions on their own line" decision in
    `docs/design/astryx-adoption/astryx-ui-adoption-design.md` §4.2.
- **`Layout/MainPanelLayout.tsx`:** band routes pass `removeTopPadding`. The `pt-[32px]` literal
  becomes `pt-[var(--titlebar-drag-height)]` for any route that keeps it.
- **`Layout/AppLayout.tsx`:**
  - Rename `isChatRoute` to `routeOwnsTopBand` and extend it to every band route. After this
    redesign that is **every route inside `AppLayout`**: `/` (Hub), `/pair`, `/crew`, `/settings`,
    `/extensions`, `/applications`, `/sessions` (History and the saved transcript), `/schedules`,
    `/workflows`, `/skills`, `/knowledge`, `/shared-session`, `/permission`, and the `*` catch-all
    (`NotFoundView` renders `PageHeader`). Write it as "inside the shell" rather than a list, so a
    new route cannot reintroduce issue #74. The routes outside the shell (`/launcher`, `/welcome`,
    `/configure-providers`) keep the strip.
  - **Verify this in the real app** with the sidebar open and collapsed: every band control is
    clickable, and the window drags from empty band space on every route. jsdom cannot see drag
    rects.
- **The body** is `px-6 pt-2 pb-6` inside `ReadableContent size="chat"`.
- **List rows** use `.biorouter-list-row` (WS-PRIMITIVES recipe):
  - line 1 is the title (`text-label`, `min-w-0 truncate`) plus `Badge` chips;
  - line 2 is the description or meta in `text-supporting` muted, clamped to 1 line;
  - on the right, `RowActions` with a `meta` slot holding the relative time, which gives way to the
    actions on hover or focus;
  - right-click and Shift+F10 open the same menu;
  - row icons are not 12px, blue meta ink is gone, and dates are not mono.
- **Empty states** use `EmptyState` with a description of at most one short sentence, and either no
  actions or one `variant="link"`. The band holds the only accent.
- **Toasts** are one line ("Link copied", "YAML copied", "Saved to {file}", "/{cmd} saved").
  Errors keep their reason.
- **Strings** move to `workflows/copy.ts`, `schedule/copy.ts` and `applications/copy.ts`.

**Workflows** ([views-a §4]):
- **The band:** "Workflows" with an InfoTip "Reusable chat setups. Start one from here or with its
  slash command." The actions are the band `FilterInput` (2.6; ⌘F focuses it), Import (`Upload`)
  and the primary "New workflow". `SearchView` is no longer used here: WS-MOTION turns that bar
  into the transcript's find overlay (3.16).
- **Rows:**
  - Line 1 is the title plus the neutral Badges `/slash`, a short cron ("Daily 2 PM") and
    Built-in.
  - The actions are **Run** (`Play`) and `⋯`. The menu holds Run in new window, Edit, Slash
    command…, Schedule…, then a separator, Copy link, Copy YAML and Export to file…, then a
    separator and Delete.
  - A row click opens Edit. Keep the 32px `shape="round"` and the `aria-label`s, and move tests off
    `findByTitle`.
- **Editors** (`CreateEditWorkflowModal`, `CreateWorkflowFromSessionModal`):
  - Both use `ModalShell size="lg" scrollBody` with a title only.
  - Share moves out of the editor.
  - One field skin: `ui/Input`, `ui/Textarea` and `ui/Field`.
  - Every label is `text-label`; there are no `text-caps` field labels.
  - Errors are `text-supporting` in danger ink.
  - No red asterisks; optional fields get a muted "(optional)".
  - The Advanced section is a `Disclosure` whose summary states the defaults ("Global model · no
    parameters · all extensions").
  - Every helper listed in [views-a §8] becomes an InfoTip, is deleted, or is shortened.
  - The resource picker uses **`Checkbox` rows, not switches**.
  - Fake progress becomes one line, "Reading this chat…".
  - The copy is "Create" / "Create and run" and "Save" / "Save and run".
  - Delete the dead `shared/WorkflowNameField.tsx`.
- **The slash and schedule dialogs** use `ModalShell size="md"` with the workflow title as the
  subtitle, plus one live helper line.

**Scheduler** ([views-a §5]):
- **The band:** "Scheduler" with an InfoTip. The actions are Refresh (ghost round) and the primary
  "New schedule".
- **Rows:**
  - A leading 8px `StatusDot` replaces the clock: Running pulses, Paused is muted, Failed is danger
    and Scheduled is success.
  - There is **one** meta line: "{status} · Every day at 2:00 PM · Last run Oct 7, 2:00 PM" in sans
    tabular. The full cron goes in the tooltip of the cron phrase. A last-error line replaces that
    meta line in danger ink.
  - The actions are Pause/Resume (or Stop while running) and `⋯` (Edit, Run now, Inspect run, then a
    separator and Delete).
  - Use **one verb pair, Pause/Resume**, everywhere.
- **The detail view:**
  - The band holds back, the name and the status, with the actions Run now (primary),
    Pause/Resume and `⋯`.
  - The facts list uses names, not IDs: Runs (with the cron in a tooltip), Workflow (the file name),
    Last run, Last error and Started, plus an "Open chat" button.
  - Delete the Actions section and the three Notes. The disabled controls explain themselves in
    tooltips ("Available when this run finishes").
  - Run rows are sans tabular.
- **Toasts and states:**
  - Toasts use sentence case with names: "Couldn't pause {name}", "Run started".
  - "Schedule not found" uses `EmptyState` with Back.
- **Other details:**
  - `ScheduleModal` uses `Field`, and the "Letters, digits, hyphens and underscores only" helper
    stays visible.
  - `CronPicker`: `w-[4.5rem]` becomes `w-20`, and the error is shown in danger ink.

**Built apps** ([views-a §6]):
- **The band** is fixed and does not scroll: "Built apps" with an InfoTip "Apps made with Agent
  Drafter. Each runs its own agent and opens in your browser." The actions are the band
  `FilterInput` and Refresh. There is no primary. `SearchView` is no longer used here.
- **Rows:**
  - The title plus a kind Badge, whose tooltip carries the surface summary ("2 actions · 1
    signal").
  - The meta line is "{model} · {kb}" in sans.
  - The actions are **Launch** and `⋯` (Open the chat it was built in, Export…, then a separator
    and Delete). Delete lives only here.
- **The empty state** reads "Ask Biorouter to build one with Agent Drafter."
- **The Export dialog** uses `ModalShell size="md"`:
  - Keep "Credentials are never included." visible.
  - The radio options are "Launcher" and "Full", each with one consequence line.
  - Group InfoTips replace the per-item hints, and the item rows are checkbox plus name.
  - The copy is "Include Biorouter (~110 MB)".

**Files:**
- `Layout/{PageHeader,MainPanelLayout,ReadableContent,AppLayout}.tsx` and their tests.
- `workflows/**` except `WorkflowActivities.tsx`.
- `schedule/**`, `applications/**`, `parameter/**`, `ParameterInputModal.tsx` and
  `NotFoundView.tsx`.
- `styles/measures.test.ts` (this workstream is its **sole owner**; other workstreams request
  edits).
- `main.css` family `.biorouter-page-header`.
- `docs/design/astryx-adoption/astryx-ui-adoption-design.md`.

**Tests:** [views-a §9]:
- `PageHeader.test.tsx`; `AppLayout.test.tsx:192-198`.
- `measures.test.ts`: `:131-143` and `:481-517` keep "views import PageHeader, no `<h1`, no
  `pt-12`"; change the `:462-466` literal to a regex. `ScheduleDetailView.test.tsx:245` asserts
  the same literal `<MainPanelLayout>` and changes with it. Add `sessions/SessionHistoryView.tsx`
  and `sessions/SharedSessionView.tsx` to `PAGE_HEADER_VIEWS` once WS-HISTORY moves them onto the
  band (their own `<h1 className="text-title">` goes).
- `AppLayout.test.tsx:192-198` asserts `/settings` and `/knowledge` are not chat routes; it now
  asserts the inside-the-shell rule, including the catch-all.
- e2e `schedule-artifact.spec.ts:212` waits for the heading "Scheduler"; the band keeps it an
  `<h1>` with that name.
- `WorkflowsView.test.tsx` (titles to names); `WorkflowFormFields.test.tsx`;
  `WorkflowActivityEditor.test.tsx`; `WorkflowResourcePicker.test.tsx` (`role:'switch'` becomes
  `checkbox`); `CreateWorkflowFromSessionModal.test.tsx`.
- `SchedulesView.test.tsx`; `ScheduleDetailView.test.tsx`; `ScheduleModal.test.tsx`.
- `ApplicationsView.test.tsx`.

**Acceptance:**
- Workflows, Scheduler and Built apps at 1440 and 1100, light and dark, show the first row within
  about 56px of the window top, not 234px.
- The band hairline meets the sidebar band at y=44.
- The window drags from the band's empty space on each of these routes, with the sidebar open and
  collapsed, and every band control takes a click.
- Each row has two lines.
- At rest a row shows only the time. On hover, one icon and `⋯` appear.
- There is one coral button per view.
- No paragraph appears under any title.
- The editor dialogs are 640px with one field skin.

---

### 3.11 WS-VIEWS-B: Extensions, extension dialogs, BAAM, Skills

**Goal:** the same Crew language as WS-VIEWS-A, on the busiest list pages.

**Extensions** ([views-b §3]):
- **The band:**
  - "Extensions", a count, and an InfoTip: "Extensions add tools, prompts and resources. Enabled
    ones apply to every new chat."
  - A **visible filter**: the shared `FilterInput` (2.6; placeholder "Filter", tooltip
    "Filter · ⌘F", ⌘F focuses it). It replaces the `SearchView` that only ⌘F revealed, and
    removes the "⌘F to search" sentence that `ExtensionsView.test.tsx:108-117` calls load-bearing.
  - **One primary "Add" dropdown:** Browse marketplace, Install from file (.brxt), Custom
    extension….
  - The body is `px-6 py-4`.
- **Group labels** are `text-caps` muted tabular, "Enabled 7" and "Available 12", with no dots.
  Groups sit `gap-6` apart.
- **Rows:**
  - Use the shared row recipe at `py-2.5`, `items-center` and `gap-3`, with a `text-label` title
    and the badges `BuiltInBadge` and `PrivacyBadge`.
  - Show **one** secondary line: the description, clamped to 1 line, in `text-supporting` muted.
  - **The three §13.5 provenance strings stay visible.** They are "Private: published on the
    Biorouter marketplace", "Public: published on the Biorouter marketplace" and "Public: installed
    from a file, not on the marketplace. Any model can call it." They sit on a `text-supporting`
    subtle line, on rows where they apply, instead of the description line when there is no
    description.
  - **The built-in sentence** ("Public: built into Biorouter, …") moves into the PrivacyBadge's
    `InfoTip asChild`, with an sr-only copy so `/built into Biorouter/i` still matches.
  - **The pairing notice is privacy state** and stays visible in short form: a warning `Badge`
    "Unavailable in new chats", with the full sentence, including the provider name, in its
    InfoTip and sr-only text.
  - The command line moves into the Configure dialog.
  - The gear becomes a `⋯` menu (Configure…, Remove…) with a Tooltip, revealed by Crew's
    `[data-row-action]` rule (`hover:none`), not by an `sm:` breakpoint.
  - The switch is named by the row title through `aria-labelledby` (2.6), stable across states.
    Update `tests/e2e/app.spec.ts:182-184` (owned by WS-SETTINGS-A; send the diff).
- **`ExtensionLoadFailureNotice`:**
  - A danger `Note` holds the title line and the clamped error.
  - Its action slot holds two ghost `sm` actions ("Ask Biorouter", "Copy error"), and Dismiss is a
    ghost round X.
  - Use "Copied" with no exclamation mark, and `mb-4`.
- **Highlight** uses `.br-highlight`, not an inline ring with `--color-block-teal`.
- **Toggle toasts:** drop the success toast and keep the error toast (owner decision 6.9) in
  `settings/extensions/extension-manager.ts:44-47`. Shorten the Chat Recall suggestion toast.
- **Delete** the dead `!hideButtons` branch and its modals and props in `ExtensionsSection.tsx`.
- **The catalog freshness line** moves into an InfoTip on the first group header (sr-only copy kept
  for the test).
- **Extension modal** (`settings/extensions/modal/*`):
  - **P0 bugs:**
    - The timeout error reads "Timeout " (`ExtensionTimeoutField.tsx:41`). It becomes "Enter a
      timeout in seconds." in normal flow, with the label "Timeout (seconds)".
    - Every `<label>` gets `htmlFor` and an input `id`.
    - The icon-only Edit and Remove buttons get `aria-label="Edit {key}"` and `"Remove {key}"`, a
      Tooltip, the 32px rung and 16px icons.
    - Errors are shown in normal flow, not `absolute`.
  - **P1 chrome:**
    - `ModalShell size="lg"` with no title icon (`text-iconStandard` is dead).
    - Sentence case: "Remove "x"?", "Unsaved changes", "Discard changes", "Installation notes",
      "Environment variables", "Request headers".
    - Help moves into InfoTips [H7-H14].
    - Installation notes use a `Note tone="info"`.
    - One full-strength hairline, or section spacing, replaces the 45% dividers.
    - Remove uses `variant="destructive"`; Cancel uses `secondary`; drop `mr-2` and the focus
      overrides.
    - `Retained — …` becomes "Kept: used by …".
  - **The Add-form tier line stays visible** (§13.5 mandate), in `text-supporting`.
- **BAAM** (`baam/*`):
  - One new shell, `baam/MarketplaceDialog.tsx`, on `ModalShell size="lg"`, with the subtitle "From
    the Biorouter marketplace" plus an InfoTip, and freshness when the catalog is not live.
  - The search uses `Input` with a leading icon and the placeholder "Search".
  - Rows are flat, with no icon tiles and no tag rows. The description is clamped to one line.
    "Installed" is a `Badge`.
  - Use `ui/Checkbox`. The category filter uses the chip pattern (`Badge variant="chip" asChild`
    plus `tint-selected` plus `aria-pressed`, `type="button"`).
  - Use no `text-[10px]` and never a background token as ink. Secondary buttons are `secondary`.
    Drop the footer that only holds Close.

**Skills** ([views-b §6]):
- **The band** follows the Extensions shape: an InfoTip, the filter Input, and an "Add" dropdown
  (Browse marketplace, From a repository or .zip…, Write a skill…).
- **Group labels** are `text-caps` "Biorouter 12", "From other agents 3", "From this project 2",
  with no dots.
- **Rows:**
  - One secondary line, `items-center` and `py-2.5`.
  - One `⋯` menu (Open folder, Copy SKILL.md, Delete…) with a Tooltip.
  - The text block is no longer a duplicate button, and there is no source-path line.
  - The switch is named by the row title through `aria-labelledby` (2.6).
- **Bundle rows:** collapsed, a bundle shows its name plus `text-supporting` "{n} skills ·
  {version}". The members and the "Entry point: x" line appear only when it is expanded.
- **Dialogs:** `AddSkillModal` and `CustomSkillModal` use `ModalShell size="lg"`. The drop zone is
  one line, "Drop a .zip here", in Knowledge's dropzone recipe. Help goes in InfoTips.

**Files:**
- `extensions/**`.
- `settings/extensions/**` (including `modal/`, `subcomponents/` and `extension-manager.ts`).
- `baam/**`, `skills/**`.
- `components/{BrxtInstallModal,ExtensionInstallModal,ExtensionUpdateReporter}.tsx` and
  `brxtInstallFlow.ts`.
- `ui/desktop/tests/e2e/brxt.spec.ts`.

**Tests:** [views-b §10] and [settings-b §5 Extensions]:
- `ExtensionsView.test.tsx:100-145`. Its "no button in the h1 parent" test encodes the decision
  being reversed; say so in the commit message.
- `ExtensionItem.test.tsx` (switch names).
- `ExtensionsSection.privacy.test.tsx`: these must keep passing through visible text or sr-only
  text: `:72-94`, `:111-115` (verbatim, visible), `:151-159` and `:169`.
- `ExtensionModal.test.tsx` and `ExtensionCredentialsDialog.test.tsx:35`.
- `BrowseExtensionsModal.test.tsx` and `BrowseSkillsModal.test.tsx`.
- `SkillsView*.test.tsx` and `AddSkillModal.test.tsx`.
- `extension-manager.test.ts:37-52`.
- e2e `brxt.spec.ts:144-187`.
- After WS-SETTINGS-A removes `'extensions/'` from the vocabulary exclusions, the rules must hold
  here.

**Acceptance:**
- Extensions and Skills at 1440 light and dark show a band with a filter and one Add menu.
- Rows have at most two text lines.
- The provenance line is visible on marketplace and file rows only.
- The pairing state shows as a warning badge.
- There are no 10 or 11px labels, no blue selection washes and no curled dividers.
- The failure notice is compact.
- Both BAAM dialogs are 640px, with flat rows and real checkboxes.

---

### 3.12 WS-KNOWLEDGE: keep the structure and logic, simplify the skin (message 6)

**Goal:** keep the same flows, controls and layout ladder, with fewer boxes, less text and one
band.

**Behaviour** ([views-b §7]):
- **One 44px band**, rendered through `Layout/PageHeader.tsx` (title, `info`, `adornment` for the
  selector and badges, `tabs`, `actions`) so the app has one band implementation and the band is a
  drag region with `no-drag` controls (3.10). If the API cannot carry the selector, WS-VIEWS-A
  extends it on request rather than Knowledge hand-building a second band. It merges the title band
  and the subject band:
  - `<h1 class="text-label">Knowledge</h1>`, which is still named "Knowledge" for e2e
    `knowledge-ingest.spec.ts:51,95`;
  - an InfoTip "Personal knowledge bases Biorouter builds and maintains for you.";
  - `/` and the existing `KBSelectorTrigger variant="subject"`;
  - the format `Badge` in sentence case (OKF, BioOKF, Legacy), with Legacy wrapped in an `InfoTip
    asChild`;
  - **the tier badge-button**, Crew's `ClassificationBadge`: a `Badge asChild` button reading
    "Private" or "Public", with the tooltip "Only private models can read or write this base" or
    "Any model can read this base". Pressing it opens a menu with "Make public…" or "Make
    private";
  - counts in `text-supporting` tabular sans (no mono);
  - the Sources/Graph tabs at the narrow step;
  - and the actions in Tooltips, replacing `title=`, with `ml-auto`.
  - Delete the `.br-knowledge-subtitle` rule and its container hide.
- **The tier control is privacy UI.** **Only the trigger moves.** `KbTierControl`'s confirmation
  (the typed base id, the blast radius from `getKbTier`, `userActionHeaders()`) and its daemon call
  stay byte-for-byte. The reviewer must diff that path. Moving the trigger fixes the narrow-pane
  case, where the tier and its control were hidden behind the default Graph tab.
- **Flatten:**
  - The Sources rail and the graph lose their cards. Each becomes a flush column separated by
    `box-shadow: inset -1px 0 0 var(--border-subtle)`.
  - Delete the 40px "SOURCES" strip and the rail's boxed tier control.
  - The gutter becomes `px-6`, and the workspace fills edge to edge under the band.
  - The facet strip is 44px (`h-chrome`).
  - **Keep the 860, 940, 1140 and 1400 container steps** (`knowledgeLadder.test.ts`).
- **Ingest:**
  - "Paste text" moves inside the dropzone as a ghost `sm` button.
  - The Digest helper line shows only for states the person must act on. The resting and checking
    reasons move into the button's Tooltip plus `aria-describedby` (K-04 intent: full opacity).
  - `IngestModelPicker`: delete the inert "Set as default" chip. Add the line "Used for digests,
    not chat" with an InfoTip.
  - Dropzone: the hand-rolled ⓘ moves onto `InfoTip`. The chooser subtitle becomes "Add files, or a
    folder or archive." and its cards show the title only.
  - `IngestWarnings`: `Note`s, "{n} warnings" and 16px chevrons.
  - `StagedList`: the count joins the caps run ("STAGED 3").
  - The paste placeholder becomes "Paste text or links".
  - Fix the stale 48px comment in `Dropzone.tsx:139-145`.
- **Base management:**
  - `KBManagerDialog`:
    - The subtitle is one sentence, "Choose the bases this chat uses.", because it remains the
      dialog's `aria-describedby`. The rest goes in an InfoTip by the title.
    - Delete the footer Tip.
    - Drop the mono id from rows; Copy ID goes in `⋯`. Fix the stale comment.
    - Badges are sentence case, and "Primary" is neutral.
  - `KbFormatChooser`:
    - Each option shows one short line, with its facts in an InfoTip.
    - The folder line reads "Folder: knowledge/{id}/" with an InfoTip and no em dash.
    - Use one warning `Note`: "You can't change the format later."
  - `KBSelectorMenu`:
    - The status row reads "Not following your default" with a ghost `sm` "Use {name}" and a
      Tooltip.
    - Drop the format badges from picker rows.
  - The "+n" count moves into the trigger's accessible name plus a Tooltip.
- **Graph and drawers:**
  - `NodePreview`, `EdgePreview`, `ChangeLogDrawer` and `LintDrawer` headers use Crew's pane
    header: 44px, `bg-sidebar`, a `text-label` title, a ghost round close with a Tooltip and a 16px
    inset.
  - ChangeLog chips are sentence case.
  - `GraphFacetStrip.tsx:241` becomes a Tooltip "Clear filters (showing {p} of {t})" with no em
    dash.
  - The edge note reads "Derived from the cited source" with an InfoTip.
  - CI ranges read "{a} to {b}".
  - The graph canvas font fallback uses the body stack (`graph/graphStyle.ts:47`).
- **Native `title=`** on help becomes a Tooltip at every site listed in [views-b §9.3].

**Files:**
- `knowledge/**`, except that **the confirmation logic in `KbTierControl.tsx` is frozen**: only
  its trigger and markup may change.
- `ui/desktop/tests/e2e/knowledge-ingest.spec.ts`.
- `main.css` families "knowledge".
- `styles/{knowledgeLadder,knowledgeTokens}.test.ts` and `styles/graphPalette.ts`.

**Tests:**
- `KnowledgeView.test.tsx:187-193` (the tier control moves into the band).
- `KbTierControl.test.tsx`: keep the confirmation tests unchanged; update the trigger name and the
  testid only.
- `KBSelectorTrigger.test.tsx` (+n).
- `KBSelectorMenu.test.tsx` and `KBManagerDialog.test.tsx`.
- `IngestPanel.test.tsx` (the visible error lines stay).
- `GraphFacetStrip.test.tsx`.
- `settingsVocabulary.test.ts` gains roots for `knowledge` and `baam` in wave 2 (WS-SETTINGS-A, on
  request). Fix `IngestWarnings.tsx:86` and `StagedList.tsx:89` (`size="xs"`) first.

**Acceptance:**
- Knowledge at 1440, 1100 and a pane under 860px, light and dark, shows one band with the tier
  badge visible at every width.
- Two flush columns, with no box inside a box.
- One visible line under the Digest button only when action is needed.
- The flows (stage, digest, preview, change log, lint, manage bases, make public) work exactly as
  before.

---

### 3.13 WS-SETTINGS-A: Settings shell, App, Chat (approvals/display), Models, Providers

**Goal:** message 5. Settings becomes minimal: one row anatomy, no paragraphs and one control per
job.

**The shell** (`settings/SettingsView.tsx`):
- `PageHeader` band holds the h1 "Settings" and the **three tabs inline in the band** (`tabs`
  slot). The tabs are text-only, with the icons removed and `flex gap-2` dropped.
- **Keep the names Models, Chat and App.** Crates and tests name these paths [settings-a §6].
  **Keep these section names and their tabs too**, because daemon and renderer copy point people
  at them: App > **Privacy** (`master_switch.rs`, `config_management.rs:755`, with a Rust test at
  `:3813-3814`), Chat > **Memory** (`global_memory.rs`), Chat > **Contexts** (`skills_extension.rs`),
  Chat > **Capabilities** (`ExtensionsSection.tsx:213`), and **Providers** under Models. Renaming
  "Mode" to "Approvals" is safe (nothing outside the renderer names it).
- **Keep every deep-link key** in `SettingsView.tsx:43-55` `sectionToTab` (`update`, `models`,
  `modes`, `styles`, `tools`, `app`, `chat`, `privacy`): callers pass them in route state
  (`PrivacyTiersOffNote.tsx:62`, `ConfigureProvidersRoute.tsx:26`, `IngestModelPicker.tsx:85`, and
  Crew's `AgentTaskPane.tsx:592`). `modes` lands on Approvals and `styles` on Display.
- Keep the testids `settings-*-tab`. There is no description.
- **Reset `scrollTop` on a tab change.**
- Deep links (`modes`, `styles`, `tools`, `privacy`, `update`) scroll to the `SettingSection` `id`.
  Smooth scrolling happens only when reduced motion is off, and the target row gets `.br-highlight`.
- Content stays in the 760 column, and all rows go through `SettingRow` and `SettingSection`.

**App tab.** Keep Configuration and then Privacy first, the order an operator recorded
(`SettingsView.tsx:175-184`; owner decision 6.5 may reorder them). Then:
- **General:**
  - Notifications, with an "Open System Settings" `secondary sm` button and the InfoTip "Managed by
    your operating system." Delete the guide dialog, whose copy ("System Preferences") is stale.
  - "Show in menu bar"; "Show in Dock" (macOS).
  - "Prevent sleep while running", with the InfoTip "The screen can still lock."
  - "Never open tabs automatically", with an InfoTip. It is folded in from the one-row Workspace
    section; keep its aria-label.
  - "Show costs".
  - Each new label changes the `aria-label` with it, because the test rule is that names equal
    labels.
- **Appearance:** Theme as a `SegmentedControl` (Light/Dark/System, with the test ids kept), Color
  palette as a `SegmentedControl` with swatches, and Text size as a `SegmentedControl`
  (Standard/Large/Larger, replacing native radios).
- **Privacy:**
  - **The locked content keeps its words and its placement.** That covers the DR-17 disclosure
    above the switch, the inline disable confirmation, the off strip and `data-privacy-panel`.
  - Only the switch row changes: it becomes a `SettingRow`, and its description moves into the
    InfoTip with the wording kept.
  - The confirm box uses `py-2.5` and lists its consequences as a `<ul>` with the same words.
- **Usage:** the WS-USAGE component.
- **About:**
  - **Version** is a row with a mono value and the action "Check for updates" (no `ExternalLink`
    icon). Status ("Up to date", progress) shows as the row's status line, and "Restart to update"
    replaces the button when an update is ready. Keep the UpdateSection test strings.
  - Delete the Block logo PNGs (a Goose leftover) after the owner confirms.
  - Feedback is a row with two ghost `sm` buttons.
- **Configuration:** one row, "Configuration", with the provider as its value, an "Edit…" button
  and an InfoTip. The editor moves to `ModalShell` (WS-SETTINGS-B owns `ConfigSettings.tsx`).
- **Danger zone:** the row "Reset data" with a `Reset…` button (`destructive sm`). It opens a
  `ModalShell size="md"` that holds the 7 category checkboxes, each with an InfoTip, plus Select
  all, the permanence line (essential and visible) and Cancel/Reset. The button label becomes
  "Reset everything" when all are checked.
- **ExternalBackendSection** is dead. Leave it, and fix the stale string at `biorouterd.ts:881`
  only if the owner asks.

**Chat tab** (this workstream's sections; WS-SETTINGS-B owns Capabilities, Memory, Contexts and App
SDK):
- **Approvals** (renamed from "Mode", which collides with the theme):
  - "Approval mode" is a select menu (`DropdownMenuRadioGroup`) with the four items, each with its
    one-line description as a muted second line. Delete the inlined radio rows.
  - "Tool permissions" has an `Edit…` button, enabled in Manual and Smart.
  - "Max turns" is a number input with no "Chat limits" disclosure.
- **Display:** "Tool call details" is a `SegmentedControl` Expanded/Collapsed (keep the keys
  `detailed` and `concise`). Spellcheck is a switch whose status line "Restart to apply" appears
  only after a change.
- **Project:** "Project hints" with `Edit…` `sm` and no icon; the InfoTip holds the filename.
- The section headers of Capabilities, Contexts and App SDK get InfoTips. **Keep the substrings
  "new chats start with" and "Existing chats keep their current"** for
  `ChatSettingsSection.copy.test.ts:9-10`.

**Models tab:**
- **Default model:** a "Model" row with the value `gpt-5.6-sol · Versa Azure` and the action
  "Change…" (**keep the accessible name "Switch models"**), plus a "Providers Manage…" row.
- **Local models:** the header carries the count "2 of 7 installed" and a refresh glyph.
  - Each row shows the name plus one facts line (`4.2 GB · 128k context`), and a warning word
    ("Large for this Mac") only when it applies.
  - The trailing action is Install (`secondary sm`) or a `⋯` menu (View info, Warm up, Delete).
- **Danger zone:** "Reset provider and model" with `Reset…` (`destructive sm`) and an InfoTip. Keep
  the accessible name and the `ConfirmationModal`.
- **Dialogs** use `ModalShell` sizes (LeadWorker `md`, SwitchModel `md`, LocalModel info `lg`),
  `Note` replaces the hand-rolled notes, type roles throughout, and "Model" replaces "Choose a
  model:".

**Providers page:**
- Delete the panel h2 that repeats the tab name. The tier note becomes one muted line (privacy
  information, visible).
- The section notes become header InfoTips.
- "Other institutions" becomes "Don't see your institution?" plus an InfoTip (with the "→" replaced
  by words).
- Rows use the shared row geometry with no initial tiles. "Configured" becomes a success dot plus a
  word.
- `ModalShell` sizes; the `Field` recipe in forms; a `Note` for warnings.
- Delete the dead `subcomponents/{CardHeader,CardContainer,CardBody}.tsx` and
  `utils/StringUtils.tsx`.
- Fix `CustomProviderForm`'s `@radix-ui/themes` Checkbox with `ui/Checkbox`.

**Permission dialogs:** `ModalShell size="lg"`, `SettingRow` rows and roles. Leave the
`/permission` route alone. It is unreachable, and only its wrong copy (#38) is fixed.

**Unnamed switches:** fix any found in owned files.

**Docs and guards:**
- Rewrite `docs/desktop-ui/settings-visual-vocabulary.md` rules 2, 3 and 7 to the
  label-plus-InfoTip-plus-control anatomy.
- `settingsVocabulary.test.ts`:
  - add **V9**: no `max-w-md text-supporting` description `<p>` in a settings row, allow-listing
    only the locked blocks in `PrivacyPanel.tsx`;
  - add **V10**: no native radio;
  - in wave 2, remove the `providers/`, `permission/` and `extensions/` exclusions and add
    `knowledge` and `baam`.

**P2:** `onboarding/**` and `ProviderGuard.tsx`, with type-role swaps and `SegmentedControl` in
`InstitutionalSetupCard.tsx` only.

**Files:**
- `settings/SettingsView.tsx`.
- `settings/{app,chat,models,providers,mode,permission,response_styles,reset_provider}/**`,
  except `models/bottom_bar/ModelsBottomBar.tsx`.
- `settings/privacy/**` (row only, under the frozen locked blocks).
- `settings/{hostManagedSettings.browserSurface.test.tsx,settingsVocabulary.test.ts,useReturnFocusToOpener*,destinationConfigKeys*}`.
- `BioRouterSidebar/{ThemeSelector,ThemeFamilySelector}.tsx`; replace or delete them.
- `contexts/ThemeContext.*.test.tsx` (the `aria-pressed` assertions only).
- `onboarding/**` and `ProviderGuard.tsx` (P2).
- `docs/desktop-ui/settings-visual-vocabulary.md`.

**Tests:** [settings-a §8]:
- `SettingsView.test.tsx`: the order, the three readable columns and the testids.
- `AppSettingsSection.test.tsx:44-58`: names equal labels.
- `WorkspaceSettingsSection.test.tsx:48` (its heading goes).
- `ResetPanel.test.tsx`: open the dialog first, and keep the ids.
- `UpdateSection.test.tsx`.
- `ConversationLimitsDropdown.test.tsx:22` (drop the "chat limits" click).
- `PrivacyPanel*.test.tsx`: the order of the statement and the switch is unchanged.
- `LocalModelInventory.test.tsx`.
- `ProviderCatalog*.test.tsx`.
- `hostManagedSettings.browserSurface.test.tsx`.
- e2e `app.spec.ts:84-100`.

**Acceptance:**
- Every tab at 1440 and 1100, light and dark:
  - the band holds the title and the tabs (about 44px, not 212px);
  - every row is 40px or one line plus an optional status line;
  - no paragraph appears except the locked privacy disclosure;
  - one switch style on the right, one segmented style and one select style;
  - one danger zone with a `sm` destructive button;
  - InfoTip glyphs are subtle and open on hover and Tab.
- The App tab shows no native radios and no white discs in dark mode.

---

### 3.14 WS-SETTINGS-B: Capabilities, Contexts, App SDK, Memory, Configuration

**Behaviour:** [settings-b §3]:
- **Capabilities:**
  - 13 `SettingRow`s with `help={meta.description}`, keeping the descriptions in `capabilities.ts`.
  - **Drop** the `aria-label` "Toggle {Label} capability" (`CapabilitiesSection.tsx:50`): the
    switch is named by its row label through `aria-labelledby` (2.6), and this workstream updates
    `CapabilitiesSection.test.tsx:34-54`, the only queries of that name (no e2e or crate reads it).
    The same goes for `ContextsSection.tsx:57` and `BrsdkSection.tsx:80`. Add `aria-describedby`.
  - Optional: two caps subgroups, "Tools" and "Agent".
  - **Copilot:** "Check setup" becomes a row-trailing `secondary sm` button, and the result panel
    becomes a sibling `Note` after the row. Split `copilot/CopilotSetup.tsx` into a trigger and a
    panel.
- **Contexts:** 9 `SettingRow`s with help. Optionally say "Loaded only for … tasks" once in the
  header InfoTip.
- **App SDK:** 4 `SettingRow`s. Keep `px-3 py-2.5` on the row, because `BrsdkSection.test.tsx:37`
  asserts it. "Agent Drafter" is spelled without a hyphen, and "stop hook" is lower case in prose.
- **Memory:**
  - The header uses `SettingSection title="Memory" help=…`, and Refresh is a ghost round
    `RefreshCw` with a Tooltip.
  - Each store heading reads "Global" or "This project", with a right-aligned count and an InfoTip
    carrying the audience sentence and the path (sr-only text, so `MemorySection.test.tsx:85,97`
    still finds the path).
  - The empty states are one muted row each: "No project open" and "Nothing has been remembered
    yet." The rest goes in an InfoTip.
  - Expanded entries render as **sibling** rows.
  - Delete glyphs are hidden at rest (`data-row-action`) and muted, turning to danger ink only on
    hover or focus. Keep their aria-labels.
  - Entry text is `text-secondary`, and the meta shows the count only. Fix the stale comment.
- **Configuration** (`config/ConfigSettings.tsx`):
  - One row, "Configuration", with the provider as its value, an "Edit…" `secondary sm` button and
    an InfoTip. Keep the accessible name `/Edit configuration/`.
  - The editor is `ModalShell size="lg" scrollBody` with the title "Configuration", no icon, and no
    lowercase "biorouter".
  - **Keep the `div.grid` rows with exactly one button each** (two browser-surface tests rely on
    them). Do **not** put an InfoTip in these rows.
  - Use `text-supporting font-mono` and a Tooltip on the label in place of `title={key}`.
  - `PrivacyConfigSummary` is type only, and its copy is kept.

**Files:**
- `settings/{capabilities,config,contexts,brsdk,memory}/**` and their tests.
- `copilot/CopilotSetup.tsx` and its test, and `copilot/PermissionCheckButton.tsx` (rendered by the
  setup panel; it was in no ownership row, which made it frozen).

**Tests:**
- `CapabilitiesSection.test.tsx`, `capabilities.test.ts`, `contexts.test.ts`,
  `BrsdkSection.test.tsx`, `MemorySection.test.tsx`.
- `ConfigSettings*.test.tsx`: the "(current settings for X)" assertions read the row value.

**Acceptance:**
- The Chat tab is no longer 3,087px tall. It is about 40% shorter, with one line per row.
- Copilot's check panel no longer swells the row.
- Memory has no red icon column and no repeated paragraph.

---

### 3.15 WS-LANDING: consolidate the site without losing content

**Goal:** match the app's scale and restraint, and keep every word of content [landing §4]. **The
landing site deploys on merge to `main`. Never merge without the owner's approval.**

**Behaviour:**
- **`site.css` tokens** (after `:52`), aligned with the app:
  - `--fs-xs` 12/16, `--fs-sm` 13/20 (all code), `--fs-md` 14/22, `--fs-body` 16/26 (web
    reading), `--fs-h3` 17/24, `--fs-h2` 24/32 (replacing 20 and 23, so the steps are shared with
    the app), `--fs-title` `clamp(28px, 3.2vw, 40px)`/1.15 at −0.025em, and `--fs-display`
    `clamp(40px, 5.2vw, 64px)`/1.08 at −0.035em (it was 84px at −0.06em).
  - `--h-control: 32px` and `--h-cta: 40px`.
  - Tracking takes four values. Headings stay at **400** and 700 is used only for docs `strong`.
    Never write 500 or 600 (Arial cannot draw them).
- **Shared fixes:**
  - One inline-code rule in `site.css`: `:not(pre) > code { font-size: var(--fs-sm); … }` and
    `td code { var(--fs-xs) }`. Delete the copies in `download.css:4-7`, `home.css:47`,
    `docs.css:80,91` and `baam.css:200,289-292`. This fixes the 10.75px code.
  - Small text moves from `--faint` (3.84:1) to `--subtle`.
  - Reveal animations take 525ms, travel 8px and use `--ease`. Every hover uses `--dur-fast
    var(--ease)`. Align `shared.css`'s curve, although no page loads it.
  - Delete the dead CSS (`.small`, `.tiny`, `.copy-row`, `.art.on-ground`, `.brand .word b/i`,
    `.soon` and `.arrow.both`).
- **Trims** (every fact stays):
  - Delete the eyebrows that repeat the nav (download, baam, about), the rule-line notes (the list
    in [landing §3.3]) and the two hero icon tiles.
  - Show Slack once per page: in the Community band, and in the header only if owner decision 6.11
    keeps it there. The default puts "Download" in the header.
  - About's Community band moves to the end of the page.
  - About's Updates show the newest 5 in `.news-list`, followed by `<details class="news-more">`
    "Earlier updates". **The first `releases/tag/v` link after `class="news-list">` must be the
    newest release** (`release.sh:1132`).
  - BAAM's hero becomes one `--fs-title` h1 with a `.soft` second line, and the search sits in the
    first fold.
  - Docs:
    - one header width for every page;
    - delete the sidebar Slack block;
    - h2 at `--fs-h2` with 48/20 margins, keeping the coral lead segment;
    - "Drag the wall to compare." becomes a handle affordance with `aria-label`;
    - pick one two-tone headline per page (the hero).
- **Buttons:** 32px for small actions and 40px for primary CTAs. Remove the 34, 36, 42, 44 and 48px
  heights except the 44px mobile-menu touch target.
- **Restyle, never rename.** Keep the BAAM classes and ids, `.ext-tags` at 20px, the badge fill
  deltas, light mode only, and the docs table markup quoted by checks.
  - `docs.html` is generated: edit `docs/website/` and reassemble.
  - Do not touch `icon*.svg` or `icon.png`.
- **The coupling.** If the app sidebar ever adds or renames a `label: '…'`, update
  `app-mockups.js:90-102` in the same change. WS-SIDEBAR must not rename any.
- **Follow-up in the same release, after the app lands:**
  - Refresh `art/*` (composer, per-tool glyphs, private and public shapes, no caps header in
    `art/schedule.css:90-92`). Snap the animation sizes to 12, 13 and 14, with weights 400 and
    700.
  - Rewrite `docs/website/pages/app.html:10-73` to describe the new sidebar, composer, Settings and
    summary rail (`:26` and `:65` still say "Recents … grouped by day" and "See all"), and fix
    `docs/website/pages/privacy.html:89` ("Choose See all under Recents"), then reassemble
    `docs.html` and run `check-crew-manual.mjs`, `check-docs-privacy.mjs` and
    `assemble-docs.mjs --check`.
  - Fix the stale facts in `landing/assets/landing-site-content.md` (the month, and the old design
    prose), keeping its version and News blocks.

**Files:** `landing/**` (`site.css`, `home.css`, `download.css`, `docs.css`, `baam.css`,
`about.css`, `art/*`, the HTML pages, `assets/landing-site-content.md`) and `docs/website/**`. The
checker scripts and `shared.css`/`app-mockups.*` are left as they are, except for the label
coupling.

**Tests and checks** (all must pass):
- `node landing/scripts/assemble-docs.mjs --check`
- `node landing/scripts/check-consistency.mjs`, both the full mode and `--check`
- `node scripts/check-crew-manual.mjs`
- `node landing/scripts/check-docs-privacy.mjs`
- `node landing/scripts/build-registry.mjs --check`
- `node --test landing/scripts/baam-privacy-facet.test.mjs landing/scripts/baam-search.test.mjs`,
  which needs Playwright from `ui/desktop/node_modules`.

**Acceptance:**
- Every page at 1440 and 390 shows at most 8 rendered sizes plus 2 mono sizes, measured with the
  probe in [landing §2.3].
- The index product animation starts inside a 900px fold at 1440.
- The brand never jumps between pages.
- No small text uses `--faint`.
- There is no horizontal overflow at 390.
- Every content fact is still present (diff the text).

---

### 3.16 WS-MOTION: tabs, find in chat, toasts, routes, windows

**Goal:** owner message 1's motion list, for the surfaces no other workstream owns [motion §3.3].

**Behaviour:**
- **Opening tabs** (`chatGroups/ChatTabStrip.tsx`, `chat-tabs.css`, the `br-tab` family):
  - **The entrance:** a new tab goes from opacity 0 and individual `scale` 0.96 to 1, with
    `transform-origin: left center`, over 175ms with `--ease-out`. Set `data-tab-entering` for one
    frame on tabs that have no previous offset (`:260-262`). Add a `br-tab-enter` keyframe beside
    `br-tab-select`. Enter wins over select.
  - Neighbours keep the existing FLIP (`translate`, 175ms, `--ease-spring`).
  - **Not on:** a reload restore, a window opening with tabs, or a tear-off. Gate it on a ref that
    is set once the strip has painted.
  - **Closing:** instant removal, with the neighbours sliding (as today). Reordering is unchanged.
  - Skip the FLIP pass while `isWindowResizing()`.
  - **Strip details:**
    - one active weight (600 renders bold, which is fine for the active tab; drop the 500/600 split
      in `chat-tabs.css:17`);
    - the gap goes from 7px to 8px;
    - the active tab is flat (no `--shadow-raised`), shared with the preview strip;
    - `.br-tabstrip__scroll` gets `scroll-padding-inline: 8px` (requested by WS-PREVIEW);
    - the chat title gets more room: the strip is truncating it at about 110px in a 1150px band
      [baseline, Chat].
  - P3: add Rename on a tab, reusing `ChatRowRenameInput`.
- **Tab content:** after a tab opens or switches, the transcript container fades in once over
  125ms. WS-TRANSCRIPT stops the per-message replay.
- **Searching** ("searching tabs" is the first item in message 1's motion list). It maps to the
  surfaces that exist; each is listed so none is forgotten:

  | Search | Behaviour |
  |---|---|
  | Find in chat (`SearchBar`/`SearchView` in `BaseChat` and `SessionHistoryView`) | The overlay below |
  | The chat tab strip's overflow ▾ menu | Dropdown spec; **gains a filter field at the top when it lists 8 or more tabs** (type to filter, results replace instantly, the highlighted row moves instantly, the empty state "No tabs match" crossfades in over 175ms) [motion A2] |
  | The band `FilterInput` on Workflows, Built apps, Extensions, Skills and History (2.6) | Instant results, empty state crossfades in over 175ms, no row FLIP [motion A4] |
  | History's search | Same as the band filter |
  | Composer pickers (Tools popover search, mention popover) | Instant results; the menu does not resize while typing |
  | Knowledge's ⌘K base picker | Unchanged behaviour; menu motion from 2.6 |

  A new ⌘K "Search chats" palette is **not** built in this round (decision 6.13).
  - **Find in chat** (`conversation/SearchBar.tsx`, `styles/search.css`) becomes an
    **overlay** pinned to the transcript's top right (`position:absolute`), so there is no layout
    push. **It stops being a list filter**: Workflows, Built apps, Extensions, Skills and History
    move to the band `FilterInput` in the same wave (their owners), so after this change
    `SearchView` is imported only by `BaseChat.tsx` and `sessions/SessionHistoryView.tsx`. Add a
    source assertion that says so. The positioned transcript cell is a BaseChat edit that
    WS-SUMMARY applies (3.5).
    - The overlay sits below the 44px header at the transcript's right edge, where a toast also
      lands (`--toast-inset-top`). The vision pass checks that a toast never covers the find
      input; if it does, inset the overlay 16px from the right and let the toast win (it is
      transient).
    - Open: opacity plus `translateY(-8px)` to 0, over 175ms. Close: opacity plus `-4px`, over
      125ms, on `animationend`. Delete the 150ms `setTimeout`.
    - The current match scrolls into view smoothly, but not under reduced motion.
    - Restyle the bar: `rounded-container`, `max-w-measure-chat`, a 32px `text-secondary` input,
      and `Button ghost sm round` with 16px icons and tooltips carrying the shortcuts.
    - Add the ⌘F load-all tip as its InfoTip.
  - **The tab overflow ▾ menu** uses the dropdown spec, plus the filter field above.
- **Routes** (`App.tsx` route outlet): the main column fades in once over 125ms, keyed on the
  route. There is no exit, no slide and no View Transitions. Rename
  `utils/navigationUtils.ts:57-64` `navigateWithViewTransition` to say what it does, and delete
  the dead `page-transition` hooks it supports.
- **Toasts** (`App.tsx:652-682`, `toasts.tsx`, `alerts/**`, the `.Toastify*` family):
  - Use `cssTransition({enter:'br-toast-in', exit:'br-toast-out', collapseDuration:125})`. In:
    opacity plus `translateY(-8px)` to 0 over 175ms. Out: opacity plus `translateX(8px)` over
    125ms. The reduced-motion rest is opacity only.
  - **Kill the Bounce.**
  - `NotificationSurface` titles use `text-secondary` and replace the `text-[13px]` sizes.
  - The position is unchanged (`--toast-inset-top`, `styles/toastLayer.test.ts`). The entrance
    starts 8px above it (y = 44 − 8 = 36), still below `--titlebar-drag-height` (32): **never
    raise the travel or the inset so a toast enters the drag rect**, or its buttons go dead while
    it animates (issue #74, CLAUDE.md "Toasts sit in the top-right corner").
- **Windows** (`main.ts`, only the window-open and grow regions, around `:1600-1680` and
  `:2803-2845`):
  - Pass `animate = !systemPreferences.getAnimationSettings().prefersReducedMotion` to
    `setContentSize` and `setBounds` (`:2823`, `:2838`).
  - Open and close animations are left to the OS. Consider `show:false` plus
    `once('ready-to-show')` for new main windows, **only after measuring** against
    `docs/desktop-ui/startup-freeze-and-main-thread-blocking.md`.
  - Never shrink the window.
- **The other items:**
  - `dragGhostWindow.ts:133` uses the body stack.
  - `hooks/useStopAcknowledgement.ts:6-7` gets a corrected comment.
  - `InAppTerminalDock.tsx` drops its literal durations; the terminal font stays.

**Files:**
- `chatGroups/**`; `Layout/useTabStripOverflow.ts`.
- `conversation/{SearchBar,SearchView}.tsx`; `styles/search.css`.
- `App.tsx`; `toasts.tsx`; `alerts/**`; `GroupedExtensionLoadingToast.tsx`.
- `main.ts` (the regions above only); `dragGhostWindow.ts`.
- `utils/navigationUtils.ts`; `hooks/useStopAcknowledgement.ts`; `InAppTerminalDock.tsx`.
- `main.css` families "tabs, toasts".
- `styles/{searchBarNote,toastLayer,tabStripFloor}.test.ts`.

**Tests:**
- `ChatTabStrip.flip.test.tsx:109-133`: the string `--motion-base` becomes `--dur-fast-max`, and a
  new-tab entrance case is added.
- `ChatTabStrip.contextMenu.test.tsx`: the strings come from WS-SIDEBAR.
- `searchBarNote.test.ts` (the max-height ceiling becomes an overlay).
- `conversation/SearchBar.test.tsx:19-22`.
- `toasts*.test.tsx`.
- `boot-splash.test.ts` (unchanged).

**Acceptance** (60fps recordings, reviewed frame by frame):
- A new tab grows in from its left edge, and a restored tab does not.
- The find bar opens over the transcript without pushing it.
- With 8 or more tabs, typing in the overflow menu filters the list with no lag and no jump.
- A toast slides 8px with no bounce and collapses over 125ms.
- A route switch is a single short fade.
- With macOS Reduce Motion on, opening a preview in a narrow window grows it instantly.
- During a window drag, nothing slides or fades.

---

### 3.17 File ownership (authoritative; paths under `ui/desktop/src/` unless noted)

| Path | Owner |
|---|---|
| `styles/main.css` | By selector family, section 3.18 |
| `styles/{motion.ts,motion.test.ts,typeScale.test.ts,themes.generated.ts,codeTheme.ts}`, `styles/{fontStacks,fontScale,noteClamp,themeNeutrals,focusFallback,focusSurface,codeTheme}.test.ts`, `utils.ts`, `ui/desktop/themes/**`, `ui/desktop/scripts/**` (theme/contrast/tokens), `design.md` | WS-TOKENS |
| `components/ui/**` (except `sidebar.tsx`, `sidebarWidth.ts`), `components/ModalShell.tsx`, `components/crew/**` (fix list only), `styles/{settingsRowHover,settingsScrollFade,tabFocus,crewParity}.test.ts`; class-only: `{DependencySetupModal,DependencyErrorBanner,UpdateAvailableModal,AnnouncementModal,ErrorBoundary}.tsx` | WS-PRIMITIVES |
| `components/icons/**`, `components/chats/{chatKind.ts,ChatKindIcon.tsx}`, `utils/{toolIconMapping.tsx,toolGlyph.ts}`, `components/ItemIcon.tsx`, `components/AnimatedIcons.tsx` (delete) | WS-ICONS |
| `components/BioRouterSidebar/**` (except `ThemeSelector`, `ThemeFamilySelector`), `components/ui/{sidebar.tsx,sidebarWidth.ts}`, `Layout/TitlebarControls.tsx`, `chats/{ChatRowContextMenu.tsx,chatRowActions.ts,ChatRowRenameInput.tsx}`, `utils/{sessionNameSync,deleteConversation,exportConversation,dateUtils,chatDateBuckets}.ts`, `sessions/**` (except `SessionsInsights`, `UsageHeatmap`, `HomeUsageCard`, `DeclassifySessionDialog`, `declassifyOnBrowser`), `ui/desktop/tests/e2e/helpers/sidebar.ts` | WS-SIDEBAR |
| `components/BaseChat.tsx`, `ChatSummary.tsx`, `components/summary/**`, `hooks/{useSummaryRail,useColumnGlide,useSessionTodos}.ts`, `Layout/{yieldLadder,summaryRailPreference}.ts`, `styles/summaryRail.test.ts`, docs listed in section 3.5 | WS-SUMMARY |
| Message, markdown, tool-call, card, activity and error files (section 3.6), `subagent/**`, `workflows/WorkflowActivities.tsx`, `utils/trailingActivity.ts`, `styles/chatSurfaceType.test.ts`; class-only: `context_management/SystemNotificationInline.tsx`; `ui/desktop/tests/e2e/{user-message-layout.spec.ts,user-message-layout.fixture.*,bioroffice-install.spec.ts,bioroffice-verify.spec.ts}` | WS-TRANSCRIPT |
| `ChatInput.tsx`, `MessageQueue.tsx`, `MentionPopover.tsx`, `Hub.tsx`, `ContextWindowIndicator.tsx`, `bottom_menu/**`, `settings/models/bottom_bar/ModelsBottomBar.tsx`, `store/reasoningEffort.ts`, `utils/keyboardShortcuts.ts`, `composerNoProvider.ts`, `copilot/CopilotControl.tsx`, `Quoted*`, `ResourceRefChip`, `WaveformVisualizer`, `extensionReferenceItems.ts`, `styles/{composerFocus,composerWorkingEdge,dirChipPath,popupType}.test.ts` | WS-COMPOSER |
| `artifacts/**`, `styles/prismGrammars.ts`, `styles/artifactPaper.test.ts`, `ui/desktop/.artifact-harness/**`, `ui/desktop/scripts/preview-panel-e2e.mjs` | WS-PREVIEW |
| `sessions/{SessionsInsights,UsageHeatmap,HomeUsageCard}.tsx`, `components/usage/**`, `utils/{usageStats,homeInsightsCache}.ts`, `common/Greeting.tsx`, `hooks/use-text-animator.tsx`, `settings/usage/**` | WS-USAGE |
| `Layout/{PageHeader,MainPanelLayout,ReadableContent,AppLayout}.tsx`, `workflows/**` (except `WorkflowActivities`), `schedule/**`, `applications/**`, `parameter/**`, `ParameterInputModal.tsx`, `NotFoundView.tsx`, `styles/measures.test.ts`, `docs/design/astryx-adoption/**`, `ui/desktop/tests/e2e/schedule-artifact.spec.ts` | WS-VIEWS-A |
| `extensions/**`, `settings/extensions/**`, `baam/**`, `skills/**`, `BrxtInstallModal.tsx`, `ExtensionInstallModal.tsx`, `ExtensionUpdateReporter.tsx`, `brxtInstallFlow.ts`, `ui/desktop/tests/e2e/brxt.spec.ts` | WS-VIEWS-B |
| `knowledge/**` (the confirmation logic in `KbTierControl` is frozen), `styles/{knowledgeLadder,knowledgeTokens}.test.ts`, `styles/graphPalette.ts`, `ui/desktop/tests/e2e/knowledge-ingest.spec.ts` | WS-KNOWLEDGE |
| `settings/{SettingsView.tsx,app,chat,models (except bottom_bar),providers,mode,permission,privacy (row only),response_styles,reset_provider}/**`, `settings/*.test.tsx` and `settingsVocabulary.test.ts` at the settings root, `BioRouterSidebar/{ThemeSelector,ThemeFamilySelector}.tsx`, `contexts/ThemeContext.*.test.tsx` (aria assertions), `onboarding/**` and `ProviderGuard.tsx` (P2), `docs/desktop-ui/settings-visual-vocabulary.md`, `ui/desktop/tests/e2e/app.spec.ts` (theme testids; WS-VIEWS-B requests its `:182-184` change) | WS-SETTINGS-A |
| `settings/{capabilities,config,contexts,brsdk,memory}/**`, `copilot/{CopilotSetup,PermissionCheckButton}.tsx` | WS-SETTINGS-B |
| `landing/**`, `docs/website/**` | WS-LANDING |
| `chatGroups/**`, `Layout/useTabStripOverflow.ts`, `conversation/{SearchBar,SearchView}.tsx`, `styles/search.css`, `App.tsx`, `toasts.tsx`, `alerts/**`, `GroupedExtensionLoadingToast.tsx`, `main.ts` (two regions), `dragGhostWindow.ts`, `utils/navigationUtils.ts`, `hooks/useStopAcknowledgement.ts`, `InAppTerminalDock.tsx`, `styles/{searchBarNote,toastLayer,tabStripFloor}.test.ts` | WS-MOTION |
| `docs/design/codex-simplicity-redesign/**` (R-24), `CLAUDE.md`, `scripts/check-crew-manual.mjs`, `docs/crew/**`, the shared dev GUI instance | Coordinator |
| Frozen (section 3, global rules) | nobody without the coordinator |

Every test file belongs to the owner of the code it tests unless this table says otherwise. A file
that appears in no row is frozen. **Read section 3.19 before dispatching agents**: it splits the
largest rows into parallel sub-owners, and its partition is the one copied into `progress.md`.

### 3.18 `styles/main.css` ownership by selector family

| Family (and banner, with lines at `f2ff06132`) | Owner |
|---|---|
| Head imports, `@font-face`, `@theme`, the `:root` tokens (1 to about 1000), `THEMES:GENERATED` (1014-1394, generator only), `@theme inline`, base and focus rules (to about 1999), the global reduced-motion reset (2000), shared keyframes `appear` and `fade-slide-up`, `@property --tint-ink` and the tint utilities, `@layer utilities` (4819-4897, including `.text-caps`), `code [class~='token']`, selection (4898-4937), `.biorouter-focus-surface` and `.biorouter-focus-region`, scrollbars (3794-3833), `body.biorouter-headless-browser *`, forced colours (6257 to the end), the new `[data-motion-layout]` rule, `.br-enter`, `.br-crossfade`, `.br-highlight` | WS-TOKENS |
| `.Toastify*` (2080-2175), DOCUMENT TABS `.br-tabstrip*` and `.br-tab*` with their keyframes and pulse (2321-2456, 2668-3041), CHAT GROUPS `.br-group-splitter*` (3042-3116) | WS-MOTION |
| `.biorouter-modal-*` (2224-2249), PROGRESS `.br-progress*` (2250-2320), `.biorouter-popover-surface` (3117-3143), `.biorouter-list-shell` and `-row` (3144-3185), `.br-swatch-ring`, `.biorouter-note-clamp`, `.biorouter-scroll-fade-top` (3605-3623), `.biorouter-settings-*` (3624-3695), shared primitives `.biorouter-copy-field*`, `.biorouter-status-dot*`, `.biorouter-disclosure*`, `.biorouter-avatar*` (5617-6256), the new `.br-info-tip` and `.br-segmented` | WS-PRIMITIVES |
| `.biorouter-composer-card*`, the composer focus edge (2457-2484), `.biorouter-dir-chip-*` (2485-2533), the working edge (2534-2667) | WS-COMPOSER |
| `.biorouter-composer-motion` (4635) | WS-SUMMARY |
| `.biorouter-message-tool`, `biorouter-tool-enter` (2034, 2184), `.biorouter-markdown*` (4287-4507), `.user-message`, `.assistant…` (4508-4520), `.virtualized-list*`, `biorouter-icon-entrance`, `.prose .katex*` | WS-TRANSCRIPT |
| `.br-knowledge-*`, `.br-graph-*`, `.br-facet*`, `@container br-knowledge-pane` (about 3186-3524, excluding `.br-swatch-ring` and `.biorouter-note-clamp`) | WS-KNOWLEDGE |
| `.biorouter-home-*` and `@container biorouter-home-content` (3525-3604), `.biorouter-heatmap-*` (5541-5612) | WS-USAGE |
| `.biorouter-sidebar-*`, `.biorouter-sidebar-resize-handle`, `.titlebar-drag-region`, `body.biorouter-sidebar-compact*` (3696-3936), `.session-item`, `.session-skeleton`, `.biorouter-history-loading-cell` | WS-SIDEBAR |
| RUNG 2 `[data-preview-split]*`, `.br-preview-*` and the resizing block (3937-4210), `.br-paper-*`, the delimited table, code and notebook views, `.artifact-document-scroll`, `.br-preview-measure*` (4938-5540, 5613) | WS-PREVIEW |
| `.biorouter-page-header` (3131) | WS-VIEWS-A |
| `.br-chat-kind-icon*` (about 3006-3012, carved out of the tab family) and the new `.br-icon-lock-badge` | WS-ICONS |
| The new `.br-md-code` and `.br-md-table` (Crew's code-block and table recipes; WS-PREVIEW uses the classes and does not edit them) | WS-TRANSCRIPT |

The summary rail's geometry lives in `components/summary/summaryRail.css`, not in `main.css`. The
rail rules carry higher specificity (`[data-summary-rail]` plus `:not([data-preview-layout])`), so
load order does not matter.

### 3.19 Agent capacity: split the largest workstreams (critic)

Five rows above are more than one agent can carry in a round, measured in non-test source lines
at `f2ff06132`: Settings A covers `settings/models` (4,418), `providers` (3,438), `onboarding`
(2,234), `app` (1,419) and five smaller folders, about 13,500 lines; the composer row holds
`ChatInput.tsx` (3,764), `bottom_menu/**` (2,102), `MentionPopover.tsx` (924) and
`MessageQueue.tsx` (502); the transcript row holds `ToolCallWithResponse.tsx` (1,855),
`MarkdownContent.tsx` (1,000) and the cards; Views B holds about 7,500 lines across four areas;
the sidebar row carries the new view store, paging, rename and folder groups **and** all of
History (`SessionListView.tsx` alone is 1,393). Split each along file lines that are already
disjoint. Each sub-owner inherits its parent's behaviour section, tests and acceptance for its
files; nothing else changes.

| Parent | Sub-owner | Files | Contract it publishes in wave 0 |
|---|---|---|---|
| WS-SIDEBAR | **WS-SIDEBAR** | `BioRouterSidebar/**` (except the theme selectors), `ui/{sidebar.tsx,sidebarWidth.ts}`, `Layout/TitlebarControls.tsx`, `chats/**` except the two WS-ICONS files, `utils/{sessionNameSync,deleteConversation,exportConversation,chatDateBuckets}.ts`, `tests/e2e/helpers/sidebar.ts` | `renameSessionOptimistically`, `chatDateBuckets`, `exportConversation`, and `ChatRowContextMenuContent`'s `onRename`/`onDiverge`/`onExport` props |
| | **WS-HISTORY** | `sessions/**` except the WS-USAGE files and the frozen declassify files; `utils/dateUtils.ts` | none; it consumes the above |
| WS-TRANSCRIPT | **WS-TRANSCRIPT** | messages (`BioRouterMessage`, `UserMessage`, `MessageMeta`, `MessageCopyLink`, `MessageDivergeLink`, `ProgressiveMessageList`, `ProvenanceChip`, `SessionNamePill`, `InlineImage`, `ImagePreview`, `WorkflowHeader`), `MarkdownContent.tsx` and the `.biorouter-markdown*` and `.br-md-*` families, activity (`TurnActivityIndicator`, `LoadingBioRouter`, `utils/trailingActivity.ts`), `conversation/{ChatTurnError,ChatTurnStopped,turnStoppedNotice}`, `subagent/**`, `workflows/WorkflowActivities.tsx`, `SystemNotificationInline.tsx`, the user-message e2e, `message-meta.css`, `styles/chatSurfaceType.test.ts` | none |
| | **WS-TOOLS** | `ToolCallWithResponse`, `PendingToolCallCard`, `ToolCallStatusIndicator`, `ToolContentPreview`, `ToolCallArguments`, `ToolCallPreview`, `ToolCallConfirmation`, `ElicitationRequest`, `SecretRequestCard`, `MCPUIResourceRenderer`, `tool-call.css`, the `.biorouter-message-tool` family, the bioroffice e2e, and the approval strings in `bridgeApprovalPrompt.test.ts` and `utils/startChatFailure.test.ts` | `TranscriptRow` (the 28px glyph, verb, chevron and well), which the thinking row in `BioRouterMessage` reuses |
| WS-COMPOSER | **WS-COMPOSER** | `ChatInput.tsx`, `MessageQueue.tsx`, `MentionPopover.tsx`, `Hub.tsx`, `composerNoProvider.ts`, `utils/keyboardShortcuts.ts`, `Quoted*`, `ResourceRefChip`, `WaveformVisualizer`, `extensionReferenceItems.ts`, the composer `main.css` families, `styles/{composerFocus,composerWorkingEdge}.test.ts` | none |
| | **WS-PICKERS** | `bottom_menu/**`, `settings/models/bottom_bar/ModelsBottomBar.tsx`, `store/reasoningEffort.ts`, `ContextWindowIndicator.tsx`, `copilot/CopilotControl.tsx`, `styles/{dirChipPath,popupType}.test.ts` | `ToolsChip`, `ModelEffortChip` and `ComposerFooter` (folder, ring, cost) with final props, so WS-COMPOSER places them inside the card and under it without touching their insides |
| WS-VIEWS-B | **WS-EXTENSIONS** | `extensions/**`, `settings/extensions/**`, `BrxtInstallModal.tsx`, `ExtensionInstallModal.tsx`, `ExtensionUpdateReporter.tsx`, `brxtInstallFlow.ts`, `tests/e2e/brxt.spec.ts` | none |
| | **WS-SKILLS** | `skills/**`, `baam/**` | `baam/MarketplaceDialog.tsx`, which both browse dialogs use |
| WS-SETTINGS-A | **WS-SETTINGS-A** | `settings/SettingsView.tsx`, `settings/{app,chat,mode,permission,response_styles}/**`, the privacy row, the settings-root tests and `settingsVocabulary.test.ts`, the theme selectors and `ThemeContext` aria tests, `docs/desktop-ui/settings-visual-vocabulary.md`, `tests/e2e/app.spec.ts` | `SettingSection` ids used by the deep links |
| | **WS-SETTINGS-M** | `settings/{models (except bottom_bar),providers,reset_provider}/**`; `onboarding/**` and `ProviderGuard.tsx` (P2) | none |
| WS-KNOWLEDGE (optional) | **WS-KNOWLEDGE** / **WS-KNOWLEDGE-INGEST** | shell, band, `SourcesRail`, the `KbTierControl` trigger, `graph/**`, `changelog/**`, `lint/**` / `IngestPanel/**`, `KBSelector/**`, `DispatchProgress.tsx` | the band's selector slot |

Wave 1 adds two dependencies: WS-HISTORY starts after WS-SIDEBAR's contract commit, and
WS-COMPOSER places the WS-PICKERS components as soon as their props land (the fallback trigger in
3.7 keeps working until then). The targeted test commands in 5.1 split along the same lines.

---

## 4. Sequencing and conflict rules

**Wave 0: contracts.** Land these first, ideally within the first working block.
- **WS-TOKENS:** the type roles, the new tokens, `motion.ts` and the shared motion classes.
- **WS-PRIMITIVES:** the exported APIs for `InfoTip`, `SegmentedControl`,
  `SettingRow`/`SettingSection`, `RowActions`, `Field`, `Textarea`, `FilterInput`, `Spinner` and
  `PopoverAnchor`, and the Tooltip provider fallback (landing together with WS-SIDEBAR's removal of
  the `delayDuration={0}` provider in `ui/sidebar.tsx:428`). They may land unstyled, but the props
  are final.
- **WS-VIEWS-A:** the `PageHeader` band API (`info`, `actions`, `tabs`, `onBack`) and the
  `routeOwnsTopBand` predicate.
- **WS-ICONS:** `toolGlyph.ts`, `chatKind.ts` with the crew kind, and the shim.
- **WS-PREVIEW:** `sideWidth` on `ArtifactPanelController`.
- **WS-SIDEBAR:** `renameSessionOptimistically` in `utils/sessionNameSync.ts`, plus
  `chatDateBuckets`, `exportConversation` and the `ChatRowContextMenuContent` props (3.19), and the
  removal of `ui/sidebar.tsx:428`'s `delayDuration={0}` provider in step with WS-PRIMITIVES.
- **WS-TOOLS:** `TranscriptRow`. **WS-PICKERS:** `ToolsChip`, `ModelEffortChip`,
  `ComposerFooter`. **WS-SKILLS:** `MarketplaceDialog` (3.19).
- **Coordinator:** copy the ownership (3.17 with 3.19) into
  `docs/design/codex-simplicity-redesign/progress.md` and set phase 2 to Done.

**While wave 0 lands**, the other workstreams start on everything that does not need those APIs:
copy and `copy.ts` extraction, the type-role sweeps of their own files, test rewrites, pure modules
(`sidebarChatView.ts`, `usageStats.ts`, `summaryRailFit`) and markup restructuring. They
import a primitive as soon as its API commit is in.

**Wave 1: the surfaces, in parallel.** WS-SIDEBAR, WS-HISTORY, WS-SUMMARY, WS-TRANSCRIPT,
WS-TOOLS, WS-COMPOSER, WS-PICKERS, WS-PREVIEW, WS-USAGE, WS-VIEWS-A, WS-EXTENSIONS, WS-SKILLS,
WS-KNOWLEDGE, WS-SETTINGS-A, WS-SETTINGS-M, WS-SETTINGS-B, WS-MOTION and WS-LANDING (its site.css
and trims only).
- The list views' move from `SearchView` to `FilterInput` lands in the same checkpoint as
  WS-MOTION's overlay, or the views are left with a filter that floats over nothing.
- WS-SUMMARY's glide waits for WS-PREVIEW's `onPresentedChange`. Until then it uses the rail-only
  path.
- WS-COMPOSER's Tools popover waits for `SegmentedControl`. The fallback trigger can ship first.

**Wave 2: sweeps and gates**, after every wave-1 owner reports done:
- WS-TOKENS deletes the aliases (`--motion-*`, `--ease-in`, `--dur-med-max`, `text-heading`,
  `text-display`), adds `typeScale.test.ts`, moves `body` to 14px and writes design.md Part 9.
- WS-PRIMITIVES removes the Switch `variant` prop and adds the controls source guard.
- WS-SETTINGS-A removes the vocabulary exclusions and adds the knowledge and baam roots.
- WS-LANDING does the art and docs follow-up.

**Wave 3: the vision loop** (section 5.4) repeats until every surface reports zero violations.

**Conflict rules:**
1. **Edit only files you own** (section 3.17) and only your `main.css` families (section 3.18),
   inside `@ws` markers. For anything else, send the owner the exact diff through the coordinator.
2. **Commit only your own paths:** `git add <path>…`, never `git add -A` or `git add .`.
   - Make small commits.
   - **Never stash.** The stash is shared with other worktrees and sessions.
   - Never `reset`, `rebase` or `checkout` other people's files, and never push without the
     coordinator.
   - Commit messages omit the `Co-Authored-By` trailer, because the `no-ai-coauthor` check rejects
     it (owner memory).
3. **Mid-flight checks:**
   - The shared tree type-checks as a whole, so during wave 1 judge `npx tsc --noEmit` by **your own
     files' errors only**.
   - The full `npm run lint:check` must be green at each checkpoint, which the coordinator calls.
   - Run Prettier only on your own files: `npx prettier --write <your files>`.
4. **Do not run `npm ci` or `npm install`.** `ui/desktop/node_modules` is a symlink to the main
   checkout's install (owner memory).
5. **Shared runtime resources** belong to the coordinator: the dev GUI instance, CDP 9471 and vite
   on 5173. Do not restart them, and never `close all` an agent-browser session. Use your own
   uniquely named session.
6. **Tests pinned by another owner.** If your change would break a test you do not own, stop and
   send the owner the change request. Do not loosen the assertion yourself.
7. **No privacy or security logic changes.** Changes to how privacy UI is presented, such as the
   Knowledge tier trigger, the extension provenance and the composer padlock, are listed in the PR
   for human review (CLAUDE.md, "Security-sensitive code requires human review").
8. **The landing site and `main`.** Nothing is merged to `main` without the owner, and the landing
   site deploys on merge.

---

## 5. Verification

### 5.1 Commands (from `ui/desktop/` unless noted)

```bash
npm run lint:check                 # typecheck + eslint + themes + contrast + tokens (NOT Prettier)
npm run format:check               # Prettier; nothing else runs it
npx vitest run src/styles          # source guards for CSS
npx vitest run src/components/<area> src/styles/<your tests>   # targeted, per workstream
npx vitest run src/components/crew # after ANY token, primitive or Crew change
npm run test:run -- --exclude '**/artifactCdnAssets.browser*'  # full run; that browser teardown hangs >30s on main too
npm run test:preview-panel         # Electron preview e2e (macOS CI job); WS-PREVIEW
node ../../landing/scripts/check-consistency.mjs   # after any AppSidebar.tsx change (label coupling)
node ../../scripts/check-crew-manual.mjs           # after any Crew string or AppSidebar label change
```

Known flakes:
- `vitest` can exit 1 with 0 failures because of the Ollama teardown or the `artifactCdnAssets`
  browser `afterAll`. Read the summary line rather than trusting the exit code.
- A background exit code can lie when it is piped through `tail`.

Targeted suites:

| Workstream | Command |
|---|---|
| WS-TOKENS | `npx vitest run src/styles src/components/crew` |
| WS-PRIMITIVES | `npx vitest run src/components/ui src/components/crew src/styles` |
| WS-ICONS | `npx vitest run src/components/icons src/components/chats src/utils/toolGlyph` |
| WS-SIDEBAR | `npx vitest run src/components/BioRouterSidebar src/components/chats src/components/sessions src/utils/sessionNameSync src/components/ui/sidebar` |
| WS-SUMMARY | `npx vitest run src/components/BaseChat src/components/ChatSummary src/hooks/useSummaryRail src/hooks/useSessionTodos src/components/Layout/yieldLadder src/styles/summaryRail src/styles/measures` |
| WS-TRANSCRIPT | `npx vitest run src/components/ToolCall src/components/BioRouterMessage src/components/UserMessage src/components/Markdown src/components/conversation src/components/subagent src/styles/chatSurfaceType src/test/uiCopySpelling` |
| WS-COMPOSER | `npx vitest run src/components/ChatInput src/components/bottom_menu src/components/Hub src/components/ContextWindowIndicator src/components/settings/models/bottom_bar src/styles/composer src/styles/popupType src/styles/dirChipPath` |
| WS-PREVIEW | `npx vitest run src/components/artifacts src/styles/artifactPaper src/styles/measures` |
| WS-USAGE | `npx vitest run src/components/sessions/UsageHeatmap src/components/sessions/SessionsInsights src/components/sessions/HomeUsageCard src/components/usage src/utils/usageStats src/utils/homeInsightsCache src/components/settings/usage` |
| WS-VIEWS-A | `npx vitest run src/components/Layout src/components/workflows src/components/schedule src/components/applications src/styles/measures` |
| WS-VIEWS-B | `npx vitest run src/components/extensions src/components/settings/extensions src/components/baam src/components/skills` |
| WS-KNOWLEDGE | `npx vitest run src/components/knowledge src/styles/knowledge` |
| WS-SETTINGS-A and B | `npx vitest run src/components/settings src/contexts/ThemeContext` |
| WS-MOTION | `npx vitest run src/components/chatGroups src/components/conversation src/styles/searchBarNote src/styles/toastLayer src/toasts` |

### 5.2 The dev GUI (from the baseline report)

- **The instance:**
  - The run is `redesign-baseline`, sandboxed in `~/biorouter-runs/redesign-baseline/` (`config`,
    `data`, `state`, `electron`).
  - CDP is on **9471** and vite on **5173**. The PIDs are in `~/biorouter-runs/redesign-baseline/state.env`
    (at capture time Electron was 49080, the daemon 49099 and vite 49039).
  - The daemon binary is the worktree's `target/debug/biorouterd`. The window was left at
    1440×900, light, on `#/`.
- **It runs with `BIOROUTER_NO_HMR=1`**, so edits do not show until a restart (and the Tailwind
  scanner misses new class strings until then). Restart with:
  ```bash
  ~/biorouter-runs/launch-dev-gui.sh stop redesign-baseline
  chmod 700 ~/biorouter-runs/redesign-baseline/electron     # the app refuses a non-0700 --user-data-dir
  ~/biorouter-runs/launch-dev-gui.sh start /Users/wgu/Desktop/BioRouter/.claude/worktrees/crew-landing-redesign-91b46e redesign-baseline 9471
  ```
  **Only the coordinator or the vision reviewer restarts it**, at checkpoints. Running a second
  instance at the same time has not been verified, and the vite port may collide.
- **Connect:** agent-browser `connect {target:"9471", session:"<unique name>"}` or
  `node ~/biorouter-runs/cdp-eval.mjs 9471 '<js>'`. Close only your own session, by name.
- **Resize the real window.** Do not use viewport emulation. Take the new PID from `state.env`
  after a restart:
  ```bash
  osascript -e 'tell application "System Events" to tell (first process whose unix id is <PID>) to set size of window 1 to {1100, 760}'
  ```
- **Theme:** `localStorage.setItem('use_system_theme','false'); localStorage.setItem('theme','dark'); location.reload()`.
  The family is set through `theme_family` (`alma-mater` or `roche-limit`).
- **Clean up.** Stop every instance you start. An orphaned Electron or vite process burns CPU.

### 5.3 Screenshots and probes per workstream

Store them in `~/biorouter-runs/redesign-2026-10-09/after/<ws>/<screen>-<theme>-<width>.png`.
Use the same names as `baseline/` so before and after can be compared. The default set is 1440×900
and 1100×760 in light and dark, plus one Alma Mater dark spot check.

| Workstream | Screens |
|---|---|
| WS-TOKENS | Home and Settings > Chat (type probe); Crew empty state (caps 12px) |
| WS-PRIMITIVES | Settings > App (switches, segmented, InfoTip open by hover and by Tab); a menu open; a dialog; Crew pages that use switches and menus |
| WS-ICONS | Sidebar with all chat kinds, private and public; a transcript with 10 tool families |
| WS-SIDEBAR | Sidebar Date, Folder and None groupings; the view menu open; the context menu; rename in progress; collapsed at 1100 (overlay); History |
| WS-SUMMARY | A chat at 1100, 1344, 1440 and 1920 with the rail; the rail with a preview at 1440 (hidden) and 2560 (shown); the popover fallback; a recording of the toggle |
| WS-TRANSCRIPT | `chat-toolcalls-*` equivalents; an expanded tool; an approval card; a code block; tables; an error row |
| WS-COMPOSER | Home composer; chat composer; Tools popover; model menu; queue with 2 items; attachments |
| WS-PREVIEW | Harness `img/preview/01-15` at 600 and 380px, light and dark, plus the GUI artifact screens; a recording of open, switch and close |
| WS-USAGE | Home both tabs, all three ranges; Settings usage dialog |
| WS-VIEWS-A | Workflows, Scheduler (list and detail), Built apps, and the workflow editor |
| WS-VIEWS-B | Extensions (with a failure notice), Skills, both BAAM dialogs, the extension modal |
| WS-KNOWLEDGE | Knowledge at 1440, 1100 and a narrow pane; KB manager; format chooser; node preview |
| WS-SETTINGS-A and B | Every tab, scrolled through (`settings-*-p1..pN`); the Reset dialog; the Permission dialog |
| WS-MOTION | Recordings: new tab, find bar, toast, route switch, window drag across 1120 and 800 |
| WS-LANDING | Every page at 1440 and 390 (names from [landing], header) |

**Type probe.** Run it in the renderer through CDP, with Settings > App > Text size on
**Standard** (Large and Larger scale every px by design, `vite-plugins/fontScale.mjs`; at Large,
check instead that nothing stayed unscaled, since inline `style={{fontSize}}` px is not scaled).
Each workstream reports the size set for its screens. The set must be a subset of sans
{12, 13, 14, 17, 24} plus mono {12, 13} (12 is the mono size for paths in tooltips, IDs and
Crew's code-block language). Ignore `.katex` (math sizes are relative by design), text inside
iframes (artifacts, the spreadsheet preview), the xterm canvas, and the Inter wordmark:

```js
[...new Set([...document.querySelectorAll('body *')].filter(e=>e.childNodes.length&&[...e.childNodes].some(n=>n.nodeType===3&&n.textContent.trim())&&e.offsetParent).map(e=>getComputedStyle(e).fontSize+' '+getComputedStyle(e).fontFamily.split(',')[0]))].sort()
```

### 5.4 The vision loop (owner message 5: "loop agents over every surface")

A reviewer agent with vision runs each surface group after every wave-1 checkpoint:

1. Take the screenshot set.
2. Check it against the list below.
3. File each violation as `{ws, file, screenshot, rule}` to the owning workstream.
4. Re-run after the fixes.

A surface is done when two consecutive passes find nothing.

The checklist:
1. A visible explanatory paragraph that is not an error, refusal, privacy disclosure, destructive
   consequence or empty-state line.
2. A text size outside the scale in section 2.1, or two families in one sentence.
3. Mono on text that is not a machine string.
4. Caps outside page section labels.
5. More than one coral-filled button in a view, or coral outside the zest list (principle 7).
6. A switch that is not 32×20 on the right, an off state that cannot be seen, a native radio or
   checkbox, or more than one segmented style.
7. Curled dividers, a box inside a box, or a shadow on a static surface.
8. Row actions permanently visible on a hover-capable display, or more than one icon plus `⋯` per
   row.
9. Title Case, an em or en dash, "...", "conversation", "BioRouter" in copy, or "Toggle …" names
   on switches. Exempt: product names (Agent Drafter, Auto Visualiser, Biorouter Copilot, Chat
   Recall), text the daemon serves, the locked privacy copy, the wordmark lockup (decision 6.12),
   and Crew (judged only on its fix list, D1 to D12).
10. A focus ring after a mouse click, or an invisible keyboard focus.
11. Left edges misaligned within a surface (measure x).
12. Motion: a jump, distorted text, an animation during a resize, a flash in dark mode, or a replay
    on a tab switch.
13. Dark mode: elevation that cannot be seen (popover or menu against the ground), an invisible off
    state or zero cell, or more than one accent hue in the sidebar (the coral rail, the lock badges
    and the Update glyph are one hue; the navy wordmark and the dev-build `EnvironmentBadge` dot
    are not counted).
14. A row, tab or menu item that moved under the pointer between hover and click.
15. A toast covering the find-in-chat input or any band control.
16. On `/crew` with the app sidebar open: the two rails side by side (report, do not fix; 6.1).

### 5.5 The artifact harness

```bash
python3 -I ~/biorouter-runs/redesign-2026-10-09/audit/preview-harness/make_fixtures.py <fx-dir>
node ~/biorouter-runs/redesign-2026-10-09/audit/preview-harness/serve-harness.mjs <fx-dir> 5317 /Users/wgu/Desktop/BioRouter/.claude/worktrees/crew-landing-redesign-91b46e/ui/desktop
```

- `serve-harness.mjs` keeps vite's cache in the run directory and does not write into the checkout.
- Pin the panel width with
  `document.querySelector('[data-testid=panel-host]').style.flex='0 0 600px'`, then 380px.
- Real fixtures can come from
  `PREVIEW_FIXTURE_DIR=<dir> cargo test -p biorouter-mcp --test preview_fixture_dump -- --ignored`.
  Run that only when no other agent holds the cargo lock.

### 5.6 The landing site

```bash
cd /Users/wgu/Desktop/BioRouter/.claude/worktrees/crew-landing-redesign-91b46e/landing && python3 -m http.server 8731
```

Drive it with a named agent-browser session at 1440 and 390, and stop the server afterwards. Run
the checks in section 3.15.

### 5.7 Geometry in a real layout engine (critic)

jsdom computes no layout, runs no Tailwind and evaluates no `:has()`, so a component test that
reads a size passes whether the rule exists or not. The repo already has the fix: vitest files
named `*.browser.test.ts(x)` launch Playwright's chromium from inside `npm run test:run`
(`crew/sidebar/crewSidebarGeometry.browser.test.ts`, `utils/previewCentring.browser.test.ts`).
They load the authored CSS with the real token values and measure boxes. Use them for every
acceptance number that is a measurement:

| Owner | New browser test | Asserts |
|---|---|---|
| WS-SIDEBAR | `BioRouterSidebar/sidebarGeometry.browser.test.ts` | 28px rows, 30px pitch, icon x=16, label x=40 at 216/288/360; band hairline at y=44 |
| WS-PRIMITIVES | `ui/controlsGeometry.browser.test.tsx` | Switch 32×20 with a 16px knob inset 2px in both states; menu rows 32px; segmented thumb under the selected segment with no transition on first placement; InfoTip adds 14px to the line |
| WS-SUMMARY | `summary/summaryRailGeometry.browser.test.ts` | The grid templates at 1055/1056 and 1067/1068 of available width; composer left edge equals the column's |
| WS-VIEWS-A | `Layout/pageBandGeometry.browser.test.tsx` | Band 44px, hairline at y=44, title x with and without the titlebar reserve |

They are slower than jsdom tests, so keep each to one file per surface. They do not replace the
vision loop, which still measures the running app.

---

## 6. Open owner decisions (each has a default so work can proceed)

| # | Decision | Default (used unless the owner says otherwise) | Alternative |
|---|---|---|---|
| 6.1 | App sidebar density | **28px rows at 13px** (Codex-compact, message 12), with Crew's recipe in every other respect. Known cost: on `/crew` at wide windows the app sidebar sits beside Crew's 32px/14px rail, so two pitches show side by side (the vision loop reports it) | 32px rows at 14px, the same as Crew's rail (no mismatch on `/crew`, but less compact than message 12 asks) |
| 6.2 | Weight of large titles (24px) | **400** in the app and on the site (Codex hero, editorial site, Arial has no 500). 17px headings stay at 600 (bold). | Keep 600 (bold) for 24px |
| 6.3 | Summary rail | **Variant A:** the column stays at 760, and the rail shows from 1344px windows with the sidebar open. The visual is a **Codex card** in the docked column. Show the owner two consequences: with a side preview open the rail is hidden at every window narrower than about 2560px (it returns through the header button's popover), and narrowing a 1300px window to 1100px makes it appear (the sidebar turns into an overlay). Message 1's "make it a little narrower" reads either as the box (A) or as the conversation (B). | Variant B: the column shrinks to 680 so the rail shows from about 1264px. Or a full-height Crew pane with its own 44px band |
| 6.4 | Usage range | **Frontend only:** a 371-day window labelled `1y · 30d · 7d`. No Peak hour tile. Favorite model comes from `/usage/report`. | A backend change for a true "All" (the clamp in `session_manager.rs:8257`) and per-day model and hour data, which needs a privacy review of the ungated `/sessions/activity` |
| 6.5 | Settings App section order | **Keep Configuration then Privacy first** (the recorded operator decision); everything else is reorganized | Lead with General and move Configuration to an "Advanced" section |
| 6.6 | Page header reversal | **The band**, with the 2026-09-07 "actions on their own line" decision recorded as reversed | Keep the old header (contradicts message 5) |
| 6.7 | Extension provenance (§13.5) | **Keep the three mandated strings visible** on one muted line | Move them behind the badge tooltip (a privacy-design change) |
| 6.8 | InfoTip glyph at rest | **Visible**, in subtle ink | Hidden until the row is hovered or focused (Crew's row-action rule) |
| 6.9 | Success toasts for toggles | **Drop them**; keep the error toasts (the switch is the confirmation) | Keep them |
| 6.10 | Detecting crew chats | **No backend change:** `working_dir` under `crew/tasks`, then the title prefix | Add a daemon `origin` field to `SessionSummary` |
| 6.11 | Landing header CTA and About updates | **"Download" in the header**, Slack once per page in the band; About shows the newest 5 updates plus "Earlier updates" | Keep "Join Slack" in the header; keep all 31 updates visible |
| 6.12 | Wordmark spelling ("BioRouter" lockup vs the brand rule "Biorouter") and Option B (the wordmark as the Home link) | **No change in this round** | Change the lockup to "Bio"+"router", or drop the Home row |
| 6.13 | "Searching tabs" (message 1) | **Polish the searches that exist** (3.16 table) and add a filter to the tab overflow menu at 8 or more tabs | Build a ⌘K "Search chats" palette (motion A1). Note that ⌘K already means "switch base" inside Knowledge |
| 6.14 | `+` on a folder group (3.4) | **Opens a new chat window in that folder** (the only existing path) | Drop the `+` |
| 6.15 | Chat markdown tables and code blocks (section 0) | **Crew's recipes, one look app-wide** | The earlier booktabs table and header-less code block (two looks: one in Crew, one elsewhere) |

Out of scope unless the owner asks:
- deleting the unmounted Settings folders (`dictation`, `tunnel`, `sessions`);
- a ⌘K chat palette;
- an app-level "Reduce motion" setting (the OS setting covers it, [motion N]);
- A-03's 36px content rows;
- Auto Visualiser figure chrome, which lives in Rust templates.

## 7. Critic changes (2026-10-09)

Each item was checked against the requirements, the reports or the worktree source; file:line
references are at `f2ff06132`.

**Requirements coverage**
1. Header said messages 1 to 10; there are 12. Added a requirement trace (M1 to M12, R-01 to R-24)
   and tied the spec to the in-repo `docs/design/codex-simplicity-redesign/` record, which R-24
   requires to stay current and which says the final ownership lives in its `progress.md`
   (header; 3.17 coordinator row; 4 wave 0).
2. **Message 11 was violated by the input the spec told WS-ICONS to port**:
   `proposed-toolGlyph.ts:193-201` gives SPOKEAgent, OMOP, CDW, Playwright, CodeGraph and BiorOffice
   their own glyphs, and its test pins `spokeagent__query_graph` → `database`. Rungs 1 to 4 now
   apply only to a `BUILT_IN_EXTENSION_KEYS` set; every other extension draws the Puzzle, cards
   included (3.3).
3. The coordinator's icon decision and message 12 ("keep … the coral lock badges") were missing:
   `withPrivateBadge` draws the badge in `currentColor`, so it would have turned muted. Badge now in
   `--text-accent` via `.br-icon-lock-badge`, added to the zest list and to the contrast checks
   (principle 7, 2.3, 3.3, 3.18).
4. The `workflow` chat kind cannot be detected without the daemon field 6.10 declines; it now ships
   dormant and no acceptance expects it (3.3).
5. Message 7's "used consistently" had no plan outside WS-ICONS' own files: added the icon sweep
   table (icons.md §6 and §8 items 5 to 10) with an owner per call site, and kept Crew out of it
   (3.3).
6. "Searching tabs" (message 1, first motion item) had no surface beyond find in chat: added a
   mapping table and a filter in the tab overflow menu at 8 or more tabs; a ⌘K palette stays out,
   recorded as decision 6.13 (3.16, 6).
7. Message 5 asks for "all kinds of different components", but `DependencySetupModal`,
   `UpdateAvailableModal`, `DependencyErrorBanner`, `AnnouncementModal`, `ErrorBoundary` and the
   transcript's `SystemNotificationInline` were frozen. They now get a class-only pass (Global
   rules, 3.17).
8. Message 5 makes Crew the reference for the chat interface, but the spec gave chat a booktabs
   table and a header-less code block while Crew uses a framed table and a head-row code block
   (crew-reference §4.4). Chat, documents, CSV and notebooks now copy Crew's two recipes, giving
   one look app-wide (section 0, 3.6, 3.8, decision 6.15).
9. Saved and shared transcripts (`SessionHistoryView`, `SharedSessionView`) had no spec and kept a
   24px `<h1>`: they move onto the band, with the privacy marker kept at mount (3.4).
10. History's menu now matches the sidebar's (message 4 "all the other actions"), and the menu's
    refusals show the daemon's sentence (3.4).

**Repo invariants**
11. Drag regions: making the 32px strip stop taking clicks on every band route also removes window
    dragging there. Bands are now drag regions with `no-drag` controls, and `routeOwnsTopBand`
    covers every route inside the shell, including `/sessions`, `/shared-session`, `/permission`
    and the `*` catch-all that renders `PageHeader` (principle 1, 3.10).
12. Toast entrance travel is bounded so a toast never enters the drag rect (3.16).
13. Settings: the section names Privacy, Memory, Contexts, Capabilities and Providers are named by
    daemon copy and a Rust test (`config_management.rs:3813-3814`), and the deep-link keys in
    `sectionToTab` have callers in four files; both are now pinned (3.13).
14. Contrast tooling: `check-contrast.mjs` resolves hex only, so the spec's `color-mix()` tokens
    would have failed as UNRESOLVED; they are now hex literals in the base blocks, and new colour
    utilities need `@theme inline` mirrors (2.3).
15. Privacy: every `loadAll` page of the sidebar carries `userActionHeaders()` (private rows vanish
    silently without it), and a rename failure shows the daemon's text (3.4).
16. The landing coupling also forbids moving the nav labels into `copy.ts`
    (`check-consistency.mjs` greps `label: '…'` in `AppSidebar.tsx` and needs at least 8), and
    `check-crew-manual.mjs` reads Crew strings byte for byte (copy rules, Global rules).

**Tests and tools that would have broken**
17. Removing the per-tooltip provider would throw in every test that renders a tooltip without
    one, and `ui/sidebar.tsx:428` wraps the whole app shell in `delayDuration={0}`, which would make
    every tooltip instant. Fallback provider plus a coordinated removal (2.6, 3.4, 4).
18. `tests/e2e/user-message-layout.spec.ts:164` measures against `[data-message-meta="end"]`, the
    row WS-TRANSCRIPT deletes (3.6).
19. `scripts/preview-panel-e2e.mjs:155` (CI, macOS) clicks "Send a region to the chat", which moves
    into a `⋯` menu; `ArtifactViewer.browserBridge.test.tsx:153,169` pins the same button (3.8).
20. `tests/e2e/app.spec.ts:182-184` queries the extension switch as "Toggle … extension" (2.6).
21. Crew D12's "dialling" fix would fail `check-crew-manual.mjs:1111-1119`; deferred to a
    coordinator commit (3.2).
22. `ScheduleDetailView.test.tsx:245` pins `<MainPanelLayout>` like `measures.test.ts:466`;
    e2e `schedule-artifact.spec.ts:212` and `enhanced-context-management.spec.ts` (`data-phase`)
    pin a heading and a testid; a global "keep testids" rule lists what e2e reads (Global rules,
    3.6, 3.10).
23. The wave-2 type guard would fail on frozen privacy files that carry legacy classes; its
    allow-list now names them (3.1).

**Internal contradictions fixed**
24. Principle 9 banned the motion the spec itself specifies (Disclosure height, Progress width, the
    segmented thumb and tab underline); motion research's named exceptions and its "never move a
    row under the pointer" rule are restored, and the sidebar applies the pointer rule to live
    re-sorts (principle 9, 3.4).
25. Principle 5 said radio rows for explained choices, while Settings made Approval mode a select;
    the rule now matches (select menu on pages, radio rows only in dialogs).
26. 2.6 banned "Toggle …" names while WS-SETTINGS-B kept "Toggle {Label} capability" and
    WS-VIEWS-B used "Enable {title}"; one rule now: the switch is named by its row label.
27. The spacing rule's "no other values" contradicted the spec's own 10px, 7px and 3px; derived
    measures are now exempt (2.2).
28. Composer growth at 220ms was not a token; now `--dur-fast-max` (3.7).
29. The heatmap had 26 fixed columns but "always shows the full window" of 371 days; columns now
    follow the window (53), with the "Active days" denominator, the window-limited longest streak
    and the undercounting Messages tile stated (3.9).
30. The heatmap reveal stagger is declared a named exception to the 5-item stagger cap (3.9).
31. The rail glide and the existing composer FLIP would both animate the composer on the first
    turn; the glide now leaves it alone then (3.5).
32. WorkflowActivities pills used `outline`, which 2.6 retires as a secondary (3.6).
33. Preview header buttons were 28px against 32px on every other band (3.8).

**Feasibility**
34. Unowned files that the work needs were frozen by the "no row means frozen" rule: `ItemIcon.tsx`
    and `AnimatedIcons.tsx` (WS-ICONS), `copilot/PermissionCheckButton.tsx` (WS-SETTINGS-B),
    `hooks/use-text-animator.tsx` (WS-USAGE), the e2e specs and `preview-panel-e2e.mjs`, and the
    `.br-chat-kind-icon` CSS inside the tab family (3.17, 3.18).
35. Five workstreams were too large for one agent (Settings A about 13,500 lines; the composer row
    with a 3,764-line `ChatInput`; the transcript; Views B; the sidebar plus all of History). Added
    3.19 with disjoint splits and their wave-0 contracts (WS-HISTORY, WS-TOOLS, WS-PICKERS,
    WS-EXTENSIONS, WS-SKILLS, WS-SETTINGS-M, optional WS-KNOWLEDGE-INGEST).
36. The list views used `SearchView`, which WS-MOTION turns into the transcript's overlay; every
    list now uses one band `FilterInput` from WS-PRIMITIVES, landing in the same checkpoint (2.6,
    3.10, 3.11, 3.16).
37. jsdom cannot verify any measured acceptance (message 12 asks for measured density): added
    browser tests in the repo's Playwright-in-vitest pattern for the sidebar, the controls, the
    rail and the band (5.7), plus first-placement rules for sliding indicators.
38. The folder group's `+` had no API behind it (a new chat takes the window's folder); it opens a
    chat window in that folder, decision 6.14. The Crew task folder is labelled "Crew" (3.4).
39. Smaller gaps: Knowledge's band goes through `PageHeader`; the find overlay's BaseChat edits and
    the `useSessionTodos` hook order are assigned to WS-SUMMARY; the preview strip fades with the
    body so nothing pops; the binary card and "Update ready" buttons are specified; the Models tab
    hides on `serve`; a parity test keeps the promoted `br-*` recipes equal to Crew's; `CLAUDE.md`
    and the website docs pages that say "Recents" and "See all" are assigned (Global rules, 3.15).
40. Owner decisions gained the visible costs of 6.1 (two rail pitches on `/crew`) and 6.3 (the rail
    hides whenever a preview is open below about 2560px), and three new rows, 6.13 to 6.15.

## Related documentation

- `/Users/wgu/biorouter-runs/redesign-2026-10-09/REQUIREMENTS.md`
- `/Users/wgu/biorouter-runs/redesign-2026-10-09/research/{codex-ui,claude-and-usage,motion}.md`
- `/Users/wgu/biorouter-runs/redesign-2026-10-09/audit/*.md` and `icons/` (the icon proposals)
- `design.md` (it gains Part 9 in wave 2), `CLAUDE.md` ("Desktop shell geometry", "Theme families",
  "Artifact side panel")
- `docs/desktop-ui/{settings-visual-vocabulary,summary-and-figures-qa,artifact-display-surfaces}.md`,
  `docs/desktop-ui/preview-panel/narrow-panes.md`, `docs/security/privacy-tiers.md` §13.5
- `docs/design/codex-simplicity-redesign/{requirements-and-plan,progress}.md` (R-01 to R-24, the
  live tracker)

## 8. Coordinator rulings on section 6 (2026-10-09, binding)

- 6.1 to 6.10 and 6.12 to 6.15: the defaults stand.
- **6.11 is overridden for the header:** the owner's standing September 2026 website decision is
  "Slack is very visible: in the header on every page, plus a band on each page". Keep "Join
  Slack" in the site header on every page AND the Community band. "Download" may sit beside it in
  the header if it fits at 1440 and collapses gracefully at 390. Do NOT delete the docs sidebar
  Slack block unless it duplicates the header on the same screen; prefer keeping it. About shows
  the newest 5 updates plus an "Earlier updates" disclosure that still contains all of them.
- Sidebar active row: keep the 2px coral rail (principle 7) even though Codex has none.
