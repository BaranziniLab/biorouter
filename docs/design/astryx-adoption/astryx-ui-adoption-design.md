# Astryx UI adoption — comprehensive interface revision

> **What this is.** The full design of record for rebuilding Biorouter's interface on the *construction* of Meta's Astryx design system while keeping Biorouter's own palette, theming architecture and calm register. It specifies every foundation, every element, every composition, and the order in which they land.
> **Status:** Proposed — awaiting review. Nothing in this document is implemented.
> **Audience:** maintainers reviewing the direction; developers and agents who will execute it.

Biorouter's interface is coherent in intent and inconsistent in fact. Three parallel audits of the shipped app found five row-title treatments, four error dialects, five spinner constructions, six modal shells, three durations for one hover, 140 hand-written `text-[11px]` classes, and a toast system that carries a 400-pixel scrollable report inside a surface designed for one line. None of this is decay from a bad plan — it is the residue of a design system whose *rules* were written down (`design.md`) but whose *parts* were never built, so every new view re-derived them.

[Astryx](https://astryx.atmeta.com/) is the corrective. It is an open-source React/StyleX design system whose value here is not its colours — its default light body is `#F8F4ED`, a warm parchment cream within a hair of Biorouter's own, which is a pleasant accident and nothing more — but its **discipline**: one control height, one easing curve, one state model, one overlay recipe, one radius ladder, everything expressed as theme tokens so a component's construction never encodes a look. Adopting that discipline is how Biorouter stops re-deriving.

This document proposes what to take, what to keep, and what to refuse. It is organised as: the contract that constrains everything (§1), foundations (§2), elements (§3), compositions — shell, headers, tabs, chat, views, **terminal**, **files and code** (§4), motion (§5), what already agrees (§6), the deletion list (§7), execution (§8), and the decisions you need to settle (§9).

> **Rendered companion:** [`astryx-design-showcase.html`](astryx-design-showcase.html) — open it in a browser. Every control specified below is live there, built from the proposed tokens, with switches for theme family and a Today/Proposed flip that turns the whole page into the diff. It is the faster way to review this proposal; this document is the argument behind it.

---

## 1. The contract

Five things are **not** on the table. Every proposal below is written to hold them.

1. **The palette stays.** Parchment's warm neutrals, the coral accent, Alma Mater's UCSF navy and teal, Roche Limit's orange — unchanged hex for hex. Astryx contributes *roles* and *formulas*, never values.
2. **The theming architecture stays.** One authored `.theme.mjs` per family, `npm run themes` generating the CSS/TS/HTML regions, the light-before-dark ordering invariant, per-family `terminalGround`, the generator that refuses to emit on a contrast failure, and the 252 contrast assertions. The audit called this the strongest part of the system; the redesign treats it as fixed infrastructure and *extends* what a family may declare.
3. **The calm register stays.** No bounce, no glow, no gradient, no lift-on-hover, no colour as decoration. Astryx agrees with this almost everywhere, which is why it is a viable donor.
4. **Corners stay square-ish.** Astryx makes buttons, badges and avatars full pills (`--radius-full`). Biorouter does not. We adopt Astryx's radius *ladder* — a semantic scale with a container-greater-than-element ratio and concentric nesting — and map it onto squarer values. Pills survive only on status dots, the switch knob, and avatars.
5. **The chat surface's information density stays.** The 760px column, the tool-call-as-a-line rule (D-17), rows-not-cards (P2), and mono-for-data (P6) are Biorouter's identity as an instrument. Astryx's roomier marketing rhythm is not imported into the transcript.

One thing **is** explicitly on the table, at your request: the typeface (§2.1), and the sidebar's vertical budget (§4.1).

---

## 2. Foundations

### 2.1 Typeface

**Today.** `--font-sans: ui-sans-serif, -apple-system, …` — the OS default on every platform, chosen by D-06 for zero-download determinism. The one bundled webfont is Inter, embedded as a 64KB data-URI for the wordmark only. The result is that Biorouter has no typographic voice: it renders as San Francisco on macOS, Segoe on Windows, and whatever a lab Linux box resolves.

**Astryx.** One family for everything — `Figtree` (a geometric humanist sans, variable, open-licensed) for both body and headings, `SF Mono`-first for code. Weights are used narrowly: 400 body, 500 for control labels, 600 for headings. Display sizes get *lighter*, not bolder — the marketing H1 is 42px at weight 400.

**Proposal.** Bundle Figtree as a latin-subset variable woff2 and make it `--font-sans`, exactly as Inter is bundled today (same mechanism, same licence class, ~30–40KB subset). Keep the mono stack unchanged — it is already byte-shared between code blocks and the terminal and is one of the few places with zero drift. Adopt the three-role structure as tokens (`--font-body`, `--font-heading` aliasing body by default, `--font-code`) so a theme family can later change its heading face without touching a component.

This is decision **A-01** — the single most visible change in the document, and the one you asked to align with "what is advertised". A native stack remains a legitimate answer if determinism outranks voice.

### 2.2 Type scale — tokenized, geometric, on a 4px grid

**Today.** The canonical scale lives in `design.md` and *has no tokens*. Tailwind's defaults stand in, the two most characteristic steps (13px secondary, 11px caps label) have no utility at all, and the result is 140 × `text-[11px]`, 18 × `text-[13px]`, 16 × `text-[10px]`, plus one-off `text-[12.5px]`, `text-[9px]`, `text-[8px]`, and a `font-size: 13px` literal in the tab strip. Four different tracking values implement one documented `+0.08em`.

**Astryx.** Base 14px, ratio 1.2, every line-height snapped to the 4px grid, and **semantic** styles on top of the raw sizes: `body` 14/20/400, `label` 14/20/**500** (all control text), `supporting` 12/20/400 (timestamps, metadata, captions), `heading-1…6`, `display-1…3`, `code` 14/20 — mono at the *same* size as body, never smaller.

**Proposal.** Tokenize the scale in `@theme` so every step has a utility, and re-express it as Astryx-style semantic roles:

| Role | Size / line-height | Weight | Tracking | Replaces |
|---|---|---|---|---|
| `display` | 30 / 36 | 400 | −0.01em | metric readouts |
| `title` | 24 / 32 | 600 | −0.01em | page titles (`text-2xl`) |
| `heading` | 20 / 28 | 600 | 0 | section headings |
| `subheading` | 17 / 24 | 600 | 0 | card and panel titles (`text-lg`) |
| `body` | 14 / 20 | 400 | 0 | `text-sm` |
| `label` | 14 / 20 | 500 | 0 | every control's text |
| `secondary` | 13 / 18 | 400 | 0 | the 18 × `text-[13px]` |
| `supporting` | 12 / 16 | 400 | 0 | `text-xs` metadata |
| `caps` | 11 / 16 | 500 | +0.08em | the 140 × `text-[11px]` |
| `code` | 13 / 20 | 400, mono | 0 | unchanged (terminal parity) |

Everything below 11px is deleted, not migrated. Body line-height moves from the documented-but-unenforced 21 to 20 so it lands on the grid, which is also what `text-sm` already renders — the doc changes, not the pixels. **A-02.**

### 2.3 Density — one control ladder, one row rhythm

**Astryx.** Three control heights and nothing else: **28 / 32 / 36px**, with 32 as the default for buttons, inputs, selects, icon buttons, nav rows, tabs and menu items alike. Font size does *not* change with height — only the box. Heights are composed (20px line + symmetric padding), not fixed, so a row that gains an icon keeps its rhythm. App header is 48px. Dense data rows 32–40px. Rails 240–300px, secondary rails ~232px, reading columns ~750px.

**Today.** Biorouter runs four bar heights (52 / 52 / 52 / 40), four item heights (34 tabs / 32 sidebar / 40 content rows / 28 dock tabs), three icon-label gaps (8 / 7 / 6), two label sizes (14 / 13), Button sizes at 24/32/36/40 that row actions bypass wholesale for an off-scale 28px, and 14px icons inside 28px buttons in Scheduler where every other view uses 16px.

**Proposal.**

| Slot | Value | Notes |
|---|---|---|
| Control sm / **md** / lg | 28 / **32** / 36px | md is the default everywhere; lg only for a view's single dominant action |
| Icon button | 32×32 with 16px icon; 24×24 compact tier | one size for row actions — the 28px zoo ends |
| Chrome bar (titlebar band, chat header, artifact header) | **44px** | one `--chrome-height` token, three declarations collapse to it |
| Terminal dock strip | 36px | one step down, same tab language |
| Tab (chat, artifact, dock) | 32px | pulled from 34; matches every other control |
| Sidebar / nav / menu / recents row | 32px | already there — now blessed as the rail rhythm |
| Content list row | **36px** | down from 40; Astryx's dense-data band, still comfortable |
| Table row | 36px | one value with the list row |
| Icon-label gap | 8px | everywhere; retires 7px and 6px |
| Sidebar width | 240px | unchanged (Astryx's 260 is a docs rail, not an app rail) |
| Reading column | 760px chat · 1120px pages | unchanged; the 896px replay fork **is now actually deleted** (2026-09-07, §4.4). **Settings and the chat-history surfaces joined the 760px chat column the same day** — each is a stack of labelled rows, not a document, so page width separated a control from its label (Settings) and the per-chat counts from the chat they count (Chat history) rather than showing more. The page measure now governs extensions, skills, schedules, workflows and applications only. |

**A-03** is the content-row change: 40 → 36px contradicts D-12·B, which fixed 40px as "one rhythm, no compact mode". The rhythm survives — it just tightens by one grid step, and the app gains ~10% more rows per screen.

### 2.4 Radius — a semantic ladder, mapped square

**Astryx.** Radius is semantic, not per-size: `inner` (nested) → `element` (controls) → `container` (cards, dialogs, popovers) → `page` → `full`. Containers are always larger than the elements inside them, and nesting follows `inner = outer − padding`, which is what makes nested corners look deliberate rather than accidental. Radius is a *theme token* with a 0–2 multiplier — the same component renders at 10px under one theme and 8px under another with no code change.

**Today.** Five tokens where `--radius-lg` is a silent alias of `--radius-md`, so 234 call sites share 8px under two names and you cannot grep which were meant to be 12px cards. The doc's table (`lg = 12`, `xl = 16`) disagrees with the shipped CSS (`lg = 8`, `xl = 12`, `2xl = 16`). Off-scale stragglers: 3px legend chips, 5px heat cells, a hardcoded `border-radius: 16px` in the modal surface.

**Proposal.** Four steps, semantically named, mapped to Biorouter's squarer taste, plus `full` restricted to three uses:

| Token | Value | Applies to |
|---|---|---|
| `--radius-inner` | 4px | inline code, chips, checkbox, swatches, elements nested inside a control |
| `--radius-element` | 8px | buttons, inputs, selects, tabs, list rows, menu items, icon buttons |
| `--radius-container` | 12px | cards, panels, dialogs, popovers, toasts, code blocks, the composer |
| `--radius-surface` | 16px | reserved: the artifact/preview sheet only |
| `--radius-full` | 9999px | status dots, the switch knob, avatars — **nothing else** |

The composer and modals move 16 → 12px, which is the single change that most makes the app read as one system rather than two. `--radius-md`/`lg`/`xl`/`2xl` become deprecated aliases for one release, then die. Nesting rule becomes normative: *an element inside a container uses the next step down; when a container's padding is `p`, its inner radius is `outer − p`.* **A-04.**

### 2.5 Colour roles and interaction tints

**Astryx.** Semantic surface layering (`body → surface/card → popover → inverted`), text and icon tokens paired 1:1, and — the part worth stealing outright — **interaction expressed as two alpha tints, not colours**: `--overlay-hover` at ~5% ink and `--overlay-pressed` at ~10%, composited over whatever surface the control sits on via a `background-image` gradient. Selection is a third achromatic wash. Status uses translucent fills at ~22% with tinted ink, never solid, with exactly one exception (the persistent error toast). Categorical hues follow one formula — hue at 24% alpha for the fill, saturated hue for the text — instead of hand-picked pairs.

**Today.** Every family hand-authors `--sidebar-hover` and `--sidebar-active` as literal hexes with no documented relationship to the surface they sit on; `.br-tab:hover` uses a bespoke `color-mix(--background-default 62%)`; list rows use 42% and settings rows 38% of the same mix for the identical gesture; and the audit flagged exactly this class of hand-authored derived value as the one that historically drifts.

**Proposal.** Add five interaction tokens per family, *derived* by the generator from the family's ink, and route every hover/press/selection through them:

```
--overlay-hover     ink @  5%     hover on any surface
--overlay-pressed   ink @ 10%     :active, with scale(.98)
--overlay-selected  ink @ 14%     selected rows, active nav, active tab fill
--accent-muted      accent @ 8%   focus rings (inset), selected accents
--border-emphasized derived       inputs and other interactive borders
```

The hand-authored `--sidebar-hover`/`-active` hexes are deleted and become the overlay tints over `--sidebar`. Status washes are regenerated from one formula at 22%. This shrinks what a new theme family must author *and* removes the drift class in one move. **A-05.**

### 2.6 Selection — wash and weight, with Biorouter's rail kept

**Astryx.** Active is `--overlay-selected` behind the whole row plus a 400 → 500 weight bump. No left bar, no accent colour: "selection is achromatic so theme accents stay reserved for CTAs." Its one indicator is a 2px rounded pill that *moves* (the tab underline sized to the label, the TOC thumb sliding on a track).

**Biorouter today** marks active with an accent rail (sidebar), an accent underline (settings tabs), an accent stripe (chat tabs), an accent-tinted glyph (recents), and an accent-filled Button variant used to encode *state* in workflow rows — five accent dialects, one of which (the variant flip) reads as a call to action when it means "scheduled".

**Proposal.** Adopt wash + weight as the base layer of every selected state, and keep exactly one accent mark per surface class — the 2px straight rail (sidebar), the 2px underline (nav tabs), the 2px bottom stripe (document tabs). Delete accent-tinted glyphs, delete state-encoding-by-variant-flip (state becomes a chip). The rails were straightened in this worktree already; this proposal keeps that geometry and adds the wash underneath, so selection reads even where colour is not the signal. **A-06.**

### 2.7 Elevation

**Astryx.** Three shadow levels, each a tight contact shadow plus a soft ambient, both mode-aware; **flat cards** (elevation is reserved for things that genuinely float); a 1px *inset ring* instead of a border on floating surfaces, whose alpha grows with elevation; and 2px inset rings for selection and validation states, which compose better than outlines because they are inside the box.

**Proposal.** Keep Biorouter's four-token elevation policy and its per-family tinting, and change three things: floating surfaces gain the 1px inset ring so their edges stay crisp in dark families; cards lose all shadow and hover by *border brightening* instead; and the two hardcoded tab-pill shadows — which painted Parchment's warm ink under Alma Mater and Roche Limit — become a fifth micro-elevation token. The modal surface's 44%-diluted hairline goes to full strength per D-18. **A-07.**

**Shipped, and the low tier came out empty.** The fifth token landed as `--shadow-raised`; both pill literals are gone and `.br-tab[data-active]` resolves it per mode, so the active document tab is the only live consumer of any elevation below the popover tier. The tier above it has none. Its one intended consumer was the composer, and the composer shipped **flat** — a single 1px border at 60% of `--border-subtle` and no shadow at all (§4.4, and the measurements in `ChatInput.tsx` with it). `--shadow-default` and `--shadow-composer` are declared in all three families and exposed as Tailwind utilities, and **neither has a call site**; `shadow-composer` survives only in a comment recording that it is the one-line change if the shadow is ever wanted back. This is written down because an elevation tier nothing stands on is a real finding, not an omission: it is either a consumer still to be found or two tokens to delete, and the next reader should not have to grep to learn which of those questions is open. The showcase swatch and the specification table both said "the composer" until this was checked.

---

## 3. Elements

Each element below states the target spec. Where Biorouter already agrees, §6 says so instead of repeating it.

### 3.1 Buttons

Four variants — primary (coral fill), secondary (ink wash at 6%), ghost, destructive (**tinted** danger fill with deep danger ink, not a solid red block). Heights 28/32/36 with 32 default; padding `8px 12px`; radius `--radius-element`; text 14/500; no border on filled variants; gap 8px to a 16px icon.

The state stack is variant-agnostic and shared by every control in the system: hover = `--overlay-hover` gradient over the existing fill; press = `--overlay-pressed` + `scale(.98)`; disabled = `opacity .5; cursor: not-allowed` with **no colour change**; focus-visible = instant 2px accent outline at 2px offset. Solid fills brighten by `color-mix(accent, tint 15%)` rather than taking an overlay.

This deletes: the `shape: 'pill'`/`'round'` misnomer (both already render 8px — the prop dies), `destructive`'s unique `hover:opacity-90` (which lets the modal scrim ghost through), and the 103 raw `<button>` elements' excuse for existing.

### 3.2 Inputs, textareas, selects

**Chrome moves to the wrapper.** The container div carries height, padding, border and radius; the native input inside is naked. This is what makes leading icons, trailing buttons, chips-inside-input and input groups all work without per-case CSS.

Container: 32px (md), padding `4px 8px`, radius `--radius-element`, 1px `--border-emphasized`, 8px internal gap, 16px icons. Hover (unfocused): `box-shadow: inset 0 0 0 2px color-mix(--border-emphasized 30%, transparent)` — a whisper, no colour jump. Focus-within: border → accent plus `inset 0 0 0 2px var(--accent-muted)`. An **inset** halo, so nothing shifts layout and D-15's "focus is a surface shift, never a ring" is honoured more literally than today's outline.

Field anatomy: label 14/500 muted → 4px gap → control → 12px status line. A 56px field, adopted as the Settings and forms rhythm.

Select trigger is the input chrome exactly; its popover is `--radius-container` with 4px padding and 32px option rows at `--radius-element` — the concentric rule in miniature.

### 3.3 Checkbox, radio, switch

22×22 visual box (radius `--radius-inner`) with a 24×24 invisible hit target and an 8px label gap; radio identical with a 10px inner dot; switch is a 40×24 track with a thumb that **grows 16 → 20px as it slides** — the one micro-delight Astryx allows itself, cheap and calm, and worth taking.

### 3.4 Chips, badges, status dots

Two tiers, and the distinction is enforced: a **chip** is squared (24px, radius `--radius-inner`, 12/500, optional 16px remove button) and carries a category or a filter; a **badge** is a 20px pill and carries status. Hue chips are generated from one formula — hue at 24% alpha behind saturated hue ink — instead of the four hand-rolled small-label recipes in the app today (`rounded` 4px chips, `rounded-md` pills, `Badge`, `BuiltInBadge`).

### 3.5 Cards

Flat: surface fill, 1px hairline, `--radius-container`, 12px padding, **no shadow**. Title 14/600, 8px gap, body in muted ink. Clickable cards hover by brightening the border only. Selectable cards flip the border to accent and add a 2px inset accent ring — zero layout shift.

Astryx's own guidance is the part to internalise: *"Cards are not the default layout tool — most content groups don't need a container at all."* Biorouter mostly agrees already; ScheduleDetailView does not (§4.5).

### 3.6 Dialogs

One shell. Three widths and one full-screen: **S 400** (confirm) · **M 480** (form) · **L 640** (palette, detail) · **full** (editors, radius 0). This replaces the fifteen distinct `max-w` values in use.

Surface `--radius-container`, no border, `--elev-modal` + inset ring. Sections own their padding (16px): header ~44px with title 20/600 and an optional 12px subtitle, body scrolling internally, footer ~56px right-aligned with 8px gap. Close is a 32px ghost icon button inset 8px. Scrim ~60% ink with a 2px blur. Enter: fade + 16px rise + `scale(.95)` at medium-max; **exit is instant** — an element you are leaving should not delay you.

Astryx's `purpose` axis is worth taking wholesale because Biorouter has the bug it prevents: `info` dismisses on backdrop click, `form` does not (protecting typed input), `required` must be answered.

The six hand-rolled modal shells are deleted into this one. Confirmations get one recipe: title is a verb phrase (never "Yes/No"), destructive confirm is the tinted danger variant, width S.

### 3.7 Toasts — sized to Biorouter's real content

This is the element where copying Astryx directly would break the app, so the census matters. Astryx's toast is one line, 400px, with an optional action and a close — its docs say "keep messages short: only a few words," and anything larger is a banner. Biorouter's toasts today carry:

- ~120 two-line confirmations (title + one sentence), the longest routine one ~95 characters — three lines at 420px;
- titles that are raw `provider/model` identifiers, 40+ characters;
- messages containing unbounded absolute file paths and stringified exceptions;
- sticky errors with a **two-button action row** ("Ask biorouter", "Copy error");
- and one maximal case: the grouped extension-load toast — a collapsible, scrollable, ~400px-tall panel with per-extension rows, each carrying *its own* pair of action buttons.

**Proposal — a three-tier notification model.**

| Tier | Content ceiling | Spec |
|---|---|---|
| **Toast** | title + ≤3 wrapped lines + ≤2 actions | 380–420px, `--radius-container`, 16px padding, status chip 28px, title 13/600, body 13/400 muted, actions in a `label`-size ghost row, close 20px in a reserved gutter. Auto-dismiss 5s; errors persist. |

**The four toast layouts.** The chip column, the close gutter and the 16px padding never move; only the number of blocks in the text column changes, so a one-line confirmation and a sticky error are visibly the same object.

| Layout | Height | When | Rule it carries |
|---|---|---|---|
| **A — title only** | 52px | most success confirmations | The chip is optically centred on the single line, not on the card. |
| **B — title + message** | 2–3 lines | the dominant shape (~120 sites) | The chip anchors to the *first line* (`align-self: flex-start` plus a 5px optical nudge), not to the block centre. |
| **C — message, no title** | ≤3 lines | one-sentence notices carrying a path or an id | With no title the message takes **full-strength ink** instead of muted; otherwise the only text in the card is the quietest thing in it. Long paths break anywhere. |
| **D — title + message + actions** | +36px | sticky errors | Two actions maximum, both quiet — a coral primary inside a toast competes with the page's own call to action. When actions are present the card stops being click-to-dismiss. |

Three lines is the hard ceiling; a fourth is what sends a notice to a banner.

**One optical axis.** The chip, the first line of text and the dismiss control all centre at **30px** from the card's top edge — 16px of padding, plus a 5px nudge on the text column, plus half of an 18px line. Without that nudge a title-only toast rides about 5px high of its own glyph, which is visible the moment you look for it. The axis holds no matter how many lines follow, because everything below the first line grows downward from it.
| **Banner** (in-place) | title + description + one action | full-bleed `section` variant for app-level notices, `card` variant inline; translucent status wash at 22% with tinted ink, 20px icon, 12/16 padding. ⚠ **2026-09-07:** the inline half shipped as `components/ui/note.tsx` with a **16px glyph and 12/10 padding**, not 20px and 12/16 — a note in a settings list is a row-scale object, and §2.4's own "never two icon sizes in one cluster" makes a 20px glyph beside the row's 16px ones the violation rather than the spec. It also has no title slot and carries a `--note-max-height` ceiling. The 20px/12/16 figures still describe the full-bleed app-level variant, which has not shipped. |
| **Notification centre** | anything with a list, a disclosure, or per-item actions | the grouped extension report moves here. A toast may *link* to it. |

Toasts stack top-right (Biorouter's corner — the composer and artifact panel own the bottom-right), 12px gap, **oldest nearest the corner** — a second notification lands directly below the first rather than shoving it down — with **dedup by key as policy** rather than the single exception that exists today. `closeOnClick` is dropped whenever a toast carries actions — today an error toast dies if you click 2px off its button. The layer moves to `--z-toast`, which the vendor default currently overrides at 9999. **A-08** is the notification-centre split.

**The stacking order supersedes this section's original, and the reason matters more than the rule.** That sentence read *"newest nearest the corner"* until the operator asked for the opposite in so many words. A user instruction outranks the document, so the document is what changed. What ships is `newestOnTop={false}` on the `ToastContainer` in `App.tsx` — spelled out rather than left to the library, because react-toastify's default happens to agree today: without the prop the behaviour would be correct *by accident*, and a reader arriving from this section would add `newestOnTop` and silently undo a decision nothing was defending. The code comment and `styles/toastLayer.test.ts` carry the same warning; if you came here to reconcile them, this paragraph is the reconciliation. **Nothing else in this section moved** — the corner and the 12px gap still hold.

The corner is now *derived* rather than measured. `--toast-inset-top` is `calc(var(--titlebar-drag-height) + 12px)` — 44px, the titlebar drag band plus one stack gap, so the first toast sits immediately under the window chrome. It was briefly **144px**, taken from the lowest right-hand ink in any top-level page header so a toast could never cover a page title or Chat history's Import Session button; that works, but it buys the clearance by moving the layer out of the corner and into the middle of the content pane, and the extension-load toast landing halfway down the chat was reported as a bug. A toast overlapping a header for a few seconds is what an overlay is for. The floor is not negotiable in the other direction either: anything above the drag band overlaps a `-webkit-app-region: drag` rect that Electron folds later in DOM order, which eats clicks on a higher-`z` control whatever the z-index says (issue #74), so a toast's close and "View details" would look present and be dead.

### 3.8 Tooltips, popovers, menus

One floating-surface recipe: popover-tier surface (one step off the app ground), `--radius-container`, no border, elevation + inset ring, 4px gap from the trigger, entry = fade + 8px directional slide + `scale(.95)` at fast-max with the origin at the anchor edge.

Menus: 4px container padding, 2px between items, 32px items at `--radius-element` with `6px 8px` padding and an 8px icon gap, hover = `--overlay-hover`, min-width = trigger. Biorouter's `DROPDOWN_ROW_CLASS_NAME` is already almost exactly this and becomes the shared row for Select too, ending the 13px-vs-14px menu-row fork.

Tooltips: inverse surface, 14/20 text, `4px 8px` padding, 4px offset, 165ms rise-and-fade, instant on keyboard focus, ~310ms hover-intent delay. The heatmap's bespoke light card is retired in favour of a *rich popover* (it is a data readout, not a tooltip) and leaves the toast layer it borrowed.

**A floating surface is as wide as its content, and a `min-width` is a floor.**
A menu whose width is pinned above what it holds reads as one that failed to
load the rest of its items. Measured case: the collapsed composer's `+` popover
carried `min-w-[15rem]` (240px) while the three readouts inside it measured
167px, and less again whenever cost reads a short value. Set a floor only to
stop a box collapsing to nothing when a child is hidden, size it against the
row's narrowest real state, and let content decide everything above it.


### 3.8b Iconography — one set, one weight, one box

Every glyph in the app comes from the shipped set: Lucide at `strokeWidth 1.5` through the `light()` wrapper that pins the weight and forces `currentColor`, plus the two hand-authored exceptions (`KnowledgeIcon`, the brand marks). The rules that follow are already written down in §3.9 of the design system; what they need is enforcement:

- **16px in a row, 20px in a banner or empty state, 14px only inside a chip.** Never two sizes in one cluster — Scheduler's 14px icons inside 28px buttons, and the composer toolbar's mix of 16 and 18px on a single row, are the two live violations.
- **One glyph, one meaning**, and the corollary: *no two meanings may share a glyph*. `NewWindow` was `SquareStack` — two equally-sized offset squares, which is the same figure `Copy` draws at 36 call sites, so the titlebar control read as a duplicate button. It becomes **`PictureInPicture2`**, the one glyph in the set that depicts the actual outcome (a second window beside the first). The near-misses were checked and rejected: an arrow leaving a box is `ExternalLink` (20 sites, meaning "leave the app"), and a rounded square with a stroke inside is `PanelLeftIcon` — this button's immediate neighbour.
- **Optical alignment is part of the spec.** Icons in a cluster share one baseline and one gap; a glyph beside a number is 16px against `tabular-nums` so the pair does not shift down a list.

### 3.9 Progress, spinners, skeletons, empty states

- **Progress:** one primitive. 8px track, pill, accent fill, `width` transition at 250ms; indeterminate sweeps `−100% → 250%` on a 1.5s loop; `role="progressbar"` always. Replaces four vocabularies (16px/8px/8px/dot-matrix) and three fill techniques.
- **Spinner:** one primitive, `currentColor`, sizes 14/20/24, rotating at ~525ms/turn linear. Replaces five constructions across ~29 files, including the theme-blind `border-white` ones.
- **Skeleton:** solid fill (no shimmer gradient), content-shaped, opacity pulse 0.25↔1 — and the behaviour worth stealing: a **1s delay before showing**, so fast loads never flash. Policy: skeletons for lists and panels, spinner for buttons and inline actions. Replaces five loading dialects.
- **Empty state:** 24px icon in plain ink, 17/600 title, 14/20 muted line, one *subtle* (never primary) 32px action, 16px stack gap, `32px 24px` padding. The quietest thing on the screen. Adopted by the three views that hand-roll it today.

### 3.10 Tables and lists

Transparent — no card chrome, no outer border, no zebra. Headers are **muted 14/600** (quieter than the body they label), cells 14/400 ink, `8px 12px` padding, 36px rows, and the only lines are 1px bottom hairlines with the last row's omitted. Row hover is `--overlay-hover` at 125ms.

List items: 8px padding, `--radius-element`, 2px gaps, two-line construction (14/400 primary over 12/400 supporting).

**A list gets no container.** Chat history already works this way and is the app's best-looking list: rows sit directly on the page, separated by hairlines, and the only box that ever appears is the hover wash under the row you are pointing at. A bordered list inside a bordered card is a box in a box, and it is what makes the rest of the app look heavier than Chat history does.

**One optical axis per row.** Every right-hand cluster — stat glyphs, figures, action buttons — centres on the same axis as the title's first line, by giving each cluster a 20px-high box with `align-items: center` rather than trusting a 32px button and a 16px glyph to agree. Figures take fixed widths on top of `tabular-nums`, so the glyphs form real columns down the list instead of drifting with each value's width.

**One row spec, enforced:** title `14/500`, metadata `supporting` in muted ink, at most **three visible actions** at 32px with 16px icons, everything else behind a `⋯` overflow. Destructive actions live only in the overflow. This ends: five row-title treatments, the 28-vs-32px delete-button fork, Scheduler's 14px icons, seven-action workflow rows, and primary-filled Launch/Play buttons that are invisible until hover.

**Running is a breathing dot, not a badge.** A green "Running" pill states the fact at the visual weight of a call to action, and it needs a lane reserved for it in every row that might ever have one. A 7px dot with a slow expanding ring shows the same thing and then gets out of the way — the chat's existing working motif, scaled down — and it rides beside the title where the eye already is. One motif, three scales: 16px in the chat, 7px in a row, and the same 1.8s period throughout.

---

## 4. Composition

### 4.1 The app shell — a more compact sidebar

The audit measured **~462px of fixed chrome above the first recent chat**: a 52px empty band, 8px padding, a 32px brand row, a 32px "MENU" header, nine 32px nav rows, two 18px divider blocks and a 32px Recents header. On a 720p window more than half the rail is spent before any history shows.

**Proposal.**

1. **Drop the band to 44px and keep the wordmark on its own line.** Folding the mark into the band saved 32px but buried the brand between two rows of controls; on its own line it introduces the app properly, and the compaction still lands because the row count does the work. Inside the band, a **16px channel** separates the OS's traffic lights from our two controls (collapse the sidebar, open a new window), and the pair itself sits at **2px** — one control group, not two loose buttons.
2. **Delete the "MENU" header** (32px labelling something self-evident) and keep only the Recents header, which is doing real work.
3. **The rail carries one destination and one action.** **Home** sits at the top — it is where the rail returns you — with **New chat** beneath it. The other six (Workflows, Scheduler, Extensions, Skills, Knowledge, Applications) move behind a single **Components** disclosure: a 32px row identical to an item with a leading chevron, no uppercase mini-label, remembering its state. Expanded, its children indent 24px and keep the same 32px height — hierarchy by indent, never by size.

   **Actions do not stay lit.** New chat is the rail's only *action*: it fires and the view moves on, so it must never take the selected wash or the accent rail that a destination keeps. It shows hover and press, and a **pointer** click drops focus immediately rather than leaving a lit row behind. Keyboard activation keeps focus, or a Tab user would be stranded mid-rail — so the blur is conditioned on the activation being pointer-driven (`event.detail > 0`), not applied blindly.
4. **Halve the divider blocks** and delete the upper one; the Components row and the Recents header now do the zoning.
5. **Recents rows** keep 32px, gain a trailing status dot for running sessions, use the app's real session glyphs (a message square for a chat, a branch for a diverged session), and lose the one-off `color-mix` well surface and its 12px radius — the list sits directly on the rail like every other list in the app.
6. Tooltip-on-every-row gets a hover-intent delay so scrubbing the list stops flashing 224px cards.

| | Today | Proposed |
|---|---|---|
| Chrome band | 52 | 44 |
| Brand row | 32 | 32 (kept — it is the branding) |
| "MENU" header | 32 | 0 |
| Nav rows | 288 (9 × 32) | 96 (3 × 32) |
| Divider block | 18 | 10 |
| Recents header | 32 | 32 |
| Air around the wordmark | 8 | 24 (16 above, 8 below) |
| Gaps between rail rows | 0 | 4 (2 × 2px) |
| **Before the first chat** | **462px** | **242px** |

Net: **220px** of rail returned to content. The sidebar keeps 240px of width, and rows stay 32px — already the rail rhythm, now written down.

The last two rows are a **correction made after the compaction shipped**, and the figure was 222px before them. Dropping the band brought the wordmark up with it, landing it 8px under the hairline with the first nav row directly beneath — three elements stacked at one rhythm, which reads as crowded rather than dense. The brand block takes 16px above and 8px below (`px-2 pt-4 pb-2` in `AppSidebar.tsx`); it is a lockup, not a list item, and the air is what says so. Rail rows sit at a 2px gap rather than flush, because at zero their rounded washes touch and a hovered row bleeds into its neighbours — 2px is the gap the menu recipe (§3.8) already uses between items, so the rail and the menus agree, and Recents takes the same (`space-y-0.5` in `RecentChats.tsx`). `--row-height-rail` is untouched at 32px; only the gaps moved, giving a 34px pitch. Twenty pixels is a cheap price for a rail that reads as a list of places rather than a block of colour.

**There is no "Apps" — there is Applications.** The rail ships two adjacent rows whose names a user cannot tell apart, and they are not the same list: `/applications` shows the apps you built with Agent Drafter (`GET /apps`), while `//apps` shows apps advertised by installed extensions (`GET /agent/list_apps`, backed by `McpAppCache`). The second is already second-class — a conditional row, hidden unless some extension happens to advertise one — so it is a destination most users never see, under a name that collides with the one they do.

One concept, one word: **Applications**. The MCP-advertised list becomes a section inside that view rather than a sibling route. This is a naming and IA decision rather than a layout saving — with both rows inside Components the adjacency costs little space — but two words for one idea is exactly the kind of drift this document exists to remove.

The same 44px band is shared by the chat header and the artifact strip from one `--chrome-height` token — today it is 52px written three ways with two different hairline colours and three different grounds meeting at a seam the code comments claim is continuous.

### 4.2 Page headers

One recipe, everywhere: a full-bleed hairline; title `24/600`; optional description in `secondary` muted; **actions right-aligned on the title row**, primary at 36px and everything else at 32px. This replaces the three placements in use (inline-with-title, a button row below the description, none) and the column-capped hairline in Settings and Knowledge. Settings additionally loses its double rule (header border + tab-strip border a few pixels apart).

Breadcrumbs — 28px, 14/400, `/` separators, muted-vs-ink only — are added to drill-in surfaces (Knowledge page, schedule detail, session replay) where the app currently offers only a Back button.

### 4.3 Tabs

Two roles, never mixed. **Underline tabs** for view switching: 32px, 2px indicator sized to the *label* (not the tab), weight 400 → 600, colour muted → ink, 125ms, with the hidden-bold-duplicate trick so bolding never changes width. **Squared chips** for filtering, `--overlay-selected` when active. Document tabs (chat, artifact, dock) keep their pill-plus-stripe language, dropped to 32px and unified on an 8px icon gap.

The document tab's accent stripe sits on the **bottom** edge, inset 11px per side so the 8px corner radius never clips it back into the curve it used to be. That is the same axis D-07's nav underline uses, so one accent vocabulary runs from the page tabs down to the document tabs. A top-edge variant (VS Code's convention) was built and reverted: on a floating pill rather than a docked rectangle it reads as a lid on the tab instead of a marker under it.

### 4.4 The chat surface

Keep: the 760px column, the 16px row rhythm, quiet tool-call lines, the code-block card, the working ring-and-glow (one motif, one owner — the duplicate `TurnActivityIndicator`/`LoadingBioRouter` pair collapses).

Change:

- **Metadata drops to `supporting`** (12px). Timestamps are currently 14px — the same size as message content — repeated under every message; this is the surface's single biggest density cost. One `MessageMeta` primitive replaces three hand-copied animation stacks.
- **One muted ink.** `text-text-default/70` and `text-text-muted` are used interchangeably across the composer toolbar, panel chrome and gauges; the semantic token wins.
- **Composer — elevation or a border, never both.** The composer carries a 1px border *and* `--shadow-composer`, so it is the one element in the app stating its edge twice; the shadow is what lifts it off the canvas and the border is the redundant half. It drops the border and gains the shared floating-surface recipe: elevation plus a 1px inset ring, which is what keeps the edge crisp in dark families where a shadow alone disappears. Focus stops being a border-colour shift and becomes the same 2px inset accent ring every other input uses (§3.2), so nothing shifts layout and drag-over can speak the same language. Radius 16 → 12, and one 12px inset grid replaces the four that meet inside the card today (`px-4 pt-3 pb-3` shell, `px-3 pt-3 pb-1.5` textarea, `px-2 pt-2 pb-1` toolbar, `p-4` attachments). **The toolbar tightens and groups.** Same controls in the same order, but the spacing is made to mean something: chips inside a group sit at 2px and carry a real 6px hit area (14px between adjacent glyphs), while a divider opens a 22px channel between groups — directory, then context, then model. The first chip's inset matches the placeholder's, so the toolbar starts on the same left edge as the text above it. Today the row runs at one flat rhythm on 2px hit targets, so it reads as an undifferentiated run of glyphs.

  **The working directory is a control before the first message and a fact after it.** It can only be chosen while a session is still empty; once a turn has run, everything the agent did is relative to it, and changing it would silently invalidate the transcript. The app enforces that already — the chip becomes a plain label — but the two states look near enough alike that the settled one reads as a control that stopped working. Editable: a trailing chevron, the hover wash, a pointer cursor; it should be the most obviously clickable thing on the row, because that is the only window in which it is offered. Settled: no chevron, no wash, no pointer, the same weight as the model beside it, with the tooltip carrying the full path *and* the reason — "Set when this session started. Start a new session to work somewhere else." The general rule: **state what is true, don't disable what was once offered.** A greyed-out control is a promise the interface has already broken.

  **No "Attach a file" row.** Dropping a file on the composer already attaches it; a menu item for a gesture the surface teaches by itself is a row that earns nothing.

  **Collapsed.** When the artifact panel takes the width, the app already folds every control behind a single `+` at the lower left — a state that exists but was never designed, inheriting the same four insets and the same repainted Send. On the proposed surface the `+` takes the 32px icon-button box so the collapsed row is exactly as tall as the expanded one (the composer does not resize when the panel opens, only its contents change), and two things never collapse: the **model**, because it changes what the next message costs and does, and **Send**. The popover behind the `+` is the standard §3.8 menu — 4px container padding, 32px rows, 2px gaps — rather than today's bespoke `w-64 p-2`, and each row carries its glyph *and* its current value on the right, so it answers "what is set?" without being opened twice. Its order matches the expanded row exactly, so collapsing never rearranges anything.

  **The menu changes with the session, the same way the toolbar does.** Before the first message it holds four items, the directory carrying a trailing chevron because it opens a picker rather than toggling something — the one row that leads somewhere else. Once a turn has run, **the directory stops being a menu item**: it moves above a divider into a small context block — a stated fact with its reason beneath it, in the same place the eye already goes for "what is set" — while extensions, skills and knowledge bases stay interactive, because those genuinely can change mid-session. Nothing is greyed out; the row does not become a dead control, it becomes a different kind of thing. The dead focus ternary is deleted, and Send becomes a real primary icon button instead of an `outline` repainted by a className override.
- **Composer toolbar chips**: one recipe — 16px icons (retiring the 18px outliers), ≥6px horizontal padding (today `px-0.5` gives ~20px hit zones), one metadata size in their popovers (today: 10, 11, 12 and 13px).
- **Tool-call interiors** get one inset grid, replacing five recipes, and arguments render in mono per the app's own mono-for-data rule. The disclosure chevron becomes visible at rest — today it appears on hover, so first-run users get no signal that rows expand.
- **Landing state — the greeting and the composer, and nothing else.** This bullet used to propose Astryx's AI-chat template: greeting, composer, and a row of ghost suggestion chips beneath it, on the argument that the empty chat is the one place a "what can I ask?" affordance genuinely belongs. The chips were built in phase 9 and **have since been removed, by decision** — `ComposerSuggestions.tsx` and its test are deleted, the render site in `BaseChat.tsx` is gone, and the `insert-chat-input` listener in `ChatInput.tsx` that existed only to serve them went with it. **This is not an oversight and must not be "restored".** The deletion is the operator's call, taken after seeing the chips shipped and running; no defect was involved, so there is nothing here for a later reader to repair. The landing state is the greeting and the composer. If a "what can I ask?" affordance is ever wanted again, it is a new proposal that has to make its own argument — this bullet is not it, and reviving it would be reinstating something already decided against.
- **User bubble** and the 896px replay column both move onto the standard radius and the 760px measure. **The replay column is done (2026-09-07):** `SessionHistoryView`'s inner `max-w-4xl` box is deleted rather than converted, and the page's own `ReadableContent` is `size="chat"` — one box, one measure, the same `--measure-chat` the composer reads. The shared transcript, which had no measure at all, joined it. `styles/measures.test.ts` now fails on any `max-w-3xl…7xl` written into those files, because a second ceiling nested inside the column takes precedence again and reads as a local tweak rather than as a fork.

#### The transcript, element by element

The chat is the product, and these are the changes least safe to describe without seeing them — the [showcase](astryx-design-showcase.html) renders each row below side by side.

| Element | Today | Proposed |
|---|---|---|
| User message | tinted, **bordered**, 12px radius, `px-4 py-2.5` | tinted, **no border** (the fill already separates it), `--radius-container`, `10×14` |
| Assistant turn | full-width, no bubble | unchanged — this is right |
| Message metadata | **14px**, the same size as the message text, hover-swaps to actions | **12px `supporting`**, actions sit *beside* the timestamp permanently; one `MessageMeta` primitive |
| `h1` / `h2` / `h3` | 18/26, 16/24, 15/22 — a 20-line utility string on one component | `heading` 20/28, `subheading` 17/24, `label` 14/20 — the same roles as a page |
| `h4` | 13/18 muted, `+0.02em` | `caps` 11/16 muted — the app's one caps style |
| Inline code | 13px on a tinted wash, 4px radius | 13px, `--radius-inner`, `--overlay-selected` wash |
| Blockquote | muted text, no mark | 2px left rail + muted ink — reads as a quote without italics |
| Tables | their own 13px style, unrelated to the app's tables | **the app's table**: muted 600 header, hairlines, `tabular-nums` |
| Lists | `my-2` / `mb-3` | 4px grid, `gap` on the list rather than margins per item |
| Links | accent ink, 40% underline | unchanged — already correct |
| Chain-of-thought | a native `<details>`, browser triangle, 4px radius, no motion | the standard 32px disclosure row, chevron **visible at rest** |
| Tool-call line | a line, not a card (D-17) | unchanged, plus a visible chevron and a legible state (below) |

**The breathing dot stays.** It is the most elegant thing the app already owns, and thinking keeps it unchanged — a 6px core with an expanding ring and a breathing glow sharing one 1.8s period. The only requirement placed on it is geometric: it occupies the same **16px lead slot** as every chevron and tool glyph, so an active thinking row is left-aligned with the entire transcript rather than sitting in its own indent. When the thought resolves, the dot gives way to a chevron and the row restates itself in the past tense ("Thought for 4.2s"), which is also what makes it obvious the row is now expandable.

**One left edge.** Every row in the transcript — a thinking row, a tool call, a paragraph — starts at the same x. **This half ships:** `ToolCallWithResponse.tsx` forces `!px-0` on the tool-call trigger precisely so tool glyphs share the left edge of assistant prose and timestamps, and a test asserts the class. It stays written down because it is easy to break by giving one row type its own padding.

**The hover wash does not ship, and this paragraph used to claim it did.** The claim was *"The shipped app already does this"*, and it was never true: no transcript row carries a hover wash at all. The tool-call trigger explicitly sets `hover:bg-transparent focus-visible:bg-transparent`; prose rows and user messages set no hover background whatever; and the negative-margin-against-equal-padding pair exists nowhere in `ui/desktop/src` — a `git log -S` over the component tree finds no commit that ever added one. The construction the claim pointed at lives only in the showcase's own specimen stylesheet. So this stands as a **proposal**, and a good one: a wash that breathes 8px wider than the text lets a row feel roomy without its content stepping right, which is exactly the density the transcript wants. Build it; do not assume it. The two commits that produced today's state moved the other way — one removed the wash the row used to have, the next zeroed the padding that would have carried the overhang.

#### Tool state — running versus finished

A tool row currently conveys its state almost entirely through its opening verb ("Working on / Ran / Problem with / Stopped") plus an 8px dot on the glyph, and `interrupted` is drawn with the same grey dot as `pending` — so *Stopped* and *never started* are indistinguishable. Five states, one row:

| State | Glyph | Motion | Right slot |
|---|---|---|---|
| **Thinking** | the **breathing dot**, kept exactly as it is | 6px core + expanding ring + breathing glow, one 1.8s period | elapsed, counting |
| Queued | tool glyph, 55% opacity | none | — |
| **Running** | tool glyph + breathing ring | the ring alone — expanding and fading on a 1.8s period | elapsed, counting, after 2s |
| Done | check in `--text-success` | none | elapsed + result summary |
| Failed | alert in `--text-danger`, 5% danger wash on the row | none | elapsed |
| Stopped | its own glyph, muted — never the pending dot | none | elapsed |

The rules that make it work: **one motion, not two** — a sweeping progress bar along the row was tried and cut, because with the ring already breathing the row had two things moving for one fact; **movement always means "still going"**, and it should say so once; the ring **stops and fades over `--dur-fast` rather than finishing its cycle**, so motion ending *is* the completion signal; a finished row is completely still, which is what makes a running one findable three rows away; the live detail replaces itself rather than appending, so the row never grows; and under `prefers-reduced-motion` the ring holds still at 45% opacity — the state still reads, the movement goes.

That leaves the transcript with exactly **one** running motif at three scales: the thinking dot, the tool ring, and the sidebar's session dot — same period, same construction, different sizes.

#### Queued and steered

The app already has both, and they are genuinely different acts. **Queueing** parks a message to send when the turn ends — the queue bar lives in the composer with a "Next" label, a pulsing dot and a `+N` badge (`MessageQueue.tsx`). **Steering** (BR-61, ⌘↵ mid-turn) injects the text into the turn that is *already running* without cancelling it, and the activity indicator echoes the words back in quotes so the press is visibly acknowledged (`TurnActivityIndicator.tsx`). Both keep their behaviour; what changes is that they stop inventing their own vocabulary:

- The queue's dot becomes the **breathing dot** rather than Tailwind's generic `animate-pulse`, so "this will run" joins the one running vocabulary; **Paused is completely still** — a muted dot, no ring. Motion means the queue will drain; stillness means it will not.
- `+N` takes the squared chip recipe instead of its own bordered pill.
- The steer action gets a **visible label**. Today it is an icon whose meaning lives only in a `title` attribute — the least discoverable place in the interface for the one control that changes what the agent is doing right now.
- The steer echo keeps the thing that makes it work — the user's exact words, quoted back — and only restyles the chip: `--radius-inner` rather than a full pill, `supporting` size, the muted-ink token instead of a `/70` alpha. It rides the working row itself, so the acknowledgement sits on the thing it modified.

**Why they must not look alike:** queued is reversible and steered is not. A queued message can be reordered, edited or dropped before it runs; a steer has already reached the model. So queued lives in the **composer**, where things go before they are sent, and steered lives in the **transcript**, where everything is already history. The queue bar also keeps its own compact rhythm rather than borrowing the 32px control height — it is a status strip, not a toolbar, and it must not make the composer taller than the message being written.

This is additive. The collapsed row, the verb grammar and the humanised summaries are all kept; what it adds is a state the eye can find without reading.

#### A long message — collapsed by default (not built yet)

Paste a stack trace, or simply write a long message — the bubble currently renders every line either way, so one message can push the conversation off screen and the reply after it becomes unreachable without a long scroll back. Nothing truncates. This is **not** part of the ten phases; it is written down so the next round starts from a decided shape.

**One mechanism, two contents.** The clamp knows nothing about what it clamps; only the type treatment and the meta unit differ:

| | A pasted log | A long typed message |
|---|---|---|
| Type | mono, own line breaks — it is quoted matter, visibly not prose | the bubble's normal 14px prose, unchanged |
| Container | none — no inner box separates the pasted part from the message | none, for the same reason: it is all one message |
| Meta | `214 lines · 8.4 KB` | `128 words` |

Nothing becomes a box in either case. The only additions are the clamp, the fade and the control below it.

The rules both carry:

- Clamp above the threshold — roughly **10 lines or 600 characters** — and never below it. A three-line message that collapses is worse than no collapse at all.
- Fade the cut using the bubble's **own fill token**, so it reads as "there is more" rather than as a rendering bug, and works in every family without a second colour being chosen.
- The control sits **below** the bubble, not inside it: it belongs to the message, not to the text. It carries the count, because that is what tells you whether expanding is worth it; a bare "Show more" does not.
- Expanded is **sticky per message** and never re-collapses while you are reading.
- **Copy takes the whole thing**, collapsed or not: the clamp is a view state, never a content state.
- The same clamp applies to the composer's queued-message chip, so a long paste does not push the toolbar off screen before it is even sent.
- Past roughly **50KB the right answer is an attachment**, not a taller bubble — a file chip with a preview, which the composer already knows how to render.
- Motion: expanding animates `max-height` at `--dur-med`, the tier for layout rearrangement; collapsing is instant, because you are moving away from it.

Prose stays capped at 68 characters and the column at 760px. What this closes is the audit's finding that the chat type ramp — the de-facto specification for the app's most-read surface — exists only as a class string on one component, and that metadata across the transcript spans **10, 11, 12, 13 and 14px**.

### 4.5 Primary views

Every list view converges on: standard header (§4.2) → optional filter band → hairline list with 36px rows → shared empty state → shared skeleton. This closes seventeen of the eighteen items in the cross-view ledger, including the four error dialects and the five loading dialects.

Two views need structural work rather than alignment:

- **ScheduleDetailView** predates the flat redesign wholesale: no `MainPanelLayout`, `h-screen` sizing (the exact anti-pattern the layout's own comment warns breaks embedded panes), a muted ground, `label:` colon sentences instead of definition rows, and per-semantic tinted button variants that exist nowhere else. It is rebuilt on the standard scaffold with Astryx's **definition-row** pattern: 36px hairline rows, muted label and value left, a text action right.
- **Settings** adopts Astryx's two-cell section grid — description left, controls right, 40px gap — which gives long tabs the scannable hierarchy their flat 11px labels currently deny them, and a sticky anchor rail (232px, one 2px thumb sliding on a 10% track at 130ms) instead of relying on `scrollIntoView` deep links.

**Knowledge** keeps its framed-panel workspace — it is a genuinely different kind of surface — but its panels move to `--radius-container`, its segmented switcher becomes the standard chip row, and its 360px fixed column becomes a `minmax` so the graph stops starving at narrow widths.

### 4.6 The terminal dock

The dock's CSS claims "one tab language, both sizes", and that holds for how a tab is *painted* — not for how it is composed. Today it runs a 40px bar with 28px tabs, a 6px icon gap where the chat strip uses 7px, close buttons always visible where the chat's are hover-revealed, a cwd readout and a hide button the chat strip would never carry, and a third overflow behaviour (scroll immediately, where chat shrinks, then scrolls, then overflows to a menu).

**Changes.** Bar 40 → **36px** and tabs 28 → **32px**, one step under the 44px chrome and on the one control ladder. Icon gap 6 → 8px. Close reveals on hover *and* focus, exactly like a chat tab. Overflow adopts the chat ladder. The cwd readout becomes `supporting` mono, truncating from the left so the leaf directory always survives. And the ground moves off `--sidebar`: that token was chosen so the chat strip could continue the *top* edge, and at the bottom edge the rationale inverts — the dock currently reads as a floating slice of sidebar. It grounds on the surface with a top hairline.

**The ground is a step off the panel surface — never black.** A terminal is not an inverted panel dropped into the app; in a paper-coloured product a black well reads as a hole punched through the page. The terminal ground is one deliberate step off the panel surface, in each mode:

| Family | Light | Dark |
|---|---|---|
| Parchment | `#f4f0e6` — a deeper warm parchment | `#16120c` |
| Alma Mater | `#f2f3f4` | `#0d2a50` |
| Roche Limit | `#f4f4f2` | `#232320` |

> **⚠ This table's premise expired on 2026-08-08, after this proposal was written.** Neutrals are now
> **shared**: all three families paint the same `--background-muted`, so the terminal ground is `#f4f4f2`
> light and `#232320` dark **for every family** — the Roche Limit row, applied to all three. The other two
> rows describe grounds that no longer exist. The proposal's *substance* survives intact, because Roche's
> values are exactly the "one step off the panel surface, never black" the paragraph argues for; what does
> not survive is "in the family's own hue", since the hue is now the same everywhere and the family shows
> up in the **ink**. See [theme system architecture §8](../theming/theme-system-architecture.md#8--shared-neutrals--one-scaffolding-three-inks).
>
> **Do not implement this table as written** — it would reintroduce the per-family greys that were
> deliberately removed.

Ink is the family's own `--text-default`, so a light-mode terminal is dark-on-warm rather than light-on-black. This is a change of *value*, not of mechanism: the ground token already exists and is already generated. It does mean the ANSI palettes must be re-verified against the light ground — which is exactly what the generator's per-slot contrast floors are for, and it will refuse to emit if a slot fails. *(That re-verification has since happened: Parchment's `yellow` and `cyan` needed darkening by ~2.5% to clear AA on the shared light ground.)*

**What must not change**, because each is an invariant with a bug behind it: the **declared terminal ground** as a concept (each family states which token its dock paints, rather than the generator assuming — an assumption would re-ground a terminal under a palette tuned for a different surface; the three now happen to give the same answer, which is a measurement, not a licence to hardcode); the **19-stop ANSI palette** and its per-slot contrast floors; and mono at **13/20**, byte-identical to code blocks.

### 4.7 Files, directories and code

Opening a directory in the artifact panel today yields "a clickable listing" — names, no hierarchy, no status, no preview. This is the surface where Biorouter sits beside a terminal and a repository, so it borrows the interface its users already read fluently: the git working tree.

**Directory tree.** A 240px pane: 28px rows (the compact tier — a tree is the densest list in the app), 24px indent with a 1px guide, one glyph per kind from the app's own icon set, and a status letter at the row end in the status ink (`M` modified, `A` added, `·` clean). The selected file takes the same treatment as a selected nav row — wash plus the straight 2px rail — so "where am I" is one vocabulary from the sidebar to the tree. Status letters are the only colour in the panel, which is the point: colour is evidence.

**File preview.** A 44px header carrying a mono breadcrumb (ancestors muted, the file in ink), a language-and-size chip, and copy / open-externally as 32px ghost icon buttons; then the code body.

**Code blocks — one presentation, two homes.** A snippet in chat and the same file in the preview are the same object and should look it. Today they differ: chat wraps long lines with no gutter, the panel numbers them and scrolls. The rule becomes **line numbers plus horizontal scroll, everywhere** — which is also the only *safe* combination, since `wrapLongLines` together with `showLineNumbers` sets `display: flex` on every line and shreds long lines across the panel. A wrap toggle lives in the header for the rare prose-in-a-fence case.

Header 32px on the control ladder; the language label becomes `supporting` mono in muted ink rather than a fourth uppercase-tracked style; copy and wrap are 24px compact icon buttons — Astryx's sanctioned compact tier, which exists precisely for controls inside a code block. Card radius follows `--radius-container`, the gutter is `tabular-nums` at 60% opacity, and diff rows keep the existing 9% / 10% `color-mix` tints. **The palette itself does not change**: one generated file already feeds chat markdown, the artifact panel and xterm so they cannot drift, and it is contrast-verified per family.

---

## 5. Motion

**Astryx** runs the whole system on **one easing** — `cubic-bezier(0.24, 1, 0.4, 1)`, a strong decelerate with no overshoot — and nine duration tokens (three tiers × min/default/max). Durations are themeable; the curve is not. The governing rule is legible in the data: *the bigger the element, the slower the motion* — menu item 95ms, button 125–175ms, popover 165ms, dialog 400–550ms. Entrances are staged; **exits are near-instant**. Nothing bounces.

**Biorouter today** has a correct three-token scale that *loses by default*: Tailwind's own 150ms/`(0.4,0,0.2,1)` was never re-pointed at the tokens, so every bare `transition-colors` runs a fourth duration and a fourth curve. The count is 49 tokened sites against ~74 literal durations and 150+ bare utilities. `--ease-in` is defined and never used; `--ease-g2` has zero consumers; `--ease-spring` directly contradicts `design.md`'s "no spring, no overshoot" while legislating its own exemption in a CSS comment.

**Proposal.**

1. **Re-point Tailwind's defaults** at the tokens in `@theme`. One change, ~120 per-site annotations deleted, and every future `transition-colors` is on-spec by default. This is the cheapest high-value fix in the document.
2. **One easing token** with Astryx's curve; `--ease-in` and `--ease-g2` are deleted. `--ease-spring` survives *only* for the tab-drag choreography — which the audit called the best-reasoned motion in the app — and `design.md` is amended to say so rather than leaving code and spec each claiming authority.
3. **Nine durations** (fast 95/125/175, medium 250/300/450, slow 525 for ambient loops), declared per family so Parchment can be calmer than a future lab-focused family.
4. **One overlay entry recipe:** fade + a small move toward the final position (8px for menus and tooltips, 16px for dialogs) + `scale(.95)`, `fill-mode: backwards`. Exits fade fast or not at all.
5. **Route changes stay instant** — and the eleven `page-transition` classes that decorate views while being defined in no stylesheet, plus the two dead `view-transition` markers, are deleted rather than implemented.
6. **Reduced motion means instant state change, not no state change** — with one nuance worth copying: spinners slow to ~3s/turn rather than freezing, so necessary feedback survives.
7. Six unanimated abrupt changes get the treatment they were specified for: the sidebar active rail (which snaps today), tool-call expand/collapse, react-select menus, `BaseModal`'s instant mount, the heatmap readout, and `Dot`'s never-implemented live pulse.

---

## 6. What Biorouter already gets right

Not everything needs changing, and the audits were explicit about it. These stand as-is and become the reference standard the rest is held to:

- **The theming pipeline** — one file per family, generate-and-refuse-on-contrast-failure, discovered families, ordered blocks, per-family terminal ground, 252 assertions. Its measured cost for a fourth family is one file, which means a redesigned palette can be trialled as a throwaway family before Parchment is touched.
- **The tab-drag choreography** — ghost lift, source dimmed in flow, deliberately unanimated drop tint, static insertion caret. Every value has a recorded reason.
- **The dropdown row recipe** (`DROPDOWN_ROW_CLASS_NAME`) — one exported class composed by item, checkbox, radio and sub-trigger. This is exactly the pattern §3 generalises.
- **The code/terminal palette** — one generated palette feeding chat markdown, the artifact panel and xterm so they cannot drift, at 13/20 in both.
- **The tool-call line** (D-17) and the collapsed-row label grammar.
- **`NotificationSurface`** — one geometry, two densities, a reserved close gutter. The toast spec in §3.7 is an extension of it, not a replacement.
- **The usage heatmap's fit-to-box engine** — server-side quartile shading, keyboard-focusable cells, and the measure-and-fit sizing shipped in this worktree.
- **Focus as a surface shift** (D-15), which Astryx shows is compatible with an instant keyboard-only outline rather than opposed to it.

---

## 7. The deletion list

A redesign that only adds is a redesign that fails. This was the list the audits proposed. Phase 10 then re-grepped every entry against the tree the nine preceding phases had actually produced, and the rule it applied was: **delete only what nothing references any more.** A deletion that merely compiles is not a passing deletion, because `cn()`/tailwind-merge drops classes it does not recognise without erroring — an unreferenced-looking token and a silently-swallowed one are indistinguishable from the build.

That re-check moved most of this list. The audits were counting call sites in the pre-Astryx tree; five migrations had since changed which aliases were load-bearing, in both directions.

### 7.1 Removed

**Tokens:** `--sidebar-primary(-foreground)` (zero consumers, and Roche's value would fail AA if anything ever used it), `--color-block-orange`, `--ease-g2`, `--font-serif`, `--shadow-modal-chrome-top/bottom` (`none` in all six family/mode combinations, read by nothing).

Removing the four semantic entries also shrinks the theme contract from 60 tokens to 58 — eight fewer hand-picked values every new family has to author across its two modes.

**CSS:** `@keyframes shimmer` + `.animate-shimmer`, `@keyframes breathe-pulse`, `.sidebar-item` and the two of its three `!important` pointer-events guards that were keyed on it alone. The third guard survives on `[data-slot='sidebar-menu-button']`, which AppSidebar still renders; with the dead selectors stripped it became a byte-duplicate of the rule above it, so the two are now stated once. `.biorouter-diagnostics-*`.

**Dependency:** `tailwindcss-animate` — the Tailwind v3 plugin, superseded by the `tw-animate-css` that `main.css` actually imports, and referenced from nowhere but `package.json`.

### 7.2 Kept, with the reason

Each of these still has live consumers. They are follow-up work in the component files, not sweep work:

| Entry | Why it survived |
|---|---|
| `--background-card` | 9 call sites in 7 components (`ui/card.tsx`, `CardContainer`, `UsagePanel` ×3, four onboarding cards) — and the theme generator resolves it as a `SURFACE_TOKENS` entry, so sandboxed `srcdoc` previews inline it as a literal hex. Not byte-identical to `--background-default` any more either: Parchment dark inverts the ladder. |
| `--border-default` | 34 call sites across 13 components, including `ui/separator.tsx`, `SecurityToggle`, `LocalModelInventory`, `SwitchModelModal` and `ExtensionInfoFields`. |
| `--color-block-teal` | 5 call sites (`BrxtInstallModal`, `ImportWorkflowForm`, `CreateEditWorkflowModal`, `ExtensionsView`, `ImportSessionModal`). Its `-orange` twin had none and went. |
| `--sidebar-accent(-foreground)`, `--sidebar-ring` | Consumed throughout `ui/sidebar.tsx`; `--sidebar-ring` additionally by `SidebarUpdateButton`'s `focus-visible` ring. They fall when the shadcn scaffolding does, not before. |
| `--ease-in` | The audit called it "defined, never used" — a phase since added `ease-[var(--ease-in)]` to `SessionListView`'s skeleton cross-fade. Live. |
| `.biorouter-settings-row` | 23 call sites across 17 Settings components. Genuinely a four-percentage-point duplicate of `.biorouter-list-row`, but converging them is a change to those components, not to the stylesheet. |

**Classes in TSX** (`page-transition` × 11, `biorouter-composer-view-transition` × 2, `animate-[wind_…]` × 2, `DocumentPreview`'s dead shadows) and **components** (the shadcn sidebar scaffolding, the hand-rolled modal shells, `SessionViewComponents` with its pre-token `bg-bgSecondary`, two `window.confirm` calls, Button's `shape` prop) are all still present and all still referenced. `animate-[wind_…]` is the sharpest of them: its `@keyframes wind` really is gone, so those two elements on `icons/BioRouter.tsx` name an animation that does not exist — inert, but it should be deleted rather than left looking intentional.

---

## 8. Execution

Ten phases, each a commit on this worktree, each independently revertible, each gated on the full frontend suite (1,654 tests), `lint:check`, the theme `--check`, and the 252 contrast assertions. Phases 1–3 are pure infrastructure and change no pixels by themselves — they are what make 4–10 small.

| # | Phase | What lands | Risk |
|---|---|---|---|
| 1 | **Motion root** | Re-point Tailwind defaults at tokens; one easing; nine durations; delete dead motion code | Low — visible only as consistency |
| 2 | **Type tokens** | The §2.2 scale as `@theme` entries; codemod ~180 arbitrary classes | Low, mechanical, high line count |
| 3 | **Radius + interaction tints** | §2.4 ladder, §2.5 overlays; generator emits both per family | Medium — touches every family |
| 4 | **Typeface** | Bundle Figtree, switch `--font-sans`, re-check contrast at the new metrics | Medium — the most visible single change |
| 5 | **Controls** | Button, input/textarea/select, checkbox/radio/switch, chips/badges on the new specs | Medium |
| 6 | **Overlays** | One dialog shell + size scale, six shells migrated, one close affordance, one toast tier model, notification centre | High — most files, most behaviour |
| 7 | **Shell** | 44px band, sidebar compaction (§4.1), tabs at 32px, one icon-label gap | Medium — the compaction you asked for |
| 8 | **Views** | Header recipe, row spec, list/table, empty/loading/error convergence, ScheduleDetailView and Settings rebuilt | High — broadest surface |
| 9 | **Chat** | Metadata sizing, `MessageMeta`, composer grid and radius, toolbar chips, tool-call interiors (~~landing chips~~ — built here, then removed by decision; see §4.4) | Medium |
| 10 | **Sweep** | The §7 deletions, doc reconciliation (`design.md` amended, not appended), gallery regenerated | Low |

Verification is not "it compiles". Each phase ships with: the existing unit tests green, the contrast script green, and **screenshots at 1440×900, 1120×800 and 800×600 in light and dark for every family** — driven through the same Vite + agent-browser harness used to verify the heatmap work in this worktree, because jsdom cannot catch a Tailwind class collision and Electron cannot be launched from an agent shell. Phases 6 and 8 additionally get a manual pass in the real dev GUI.

---

## 9. Decisions for you

Everything above is a recommendation. These ten are the ones where a different answer changes the work, listed with what I would do and why.

| # | Decision | Recommendation |
|---|---|---|
| **A-01** | **Typeface** — bundle Figtree as `--font-sans`, or keep the native stack? | **Bundle Figtree.** You asked for the advertised font; Inter is already bundled for the wordmark, so the mechanism and the licence question are both settled. Cost: ~35KB and a re-run of the contrast script at new metrics. |
| **A-02** | **Type scale** — adopt the 14-base geometric scale with semantic roles? | **Yes.** It is the largest cheap win in the audit: ~180 arbitrary classes deleted and the first time the documented scale is enforceable. |
| **A-03** | **Content row height** 40 → 36px, contradicting D-12·B | **Yes, tighten.** D-12's principle (one rhythm, no compact mode) survives; only the value moves, and the app gains ~10% more rows per screen. |
| **A-04** | **Radius** — composer and modals 16 → 12px; `full` restricted to dots, knob, avatars | **Yes.** This is what makes the app read as one system. Keeping 16px anywhere but the artifact sheet reintroduces the fork. |
| **A-05** | **Interaction tints** replace hand-authored `--sidebar-hover`/`-active` per family | **Yes.** Removes the exact class of derived-value drift the theming doc warns about, and shrinks what a new family must author. |
| **A-06** | **Selection** — achromatic wash + weight as the base, accent kept as one 2px mark per surface | **Adopt the wash, keep the rail.** Astryx would drop the coral entirely; you asked to keep the colour, and the straightened rail already shipped here. |
| **A-07** | **Toast tiering** — a notification centre for compound reports | **Yes.** The grouped extension report is a 400px interactive panel living in a transient layer; nothing else in the census needs more than three lines and two buttons. |
| **A-08** | **Sidebar compaction** — wordmark folded into the band beside the two titlebar controls, "MENU" deleted, and everything except New chat and Home behind one **Components** disclosure | **Yes.** It returns 272px — more than half the rail on a 720p window — and the two-destination rail matches how the app is actually used. The seven views inside Components are one click away and keep their real icons. |
| **A-09** | **Focus** — keep the surface shift, add an instant 2px accent outline for `:focus-visible` only | **Yes.** These are separable; today keyboard focus is nearly invisible on several controls. |
| **A-10** | **Scope** — all ten phases, or a subset? | **All ten, in order.** Phases 1–3 are what make the rest small; stopping after 5 would leave two systems running at once, which is the state the app is already in. |

Tell me which of these you want changed and I will revise the document before any code moves. If you approve as written, phase 1 begins with the motion root.

---

## Related documentation

- [Biorouter Design System](../../../design.md) — the current source of truth, cited throughout by its `D-nn` decisions; this document proposes amendments to §3.2, §3.4, §3.6 and §4.
- [Theme system architecture](../theming/theme-system-architecture.md) — the pipeline this redesign treats as fixed infrastructure and extends.
- [UI cohesion redesign](../ui-overhaul/ui-cohesion-redesign.md) — the 2026-07 pass whose open items (z-ladder, close affordance, toast layer) this document closes.
- [Astryx design system](https://astryx.atmeta.com/) — the external reference. All measurements in this document were read from live computed styles on 2026-07-27.
