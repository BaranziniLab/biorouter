# Knowledge section — binding UI specification

> **What this is.** The normative design for BioRouter's Knowledge section after the OKF migration:
> every surface, every token, and the complete graph visual specification. It is the binding output
> of [Stage 7](stages.md) and the thing implementation is checked against.
> **Status:** **Current — binding for Stage 7, in slices.** Revised after the three-reviewer committee
> (design-system fidelity, accessibility with measured contrast, implementation feasibility); all ten
> blocking findings are resolved below and every major finding is either fixed or answered with
> numbers. **Slice A (§2) is binding and unblocked and may start immediately.** **Slices B and C are
> binding as design but BLOCKED on Stage 6** — they consume typed graph fields that do not exist in
> `types.gen.ts` until `just generate-openapi && npm run generate-api` has run. Nothing in Slice B or C
> may be started against hand-written JSON.
> **Audience:** Contributors working on the Knowledge subsystem and on the desktop design system.
>
> ⚠ **A redesign amends eight sections of this document, and you should read it before acting on any
> of them.** [`../knowledge-ui-redesign/redesign-spec.md`](../knowledge-ui-redesign/redesign-spec.md)
> targets §3.1/§3.2 (the KB selector leaves the title row), §4.5/§4.7/§3.1 (the legend leaves the
> bottom dock for a dismissible right rail), §4.6 (facets become boxed controls, not secondary
> buttons, plus a priority-order overflow ladder), §4.8 (the inspectors stop being bare
> `role="dialog"` panels), §5.2/§5.3 (fills are solved to a lightness band, not to contrast rungs),
> §5.3.1 (the seven-silhouette shape channel is **removed outright** — every node is a circle; the
> opt-in preference that record first proposed was withdrawn and the channel deleted from the
> codebase), §6.3 (the DOM swatch moves to `--radius-inner`) and §3.4/§3.5 (the responsive ladder).
>
> ⚠ **§3.4/§3.5 is a bug report, not a preference.** `md:` is a **viewport** media query, but the
> thing that changes size is the **pane**. At the app's own minimum window (`minWidth: 1048`, derived
> in `main.ts` as the 288px sidebar *default* + the 760px column) with the sidebar open, the pane is
> **760px** while the viewport is 1048 — so the breakpoint fires and lays two columns into a space
> that cannot hold them. (This read "1000 … 240px sidebar" until the sidebar became user-resizable
> at 216–360px; the window floor moved with the default, the pane at that floor is still exactly 760,
> and the finding is unchanged.)

> **Superseded in part (2026-10).** The Codex simplicity redesign (section 3.12 of
> [its implementation spec](../../design/codex-simplicity-redesign/implementation-spec.md)) merged
> the title and subject bands into one 44px `PageHeader` band, moved the tier control's trigger
> into that band as a badge-button (its confirmation is unchanged), flattened the Sources rail and
> graph into flush columns, made the facet strip 44px, and retired the height step. The container
> ladder (860/940/1140/1400) is unchanged.

The Knowledge section is the one place in BioRouter where the app draws a *data* surface — a canvas
of typed nodes and edges — rather than chrome around prose. That is why it drifted: the graph subtree
grew a second visual language (thirteen hardcoded hexes, a gradient wash, fractional type sizes), and
the surfaces around it re-rolled objects the app already owns (a field edge, a select, a selected row,
a progress indicator, a dot at five diameters, seven hand-written empty states). This document does
two things at once. It brings every non-canvas surface back onto the app's tokens and primitives, and
it gives the canvas a real, generated, contrast-audited, **colour-vision-audited** palette so the one
genuinely new thing in the section — 28 node types, 35 predicates, typed provenance — is legible
without inventing a visual language for it. Where a decision follows from a decision record it names
it (DR-9, DR-9b, DR-10, DR-11, DR-22); where it follows from the design system it names the rule.

Two identifier schemes appear below. `DR-n` are the OKF migration decision records in
[design.md](design.md). `D-nn`, `A-nn`, `P-n` and `DR-nn` in the *drift register* sense are the
desktop design system's records in the repo-root `design.md`; where both could be read, the OKF
records are written `DR-9b`-style with a section reference and the design-system ones are written
`design system D-15`-style. The previous draft cited `OPEN-1` and `OPEN-2`, which exist in neither
scheme; the real records are **raw `<button>` elements outside `components/ui/` (DR-24)** and
**content rows still render 40px against the 36px canonical (DR-62)**.

---

## 1. Design intent

**A quiet instrument that happens to draw a map.** The Knowledge section should feel like the rest of
BioRouter with one extra faculty, not like a graph tool bolted to a chat app. Everything that is not
the canvas — the header, the sources rail, the inspector, the change log — is built from the app's
rows, badges, popovers and empty states, on the app's shared neutral ground, with the app's one focus
treatment and its one hairline weight. There is no second field edge, no second modal system, no
second dot size, no gradient anywhere. A user who has spent eight hours in Chat and opens Knowledge
should recognise every control before reading a single label, and should be able to tell at a glance
which knowledge base they are pointed at, how healthy it is, and what it is made of.

**Colour is evidence, and on the canvas that evidence is the data.** The one deliberate exception to
"nothing else is coloured" is the canvas itself, and it is the same exception the app already grants
its syntax palette and its terminal ANSI palette: a generated, per-mode, contrast-audited data
palette that no component authors by hand. Off the canvas that palette appears in exactly three
places — an 8px legend swatch, an 8px inspector dot, an 8px row dot inside an edge list — and never
as a fill behind text, never as a chrome tint, never as an interaction state. The graph is dense
because a knowledge base is dense; the chrome around it stays silent so the density reads as
information rather than noise.

**Type is carried by colour; the redundant channel for it is spoken, not drawn.** This is where the
accessibility review's correction ended up after R-04 was amended, and it is the largest change
between this version and the draft. Every node is a circle (§5.3.1), so **fill is the only *visual*
type channel** — the earlier claim that "no encoding on the canvas is carried by colour alone" no
longer holds in the sense it was written. What discharges SC 1.4.1 instead is §5.12's
`aria-live="polite"` region, which speaks `<identifier>, <node_type>, <family>` on every focus
change, backed by the haloed labels §5.8 places. ⚠ **Labels are not "always on"** — §5.8's priority
ladder still leaves roughly 82% of nodes unlabelled at fit scale, which is why the live region and
not the label is what the argument rests on.

Everything that is *not* type still carries a monochrome-safe encoding, and those must not be
described as gone: a **solid disc versus a hollow ring** separates the 20 Biomedical Entity types
from the 8 Provenance & Context ones; a **dashed ring** marks an external node; a ring's **arc
count** says how well-sourced a source is; a **dash** says the link is negated, a **dot** pattern
says it was synthesized, a **taper** says which way the claim points. Hue rides along on top of each
of those as the fast channel for trichromatic vision and carries nothing on its own.

⚠ **The residual cost is real, and it was accepted rather than mitigated.** §5.3 gives the
measurements: under simulated protanopia the draft palette put `BiologicalFunction` and `Concept` at
ΔE00 **0.35** — the same colour — and under simulated tritanopia this palette still puts `Phenotype`
and `Food` at **0.00** in dark mode, a floor no palette of 28 marks can raise on one surviving
opponent axis. (The draft's guard tested normal trichromacy only and passed green, which is how the
draft shipped without noticing.) Those two types are now told apart by label, hover, legend and
inspector — all **deliberate** acts, where the silhouette that used to carry family was passive and
told you at a glance. What was bought in exchange is breadth: a spoken announcement serves a blind
reader and a screen-reader user as well as a viewer with dichromacy, and it does not degrade with
mark size, where a silhouette served only the last group and only while the mark was large enough to
resolve. The ruling and its amendment are
[R-04](../knowledge-ui-redesign/redesign-spec.md).

---

## 2. Delivery slices, and what blocks each one

The committee's feasibility review established that this document is not deliverable as one change,
and the reason is dependency rather than effort. Measured in `ui/desktop/src/api/types.gen.ts`
**today**:

```ts
export type GraphEdge = { from: string; relation?: string | null; to: string };
export type GraphNode = { credibility_tier?: CredibilityTier | null; id: string; kind: PageKind;
                          label: string; path: string; retracted?: boolean };
```

There is no `node_type`, `subtype`, `identifier`, `status`, `stale` or `external` on a node, and no
`predicate`, `negated`, `knowledge_level`, `agent_type`, `primary_source`, `synthesized`,
`publications`, quantitative bundle or `qualifiers` on an edge. Every typed surface in §4.6–§4.8 and
§5.3–§5.5 therefore has no data source until Stage 6 has run.

### 2.1 The contract Stage 2 must now emit

**This is a change to [stages.md](stages.md) Stage 2, taken here and to be mirrored there.** Stage 2
as drafted planned `node_type`, `subtype`, `identifier`, `status`, `stale` on `GraphNode` and
`knowledge_level`, `negated`, `primary_source` on `GraphEdge`. That is not enough for §4.8's edge
inspector, which the draft specified against fields nobody was scheduled to produce. Stage 2 now
emits the full payload, because all of it is already in the page frontmatter and emitting it is a
serialisation change rather than a derivation change:

| Type | Field | Notes |
| --- | --- | --- |
| `GraphNode` | `node_type: string` | The OKF type name. Arbitrary in an OKF base; one of 28 in BioOKF. |
| `GraphNode` | `subtype?: string` | |
| `GraphNode` | `identifier: string` | The display identity; `label` stays the raw filename stem. |
| `GraphNode` | `status?: 'draft' \| 'stable' \| 'deprecated' \| 'retracted'` | |
| `GraphNode` | `stale?: boolean` | Derived from `stale_after` at derivation time, not in the renderer. |
| `GraphNode` | `external?: boolean` | A referenced entity with no page yet. |
| `GraphNode` | `degree?: number` | Optional. §5.6's `deg(n)` floors at it when present; the renderer computes it otherwise. The draft referenced this field without anyone planning it. |
| `GraphEdge` | `predicate: string` | Replaces the ambiguity of `relation`; `relation` stays as the deprecated alias for one release. |
| `GraphEdge` | `negated?: boolean` | Emitted, **not** inferred from a `not_` prefix in the renderer. |
| `GraphEdge` | `synthesized?: boolean` | Provenance-derived rather than authored. |
| `GraphEdge` | `knowledge_level?: string`, `agent_type?: string`, `primary_source?: string` | The provenance triplet. `primary_source` is a node id. |
| `GraphEdge` | `publications?: string[]` | |
| `GraphEdge` | `quantitative?: Record<string, string \| number>` | `effect_size`, `sensitivity`, `frequency`, `direction`, `unit`, `ci_lower`, `ci_upper`, … — an open map, so a vocabulary addition needs no renderer change. |
| `GraphEdge` | `qualifiers?: Record<string, string>` | Same shape, same reason. |

**Slice A must not depend on any of it.** That is the design constraint on the decomposition below,
not an observation about it.

### 2.2 The three slices

| Slice | Name | Blocked on | Contents |
| --- | --- | --- | --- |
| **A** | *Native, and honest* | **Nothing.** Lands immediately, in parallel with Stages 2–6. | §3 shell entire (the measure, the full-bleed hairlines, the grid, the three new tokens, the `--dock-height` comment); §4.1 trigger chrome + the anchored searchable picker; §4.2 manager on `ModalShell size="lg"`; §4.4 all five ingest states **including the determinate `Progress`**; §4.10 change log; §4.11 tier control; §4.12 all nine `EmptyState`s and both loading behaviours; §4.5's gradient deletion; §5.11's `useCanvasTheme` extension and the single shared resolver; §5.6 node geometry, which is now the *whole* mark story — every node is a circle in every mode, not "until types arrive" (§5.3.1's shape channel was removed, not deferred); §5.7's resolved-ink edge web and alpha ladder (on today's untyped edges: no taper meaning, no negation case); §5.8 label pass, memoisation and the `wrapLabel` deletion; §5.9 density and culling; §5.12's keyboard model (**built** — `graph/graphKeyboard.ts` + the DOM half in `ForceGraphCanvas.tsx`); the `Badge` `asChild` primitive change; the browser harness and the Rust fixture dump. |
| **B** | *Shows typed structure* | **Stage 6.** Not startable until `types.gen.ts` carries §2.1. | §5.2–§5.3 the palette and the CVD audit (**not** the seven family shapes — R-04 deleted that channel; the hollow Provenance & Context ring in §5.3.1 is what remains of it); §6.1 the generator and §6.4 the split guards; §5.4 the hashed OKF fallback; §5.5 the credibility ring; §4.6 facets; §4.7 the interactive legend; §4.8 node inspector including frontmatter rows; §5.7's negated and synthesized cases; §4.3 the format chooser (also needs Stage 3's format-on-create). |
| **C** | *Deferred, with reasons recorded* | Named routes that do not exist. | §4.9 lint, entire — **cut**, per §2.3. §4.8's edge inspector beyond head + headline + `primary_source`: land the minimal panel in Slice B behind `onLinkClick`, defer the provenance triplet, publications, stats and qualifiers until a real base exercises them. §5.10's seeded component layout: the force-parameter table and `containNode` are the value and stay in Slice A; the BFS/ring/golden-angle initialiser is ~300 lines buying "the first paint already looks like a graph" and is severable. |

**The browser harness and the Rust fixture dump are not severable.** They gate Slice A as hard as
Slice B — §10 lists what jsdom cannot see, and most of Slice A is on that list. Build them first.

### 2.3 Lint is cut from this pass, and why

The draft specified a lint pill in the subject band and a findings popover, both against a cheap,
cached, idempotent `GET` returning structured `findings[]`. **That route does not exist.** Today
`POST /knowledge/bases/{id}/lint` (`crates/biorouter-server/src/routes/knowledge.rs:61`) takes a
`LintBody { model: ModelRef, autofix?: boolean }` and its 200 is an SSE stream of sub-agent events —
an LLM macro that costs tokens and tens of seconds. Specified as drafted, the pill would either fire
an LLM run on every view mount or read `Not linted` for ever, and it would be dead in exactly the
no-model-configured state §4.4's blocked state already handles.

**Cut.** The subject band survives on counts alone. The change costs the section nothing it had, and
it removes the one surface in the document that assumed a backend contract nobody had written.

**To un-defer it**, three things must be specified first and added to Stage 4 and Stage 6, in this
order: (1) the route — a cached `GET /knowledge/bases/{id}/lint/report` returning
`{ findings: [{ severity, rule_id, subject, message, path }], checked_at }`; (2) where the report is
cached and what invalidates it; (3) what `subject` is and how it resolves to a graph node or edge id,
because the whole value of the surface was that a finding row selects the offending object. Until
those exist there is nothing to design against.

Two rules from the cut surface are kept, because they generalise and both had to be stated somewhere:

- A severity dot is **decorative-redundant**. It is `aria-hidden` and never appears without its
  group heading or an adjacent word carrying the same meaning. Measured: under deuteranopia the
  `--background-danger` / `--background-warning` pair collapses to ΔE00 **0.8** in Parchment light
  (25.7 in normal vision), 5.4 in Roche Limit light and 11.1 in Alma Mater light. The dot may never
  be promoted to a standalone signal.
- Counts shown anywhere are **derived by counting**, never read from a report's own scalars.

---

## 3. The section shell

### 3.1 Structure

```text
MainPanelLayout                                   bg-background-canvas, pt-[32px]
└ view column                                     flex flex-col min-h-0 flex-1, data-search-scroll-area
  ├ HEADER BAND        flex-shrink-0  border-b border-border-subtle
  │   └ ReadableContent size="graph"  px-8 pt-12 pb-6
  │       row 1:  <h1 class="text-title">Knowledge</h1>  ·······  [KB selector] [Manage]
  │       row 2:  <p class="text-secondary text-text-muted">…</p>
  ├ SUBJECT BAND       flex-shrink-0  border-b border-border-subtle   h-row
  │   └ ReadableContent size="graph"  px-8
  │       [8px dot] Name · OKF · Private · 42 pages · 88 links  ····  [⋯]
  └ WORKSPACE          flex-1 min-h-0
      └ ReadableContent size="graph"  px-8 pt-6 pb-8   grid gap-4
          ┌ SOURCES rail ┐ ┌ GRAPH column ─────────────┐ ┌ DETAIL rail ┐
          │ 300px        │ │ minmax(0,1fr)             │ │ 340px       │
          │              │ │  facet strip   h-dock     │ │             │
          │              │ │  canvas        flex-1     │ │             │
          │              │ │  legend dock   h-dock     │ │             │
          └──────────────┘ └───────────────────────────┘ └─────────────┘
```

`size="graph"` is `max-w-[clamp(1440px,96%,2200px)]`, already present in
`components/Layout/ReadableContent.tsx:32` and with zero consumers. Header, subject band and
workspace all take it, so the three left edges are one line.

**Correction to the draft's justification.** The draft claimed this "closes the current fork where
the header uses `size="text"`". It does not: `KnowledgeView.tsx:50` and `:59` are **both**
`size="text"` today, so the measure already agrees between the bands. The real fork is the gutters —
line 50 runs `px-4 pb-4 pt-8 sm:px-6 lg:px-8 lg:pb-6 lg:pt-12` against the app's flat `px-8`, and
that is what this section flattens.

Adopting `graph` is therefore a deliberate widening, not a repair, and it must be recorded as one.
**Knowledge becomes the app's one canvas view and gets the app's one canvas measure**, tokenised as
`--measure-graph` beside `--measure-chat` and `--measure-page` (§6.2) and cited against **content
max-width still varies across views (DR-52)**. Leaving the widest column in the app as an
un-tokenised arbitrary value inside a document that tokenises two rail widths would have been the
inconsistency; the divergence itself is the point of the view.

### 3.2 Header band

| Element | Spec |
| --- | --- |
| Outer div | `flex-shrink-0 border-b border-border-subtle` — the hairline is **full-bleed**, outside the measure (design system §4.2) |
| Inner | `<ReadableContent size="graph" className="px-8 pt-12 pb-6">` |
| Title row | `flex items-center justify-between gap-4 mb-1` |
| Title | `<h1 className="text-title">Knowledge</h1>` |
| Description | `<p className="text-secondary text-text-muted">Personal knowledge bases Biorouter builds and maintains for you.</p>` |
| Right cluster | KB selector trigger (§4.1) then `<Button variant="secondary">Manage bases</Button>` — `--control-md`, `gap-2` |

**`page-transition` is dropped.** The draft carried it "for consistency with the other eleven views"
while conceding it animates nothing. Verified: ten call sites across `src/components/`, and **zero**
matching rules in `src/styles/`. The design system's implementation-status table records "Dead
Tailwind classes: 0" as a standing on-disk check, so propagating an eleventh call site would falsify
a published number to no effect. New code does not carry it. Defining `.page-transition` for real is
a three-line authored rule beside `fade-slide-up` and is a design-system change, not this one.

### 3.3 Subject band

A single `h-row` row naming the base the whole view is about. It is not a chrome band and must
**not** read `--chrome-height`: the three 44px bands (sidebar titlebar, chat header, artifact tab
strip) meet at one continuous top edge and move together or not at all (design system GEOMETRY-2).
This band is inside the page, below the page header, and takes the content row rhythm instead.

Written `h-row`, not `h-[--row-height]`. `--spacing-row` is mirrored in `@theme inline` and `row` is
registered in `src/utils.ts`'s `spacing` list, which is what makes a call-site override actually beat
a primitive's default instead of both surviving the tailwind-merge. The same rule gives `h-dock` in
§5.1. The arbitrary `[var(…)]` form is reserved for the two rail widths, which have no utility until
§6.2's registration lands.

Left, in order, `flex items-center gap-2 min-w-0`:

1. 8px `rounded-full` base colour dot (`aria-hidden`; the name is the label), with the swatch ring of
   §6.3.
2. Base name — `text-label text-text-default truncate`. Not `font-semibold`: `text-label` already
   carries 500, and `text-label font-semibold` (14/600) is not a step in the type scale.
3. Format badge — `<Badge uppercase>OKF</Badge>` / `<Badge uppercase>BioOKF</Badge>` /
   `<Badge uppercase>Legacy</Badge>`, `tone="neutral"`. The legacy badge carries
   `title="Created before the format chooser. It reads fine and is not validated."`
4. `<PrivacyBadge tier dense>` when `tier !== 'public'` — the app's padlock, unchanged.
5. Counts — `text-supporting text-text-muted font-mono tabular-nums`: `42 pages · 88 links`.
   `font-mono tabular-nums` because these are digits that change under the user and must not jitter
   (design system TYPE-13).

Right, `flex items-center gap-2 shrink-0`: a Refresh graph `<Button variant="ghost" shape="round">`
(32px, the one action the user repeats) then a single `⋯` overflow holding Export as `.brkb`, Change
log, Open folder, and (destructive, at the bottom, separated) Delete knowledge base. That is two
visible actions against the app's ceiling of three, with every destructive action behind the one `⋯`
(design system ROWS-3).

### 3.4 Workspace grid and responsive behaviour

**Two named steps, and no third.** The draft introduced a bare `1100px` twice, in a document that
says "do not add a third hardcoded breakpoint" about `768px` (DR-57). It is removed: the facet
strip's intermediate label-drop rung is folded into `--breakpoint-md`, and between 930px and 1280px
the strip simply scrolls, which it already does (`overflow-x-auto` with `scrollbar-gutter: stable`).

| Step | Grid | Detail rail |
| --- | --- | --- |
| ≥ `xl` (1280px, Tailwind's default, used app-wide and unmodified) | `grid-cols-[var(--knowledge-rail-sources)_minmax(0,1fr)_var(--knowledge-rail-detail)]` | A real column. It **pushes** the canvas; it never covers it. |
| `--breakpoint-md` (930px) – 1279px | `grid-cols-[var(--knowledge-rail-sources)_minmax(0,1fr)]` | An overlay on the canvas, top-right, `--radius-container` + `--shadow-popover` + `--inset-hairline`, `z-[var(--z-dropdown)]`, `w-[var(--knowledge-rail-detail)]`, `max-h-[calc(100%-2rem)]`. |
| < 930px | Single column with a tab pair | A right-side `Sheet`, `w-[var(--knowledge-rail-detail)]`. |

**When both the detail overlay and the change-log `Sheet` are open, the `Sheet` wins and the overlay
closes.** They are `--z-dropdown` (200) and `--z-overlay` (300) respectively, so the `Sheet` would
cover the overlay anyway; closing it explicitly means the user does not return to a panel they cannot
see. The draft left this undefined.

Below 930px the workspace is a two-tab pair — **Sources** and **Graph** — rendered with
`<Tabs>` / `<TabsList>` / `<TabsTrigger>` from `components/ui/tabs.tsx`, moved into the subject band's
left slot so the band still names the base.

**Not the hand-rolled segmented pill the view carries today.** `KnowledgeView.tsx:73–87` draws raw
`<button>`s as a segmented control, which is design system **D-07 option B** — the option whose
record reads "Reads as a *filter*, not navigation… **Delete B.**" Carrying it forward would make
Knowledge the only place in the app with a third horizontal tab language, in a document whose §9
promises "new code in this section uses the primitives". The objection that the underline does not
fit a 36px in-band slot does not survive measurement: `tabs.tsx:39` is already `h-9` (36px) and draws
its indicator with `after:absolute after:-bottom-px after:h-0.5`, so it fits exactly. The primitive
also supplies `role="tablist"`, `role="tabpanel"`, and ←/→/Home/End, which the hand-rolled pair does
not. Both `data-testid`s survive; §9 requires it.

Column gap is `gap-4` (16px, "between groups"). Panes are `rounded-container border border-border-subtle
bg-background-default` with `box-shadow: none` — the app's flat card recipe.

**The canvas paints `bg-background-muted`, and the ladder inverts between modes on purpose.** The
draft justified this as reading "as a work surface *inside* a card", which is a light-mode-only
reading and design system **P7** forbids exactly that ("dark mode is not a filter applied to light
mode"). Measured: light `--background-default` `#ffffff` → `--background-muted` `#f4f4f2`, so the
canvas **recedes**; dark `#1b1b19` → `#232320`, so the canvas **lifts**. Both are correct, because
this is the app's own unified neutral ladder after the three-family unification — canvas darkest,
cards a step up — and the theme-system architecture records that direction change as the largest
visible consequence of unification. The canvas is a distinct *plane* from its pane in both modes; it
is inset in light and raised in dark, and it is not asked to mean "inset" in either.

⚠ **Do not repaint the canvas `--background-canvas` to make the ladder monotone.** It would improve
the palette's floors (light 3.50 → 3.86, dark 3.48 → 4.11, both measured) and it would invalidate
§5.1's entire shared-palette argument, which rests on `--background-muted` being byte-identical in
all six family × mode scopes. Re-derive the palette first or do not move the ground.

### 3.5 Yield order under a narrow window

The app's yield ladder applies in this order and nothing may reorder it:

1. The sidebar collapses to an overlay below 1120px (global, unchanged).
2. The **detail rail** yields its column first and becomes an overlay — it is the least-often-open pane.
3. The **sources rail** yields next, becoming the `Sources` tab.
4. The **canvas never yields.** It is the reason the view exists.
5. The facet strip **degrades by priority — it neither wraps nor scrolls** (R-09, superseding this
   step's original "scrolls horizontally between 930px and 1280px"). Three container-query states on
   the pane: full at ≥ 1140px, condensed to `Type` + `More (3)` at 860–1139px, and one
   `Filters (3)` control below 860px. §4.6 carries the detail.

---

## 4. Every surface, specified

### 4.0 One searchable-picker pattern, and it is not a menu

Five surfaces in this section need a pinned search field over a list: the KB selector (§4.1), the
ingest model picker (§4.4), and the Type, Predicate and Source facet menus (§4.6). The draft
specified all five as a Radix `DropdownMenu` with an `<Input>` at the top. **That combination does
not work**, and specifying it once would have replicated the defect five times.

Radix's `DropdownMenu` implements the ARIA *menu* pattern, which includes typeahead: printable
characters are captured at the menu root and move focus to the matching item. A nested text input
competes for every keystroke — characters are intercepted, or focus jumps out of the field mid-word.
Arrow keys are likewise owned by the menu's roving focus, so the field cannot be exited predictably.

**All five use the combobox/listbox pattern instead: `Popover` + `Command` (cmdk).** That is what the
chat-side KB chip is already described as elsewhere in the repo's documentation ("a cmd-K-style
palette"), and it gives typeahead, arrow navigation and `aria-activedescendant` for free.
`DropdownMenu` survives only for the menus with no search: the Status facet and the `⋯` overflows.

**Field heights.** The draft wrote `<Input className="h-8">` three times and `h-control-sm` (28px)
once. The first is a no-op — `input.tsx:34` already ships `h-8` — and the second is a height override
of a primitive, which design system **P4** forbids ("overrides that change colour, radius, height, or
border are forbidden — they indicate a missing variant"). Both are removed: **every field in this
section is the `Input` primitive at its own 32px**, with no `className` height. There is no sub-32px
field in the app — `Select.tsx:52` is `h-8` too — and adding a `size` variant to `Input` is a
design-system change this document does not own.

### 4.1 KB selector trigger

Replaces `KBSelectorTrigger.tsx`'s bespoke field. It is a Select-shaped control, so it takes the
Select trigger's chrome exactly.

| Property | Value | Why |
| --- | --- | --- |
| Height | `h-control-md` (32px) | A selector is `md`, not `lg`. Written as the token, not `h-9`. |
| Radius | `rounded-element` | 8px, the control rung. |
| Border | `border border-border-emphasized` | The app's resting edge for an interactive element. **Not** `border-border-input`: that token is darker than `--border-strong` in light and lighter in dark, so the current `hover:border-border-strong` measurably weakens the edge on hover in both modes (light 1.66:1 → 1.52:1; dark 1.91:1 → 1.59:1). |
| Hover | `hover:inset-ring-2 hover:inset-ring-border-emphasized/30` | The Input primitive's whisper. Nothing shifts, no hue change. |
| Focus | nothing at the call site | Global. `--background-focus` + `--border-focus` (design system D-15). |
| Fill | `bg-background-default` | |
| Gap | `gap-2` (8px) | Not `gap-2.5`; 10px is off the 4px grid. |
| Weight | `text-label` | Not `text-label font-semibold` — 14/600 is not a step in the scale. |
| Trailing | `ChevronDown` at `--icon-row` (16px), `text-text-muted`, `rotate-180` on open over `--dur-fast` | Matches `Select`. |
| Name | `aria-label="Knowledge base"` + `aria-expanded` | The `KB` badge is dropped, so the control needs an accessible name of its own; the surrounding `<h1>` is not one. |

> **The replacement edge is a repair, not compliance.** `--border-emphasized` is
> `color-mix(in oklab, var(--text-default) 24%, transparent)`, so composited over
> `--background-default` it measures **1.62 / 1.66 / 1.66** in the three light families and
> **2.07 / 2.10 / 2.04** in the three dark ones — all below SC 1.4.11's 3:1 for the visual boundary
> of an active control. The swap removes a hover regression that made a bad edge worse; it does not
> make the edge compliant. Reaching 3:1 is an app-wide `Input` token change and is tracked outside
> this document, exactly as `--row-height` (DR-62) is.

Content, left to right: 8px `rounded-full` colour dot (`aria-hidden`, with the §6.3 swatch ring) ·
name (`truncate`, `min-w-0 flex-1`) · `+N` in `text-supporting text-text-muted` when other bases are
in this chat · format `Badge uppercase` · chevron.

**Opening behaviour changes.** Clicking the trigger opens an anchored `Popover` + `Command` (`w-64`,
`sideOffset={6}`, `.biorouter-popover-surface`), not a 760px modal: a `CommandInput` at the top, one
`CommandItem` per visible base carrying its dot, name and format badge, then a separator, then
`Manage bases…` and `Create knowledge base…`. Picking a base sets the primary and closes.

### 4.2 KB manager, and the file split this implies

**`KBSelector/KBSelectorPalette.tsx` (493 lines) is today both the picker and the manager.** §4.1
makes the trigger open an anchored picker and this section makes the manager a separate modal, so the
file is **split in two**: `KBSelectorMenu.tsx` (the `Popover` + `Command` picker) and
`KBManagerDialog.tsx` (the `ModalShell`). The draft never said so, and three consequences follow that
an implementer would otherwise have to discover:

- **`KnowledgeView.tsx`'s ⌘K handler currently toggles the palette.** It re-points at the picker, not
  the manager — ⌘K is "switch base", which is what it has always meant.
- **`KBSelectorTrigger` has two consumers**, the Knowledge view and
  `components/bottom_menu/BottomMenuKnowledgeSelection.tsx`, the chat-side KB chip. **The chip adopts
  the new trigger.** The same task should be the same shape in both places, and the chip is the
  surface that already behaves like a picker. The chip's own `Manage bases…` item opens the manager
  dialog over the chat, unchanged in behaviour.
- **`KBSelectorPalette.test.tsx`'s ten cases are ported, not deleted.** Six of them encode the DR-12
  primary-repair contract ("following the default again") and belong with the picker; the remaining
  four are list and search behaviour and belong with the manager. Deleting the file and re-writing
  tests for the two new ones would drop the DR-12 contract silently, which is the failure mode this
  clause exists to prevent.

The 760px `Dialog` becomes a `ModalShell size="lg"` (`--dialog-lg`, 640px) with `purpose="form"` —
backdrop click must not dismiss it while a name is half-typed. `760px` is not a dialog width; it is
`--measure-chat` borrowed, and there is no fourth width.

| Region | Spec |
| --- | --- |
| Header | `DialogTitle` at `text-subheading` (unchanged primitive). Description as today. |
| Toolbar | `px-4 pt-3 pb-3`, `border-b border-border-subtle`. `<Input>` (the primitive, no height override) with a leading 16px `Search`, then `Create knowledge base` (`variant="default"`) and `Import from .brkb` (`variant="secondary"`), both `size="default"` (32px). Icon gap comes from `Button`'s own `gap-2`; no `mr-1.5`. |
| Follow-the-default notice | Unchanged in behaviour and copy. Surface becomes `rounded-element` (an element inside the 12px dialog container, one step down). |
| List | `.biorouter-list-shell` inside `max-h-[52vh] overflow-y-auto px-4`. |
| Footer | `DialogFooter`, `variant="secondary"` Close, tip text unchanged. |

**`secondary`, not `outline`.** Design system §4.1 defines `outline` as "secondary action on a
**tinted** ground" and **D-25** narrows it to "survives only for a secondary action on an already-
tinted ground", the decision having been taken because "a 1px box drawn around the panel's quietest
actions was the heaviest line in it". Every ground in this section — the page canvas, the dialog
body, the pane, the facet strip — is `--background-canvas` or `--background-default`, i.e. untinted.
So **every secondary action in this document is `variant="secondary"`**: Manage bases, Import from
`.brkb`, Stop, Retry failed, Cancel, the four facet buttons in §4.6, and the `EmptyState` actions in
§4.12. `outline` appears nowhere in this section.

**Row.** `.biorouter-list-row flex items-center gap-3 px-3` with `role="option" aria-selected`.

- Selected (primary) row: `tint-selected tint-interactive` — **not** `bg-background-medium`.
  `.biorouter-list-row:hover` is declared *unlayered* in `main.css` and repaints at
  `color-mix(--background-medium 42%, transparent)`, which is *lighter* than the hardcoded fill, so
  today the primary base visibly un-highlights under the pointer. The `tint-selected.tint-interactive`
  pair exists at specificity (0,2,1) for exactly this collision.
- Dot: 8px via a shared `KbDot` component — one diameter for the object everywhere. Today the object
  is drawn at 6, 8, 10 and 12px across five files. It carries the §6.3 swatch ring, which is what
  keeps it legible on this very row.
- Name `text-label truncate`, id `text-supporting font-mono text-text-muted truncate`.
- Badges: format, `BuiltInBadge`, `PrivacyBadge` (only when not public), `Not in this chat`,
  `Primary` (`tone="accent"`).
- Actions: the visibility `Switch` (unwrapped — the ad-hoc `px-2 py-1` bordered box goes; the switch
  is its own affordance and carries an `aria-label`), then Export and Rename as
  `<Button variant="ghost" shape="round">` (32×32, 16px icon, each in a `Tooltip`), then one `⋯`
  overflow holding **Delete** with `variant="destructive"`. A destructive control never sits visible
  in a hover cluster, and `text-text-danger hover:text-text-danger/80` — which lowers contrast on
  hover — goes with it.

### 4.3 The format chooser

New surface. Reached from `Create knowledge base` in the manager and from the trigger menu.
**Slice B** — it needs Stage 3's format-on-create.

`ModalShell size="md"` (`--dialog-md`, 480px), `purpose="form"`.

Body, in order:

1. **Name** — `<Input>` with label `Name`, `text-label`. Below it, `text-supporting text-text-muted`
   showing the derived id in `font-mono`: `Will be created as knowledge/<id>/`.

   > **The id shown is a preview, and the surface says so.** Slug derivation and collision handling
   > are owned by the daemon (`create_base_as`), not the renderer, so a client-derived id the server
   > then alters is a real footgun. The line reads `Will be created as knowledge/<id>/ — the final id
   > may differ if that name is taken.` and the created base's actual id is echoed in the success
   > toast. The draft showed a client-derived id as fact.

2. **Format** — a `role="radiogroup"` of two rows driven **through `CustomRadio`'s own slots**, not
   two cards and not a re-rolled `.biorouter-list-row` with a `CustomRadio` glyph inside it.

   The draft specified "a `CustomRadio` mark (16px, `--radius-full`, 6px accent dot)". That is
   design.md §4.9's canonical text, but the shipped primitive was retuned:
   `CustomRadio.tsx:45–70` draws **a 22px visual ring inside a 24px hit target, with a 10px inner dot
   and an 8px gap to the label** — its own comment says so. Implementing 16/6 would author a fourth
   radio geometry. `CustomRadio` is also already a complete row: it renders its own `<label htmlFor>`
   with `label`, `secondaryLabel` and `rightContent` slots, so nesting it inside a `div
   role="radio"` would put a `<label>` inside a radio role and leave three slots unused.

   The three slots map exactly: `label` → the format name plus its `Badge uppercase` short code;
   `secondaryLabel` → the "pick this when" line; `rightContent` → nothing. The three-item fact list
   is rendered under the row, `text-supporting text-text-muted`, as a plain `<ul>` with no bullet
   marks. The draft gave each item "a 4px `--radius-full` `bg-background-strong` dot"; `--radius-full`
   is permitted on exactly three things — status dots, the switch knob, avatars — and a 4px list
   bullet is none of them and is below the 8px diameter §4.2 fixes for the dot object.

   Keyboard: the primitive supplies the radiogroup contract (one tab stop, roving `tabindex`,
   Up/Down/Left/Right both moving and selecting, the whole row as the target). The draft named the
   role and specified none of it.

   | | **OKF** — general knowledge (default, preselected) | **BioOKF** — curated biomedical |
   | --- | --- | --- |
   | Pick this when | You are keeping notes, project context, retrieval material, or anything that is not curated biology. | You are curating biomedical literature or building a base another institution will read. |
   | Fact 1 | Any page type, any link name. Nothing is ever rejected. | 28 page types and 35 link predicates, checked. |
   | Fact 2 | Best when you do not yet know how the material will be structured. | Every link must name its evidence: knowledge level, agent type, and a primary source. |
   | Fact 3 | Validation reports broken links only. | Validation flags anything outside the vocabulary and names the closest legal value. |

3. **The irreversibility notice.** A `--wash-warning` banner (`rounded-element`, 20px `AlertTriangle`
   in `--text-warning`, `text-supporting`): *"You cannot change a knowledge base's format yet. Pick
   BioOKF only if you want the biomedical vocabulary enforced from the first page."* This is not
   decoration — `kb_migrate_format` is deferred by DR-22, so the choice is currently permanent and
   the UI is obliged to say so. When migration ships, this banner is deleted and the record updated;
   nothing else in the surface changes.

4. Footer: `Cancel` (`variant="secondary"`) · `Create knowledge base` (`variant="default"`).

The chooser is also the surface a future `Convert format` action reuses; do not build a second one.

### 4.4 Sources rail — the ingest panel and its five states

The rail is a flat column pane. Header strip: `h-row`, `px-3`, `border-b border-border-subtle`,
holding `<h2 className="text-caps text-text-muted">Sources</h2>` and, on the right, the staged count
as a `Badge` when non-zero. Body scrolls; the footer is pinned.

Body order: tier control (§4.11) · dropzone · Paste text · warnings · staged list · digest progress.
Footer (`border-t border-border-subtle p-4`, `flex flex-col gap-2`): model picker · Digest button ·
blocked-reason line.

Fixes that apply across the rail regardless of state:

- **Dropzone medallion** — `h-12 w-12 rounded-container border border-border-subtle bg-background-muted`
  with a **24px** icon, matching `EmptyState`'s own plate (`empty-state.tsx:34–35`: an `h-12 w-12`
  plate with an `h-6 w-6` icon). The draft said 20px "matching `EmptyState`'s own plate"; the plate is
  right and the icon size is not. 20px is `--icon-banner`, commented "banners, empty states", so the
  primitive and the token already disagree — that disagreement is real and is a design-system item,
  not something this document silently picks a side of. **Draw 24px, matching what ships.**
  `--radius-full` on a 40px medallion is dropped for the same reason as §4.3's bullets.
- **Format chips** in the dropzone lose `font-mono`. A file extension in a chip is chrome naming a
  thing, not data to be read character by character (design system TYPE-11 / D-33).
- **Paste box** uses `<Input>` and a real textarea, both at `text-label`, and relies on the global
  focus treatment; the bespoke `focus-within:border-border-strong` goes. Detected URLs become
  `<Badge variant="chip">` toggles, not hand-rolled `py-0.5` spans.
- **Staged rows** sit flush in a `.biorouter-list-shell` — the `gap-1.5` between rows goes, so each
  row's bottom hairline divides rather than floats. Icons are 16px (`--icon-row`) throughout; the
  remove control becomes a `--control-compact` (24px) `shape="round"` ghost button, not a 12×12
  pointer target.
- **Warnings** stop using `.biorouter-list-row` as a card. The panel becomes
  `rounded-container border border-border-subtle`, and the per-warning cards inside it drop to
  `rounded-element` — the nesting ladder runs downward, and today it runs upward. `opacity-90` on the
  message body becomes `text-text-muted`; `opacity-70 hover:opacity-100` on the dismiss becomes a
  `--control-compact` ghost `shape="round"`.
- **One ellipsis.** `…` everywhere. Today `IngestPanel` writes `Stopping…` directly above
  `DispatchProgress`'s `Stopping...`, simultaneously.
- **One sentence case.** `Clear` (capital) everywhere; today `StagedList` says `clear all` and
  `IngestWarnings` says `Clear`, six components apart in the same rail.

#### The five states

| # | State | What renders |
| --- | --- | --- |
| 1 | **Empty** | Dropzone + Paste text. Staged list is `<EmptyState compact icon={Inbox} title="Nothing staged" description="Drop files above, paste text, or choose a folder." />` with no action — the actions are directly above it. Digest button is full-opacity with `cursor-not-allowed` and the helper line `Stage a file to digest.` (K-04 is preserved verbatim: the one primary action never trains the eye to ignore a permanently half-lit button.) |
| 2 | **Staged** | `.biorouter-list-shell` of staged rows under a `text-caps text-text-muted` `Staged · N` header with a `Clear` ghost. Digest enabled. |
| 3 | **Digesting** | `<Progress>` — **determinate on the queue**: `value={completed}` `max={queue.length}`, `aria-label="Digesting staged sources"`. The per-item sub-agent stream stays a log below it. This is the fix for the section's longest operation having no `role="progressbar"` anywhere. `indeterminate` is used only for the pre-flight model check, where there is genuinely no denominator. Digest button becomes `Stop` (`variant="secondary"`, `--control-lg`); the bare-text Stop link inside the log goes. |
| 4 | **Blocked** | Digest stays full-opacity and `aria-disabled`; the helper line states the one true reason, in the existing precedence (no base → base unavailable → model loading → no model → nothing staged). `Retry` stays a real `<Button variant="ghost" size="sm">`, not an underlined text span. Unchanged logic; changed spelling. |
| 5 | **Failed** | Errored rows stay, each with its message in `text-supporting text-text-danger`. Above the footer a `--wash-danger` summary row appears: `N sources failed` + `Retry failed` (`variant="secondary"`) + `Clear failed` (`variant="ghost"`). Successful rows still auto-clear. |

**Digest button** is `size="lg"` (`--control-lg`, 36px) — the view's single dominant action — and
carries no `className` height override. `size="sm" className="min-h-9"` is a contradiction: it forces
a 28px rung to render at 36px while keeping `sm`'s `gap-1.5`.

**Model picker** takes the same Select chrome as the KB trigger (§4.1) and opens the §4.0 combobox
(`Popover` + `Command`, `w-64`, `CommandGroup` per provider, `max-h-[400px] overflow-y-auto`) — not a
second 760px modal, and not a `DropdownMenu`. The `Set as default` affordance becomes a
`<Badge variant="chip">` inside the menu footer, not a hand-rolled `px-2 py-1` span in the trigger.
Its empty state is `<EmptyState compact icon={Brain} title="No models available"
description="Configure a provider in Settings." actions={<Button variant="secondary">Open settings</Button>} />`.

### 4.5 Graph panel

Three stacked regions inside the pane, no gutter between them.

1. **Facet strip** — `h-dock` (36px), `px-3`, `border-b border-border-subtle`,
   `bg-background-default`, `flex items-center gap-2`, `overflow-x-auto` with `scrollbar-gutter: stable`.
2. **Canvas** — `flex-1 min-h-0 relative bg-background-muted`. No gradient, no card, no shadow, no
   inner padding. The canvas is the content; the pane's own border is the only edge.
3. **Legend dock** — `h-dock`, `px-3`, `border-t border-border-subtle`, `bg-background-default`.

`--dock-height` is reused deliberately: this is the same object it already names — a horizontal
control strip docked to a pane edge, one step down from a chrome band. Its comment in `main.css`'s
hand-authored `:root` block is amended in the same change so the name stops claiming to be
terminal-only. The value does not move.

**The inline gradient is deleted.** `ForceGraphCanvas.tsx:223` currently sets a three-layer
`radial-gradient` + `linear-gradient` on the container. It is the only gradient-washed surface left
in the application, it has no dark variant (the white→black layer paints identically on the near-black
dark ground), its two tints are two of the off-system node hexes leaking into the background of the
panel that displays them, and it breaks the runtime ground resolve —
`getComputedStyle(el).backgroundColor` returns `rgba(0,0,0,0)` when the colour lives in
`backgroundImage`. Removing it is one property and is the single highest visible-effect edit in this
document.

### 4.6 Facet rail

**Slice B.** Four facets plus a search box, in one 36px strip. Semantics: **OR within a facet, AND
across facets.** A node that fails the filter takes the search-miss alpha (0.12) and stays in place —
one dimming mechanism, never a second.

| Control | Spec |
| --- | --- |
| Search | `<Input>` at its own 32px, `w-[200px]`, leading 16px `Search` icon, `placeholder="Filter by name or type"`. Matches `identifier`, `node_type` and `subtype`, case-insensitively, as a substring — so `Disease` selects a class and `IL6` selects a node without a mode switch. Live on every keystroke, no debounce, no submit. |
| Type | `<Button variant="secondary" size="sm">` + `Badge` count when active. Opens the §4.0 combobox (`w-64`): `CommandItem` rows carrying an 8px `NodeSwatch` in the type's palette fill — **hollow for the eight Provenance & Context types, exactly as the canvas draws them** — the type name, and a `font-mono tabular-nums` count. **BioOKF:** grouped by the seven families under `CommandGroup` headings; the heading is the family **name** and nothing else, because there is no per-family glyph to carry (R-04). **OKF:** flat, sorted by count descending. |
| Predicate | Same shape. Rows are `font-mono` (a predicate is a machine token). A negated predicate is listed **immediately after its positive when the positive is present in the graph, and in its own alphabetical position otherwise** — the draft's rule was undefined for the common case where only the negation occurs. Negations render in `--text-danger` with `line-through` and spell the word out (`not prevents`). |
| Source | Same shape. Rows are the source nodes present (`Publication`, `Study`, `Dataset`), plus a synthetic `No primary source` entry. **Selecting one keeps every edge whose `primary_source` resolves to it, and every node incident to such an edge.** The draft said "nodes and edges whose `primary_source` resolves to it"; a node has no `primary_source` in the §2.1 contract, only an edge does, so the node half is defined by incidence or it is not defined at all. |
| Status | Fixed set, no search, so a plain `DropdownMenu`: `draft`, `stable`, `deprecated`, `stale`, `retracted`. |
| Clear | `<Button variant="ghost" size="sm">Clear filters</Button>`, present only when something is active, preceded by `text-supporting text-text-muted font-mono tabular-nums` reading `Showing 18 of 42`. |

> **Superseded by R-09.** This paragraph read: *"Below `--breakpoint-md` the four facet buttons
> collapse into a single `Filters` button whose menu carries all four as sections. Between 930px and
> 1280px the strip scrolls. They never wrap."* The strip **never scrolls** now — that is the exact
> failure the legend dock already demonstrated, and the control it hides first is `Clear`, the only
> one that undoes invisible state. The shipped ladder is a container query on the *pane*, not a
> viewport media query, in three states: **≥ 1140px full** (all four facets plus the `Showing N of M`
> readout), **860–1139px condensed** (`Type` + **`More (3)`** — `Predicate`, `Source` **and**
> `Status` all fold, and crossing 1140 therefore restores three chips at once), **< 860px compact**
> (a single `Filters (3)` control). ⚠ **The full-row step is 1140, not the 1060 a first measurement
> suggested** (`--knowledge-pane-full-filters` in `main.css`): swept in 10px steps across 720–1760,
> at 1060 the row overflowed its column by 44px and did not clear until 1110. See R-09 in
> [`../knowledge-ui-redesign/redesign-spec.md`](../knowledge-ui-redesign/redesign-spec.md).

**Status is deliberately not a canvas channel.** The canvas already carries three encodings (fill =
type, ring arcs = credibility, dash/dot/taper = negated / synthesized / direction), plus the
two-state solid/hollow split that separates Biomedical Entities from Provenance & Context. It was
four before R-04 deleted the family shape channel; losing one did not create room for another, and
the argument is unchanged. A further encoding is not readable at a 6px node, and the alternatives all
collide: a
tinted fill fights the dim system, a dashed ring is already taken. Status lives in the facet and in
the inspector badge, and a status the user has filtered out simply dims. `retracted` keeps its
existing `!` badge because a retraction is a fact, not a lifecycle state.

### 4.7 Legend

**Slice B.** Collapsed (default) — one horizontally scrolling row:

- Per family: the family name in `text-caps text-text-muted`, then a run of 8px swatches (`gap-1`),
  separated from the next family by `gap-4`. **There is no per-family glyph** — R-04 removed the
  shape channel, so a family is a heading over a run of colours and nothing more. The swatches are
  drawn solid or hollow exactly as the canvas draws the mark (`NodeSwatch`, hollow for the eight
  Provenance & Context types): a key that disagrees with the mark teaches the wrong thing.
- Then a `w-px h-4 bg-border-subtle` separator, then the credibility key: four 10px rings drawn
  exactly as the canvas draws them (§5.5) — four arcs, one arc, dashed, and solid-with-`!` —
  labelled `Well sourced`, `Weakly sourced`, `Not academic`, `Retracted`.

  Four entries, not seven, and that is DR-9b's honesty clause made visible: *"a 1.6px ring reads as
  high-versus-low, not as six distinguishable tiers."* After §5.5 the legend and the canvas now
  agree exactly — the canvas draws four distinguishable ring treatments, not seven. The exact tier is
  in the inspector and in the Source facet.

- Right: a `ChevronUp` `--control-compact` ghost toggle. State persists to `localStorage` under
  `biorouter:knowledge-legend-expanded` inside a try/catch, default `false`.

Expanded — the dock grows to `max-h-[40%]` and becomes the chip grid: per family, a `text-caps`
header carrying the family name, then `flex flex-wrap gap-2` of `<Badge variant="chip"
tone="neutral">` entries, each an 8px swatch plus the type name. `gap-2` (8px), not the draft's
`gap-1.5`: design system §3.3 is "six steps, nothing between them" and P8 is "you spend from them;
you don't invent new values" — the same rule this document uses to reject `gap-2.5` on the trigger.

`variant="chip"` and not `badge` is the primitive's own contract: a chip carries a *category or a
filter*, which is what a node type is, and it is the tier that may be acted on. `tone="neutral"`
because the six tones are semantic (accent/info/success/warning/danger) and a node type is none of
them — the hue belongs on the swatch, never on the chip's fill.

**Every chip is a real button, and that needs a primitive change.** `badge.tsx:41` types the
component as `React.ComponentProps<'span'>` and line 57 renders a `<span>`; there is no `asChild`, no
`as`, no element prop. Wrapping a `Badge` in a raw `<button>` would add to the raw-`<button>` backlog
(DR-24) **and** defeat design system D-15: the global focus rule paints
`background-color: var(--background-focus)` on the focused `<button>`, and `Badge tone="neutral"` is
`bg-background-medium` — an opaque fill that covers the parent's focus surface completely, so a
keyboard user would get no focus indication on any legend chip, facet chip or change-log filter.

**`badge.tsx` gains `asChild`** (Radix `Slot`), so a chip *is* the button and takes the focus fill
directly. This is P4 — "if a surface needs a variant, the variant lives in the primitive" — and it is
one file, in Slice A. The pressed state is **`tint-selected tint-interactive`**, not `tone="accent"`,
because the tints are translucent and the focus fill reads through them; an opaque accent tone would
reintroduce the same problem it was meant to solve. Family headers toggle the whole family. Swatches
carry `aria-hidden` — the name is the accessible label, never the colour alone, and after R-04 the
name is the *only* alternative the legend offers, which is why §5.12's live region and not this
legend is what discharges SC 1.4.1. The current legend is inert, which is the most obvious missing
affordance in the section.

In OKF mode there are no families — every node takes the hashed fallback, and since every node is a
circle in every mode there is no mark difference to lose. The
legend lists the types actually present sorted by count descending, capped at 24 with a `+N more`
that opens the Type facet. Two extra rows appear in both modes when the graph contains them:
`External` (a hollow dashed 8px ring in `--text-muted` at 45%, labelled *Referenced, no page yet*)
and — **BioOKF only, and only when present** — `Unrecognised type` as a `tone="warning"` badge.

### 4.8 Inspector

One rail, two subjects. It replaces `NodePreview.tsx` and adds an edge inspector, which does not
exist today. **Slice B**, except the shell and the two dismissal contracts below, which are Slice A.

Chrome, shared: `rounded-container border border-border-subtle bg-background-default`, an `h-row`
head with `border-b border-border-subtle bg-background-muted`, a scrolling body, and a `border-t`
footer. As a column it carries no shadow; as an overlay it carries `--shadow-popover` plus
`--inset-hairline` and `z-[var(--z-dropdown)]`. **Never `z-10`** — an off-scale z-index is exactly the
class of value that soft-locked the app once already.

Close control: `<Button variant="ghost" shape="round">` — 32×32 with a 16px icon, per the app's one
dismiss geometry for a full panel.

**Two behaviours are ported from `NodePreview.test.tsx`, not re-derived.** The draft replaced
`NodePreview` and mentioned neither, which would have dropped a fixed bug on the floor:

- *"has dialog semantics and dismisses with Escape"* — the panel keeps its dialog semantics and its
  Escape handler.
- *"dismisses when another control is clicked without swallowing that click"* — an outside
  `pointerdown` closes the panel **and the click still reaches its target**. This encodes a real
  regression that was already fixed once.

Both cases move onto the new inspector in the same commit that deletes `NodePreview.tsx`.

#### Frontmatter needs a YAML parser, and it is not on the dependency list

`NodePreview.tsx`'s `splitFrontmatter` returns the raw string and dumps it into a `<pre>`. Rendering
arrays as chips, `sources[]` objects as rows, and "unknown keys with the same treatment, no
allowlist" all require real parsing. `js-yaml@4` and `yaml@2` are on disk **only transitively**
(pulled by `@hey-api/openapi-ts`, `electron-updater`, `eslint`, `knip`, `lint-staged`); neither is in
`package.json`'s `dependencies` or `devDependencies`, so relying on the hoist would break on any
lockfile change.

**Declare `js-yaml@4` as a direct dependency** (MIT; the smaller and better-known of the two) and use
`load`, whose v4 default is the safe schema. It is CSP-clean — no `eval`, no `new Function`, no
`blob:` workers — so `script-src 'self'` and `worker-src 'self'` in `src/main.ts` are satisfied.
§6.1's claim that "this design adds zero semantic tokens" is about *theme tokens* and must not be
read as "this design adds nothing"; this is the one runtime dependency it adds.

#### Node inspector — body order

1. **Identity.** 8px `NodeSwatch` — a circle in the type's palette fill, hollow for Provenance &
   Context, the same mark the canvas paints · `text-subheading`
   `identifier` · sub-line with `<Badge>` `node_type` · `subtype` in `text-supporting
   text-text-muted` · status badge (`draft`→`tone="warning"`, `stable`→ nothing,
   `deprecated`→`tone="neutral"` with `line-through`) · a `tone="warning"` `Stale` badge when
   `stale` is set.
2. **Frontmatter, as labelled rows.** The single biggest inspector fix: today this is a raw `<pre>` of
   YAML. Each row is `grid grid-cols-[96px_minmax(0,1fr)] gap-3 py-2`; key in `text-caps
   text-text-muted`, value in `text-body`. (`py-2`, not the draft's off-scale `py-1.5`. The 96px label
   column is 24 × 4 and is on the grid; it is a fixed column and is stated as one rather than left as
   a bare literal beside a rejected 10px.) Arrays render as `Badge variant="chip"` runs (`synonyms`,
   `tags`, `xref`). An `xref` whose prefix is recognised (`DOI`, `PMID`, `PMCID`, `arXiv`,
   `UniProtKB`, `HGNC`, `MONDO`, `HPO`) renders as a real external link. **Unknown keys render with
   the same treatment** — there is no allowlist, so a frontmatter addition appears as another row
   with no renderer change.
3. **Sources and provenance** — present only when the page carries `sources[]` or `br_credibility`.
   A 10px credibility ring drawn as §5.5 draws it, followed by `<Badge tone="neutral">` naming the
   tier, then `confidence` as `font-mono tabular-nums`, then a `tone="danger"` `Retracted` badge when
   set. Then one row per `sources[]` entry: title, author, `last_modified`, and `resource` rendered
   in `font-mono` with a `--control-compact` "reveal in folder" ghost button pointing into `raw/`.
   **The tier hue never sits behind text** — it is a ring, and the word is app ink on the app ground.
4. **Links out, grouped by predicate.** Group header: the predicate in `font-mono text-caps
   text-text-muted` on a `bg-background-medium rounded-inner px-1.5` pill. A negated predicate's
   header is `--text-danger` with `line-through`. Under it a `.biorouter-list-shell` of rows, each a
   button that selects that edge:

   `[→ or ⇄] · [8px object-type NodeSwatch] · object identifier (truncate) · [ext Badge] · [stat]`

   The stat is one right-aligned `font-mono tabular-nums text-supporting` value — the first of
   `effect_size`, `sensitivity`, `frequency`, `direction`, `unit` present in `quantitative`. One
   number, never a table.
5. **Referenced by (N)** — the mirror of the same row: the *source* node's dot and identifier on the
   left, the predicate moved to the right-hand slot in `font-mono` with its arrow (`treats →`). One
   row shape, two readings. Capped at 10 with a `Show all N` expander.
6. **Document** — `MarkdownContent`, body only, `text-body`.
7. **Footer** — `node.path` in `font-mono text-supporting text-text-muted break-all`, with a
   `--control-compact` reveal-in-folder ghost button.

**These link rows are the inspector's keyboard surface for edges** (§5.12). They are the reason the
canvas does not need per-edge focus.

#### Edge inspector — body order

**Head, headline and `primary_source` are Slice B; everything from the provenance triplet down is
Slice C** and is not built until a real base exercises it.

1. **Head.** `<Badge tone="neutral" uppercase>Edge</Badge>`, or
   `<Badge tone="danger" uppercase>Negative edge</Badge>` when `negated`.
   Sub-line: `directed` / `symmetric` / `synthesized from primary_source`.
2. **Headline — the edge as a sentence.** Three stacked rows at 340px (a single line would truncate
   both endpoints): subject row (8px `NodeSwatch` + identifier, a button that selects that node),
   predicate row (`<Badge variant="chip">` in `font-mono`; `tone="danger"` with `line-through` when
   negated, with the arrow glyph), object row (same as subject).
3. **Provenance triplet** — three labelled rows, not a two-column grid: `knowledge_level`,
   `agent_type`, `primary_source`. `primary_source` is a button that selects that source node.
   For a **synthesized** edge the triplet is replaced by a `--wash-info` note: *"Implicit link
   derived from the cited primary source so the provenance is visible. Author an explicit
   `reported_in` edge to make it first-class."*
4. **Publications** — real external links.
5. **Stats and qualifiers** — every key in `quantitative` and `qualifiers` rendered uniformly as
   `label: value`, with exactly one privileged merge: `ci_lower` + `ci_upper` → a single `95% CI` row.
   Nothing else is special-cased, so a vocabulary addition shows up automatically.

#### Loading and selection behaviour

Render partially, immediately: identity from the graph node (which is already in memory), then
`Loading page…` in the body while the page fetch is in flight, and re-render **only if the selection
is still the same object**. A fast click-through must never paint a stale panel. A failed fetch shows
the error in the body; it does not blank the panel.

### 4.9 Lint — deferred

No lint pill and no findings popover in this pass. §2.3 records the route that would have to exist
first, the three things that must be specified before it can be designed, and why the subject band is
better off with counts alone than with a control that fires an LLM run on view mount.

### 4.10 Change-log drawer

Kept as a right-side `Sheet`. Fixes:

- Width: `sm:max-w-[var(--knowledge-rail-detail)]`, so the drawer and the detail rail are the same
  object at the same width instead of two arbitrary numbers.
- `SheetTitle` keeps `text-subheading` — the `text-label` override goes. The section currently has two
  overlay titles at two sizes, opened from the same screen.
- Kind filters become `<Badge variant="chip" asChild>` toggles wrapping a real `<button
  aria-pressed>`, matching the legend's chips exactly (§4.7) and taking `tint-selected
  tint-interactive` when pressed. Today they are hand-rolled at ~20px with no background in the
  unselected state, so the filter row reads as a run of lowercase words.
- **`ChangeKindChip` stops using status hues for a taxonomy.** `flag` currently renders danger-red and
  `query` renders success-green, so a routine log entry looks like an error. All seven kinds become
  `variant="chip" tone="neutral"` with a 14px leading kind glyph; only `flag` keeps a `danger` tone,
  because a flag genuinely is a problem marker.
- **A `ChangeKindChip.test.tsx` lands in the same commit.** No test file exists for `ChangeKindChip`
  or `ChangeLogDrawer` today, so this fix is currently unguarded in both directions — nothing stops
  the status hues coming back, and nothing would have caught them going in.
- Row actions become `size="sm"` (28px). `size="xs"` is the compact tier and its contract is
  glyph-only — *"a control carrying a label never uses it."*
- `tint-interactive` comes off the non-clickable entry rows (the targets are the two buttons inside
  them) and the row becomes a `.biorouter-list-row` for its hairline only.
- Loading / error / empty all become `EmptyState compact`.

### 4.11 Tier control

Behaviour, copy and the typed-phrase confirmation are unchanged — they are a signed-off privacy
design (issue #56 DR-18) and this pass does not touch them. Two visual corrections only:

- The confirmation dialog's summary block (`KbTierControl.tsx:134-138`) is the section's only pocket
  of pre-system classes: `rounded-lg` → `rounded-element`; `bg-background-muted/40` → `bg-background-muted`
  (an arbitrary alpha on an opaque surface step); `text-sm font-medium` → `text-label`;
  `text-xs` → `text-supporting`.
- The panel itself moves to the top of the Sources rail, above the dropzone, inside a
  `rounded-element border border-border-subtle` block — beside the base it acts on, which is the
  placement DR-18 already argues for.

### 4.12 Empty, loading and error states

Every one is the `EmptyState` primitive. The section currently hand-rolls seven of them as bare
centred sentences, which is what makes it read thinner than its siblings on exactly the screen a new
user sees first.

**`description` is a required `string` on the primitive** (`empty-state.tsx:9`), so every row below
carries one; the draft left three blank, which would not have compiled. **`Filter` and `Inbox` are
not re-exported from `components/icons/app-icons.tsx`** — every other icon named here is — so the two
re-exports land in the same commit.

| # | Where | Icon | Title | Description | Actions |
| --- | --- | --- | --- | --- | --- |
| 1 | Workspace — no bases at all | `ENTITY_ICONS.knowledge` | No knowledge bases yet | Create one and Biorouter will build and maintain notes, sources and links for you. | `Create knowledge base` (default) · `Import .brkb` (secondary) |
| 2 | Workspace — bases exist, none primary | `ENTITY_ICONS.knowledge` | No primary knowledge base | Choose which base this chat reads and writes. | `Choose a base` (default) |
| 3 | Canvas — base primary, zero pages | `Sparkles` | Nothing digested yet | Stage a source in the Sources rail and press Digest. | none |
| 4 | Canvas — graph load failed | `AlertCircle` | Could not load the graph | *the error message* | `Try again` (secondary) |
| 5 | Canvas — filters exclude everything | `Filter` | No pages match these filters | Widen or clear the filters to bring the map back. *(compact)* | `Clear filters` (ghost) |
| 6 | Staged list — nothing staged | `Inbox` | Nothing staged | Drop files above, paste text, or choose a folder. *(compact)* | none |
| 7 | Change log — no history | `History` | No changes yet | Digesting a source records a commit here. *(compact)* | none |
| 8 | Model menu — no models | `Brain` | No models available | Configure a provider in Settings. *(compact)* | `Open settings` (secondary) |
| 9 | KB manager list — search matches nothing | `Search` | No knowledge bases match | Try a different name or id. *(compact)* | none |

Nine, not ten: the lint popover's clean state goes with §4.9.

**Loading.** Two distinct behaviours, and conflating them is the current bug:

- **First load of a base's graph** — a centred `role="status"` block with an `sr-only` label and a
  16px spinner over `text-secondary text-text-muted` `Loading graph`. Cross-faded against the canvas
  as two absolutely-stacked opacity layers, loading out at `--dur-fast` and content in at
  `--dur-med`, never a swap. **Both token names verified present in `main.css`**; the older
  `--motion-*` names are deprecated and the `Progress` primitive's `--dur-med-min` is a different
  token for a different purpose. A token name that does not exist produces a class that never
  generates, which is §10's trap.
- **Refresh of a graph already on screen** — the canvas is **not** blanked. The Refresh button's icon
  spins and the facet strip shows nothing else. Blanking a graph the user is reading, to redraw the
  same graph, is a regression disguised as feedback.

The sources rail and the KB manager use `Skeleton` rows shaped like the rows they will replace, with
staggered negative `animationDelay`, wrapped in `role="status"` with `aria-hidden` children.

---

## 5. The graph visual specification

Every number below is stated. Where a number is measured, the measurement is reported and must be
re-run by the guard rather than trusted from this page.

### 5.1 The ground is resolved, never authored

The graph paints `--background-muted`, which resolves to **`#f4f4f2`** in light and **`#232320`** in
dark in Parchment, Alma Mater *and* Roche Limit. The palette therefore needs a light pair and a dark
pair, not a per-family set. That is what DR-10's "node hues get a light/dark pair" means in practice.

**The two hexes are not written down anywhere in the authoring path.** The draft pinned
`GROUND = { light: '#f4f4f2', dark: '#232320' }` in the solver and `GROUND_FALLBACK = '#f4f4f2'` in
the renderer, while claiming in the same document that the authoring file "holds **no hex values**".
Those statements contradict each other and the literal loses, for two independent reasons:

- A canvas ground is exactly the category CLAUDE.md lists under **"Derived, never authored:
  terminal/code/splash grounds… These are the values that historically drifted."** The generator
  already resolves `terminalGround` and `--background-code` out of the stylesheet; a graph ground is
  the same object with a different consumer.
- `GROUND_FALLBACK` was **a single light value for a dual-mode quantity**, which is the precise shape
  of **DR-61**: the boot splash's `--br-navy` never flipped for dark mode and the BR mark measured
  1.02:1 — invisible — on every dark splash. That record is closed; reopening its shape in a new file
  is not acceptable.

**Therefore:** `scripts/generate-themes.mjs` resolves `--background-muted` out of the stylesheet with
the `buildScopes()` it already computes, passes the resolved pair into the solver, and emits it as
`GraphPalette.ground` — a field §6.1 already declares, so only the *source* was wrong. The renderer's
fallbacks derive from `GRAPH_PALETTE[mode].ground`; `graphStyle.ts` already imports the palette
module, so this costs one expression and restates no hex.

**The six-scope identity assertion.** Nothing enforces the neutral sharing today —
`check-contrast.mjs` audits each family independently, so a diverged neutral would pass every
existing assertion while silently invalidating the palette. Before emitting the shared block the
generator resolves `--background-muted` in all six (family × mode) scopes and **dies** if the three
light values are not identical or the three dark values are not identical:

```text
graph palette is emitted once because the three families share --background-muted;
they no longer do — move GRAPH_PALETTE per-family or re-derive it.
```

That assertion is the entire justification for a shared block. Without it the block is an assumption.

### 5.2 The derivation rule

The rule is normative; the hex tables in §5.3 are its pinned output. If the solver ever produces a
different last bit, the **table is corrected from the measurement**, never the other way round.

Working space is OKLCH — perceptually uniform, so a fixed chroma reads as an equal amount of colour
across hues and a hue spread reads as an equal amount of rotation.

**Step 1 — family anchor hue `H0`, chroma `C`, spread `S`.**

> **A `Shape` column stood here.** It carried one silhouette per family and it is gone with the
> channel (§5.3.1, R-04). `themes/graph.mjs` no longer holds a `shape:` key on any family row, so a
> table with the column would specify a field the authoring file does not have.

| Family | `H0` (OKLCH°) | `C` | Spread `S`(°) | Rungs | Members, in order |
| --- | --- | --- | --- | --- | --- |
| Genomic | 288 | 0.135 | 30 | primary | Gene, Variant, SequenceFeature, Structure |
| Molecular & process | 192 | 0.105 | 34 | primary | Molecule, MolecularClass, BiologicalPathway, BiologicalFunction |
| Anatomy & organism | 148 | 0.115 | 26 | primary | Anatomy, CellType, Organism |
| Clinical | 18 | 0.145 | 34 | primary | Disease, Phenotype, BiomedicalMeasure, MethodOrProcedure |
| Exposome | 78 | 0.120 | 24 | primary | Exposure, SocialFactor, Food |
| Physical | 250 | 0.090 | 26 | primary | Device, MaterialSample |
| Provenance & context | 250 | 0.030 | 190 | **provenance** | Publication, Study, Dataset, Agent, Population, GeographicLocation, Concept, Other |

**Step 2 — hue within the family**, distributed evenly across the spread:

```text
hue_i = H0 + (n === 1 ? 0 : (i / (n - 1) - 0.5) * S)      // i = index in the declared order, n = family size
```

**Step 3 — the contrast rung**, by index within the family. Two ladders:

```text
PRIMARY    = [3.50, 4.50, 5.80, 7.30]                          // families of 2-4 members
PROVENANCE = 3.50 * (12.00 / 3.50) ** (i / 7)                  // 8 members, monotone, geometric
           = [3.50, 4.17, 4.98, 5.93, 7.08, 8.44, 10.06, 12.00]
```

> **The Provenance ladder is the accessibility fix, and it is the only palette parameter that moved.**
> The draft gave Provenance eight members on an *interleaved* ladder — the primary four
> `[3.50, 4.50, 5.80, 7.30]` followed by a secondary four `[4.00, 5.10, 6.50, 8.20]` — at chroma
> 0.030. Under any dichromacy the family's near-zero chroma leaves lightness as the only surviving
> channel, and interleaving put members at neighbouring lightness with no hue left to separate them:
> measured within-family minimum **ΔE00 1.82** (light/protanopia, `Agent`/`Concept`).
> A monotone ladder over a wider range raises that to **3.55** while changing nothing else — same
> chroma, same anchor hue, same spread, same member order. Chroma was tested as the alternative lever
> and rejected on measurement: at C = 0.045 the same interleaved ladder measures 1.34, and at
> C = 0.055 it measures **0.81** — raising chroma makes it *worse*, because the added colour is on
> axes dichromacy removes while the lightness collisions remain. Twenty of the twenty-eight hexes are
> byte-for-byte unchanged.

**Step 4 — solve `L`.** `L` is the OKLab lightness at which the resulting sRGB hex hits the rung's
contrast ratio against the mode's ground. Bisection on `L ∈ [0.05, 0.99]`, 50 iterations; at each
probe the chroma is gamut-mapped (bisection on `C`, 24 iterations, in-gamut tolerance ±1e-4 on linear
RGB) and the result **rounded to 8 bits before the ratio is taken**, so the measured value is the
shipped value.

Reproduce this convention exactly or the strings differ in the last bit: on a **light** ground
contrast falls as `L` rises, so keep the largest `L` whose ratio ≥ target; on a **dark** ground
contrast rises with `L`, and the implementation keeps the largest `L` whose ratio ≤ target — which is
why the dark floor measures 3.48 against a 3.50 nominal rather than 3.50 or above.

The top Provenance rung is reachable in both modes and was chosen to be: maximum attainable contrast
is **19.07** on the light ground (against black) and **15.76** on the dark ground (against white), so
12.00 leaves headroom in both and keeps the highest node below the canvas label ink, which sits at
13.4–15.1:1.

**Why not BioOKF Studio's 28 hexes.** They are hand-picked against one near-white
`radial-gradient(#fff → #eef1ef)`. Nine fall below 3:1 even there; `SequenceFeature` `#AAA6DA`,
`Organism` `#8FCBA6`, `Other` `#AEB2B8` and `EXTERNAL_COL` `#D7DBE1` (≈1.2:1) would be near-invisible
on `#f4f4f2`, and all 28 are unusable on `#232320`. The rule above keeps what is actually recognisable
about that palette — the family hue grouping — and makes contrast a derived property instead of an
accident.

### 5.3 The 28-type palette and the colour-vision audit

Contrast is against the graph's own ground. Measured with WCAG 2.x relative-luminance arithmetic
identical to `scripts/lib/theme-tokens.mjs::luminance` / `contrast`. Rows marked **★** are the eight
that moved when the Provenance ladder was re-solved; every other row is unchanged from the draft and
was independently reproduced by two reviewers.

| Type | hue | C | rung | Light | vs `#f4f4f2` | Dark | vs `#232320` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Gene | 273.0 | 0.135 | 3.50 | `#6a7cd4` | 3.50 | `#5f70c8` | 3.48 |
| Variant | 283.0 | 0.135 | 4.50 | `#6965be` | 4.53 | `#817fdb` | 4.50 |
| SequenceFeature | 293.0 | 0.135 | 5.80 | `#6750a7` | 5.81 | `#a48fec` | 5.79 |
| Structure | 303.0 | 0.135 | 7.30 | `#643d90` | 7.30 | `#c69efb` | 7.27 |
| Molecule | 175.0 | 0.105 | 3.50 | `#1d927a` | 3.50 | `#00866f` | 3.48 |
| MolecularClass | 186.3 | 0.105 | 4.50 | `#007d75` | 4.55 | `#13998f` | 4.49 |
| BiologicalPathway | 197.7 | 0.105 | 5.80 | `#006a6d` | 5.81 | `#2fadb0` | 5.80 |
| BiologicalFunction | 209.0 | 0.105 | 7.30 | `#005963` | 7.31 | `#4cbfd1` | 7.26 |
| Anatomy | 135.0 | 0.115 | 3.50 | `#608d44` | 3.54 | `#558239` | 3.48 |
| CellType | 148.0 | 0.115 | 4.50 | `#367e45` | 4.51 | `#50985d` | 4.49 |
| Organism | 161.0 | 0.115 | 5.80 | `#006d48` | 5.81 | `#4dae81` | 5.77 |
| Disease | 1.0 | 0.145 | 3.50 | `#cb5d82` | 3.51 | `#bf5177` | 3.50 |
| Phenotype | 12.3 | 0.145 | 4.50 | `#ba4a5e` | 4.51 | `#d76476` | 4.48 |
| BiomedicalMeasure | 23.7 | 0.145 | 5.80 | `#a73939` | 5.80 | `#ef7a76` | 5.79 |
| MethodOrProcedure | 35.0 | 0.145 | 7.30 | `#942b0f` | 7.30 | `#ff9379` | 7.28 |
| Exposure | 66.0 | 0.120 | 3.50 | `#b47327` | 3.52 | `#a86817` | 3.49 |
| SocialFactor | 78.0 | 0.120 | 4.50 | `#966700` | 4.50 | `#b18023` | 4.49 |
| Food | 90.0 | 0.120 | 5.80 | `#755c00` | 5.80 | `#ba9938` | 5.78 |
| Device | 237.0 | 0.090 | 3.50 | `#4788b0` | 3.52 | `#3b7da4` | 3.49 |
| MaterialSample | 263.0 | 0.090 | 4.50 | `#546fa5` | 4.55 | `#6d89c0` | 4.49 |
| Publication | 155.0 | 0.030 | 3.50 | `#738679` | 3.52 | `#697b6e` | 3.50 |
| ★ Study | 182.1 | 0.030 | 4.17 | `#617a76` | 4.19 | `#6f8884` | 4.15 |
| ★ Dataset | 209.3 | 0.030 | 4.98 | `#556d72` | 5.00 | `#7c959a` | 4.97 |
| ★ Agent | 236.4 | 0.030 | 5.93 | `#4e606c` | 5.93 | `#8da1af` | 5.89 |
| ★ Population | 263.6 | 0.030 | 7.08 | `#4a5263` | 7.12 | `#a4aec2` | 7.06 |
| ★ GeographicLocation | 290.7 | 0.030 | 8.44 | `#474557` | 8.45 | `#bebbd1` | 8.42 |
| ★ Concept | 317.9 | 0.030 | 10.06 | `#433847` | 10.06 | `#d8cadc` | 10.06 |
| ★ Other | 345.0 | 0.030 | 12.00 | `#3c2b34` | 12.01 | `#f2dae7` | 11.97 |

**The ladder inverts between modes by construction.** In light a higher rung is a darker colour
(`Structure` is the darkest violet); in dark a higher rung is a lighter one (`Structure` is the palest
violet). Same rung index, same relative position within the family, opposite direction — which is what
keeps a family readable in both modes without a second authored table.

#### 5.3.1 The mark: every node is a circle

> **Superseded.** This subsection defined a **seven-silhouette shape channel** — one shape per
> family, an assignment justified by a measured confusability table, and four drawing rules. The
> channel was **removed outright**; see
> [R-04, as amended](../knowledge-ui-redesign/redesign-spec.md). The operator's ruling was that a
> viewer who cannot separate two hues can hover, read the label, or open the inspector, and directed
> that node shape leave the codebase. The confusability table is not reproduced here: it graded 21
> shape pairs against a channel that no longer exists, and a live table would read as binding.

**Every node is a circle, in every format and at every zoom.** There is no family silhouette, no
per-family path generator, and no preference that restores one.

**What replaced it** is §5.12's keyboard model and its `aria-live="polite"` region, which speaks
`<identifier>, <node_type>, <family>` on every focus change (`Multiple sclerosis, Disease, Clinical`),
backed by §5.8's haloed labels. That is a **text** alternative rather than a second visual one,
and it is the broader of the two: a silhouette served a sighted viewer with dichromacy and nobody
else, and only while the mark was large enough to resolve — at a 7px radius, seven silhouettes are a
demanding discrimination even with full colour vision.

**What the canvas still draws structurally**, all of it monochrome-safe and none of it type:

- **Solid disc versus hollow ring.** The 20 **Biomedical Entity** types paint as filled circles; the
  8 **Provenance & Context** types paint as open rings. That is BioOKF's own top-level division — the
  science, versus where the science came from — so the distinction is semantic rather than invented.
  The ring is **1.7**, floored (`ForceGraphCanvas.tsx`: `max(1.7, nodeStrokeWidth * 1.55) /
  globalScale`); the hollow/solid decision itself is `nodeMark.ts`'s `isHollow` / `isHollowType`,
  matched by family **name** against the generated palette rather than by a hardcoded list of eight
  type strings, so a vocabulary change moves it. At the 2.6 a first draft used, the ring ate most of
  a 7px mark and read as a donut. The DOM key is thinned to a **1.5px inset** on a 10px swatch to
  match, because a key that disagrees with the mark teaches the wrong thing.
- **External nodes** — a referenced entity with no page yet, not a type — are hollow with a **dashed**
  ring (§5.6). External is a state, not a type.
- **Retracted** keeps its existing treatment: a solid ring plus the `!` badge (§5.5).
- **Untyped nodes are never hollow**, and neither are OKF nodes on the hashed fallback. In a legacy
  base every page would be hollow, and a marker that applies to every node in the base carries
  nothing — the same argument `hashedFill` makes for not giving OKF nodes a marker at all.

⚠ **The solid/hollow split is a two-state channel, not a replacement for seven.** It says *entity or
provenance*; it does not say which family, and nothing on the canvas does. Type identification is
fill, label, hover, legend and inspector — plus the live region for a non-visual reader.

⚠ **Do not reintroduce shape by the back door.** `nodeShapes.ts`, `GraphShapeGlyph.tsx` and
`graphPreferences.ts` were deleted; `NODE_SHAPES` and all seven `shape:` keys left
`themes/graph.mjs`; `shapeOf` left the palette generator; and `typeShape` / `NodeShape` left
`styles/graphPalette.ts`. `GraphShapeGlyph` was replaced by `NodeSwatch.tsx` rather than simply
removed — it drew the silhouette **and** filled it with the type's palette hex, so deleting it
wholesale would have taken the colour with it and turned the legend, the facet rows and both
inspectors monochrome in the same edit.

#### 5.3.2 Colour-vision audit

The simulation model is **Viénot, Brettel & Mollon (1999)**, applied in linear-light sRGB via the
LMS transform, for `protan`, `deutan` and `tritan`; CIEDE2000 on CIELAB D65 after simulation. Stating
the model is part of the specification — a guard whose simulation is unstated is not reproducible.

**All 378 pairs, minimum ΔE00, before and after the Provenance re-solve:**

| Condition | Draft palette | This palette | Nature of the closest pair |
| --- | --- | --- | --- |
| light / normal | 6.33 | **5.54** | within family (`Concept`/`Other`) |
| light / deuteranopia | 2.34 | 1.26 | cross-family (`MolecularClass`/`Dataset`) |
| light / protanopia | **0.35** | 0.97 | cross-family (`MolecularClass`/`Study`) |
| light / tritanopia | 0.30 | 0.30 | cross-family (`CellType`/`MaterialSample`) |
| dark / normal | 6.95 | **5.65** | within family (`Concept`/`Other`) |
| dark / deuteranopia | 3.22 | 3.27 | cross-family (`BiologicalPathway`/`Agent`) |
| dark / protanopia | 1.49 | 1.49 | cross-family (`Organism`/`MethodOrProcedure`) |
| dark / tritanopia | **0.00** | 0.00 | cross-family (`Phenotype`/`Food`) |

Two things this table says plainly. **Cross-family colour distance under dichromacy cannot be fixed
by any palette** — 28 marks cannot be mutually separated on one surviving opponent axis, and chasing
it is what would have forced the whole palette darker for no gain. **And tritanopia is the worst case
here, not the red-green deficiencies** — 0.30 light and 0.00 dark, a fact the committee's own
analysis did not surface because it tabulated protan and deutan only. Both were originally the
argument for the shape channel rather than against the palette; with that channel removed (R-04)
they are the argument for §5.12's spoken announcement, and they stand as the **accepted cost** the
top of this document states plainly: `Phenotype` and `Food` are one colour under dark tritanopia and
are told apart by label, hover, legend and inspector.

**Within-family minima — the pairs where colour is the sole channel.** This was always true within a
family, because every member shared one silhouette; since R-04 it is true of every pair on the
canvas, and this table is simply the tightest half of that:

| Family | l/norm | l/deut | l/prot | l/trit | d/norm | d/deut | d/prot | d/trit |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Genomic | 6.51 | 5.06 | 5.88 | 8.89 | 6.95 | 5.99 | 5.00 | 8.48 |
| Molecular & process | 6.77 | 5.58 | 6.42 | 5.21 | 8.01 | 7.08 | 8.02 | 5.42 |
| Anatomy & organism | 8.63 | 8.51 | 7.22 | 6.73 | 8.50 | 8.35 | 8.01 | 5.13 |
| Clinical | 9.25 | 7.56 | 9.35 | 5.31 | 8.82 | 6.01 | 8.80 | 4.60 |
| Exposome | 8.98 | 7.28 | 5.61 | 8.23 | 8.84 | 6.14 | 7.46 | 4.67 |
| Physical | 12.17 | 6.59 | 8.96 | 8.22 | 11.53 | 7.74 | 6.49 | 8.31 |
| Provenance & context | 5.54 | **3.55** | 4.10 | 4.31 | 5.65 | 4.67 | 3.83 | 4.77 |

**Within-family floor: ΔE00 3.55** (Provenance, light/deuteranopia, `Population`/`GeographicLocation`).
The draft's floor was **1.82**.

#### 5.3.3 The floor the guard asserts, and why it is what it is

Two floors, because the palette has two kinds of pair:

- **Within-family pairs: ΔE00 ≥ 3.0 under normal trichromacy and under each of the three simulated
  deficiencies, in both modes.** Measured worst case 3.55, so the assertion holds with 0.55 of
  headroom. **This guard is live and must not be removed.** It is the one CVD floor that survived
  R-04: `graphPalette.test.ts` still asserts ≥ 3.0 for every same-family pair × 4 vision types × 2
  modes, and the redesign re-measures the worst case at **4.28** after R-05 lifted the fill band.
- **Normal trichromacy over all 378 pairs: ΔE00 ≥ 5.0**, unchanged from the draft. Measured 5.54 /
  5.65, so this survives the re-solve.
- **Cross-family pairs under simulated deficiency are measured and *reported*, and asserted to no
  floor at all.** Asserting a colour floor there would be a lie: the measured minimum is 0.00 and no
  palette of 28 marks can do better on one axis.

  > **Superseded.** This bullet used to add a *structural* assertion in place of the colour floor —
  > that every cross-family pair differs in `shape`, and that the shape assignment still satisfies
  > §5.3.1's below-3.0 rule — and called it "the one worth having". R-04 deleted the shape channel,
  > so all three shape-coverage tests were deleted with it and **no assertion stands in that slot**.
  > What survives is a report over all 21 family pairs plus a deliberately loose sanity bound
  > (`worst < 3.0`, `worst >= 0`) that catches the palette being rebuilt into something wildly
  > different without pretending cross-family separation exists. ⚠ **Deleting the tests did not
  > delete the measurement**: the collapse is still real, it is still reported on every CI run, and
  > the redundant channel that answers it is text (§5.12), not another visual encoding.

**Why 3.0 and not 5.0 within a family.** ΔE00 ≈ 1.0 is the classic just-noticeable difference under
ideal conditions — large uniform patches, side by side, controlled viewing. None of those hold for a
10px mark on a busy canvas, so a floor of 1.0 would be meaningless. But 5.0 is not *reachable* for an
eight-member family once an opponent axis is gone: lightness is the only channel left, and eight
distinguishable lightness levels between contrast 3.50 and 12.00 give ~3.5 at best without pushing
the extremes into the label ink. 3.0 is set at roughly three times the ideal-condition JND, which is
the point where the difference is reliably suprathreshold for two marks a user is comparing — i.e. it
is a floor that guarantees "not the same colour", which is all this design asks colour to do. It does
not guarantee "identifiable in isolation"; the label, the inspector and §5.12's announcement carry
that — the shape channel used to appear in this list and no longer does (R-04). Say
that plainly in the guard's comment, because a floor whose meaning is overstated is worse than a
lower one.

#### 5.3.4 Measured worst cases

| Measurement | Light | Dark |
| --- | --- | --- |
| Contrast floor on the graph ground `--background-muted` | **3.50** (Gene) | **3.48** (Gene) |
| Contrast floor on `--background-default` | 3.86 (Gene, `#ffffff`) | 3.81 (Gene, `#1b1b19`) |
| Contrast floor on `--background-canvas` | 3.86 (Gene, `#ffffff`) | 4.11 (Gene, `#131312`) |
| Contrast floor on `--background-medium` | 3.26 (Gene, `#ecece9`) | **3.09** (Gene, `#2c2c29`) |
| Contrast floor on `--background-strong` | **2.80** (Gene, `#dcdcd8`) — *fails 3:1* | **2.52** (Gene, `#3a3a36`) — *fails 3:1* |
| Contrast floor on a `tint-selected` row (`.14`) | 2.89–2.93 — *fails 3:1* | 2.55–2.60 — *fails 3:1* |
| Contrast floor on a `tint-selected tint-interactive` row (`.19`) | 2.60–2.66 — *fails 3:1* | **2.15–2.23** — *fails 3:1* |
| Minimum ΔE00 over all 378 pairs, normal trichromacy | **5.54** (Concept / Other) | **5.65** (Concept / Other) |
| Minimum ΔE00 within a family, worst of 4 vision types | **3.55** (Provenance) | 3.83 (Provenance) |

Both graph-ground floors clear **WCAG 2.1 §1.4.11 non-text contrast (3:1)**, which is the correct
criterion for a coloured dot; a node fill is never text, so 4.5:1 is not the bar and must not be
asserted.

> **Correction to a claim in the draft.** It said "every cross-family pair is further apart than every
> within-family pair". That is false and always was: within-family distances run up to **38.93**
> (light) and **40.72** (dark), far above the closest cross-family pair. The true and useful claim is
> the narrower one: **in both modes the globally closest pair is a within-family pair** — 5.54 light
> and 5.65 dark, against cross-family minima of 7.77 and 7.42 (`Variant`/`MaterialSample`) — which is
> what makes the family the unit that colour has to separate, and what makes §5.3.3's two-floor
> structure the right shape.

> **The graph ground is not uniformly the worst case.** On `--background-medium` the light floor
> drops to 3.26 and the dark to 3.09; on `--background-strong` both fail 3:1 outright. The palette is
> legal on the graph ground, on `--background-default` and on `--background-canvas`, and **must not be
> painted on `--background-strong` or on any tinted row without §6.3's swatch ring**. Do not repaint
> the pane without re-measuring.

### 5.4 Arbitrary OKF types — the DR-11 fallback

In OKF mode essentially every node takes this path, so it must look native, not like an error state.
Every node in an OKF base is a **circle** (§5.3.1).

**Hash.** FNV-1a, 32-bit, unsigned — the same hash the renderer already uses for jitter, so there is
one hash in the file:

```ts
function fnv1a(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); }
  return h >>> 0;
}
```

The input is the **raw type string, byte for byte** — no case folding, no trimming. `Gene` and `gene`
are different types to OKF, so they get different colours; folding would make an exact match with a
curated type invisible in the UI.

**Derivation**, given `H = fnv1a(type)`:

```text
hue    = H % 360
chroma = 0.055                                   // FIXED
rung   = [3.90, 4.95, 6.20, 7.90][(H >>> 9) & 3]
L      = solved against the resolved ground for `rung`, by the §5.2 solver
```

`chroma = 0.055` sits deliberately between Provenance (0.030) and every biological family
(0.090–0.145), so an unrecognised type reads as quieter than a curated biological family and more
coloured than provenance — an honest signal, at no cost, that the vocabulary did not recognise it.
The Provenance re-solve did not move this chroma, so the sentence still holds exactly.

Measured over all 1,440 `(hue, rung)` combinations against the palette in §5.3: light contrast floor
**3.90**, closest approach to any of the 28 **ΔE00 3.50** (hue 206, rung 7.90, against
`BiologicalFunction`); dark floor **3.86**, closest approach **ΔE00 5.05** (hue 260, rung 4.95,
against `MaterialSample`). With the naive scheme (chroma 0.075 on the curated rungs) the measured
closest approach was **ΔE00 0.00** — an exact collision at hue 207 / rung 7.30 with
`BiologicalFunction`.

> **One argument in the draft weakened and is restated honestly.** It said the fallback rungs "are
> offset from the curated ladder so a hashed colour can never coincide in lightness with a curated
> one, which puts a floor under `ΔE` from the lightness term alone." That was true against the old
> interleaved Provenance ladder. Against the new one it is not: the fallback's 4.95 sits 0.03 from
> Provenance's 4.98, so two colours *can* now coincide in lightness. The separation there is carried
> by chroma instead — 0.055 against 0.030 — and it is measured, not assumed: the closest a hashed
> colour comes to **any Provenance member** is ΔE00 **5.19** light and **5.06** dark (against `Agent`).
> The dark global closest approach moved from 5.02 to 5.05 for the same reason: it used to be against
> `Population` and is now against `MaterialSample`. The guarantee is the measured floor and the guard
> that re-measures it, never the rung arithmetic.

State plainly in the code comment that ΔE 3.50 is a subtle-but-nonzero difference, not a guarantee of
distinguishability; the guarantee is only that no arbitrary string can exactly reproduce a curated
colour.

**Test vectors** — pin these; they fix both the hash and the solver. Hashes verified independently
twice, and all eight reproduce exactly.

| Type | `H` | hue | rung | Light | Dark |
| --- | --- | --- | --- | --- | --- |
| `ClinicalTrial` | 3701338732 | 172 | 4.95 | `#457264` (4.96) | `#6b998a` (4.91) |
| `Protocol` | 1247275989 | 189 | 4.95 | `#40726e` (4.95) | `#669995` (4.92) |
| `Cohort` | 1729218152 | 272 | 7.90 | `#414a6a` (7.90) | `#abb6dc` (7.85) |
| `Assay` | 1972765544 | 104 | 4.95 | `#6e6b45` (4.95) | `#95916a` (4.90) |
| `Recipe` | 890010351 | 351 | 4.95 | `#855e6f` (4.98) | `#ae8596` (4.95) |
| `Person` | 3278826400 | 40 | 4.95 | `#876053` (4.98) | `#b08779` (4.94) |
| `Meeting` | 3228369114 | 354 | 3.90 | `#976e7e` (3.93) | `#9c7383` (3.90) |
| `Repository` | 3882076341 | 141 | 3.90 | `#668162` (3.91) | `#6b8567` (3.88) |

**Runtime cost.** The solver is a 50-iteration bisection with a 24-iteration inner gamut bisection —
far too slow per frame. Memoise on `` `${type}|${mode}` `` in a module-level `Map`, computed lazily on
first sight of a type. A base has O(10) distinct types, so the map is tiny and the cost is paid once
per session. Do **not** precompute all 360 hues.

**Rejected:** a hollow or donut marker to distinguish "not one of the 28". In OKF mode that marker
would apply to every node in the base, so it carries no information and only costs fill-rate. In
BioOKF mode an off-vocabulary type is already a validation finding and gets its own legend row
(§4.7), which is the right place for that signal.

### 5.5 Credibility on the ring — DR-9b

**Which nodes.** Exactly those with a verdict. The gate is written in `node_type`, not `kind`,
because Stage 2 introduces `node_type` and the draft left the relationship between the two axes for
an implementer to invent:

```text
showsCredibility(n) = n.credibility_tier != null
                   && (n.node_type === 'Publication' || n.node_type === 'Study' || n.node_type === 'Dataset')
```

`kind` (the pre-OKF `PageKind`: `source` / `entity` / `concept` / `hub` / `note` / `flag`) **survives
Stage 2 unchanged and is not used by this specification for anything** — but note that it is not
inert in the app. It is the untyped-node fill (`nodeMark.ts`, which reads `node_type` and falls back
to `kind`) and the noun in an `Untitled {kind}` label (`labelText.ts`), so a change to what
`page_kind_of` returns is visible on any node that declares no `type`. The three source types above
are exactly the members of the Provenance family that §4.6's Source facet lists, so the facet and the
ring now agree by construction. A source with no tier keeps the neutral separation ring — absence of
a verdict is not a verdict.

**Geometry — an orbit ring, not an outline.** Two concentric circles. This used to be a compromise —
a circular annulus around a hexagon still reads as a ring, and the alternative was seven ring path
generators — and since R-04 it is simply what the mark is: a circle inside a circle.

| Stroke | Radius | Width |
| --- | --- | --- |
| Neutral separation ring | on the fill path | from the density ladder (§5.9) |
| Credibility ring | `r + 1.8 / globalScale` | `1.6 / globalScale` |

`1.8` = 1.0px of ground gap + half the 1.6px stroke. **The gap is load-bearing.** Without it the
ring's legibility depends on ring-versus-*fill* contrast, which cannot be guaranteed across 28 fills
× 7 tiers: `gray_lit` `#768290` on `Publication` `#738679` is 1.03:1 — luminance-identical. With the
gap the ring is read against the ground alone, and its contrast is guaranteed ≥ 3.55:1 in both modes.

#### 5.5.1 The tier is an arc count, not a hue

**This is the second accessibility correction, and it replaces the draft's hue-only ring.** The
draft asked a 1.6px stroke to carry seven tiers by hue. Two measurements say it cannot:

- **Angular subtense.** At a 96 CSS-px-per-inch reference, 1.6px subtends 2.91 arcmin at 50cm and
  2.24 arcmin at 65cm; the 1.0px gap is 1.82 / 1.40 arcmin. Reliable *chromatic* judgement needs
  roughly ≥10 arcmin, and small-field tritanopia sets in below ~20 arcmin. A 2–3 arcmin annulus is
  well inside the regime where the visual system reads luminance only — and the four academic tiers
  are precisely the ones that separate in **chroma** (ΔC\* ≈ −10 per step) rather than lightness
  (ΔL\* only 3.4–5.3). The claimed "three-band read… how much blue is left" is a large-patch result.
- **Colour-vision simulation**, over the seven ring hues:

  | Condition | min ΔE00 | Closest pair |
  | --- | --- | --- |
  | light / normal | 5.18 | `peer_reviewed` / `book` |
  | light / deuteranopia | 3.97 | `web` / `retracted` |
  | light / protanopia | 5.79 | `peer_reviewed` / `book` |
  | light / tritanopia | **1.13** | `web` / `personal` |
  | dark / normal | 5.92 | `peer_reviewed` / `book` |
  | dark / deuteranopia | 4.79 | `web` / `retracted` |
  | dark / protanopia | **2.55** | `preprint` / `personal` |
  | dark / tritanopia | 1.63 | `web` / `personal` |

  `web` versus `retracted` at 3.97 under deuteranopia is the consequential one: `retracted` is the
  most important value in the set.

**So the ring encodes count and texture, in the tier hue.** Hue rides along as the fast channel for
trichromats and carries nothing alone:

| Tier | Ring treatment | Legend entry |
| --- | --- | --- |
| `peer_reviewed` | **4 arcs** | Well sourced |
| `book` | **3 arcs** | — |
| `preprint` | **2 arcs** | — |
| `gray_lit` | **1 arc** | Weakly sourced |
| `web` | **fine dashed ring** (8 equal dashes) | Not academic |
| `personal` | **fine dashed ring** (8 equal dashes) | Not academic |
| `retracted` | **continuous ring** + the `!` badge | Retracted |

Arcs start at `−π/2` (top) and are evenly spaced; each arc spans `(2π / N) − gapAngle`, with
`gapAngle` the angle subtending 3 screen px at the ring radius, clamped to `[0.12, 0.5]` radians.

Three things this buys, and one thing it costs:

- Arc count survives 2 arcmin, monochrome, and every simulated deficiency, because counting is not a
  colour judgement.
- The canvas and the legend finally agree. The draft drew seven treatments against a four-entry
  legend, so `web` and `personal` had no legend row at all.
- It states an honesty the draft only implied: **`web` and `personal` are not distinguished from each
  other on the canvas.** They are one category — *not academic* — and their hue difference is a bonus
  for trichromatic vision, not an encoding. The exact tier is in the inspector and the Source facet.
- The cost is that four arcs must be countable at the LOD boundary. At `r * globalScale = 3.5` the
  ring's screen circumference is ≈33px, so four arcs are ~5px with ~3px gaps. That is marginal and it
  is the harness's job to check it at the boundary (§10), not this page's to assert.

**Colours**, by the same derivation, hue and chroma stated so they are regenerable:

| Tier | hue | C | rung | Light | vs `#f4f4f2` | Dark | vs `#232320` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `peer_reviewed` | 250 | 0.120 | 5.80 | `#1c619f` | 5.85 | `#5fa1e4` | 5.78 |
| `book` | 250 | 0.090 | 4.80 | `#406e9d` | 4.85 | `#6291c2` | 4.76 |
| `preprint` | 250 | 0.060 | 4.00 | `#5d7b9a` | 4.00 | `#6583a3` | 4.00 |
| `gray_lit` | 250 | 0.025 | 3.55 | `#768290` | 3.55 | `#6d7986` | 3.55 |
| `web` | 60 | 0.110 | 4.40 | `#a26227` | 4.42 | `#ba783f` | 4.39 |
| `personal` | 345 | 0.100 | 4.40 | `#9f5c83` | 4.40 | `#b67299` | 4.39 |
| `retracted` | 25 | 0.160 | 4.60 | `#c04441` | 4.60 | `#e1625d` | 4.58 |

Measured: ring contrast floor **3.55:1 in both modes** (`gray_lit`); minimum ΔE00 within the ramp
**5.18 light / 5.92 dark** (`peer_reviewed` vs `book`). The four academic tiers share one hue (250)
and fade from saturated blue to neutral grey, which reads as *how much blue is left*; `web` (amber)
and `personal` (rose) step off the ramp because they are a different **kind** of source.

#### 5.5.2 Below the LOD threshold, credibility is suppressed entirely

Draw the orbit ring only when `r * globalScale >= 3.5`; below that the 1px gap collapses into the
anti-aliasing and the two strokes merge into mud.

**Below that threshold the credibility encoding is not drawn at all.** The draft instead recoloured
the neutral separation ring in the credibility hue, calling it a degradation from "ring around the
node" to "coloured outline". Measured over all 7 ring hues × 28 fills:

- light: **112 of 196** combinations below 1.3:1. Worst `gray_lit` `#768290` on `Anatomy` `#608d44` =
  **1.005:1**.
- dark: **112 of 196** below 1.3:1. Worst `peer_reviewed` `#5fa1e4` on `Food` `#ba9938` = **1.000:1**.

Some of those are isoluminant-but-chromatic — `peer_reviewed` on `Food` is ΔE00 54.7 at contrast
1.000 — and an isoluminant chromatic edge at a 1.1px stroke is the single worst case for the visual
system, because the chromatic channels are spatially low-pass while only the luminance channel
resolves that frequency. Others fail both ways: `preprint` `#5d7b9a` on `Population` measures 1.006:1
*and* ΔE00 under 7 — genuinely invisible.

And it is the **default** state, not a rare one. With §5.6's radius clamp `[5.6, 13.4]`, the ring
needs `globalScale ≥ 3.5 / r` — 0.625 at the minimum radius, 0.438 at r = 8, 0.261 at r = 13.4 — and
`zoomToFit` on a large base lands well under 0.625. So on first paint of any substantial graph the
draft's degraded path *was* the encoding.

**A signal that is present but wrong is worse than one that is absent, because the user cannot tell
which regime they are in.** Below the threshold the Source facet and the inspector carry credibility,
which §1 already says they do. No low-zoom substitute is drawn: a broken neutral ring meaning "has a
verdict, tier unstated" was considered and rejected, because at that zoom the neutral ring is itself
being faded out by the density ladder and would compete with it for the same pixels.

**Retracted is a flag, not a tier.** The existing badge is kept: a filled disc at `(x + 0.7r,
y − 0.7r)`, radius `max(3, 0.45r)` in world units, filled `retracted`, with a `!` glyph at
`700 ${0.6 * badgeRadius}px`. Two changes: the glyph fill was `'#fff'`, which is invisible on the
light-mode badge ground — use the **resolved ground**; and the badge is suppressed below
`r * globalScale >= 4` along with the orbit ring. A retracted source also takes the retracted colour
and the continuous ring, overriding its tier — retraction is the more important fact.

**The ring hue never sits behind text.** In the legend it is a 10px ring with a transparent centre; in
the inspector it is a 10px ring beside a `tone="neutral"` badge carrying the tier word. Nothing
anywhere fills a surface with a ring hue.

### 5.6 Node geometry

Radii and centrality replace the fixed 4.5 / 7.5 pair and `HUB_TOP_N = 6`.

```text
deg(n)       = incident edge count, floored at n.degree when the server supplies it (§2.1)
max          = largest degree (min 1)
p75          = degrees sorted ascending, value at index floor((count - 1) * 0.75)
pivot        = max(2, min(max(3, p75), sqrt(max) * 1.6))
hubThreshold = max(3, degrees[floor((count - 1) * 0.82)] || 3)

centrality   = max > 0 ? log1p(deg) / log1p(max) : 0
radius       = external ? clamp(4.5 + 1.4 * centrality, 4.5, 6.2)
                        : clamp(5.4 + 7.6 * (1 - exp(-deg / pivot)), 5.6, 13.4)
hub          = !external && deg >= hubThreshold
```

`radius` is the node's **radius** — every node is a circle (§5.3.1), so there is no circumradius/
inradius distinction left to make and force-graph's shadow-canvas hit circle is the mark exactly
rather than a generous approximation of it. Non-finite radius falls back to
`external ? 5 : hub ? 10 : 6`. All radii are **world** units and scale with zoom.

**Why the percentile replaces top-N.** Top-N is size-blind: on a 12-node base it makes half the graph
a hub; on a 2,000-node base it makes six. The 82nd percentile is proportional by construction. The
same objection applies to the fixed 4.5 / 7.5 pair, which encodes no centrality at all — the most
connected page in a base looks identical to a leaf.

Setting this radius also fixes the LOD calibration for free: with base `r ≈ 5.6–6.0` world units, the
world scale matches the renderer these constants come from, so every threshold in §5.9 transfers to
`globalScale` 1:1 with no recalibration. Keep `nodeRelSize={5.6}` so force-graph's shadow-canvas hit
circles track the fills.

**Focus glow** (focused node only), painted **before** the fill:

```text
createRadialGradient(x, y, r, x, y, r + 13 / globalScale)
  stop 0.00 → fill at alpha 0.34
  stop 0.55 → fill at alpha 0.14
  stop 1.00 → fill at alpha 0.00
filled over arc(x, y, r + 13 / globalScale)
```

**Neutral ring** (every node not showing an orbit ring), drawn on the fill's own circular path:

```text
sw = isFocus ? max(1.1, densityStrokeWidth) / globalScale : densityStrokeWidth / globalScale
sa = isFocus ? 0.92 : 0.92 * densityStrokeAlpha
colour = resolved ink at sa
drawn only if sw > 0.05 / globalScale && sa > 0.03 && r * globalScale > 1.2
```

**Node alpha** (`globalAlpha` around the whole node paint):

| Condition | Alpha |
| --- | --- |
| Search or facet active, node fails | 0.12 |
| Focus active, node is neither focus nor neighbour | 0.26 |
| Outside `visibleSet` (preview-at-SHA) | 0.26 |
| Otherwise | 1.0 |

Two dim levels replace `DIMMED_OPACITY = 0.22`, because "not related to what I am looking at" and
"does not match my filter" are different states and one constant cannot say both.

**External nodes** — a referenced entity with no page yet, *not* a type — are a hollow **dashed**
circle: fill = resolved ground, ring = resolved ink at 0.45, width `1.0 / globalScale`, dash
`[2.5, 2] / globalScale`. No orbit ring, no glow, never labelled below priority 4, and **no palette
entry**. This is the deviation from `EXTERNAL_COL = '#D7DBE1'`, a fill that measures ≈1.2:1 on the
light ground and is simply not visible. A hollow dashed marker says *placeholder* better than a pale
fill does, and cannot fail contrast because it is drawn in ink.

### 5.7 Edge rendering

`linkCanvasObject` receives a ctx already transformed to world space, so **every constant expressed in
screen pixels is divided by `globalScale`**. Radii and world offsets are not.

**Emphasis and dim**, per edge per frame:

```text
emph = 2 if this is the selected-or-hovered edge
     | 1 if a node is focused and this edge touches it
     | 0
dim  = a node is focused and this edge does not touch it
     | another edge is focused
     | a search/facet is active and either endpoint fails   (which also forces emph = 0)
```

**Alpha composition** — an emphasised edge never fades out under density:

```text
restore    = emph === 2 ? 0.92 : emph === 1 ? 0.72 : 0
mul        = max(densityEdgeAlpha, restore)
finalAlpha = baseAlpha * mul
```

**Geometry:**

```text
ux, uy            = unit vector source → target
rawPx, rawPy      = (-uy, ux)
canonicalForward  = String(source.id) <= String(target.id)
px, py            = canonicalForward ? (rawPx, rawPy) : (-rawPx, -rawPy)
trim              = r + 1.5 / globalScale at each end;  len = max(1, len - trims)
laneKey           = min(a,b) + '\0' + max(a,b)
lane_i            = count > 1 ? i - (count - 1) / 2 : 0
bend  (screen px) = max(12, min(44, 14 + globalScale * 6))
cx, cy            = midpoint + (px, py) * lane * bend / globalScale;  curved iff |lane| > 0.001
```

Canonicalising the perpendicular by id makes both directions of a pair bend the same way. `bend` is
screen-**constant**, so parallel edges stay legibly apart at every zoom. Set force-graph's own
`linkCurvature` accessor to `lane * (bend / globalScale) / len` as well, or the shadow canvas
hit-tests the straight chord and hovering a multi-edge picks the wrong one.

**Five render cases, in this order:**

| # | Case | Treatment |
| --- | --- | --- |
| 1 | **Synthesized** (`edge.synthesized`) | `setLineDash([1,4]/gs)` — a **dotted** pattern, `lineWidth 0.8 * densityEdgeWidth / gs`, resolved ink at 0.13 (0.05 when dim). No taper. Painted first, returns early. |
| 2 | **Negative** (`edge.negated`) | `setLineDash([4,3]/gs)` — a **dashed** pattern, `lineWidth 1.1 * densityEdgeWidth / gs`, **resolved `--text-danger`** at 0.10 / 0.34 / 0.46 / 0.62 (dim / base / emph1 / emph2). A dashed stroke, never a taper: the dash *is* the negation signal. |
| 3 | **Symmetric** (non-negative) | Plain stroke, no taper, `lineWidth 0.9 * densityEdgeWidth / gs`. A symmetric relation has no direction to encode. |
| 4 | **Curved** (non-negative, `|lane| > 0`) | Plain quadratic stroke, `lineWidth (emph===2 ? 1.35 : emph===1 ? 1.05 : 0.85) * densityEdgeWidth / gs`. |
| 5 | **Default — the tapered quad** | A filled quadrilateral, not a stroke. `w0 = 0.85`, `w1 = 0.42` screen-px **half**-widths, `wm = densityEdgeWidth / gs`, with **both half-widths floored at 0.5 screen px** before the path is built. Path: `(sx+px·w0·wm, sy+py·w0·wm) → (ex+px·w1·wm, ey+py·w1·wm) → (ex−px·w1·wm, ey−py·w1·wm) → (sx−px·w0·wm, sy−py·w0·wm) → close → fill`. |

**Dotted for synthesized, dashed for negative — not two dashes at two alphas.** The draft gave both
cases a dash (`[2,3]` at 0.8 × width and `[4,3]` at 1.1 × width) and left the reliable separator as
colour and alpha: resolved ink at 0.13 versus resolved danger at 0.34. That is a colour-alone
distinction between "the system inferred this" and "this claim is negated", which are very different
meanings. At full density fade the two strokes are 0.53px and 0.73px with dash periods of 5 and 7
screen px — indistinguishable as geometry. A `[1,4]` dot pattern reads as a different *texture* from a
`[4,3]` dash at 1px where an alpha difference does not.

**The taper's thin end is floored.** `w1 = 0.42` paints 0.84px at the object end, and §5.9's
`edgeWidth = 1 − 0.34 × fade` takes that to **0.55px** at full fade — sub-pixel, so antialiasing sets
the apparent width by coverage rather than geometry and the 2:1 taper ratio that carries direction
compresses toward 1:1 exactly when the graph is dense enough to need it. Clamping both half-widths at
0.5 screen px keeps the painted ratio at least 1.7:1 at every density.

**Base edge colour** for cases 1, 3, 4 and 5 is the **resolved ink** at:

| dim | base | emph 1 | emph 2 |
| --- | --- | --- | --- |
| 0.07 | 0.18 | 0.32 | 0.46 |

This is the largest DR-10 correction in the file. Today the file hardcodes
`rgba(119, 128, 145, 0.42)` for a resting edge and `rgba(99, 141, 104, 0.75)` for a focused one — a
cool blue-grey against the app's warm neutrals, and a **green** focus state in an app whose
interaction accent is coral. Resolving the ink means the identical alpha ladder gives a dark web on
light and a light web on dark, automatically, in every family.

**Edge labels** — drawn only at `emph === 2`, and only when the trimmed length exceeds 26 screen px:

- position: the curve point at `t = 0.5` (quadratic Bézier if curved, midpoint if straight)
- `font = 500 ${11 / globalScale}px ${resolvedMonoFamily}`, `textAlign center`, `textBaseline middle`
- halo: `shadowColor` = resolved ground at 0.95, `shadowBlur = 4 / globalScale`, and `fillText` called
  **twice** at the same point — the double call doubles the shadow density and is the mechanism, not
  a bug
- fill: resolved `--text-danger` for negatives, resolved ink otherwise
- text: negatives spell the word out — `not_prevents` → `not prevents`
- strike-through on negatives: measure the text, then stroke a `1 / globalScale` line in the danger
  ink from `(mx − tw/2, my)` to `(mx + tw/2, my)`. The dash carries the negation on the wire; the
  strike carries it on the word.

> The word-level redundancy exists only for the hovered or selected edge, because that is the only
> edge that carries a label. The dash is the channel that is always present; the strike-through is
> confirmation once the user has committed attention to one edge.

**Edges become selectable.** Wire `onLinkClick` and `onLinkHover`; a click opens the edge inspector
(§4.8). This is new — an edge is decoration today. **Hit tolerance is ≥ 8 screen px perpendicular to
the chord, independent of the painted width**, because the painted width runs from 1.70px down to
1.0px and a 1px pointer target is not a target.

### 5.8 Labels

**The pass moves.** Labels cannot live in `nodeCanvasObject`: force-graph paints nodes one at a time,
so a later node overdraws an earlier node's label and collision avoidance has no global view. Draw
them in `onRenderFramePost(ctx, globalScale)`, after every node and edge is down. This is a required
restructuring of `ForceGraphCanvas.tsx`.

**Priority ladder:**

```text
search/facet active: passes ? pr = 2 : skip entirely
otherwise:  focused → 5 · hovered → 4 · hub → 3 · neighbour-of-focus → 2 · globalScale >= 1.55 → 1 · else skip
label alpha: 0.3 when a node is focused and this node is neither the focus nor a neighbour, else 1.0
```

Sort candidates by priority **descending**, then place greedily. The sort is the whole point: a hub's
label beats a leaf's for the same square of canvas.

> **The label is not the type channel, and this ladder used to be the argument for the shape
> channel.** ⚠ **The measurement stands; the conclusion is superseded** — R-04 deleted the shape
> channel, so at fit scale the unlabelled majority now carries *colour and nothing else*, which is
> precisely the residual cost §1 records rather than a gap this section can close. `pr = 1`
> fires only at `globalScale ≥ 1.55`. Hubs are the top ~18% by degree and `zoomToFit` caps at
> 1.75/2.15/2.55/3.1 but takes the *computed* fit, which for a few-hundred-node graph is far below
> 1.55. So at the view's default state roughly **82% of nodes carry no label**, and for those nodes
> the mark itself is the only carrier of type. Lowering the `pr = 1` threshold so everything is
> labelled at fit scale was considered and rejected: it would put several hundred labels on a canvas
> where the greedy AABB pass would drop most of them anyway, which is a worse failure than an
> unlabelled mark because it is *unpredictable* which ones survive.

**Collision avoidance — greedy AABB, first-come-first-served:**

```text
lx = n.x + r + 6 / globalScale ;  ly = n.y            // left-aligned, vertically centred, right of the node
rect = { x: lx - 1/gs, y: ly - (fs/2 + 1)/gs, w: (tw + 2)/gs, h: (fs + 2)/gs }
if rect overlaps ANY already-placed rect → skip this label entirely (do not nudge, do not shrink)
else push the rect and paint
```

Do the test in world space with the screen-constant terms divided by `globalScale`. That is exactly
equivalent to testing in screen space — the canvas transform is a uniform scale plus a translation and
AABB overlap is invariant under both — and it avoids needing the pan offset, which the callback does
not hand you.

**Font:** `${pr >= 4 || n.hub ? 600 : 450} ${fs / globalScale}px ${resolvedFontFamily}`, with
`fs = n.hub ? 12 : 11.5` screen px, `textBaseline: 'middle'`, `textAlign: 'left'`. Weight 450 is
deliberate and not a typo for 400.

> **Note.** 11.5 and 12.5 are fractional sizes off a type scale whose floor is 11px. They are
> tolerated **only** on the canvas, which is a pixel surface with its own optical requirements and is
> already outside the `--text-*` role system; nothing in the DOM may use them. The DOM label sizes in
> this section are `text-supporting` (12) and `text-caps` (11) and nothing smaller.

**Truncation** is by character count, not width: `text.length > 32 ? text.slice(0, 31) + '…' : text`,
applied to `prettyLabel(n.label, n.kind)`.

**Memoisation — the DR-9 fix, in two halves.**

1. `prettyLabel` + truncation are pure functions of `(label, kind)`. Compute once per node into a
   `Map<nodeId, string>`, rebuilt only when `graph` changes. Zero regex per frame.
2. Width still needs `measureText` for the AABB, and the font size changes at every zoom step.
   Exploit linearity: measure each unique `(displayText, weight)` **once** at a fixed 100px font,
   cache `w100`, and compute `tw = w100 * fs / 100`. Advance widths scale linearly with font size, so
   this is exact to within sub-pixel hinting — irrelevant for a collision box. The result is one
   `measureText` per unique label per weight for the lifetime of the graph, instead of one per *word*
   per labelled node per frame.

**Halo:** resolved ground at 0.95, `shadowBlur = 4 / globalScale`, `fillText` twice.
**Fill:** the resolved ink. The `#1c2128` this pattern comes from is a near-black that is correct in
light and invisible in every dark theme — the exact bug `graphStyle.ts` was written to prevent.

**Single line, not three.** `wrapLabel()` is deleted. Three reasons in order: a wrapped label has no
single AABB, so priority-ranked collision avoidance cannot work against it; it is the per-word
`measureText` loop plus a per-character shrink loop that DR-9 identifies as the real per-frame cost;
and at 28 node types the graph is read as a map, and a map wants short consistent labels — the full
title is one click away in the inspector. `prettyLabel` stays (it rescues UUID and hash filenames and
earns its place); `wrapLabel` and its three tests in `labelText.test.ts` go with it.

### 5.9 Density, level of detail, and culling

```text
edgeDensity     = clamp01((visibleEdges - 260) / 2200)
nodeDensity     = clamp01((visibleNodes - 220) / 1200)
zoomCrowd       = clamp01((0.72 - globalScale) / 0.62)
fade            = clamp01(max(edgeDensity, nodeDensity) * zoomCrowd)
outlineFade     = clamp01(max(nodeDensity, edgeDensity * 0.75) * zoomCrowd)

edgeAlpha       = 1 - 0.82 * fade
edgeWidth       = 1 - 0.34 * fade
nodeStrokeAlpha = 1 - outlineFade
nodeStrokeWidth = outlineFade >= 0.72 ? 0 : 1.1 * (1 - outlineFade / 0.72)
```

Read them as: nothing fades until you are zoomed out past `globalScale 0.72` **and** the viewport
holds more than 220 nodes or 260 edges; at `globalScale ≤ 0.10` the crowding term is fully on. Node
outlines vanish entirely at `outlineFade ≥ 0.72`, where the outlines would be more ink than the nodes.

> **Do not add a "suppress labels when dense" guard.** The bottom label rung fires at
> `globalScale >= 1.55`, and `zoomCrowd` is 0 for any `globalScale >= 0.72`, so `fade` is provably 0
> whenever every node is labelled. Density fade and full labelling are mutually exclusive by
> construction, and the rule would be unreachable.

**Culling is an architectural change, not an optimisation.** `force-graph@1.51.4` does no viewport
culling: `paintNodes` iterates `graphData.nodes.filter(nodeVisibility)` and `paintLinks` iterates
`graphData.links.filter(linkVisibility)`, both over the full arrays, every frame. So:

1. `onRenderFramePre(ctx, globalScale)` — verified to run **before** `tickFrame()` → `paintLinks()` →
   `paintNodes()` — computes the visible world rect from `screen2GraphCoords` on the four canvas
   corners plus 80 screen px of padding, sweeps the node and edge arrays to count what is inside,
   computes the density style, stashes both in a ref, and paints the grid.
2. `nodeVisibility` / `linkVisibility` accessors read that ref and return the in-rect test. Both props
   are in force-graph's `bindBoth` list, so they apply to the shadow (hit-test) canvas as well.
3. Every painter reads the density style from the ref. It is exactly one frame fresh, by construction.

An edge is visible if **either** endpoint is in the rect. Culling is skipped entirely while a node or
edge is focused, so a focused relation is never culled out from under the user.

> **Hit-test culling is eventually consistent, and that is not a bug to file.** force-graph wraps
> `refreshShadowCanvas` in `throttle(…, HOVER_CANVAS_THROTTLE_DELAY)`, so for up to one throttle
> window after a pan a just-culled node is still hit-testable and a just-revealed one is not. It is
> harmless and it is written down here so that it is not re-diagnosed as a culling defect.

> **Do not add `nodePointerAreaPaint`.** DR-9 records the measurement: `nodeCanvasObject`,
> `nodeCanvasObjectMode` and `linkCanvasObject` are all in `bindFG` (visible canvas only), and the
> shadow graph is built with `.nodeColor('__indexColor')` and no canvas object, so it paints plain
> arcs. There is no duplicate label pass to remove, and adding a pointer-area painter would
> *introduce* a second per-node painter where none exists.

**Grid.** Dots on a 34-world-unit lattice, radius `0.8 / globalScale`, **resolved ink** at alpha
0.045, skipped entirely when `34 * globalScale < 11` (i.e. `globalScale < 0.3235`). Drawn in
`onRenderFramePre`, iterating only the culled world rect. In dark mode this is a faint *light* dot
field on a dark ground, which is right and which a hardcoded `rgba(20, 24, 31, 0.045)` could never
produce.

### 5.10 Layout

Parameters band by node count: **small ≤ 350 · mid 351–900 · large 901–1500 · huge > 1500**.

| Parameter | small | mid | large | huge |
| --- | --- | --- | --- | --- |
| `forceLink` base distance `L` | 92 | 78 | 82 | 96 |
| `forceLink` base strength | 0.22 | 0.20 | 0.16 | 0.12 |
| `forceManyBody` strength | −135 | −155 | −175 | −200 |
| `forceManyBody` `distanceMax` | 260 | 300 | 340 | 380 |
| `forceCollide` radius pad | +6 | +6 | +5 | +4 |
| `forceCollide` iterations | 2 | 2 | 1 | 1 |
| `forceCollide` strength | 0.64 | 0.64 | 0.62 | 0.62 |
| component pull strength | 0.006 | 0.007 | 0.0035 | 0.0022 |
| anchor pull strength | 0.024 | 0.020 | 0.0075 | 0.0045 |
| `d3VelocityDecay` | 0.18 | 0.20 | 0.24 | 0.28 |
| `d3AlphaDecay` | 0.060 | 0.090 | 0.100 | 0.090 |
| `cooldownTicks` | 240 | 140 | 124 | 36 |
| `cooldownTime` (ms) | 4200 | 2400 | 2600 | 900 |
| `maxVelocity` (world/tick) | 30 | 18 | 15 | 18 |
| warm ticks before first paint | 10 | 2 | 0 | 0 |

The `L` row is non-monotonic on purpose: mid-sized graphs pull tighter because they are dense enough
to sprawl, and huge ones need room back. Copy it; do not "fix" it.

**Per-link accessors, not scalars:**

```text
distance(l) = L * (l.source.hub || l.target.hub ? 1.18 : 1) + (r(l.source) + r(l.target)) * 1.8
strength(l) = baseStrength / sqrt(max(1, (deg(l.source) + deg(l.target)) * 0.5))
```

A hub earns 18% more room so its spokes do not crush it. The degree weighting **replaces** d3's
default `1 / min(deg)`; without it a hub's many links out-pull everything and the graph collapses to a
star.

**Three mechanisms stop a disconnected node flying off, and the first two already exist:**

1. `charge.distanceMax` — already set to 260 with a correct comment. Keep it and scale it by band.
2. `forceX(0)/forceY(0).strength(0.07)` — also already present. These are **subsumed by, not replaced
   by**, the component pull below: an isolated node is its own component, so it gets an anchor. But
   that is only true when component anchors exist. Register the component pull as forces named
   `cx`/`cy` and **keep `x`/`y` registered at 0.07 as the fallback path**, active only when the
   component layout was skipped (restored cached positions, or a graph handed in with pre-set
   coordinates). Deleting them unconditionally looks safe and fails exactly on the reload path.
3. `containNode` — a **hard post-tick clamp**, and the strongest of the three. In `onEngineTick`, for
   every node: let `c` be its component anchor, `d = hypot(n.x − c.x, n.y − c.y)`,
   `lim = max(220, min(4200, c.r * 1.10 + 120))`; if `d > lim`, snap the node onto the limit circle
   and multiply `vx, vy` by 0.25. Also clamp speed to the band's `maxVelocity`. A non-finite `x` or
   `y` is re-seeded at the anchor plus a 6–8 unit jitter from `jitterUnit(id)`. The forces are a
   tendency; this is a guarantee.

**Seeded positions — Slice C.** The force-parameter table above and `containNode` are the value and
land in Slice A. The initialiser below is ~300 lines buying "the first paint already looks like a
graph instead of a hairball relaxing for two seconds", and it is genuinely severable; ship it when the
rest is stable. force-graph honours pre-set `x`/`y` on the node objects, so it ports as an initialiser
with no new machinery:

```text
components by BFS over the undirected adjacency, sorted by size DESCENDING
denseScale = n > 1500 ? 1.85 : n > 900 ? 1.52 : n > 350 ? 1.22 : 1
root       = max(140, sqrt(n) * 18 * denseScale)
component 0 at the origin, radius max(240, sqrt(|c|) * 28 * denseScale)
the rest on concentric rings: a new ring starts once used >= max(8, ring * 8); slots = max(8, ring * 8)
  angle = (used / slots) * 2π + ring * 0.37
  dist  = max(root, mainR + r + (|c| > 18 ? 112 : 64)) + ring * 64
inside a component:
  hubCount  = min(max(1, ceil(sqrt(|c|) / 2)), 18/14/10/8/5/3 by size band 1200/700/70/32/14)
  threshold = max(2, min(maxDeg, ceil(maxDeg * (|c| > 900 ? 0.12 : 0.34))))
  minHubs   = 12/9/6/1 by 1200/700/250
  hubs on a ring of radius min(c.r * 0.66, 64 + hubs.length * 26 + sqrt(|c|) * 13 * denseScale)
       at angle (i / hubs) * 2π + ci * 0.31
  every other node joins the hub maximising  direct*4 + shared*0.35 + deg(hub)*0.025 − hubIndex*0.01
       (direct = 4 if adjacent to that hub; shared = count of common neighbours)
  members on a golden-angle spiral around their hub:
       a      = i * 2.39996323 + (deg(hub) % 11) * 0.17
       localR = max(52, sqrt(m) * 22 * denseScale + 34)
       rr     = localR * (0.62 + 0.82 * sqrt((i + 0.5) / m)) + (deg ? 0 : 24)
each node's seeded position is also its anchorX/anchorY; its component's centre is its cx/cy
```

**Fit.** After the cooldown, `zoomToFit(500, pad)` with `pad = max(56, min(132, min(W, H) * 0.085))`
and the scale capped at `n > 1500 ? 1.75 : n > 700 ? 2.15 : n > 250 ? 2.55 : 3.1`. Zoom limits:
`min = max(0.0005, min(0.02, fitScale * 0.08))`, `max = 32`.

### 5.11 Resolving structural colour — DR-10

A canvas cannot parse `var(--…)` in `ctx.font` or `ctx.fillStyle`: those strings are parsed against the
canvas, not the cascade, so the assignment is silently dropped and the previous value stays. Reading
the custom property by name does not help either — `getPropertyValue('--text-default')` returns the
*declared* value, which in the dark blocks is itself `var(--color-neutral-100)`. **Only the used value
is safe.**

Extend the existing `useCanvasTheme(ref)` hook — do **not** add a second, parallel piece of theme
plumbing, which is exactly how the ink came to be hardcoded the first time. One hook, one
`MutationObserver` on `document.documentElement` filtered to `['class', 'data-theme']`, one state
object.

**All seven resolved fields go through one function, and that is a requirement, not a style note.**

```ts
function resolveComputed(el: Element | null | undefined,
                         read: (s: CSSStyleDeclaration) => string,
                         fallback: string): string {
  if (!el || typeof window === 'undefined' || typeof window.getComputedStyle !== 'function') return fallback;
  const v = read(window.getComputedStyle(el));
  return v && v.trim().length > 0 && !v.includes('var(') ? v : fallback;
}
```

Two guards, written once: **reject an unresolved custom property**, and **reject the empty string**.
`graphStyle.test.ts` carries four cases that exist specifically to stop the canvas-`var()` bug
(`resolveCanvasFontFamily :: never hands a canvas an unresolved custom property`, `… falls back rather
than emitting an empty family`, and the same pair for `resolveCanvasInk`), and the draft added five
new resolved fields while mentioning only two new fallback constants — reintroducing the exact defect
those cases exist to prevent, in five new places, in a file that has already been burned by it.

The evidence that copy-paste is the wrong shape is already in the file: **the two shipped resolvers do
not agree.** `resolveCanvasInk` (`graphStyle.ts:84`) rejects `var(`; `resolveCanvasFontFamily`
(`graphStyle.ts:57`) does not. Both are covered by a test named "never hands a canvas an unresolved
custom property", and one of them is not actually guarded — the case passes because a computed
`fontFamily` happens never to be a `var()`, not because the code prevents it. Two resolvers have
already diverged; seven would diverge further.

| Field | Resolved from |
| --- | --- |
| `fontFamily` | `getComputedStyle(container).fontFamily` |
| `monoFamily` | `getComputedStyle(probeMono).fontFamily` — a 0×0 `<span className="font-mono" aria-hidden>` |
| `ink` | `getComputedStyle(container).color` |
| `ground` | `getComputedStyle(container).backgroundColor` — **requires deleting the inline gradient** (§4.5) |
| `danger` | `getComputedStyle(probeDanger).color` — a 0×0 `<span className="text-text-danger" aria-hidden>` |
| `muted` | `getComputedStyle(probeMuted).color` |
| `border` | `getComputedStyle(probeBorder).borderTopColor` |
| `mode` | `useResolvedTheme()` — selects `GRAPH_PALETTE.light` vs `.dark`; it falls back to `light` outside a provider instead of throwing like `useTheme()` |

The four probe spans are the cost of a correct dark mode. **The test is one parameterised table over
all seven fields**, not five copies of two cases and two fields left unguarded.

#### The fallbacks

`CANVAS_FONT_FALLBACK` and `CANVAS_INK_FALLBACK` stay as they are. Two more are needed and neither is
a new literal:

- **`ground`** falls back to `GRAPH_PALETTE[mode].ground` — the value §5.1 makes the generator resolve
  and emit. `graphStyle.ts` already imports the palette module. No hex is restated, and the fallback
  is per-mode, which is what DR-61 is about.
- **`danger` has no constant.** The draft proposed `DANGER_FALLBACK = '#c4232b'`. That hex is **Roche
  Limit's** light `--text-danger` (`main.css:1007`); Parchment's is `#b3261e` (`main.css:562` →
  `--color-red-600` at `:235`) and Alma Mater's is `#c40d3e` (`main.css:851`), and the dark values
  are a different family again (`#f07575`-class, `#f5768a`, `#ff9592`). The theme-system architecture
  is explicit that **"the status hues stay per family"** — `background-danger` / `success` / `info` /
  `warning` and their `text-` and `border-` twins are neither neutral scaffolding nor the family
  accent — so a single light hex from a non-default family, baked into a shared path that §5.7's
  negative-edge stroke and §5.8's strike-through both draw in, is a per-family value in a shared
  place.

  **Drop the constant and let the resolve fail loudly.** If `probeDanger` does not resolve, the
  negative-edge case throws in development and the harness catches it, rather than silently painting
  Roche's red in Parchment. §10's own verification table already says jsdom "returns nothing, so the
  fallbacks pass the test and the real bug ships" — that argument applies to this constant most of
  all. If a fallback is judged necessary later, it takes the **base** `:root` value the cascade itself
  falls back to (`--color-red-600` `#b3261e`) **with an authored dark twin beside it**, never one
  family's value alone.

`withAlpha()` already parses `rgb()/rgba()` and hex and returns anything unrecognised **unchanged**
rather than mangling it into an invalid colour a canvas would silently ignore. Every alpha in §5.5,
§5.6, §5.7 and §5.9 goes through it.

**What stays a literal, and why that is not a contradiction.** The 28 type fills and the 7 credibility
hues. They are not tokens, they have no CSS consumer, and they are contrast-audited and CVD-audited
against a ground §5.1 proves is constant *and now resolves rather than restates*. Everything drawn in
the app's *own* colours — glyphs, outlines, the edge web, the halo, the grid, the danger red, the
ground — resolves. That is exactly the line DR-10 draws.

### 5.12 Keyboard model for the canvas

**This is built.** The pure half is `graph/graphKeyboard.ts` (`nextNodeInDirection`, `tabOrder`,
`stepThrough`, `announce`, `highestDegree`), covered by 18 tests in `graphKeyboard.test.ts`; the DOM
half is in `ForceGraphCanvas.tsx`, and it has been verified in a real browser. Everything below is
present-tense behaviour, not a proposal.

The draft gave the canvas no keyboard access at all: no tab stop, no focus model, no traversal, no
selection. §5.7 then *added* a capability — edges become selectable — and defined it purely in pointer
terms, while §3.5 declares the canvas "the reason the view exists" and the one pane that never yields.
The primary content of the section was reachable only with a pointer, which is WCAG 2.1.1 at Level A.

⚠ **This section is now load-bearing for a second reason.** R-04 deleted the node shape channel, so
colour is the only *visual* type encoding left; the `aria-live` region below is the redundant channel
that replaces it, and it is the broader substitute — it serves a blind reader and a screen-reader
user as well as a viewer with dichromacy. Weakening it is not a keyboard regression, it is a
colour-vision one.

| Key | Behaviour |
| --- | --- |
| `Tab` | The canvas is a single tab stop (`tabIndex={0}`, `role="application"`, `aria-label="Knowledge graph"`), with a `focus-visible` inset ring in `--border-accent`. |
| `Arrow` keys | Move the focused node within the **current filter set**, in the arrow's direction, choosing the nearest candidate within a ±60° cone and falling back to the nearest node in that half-plane. |
| `Tab` / `Shift+Tab` *inside* the canvas | Step through the visible set in **descending degree** order — hubs first, which is the same priority the label ladder uses. |
| `Enter` / `Space` | Open the inspector on the focused node. |
| `Escape` | Clear focus; a second press closes the inspector. |
| `Home` | Focus the highest-degree node and `zoomToFit`. |

**Edges are traversed through the inspector, not the canvas.** §4.8 already makes every "Links out"
and "Referenced by" row a real button that selects the edge or the node at the other end, so the
inspector *is* the keyboard surface for edges and no per-edge canvas focus is needed. That is also
why §5.7's 8px pointer tolerance is a pointer concern only.

**An `sr-only` `aria-live="polite"` region announces the focused node** as `<identifier>,
<node_type>, <family>` — for example `Multiple sclerosis, Disease, Clinical`. It is `aria-atomic`,
because the three parts are one sentence. This is the text alternative that makes the type encoding
non-visual as well as non-colour, and — with the shape channel gone — it is what closes SC 1.4.1,
rather than the legend, which is a colour key and therefore still requires discriminating the hue to
use.

⚠ **Both optional parts really are optional.** `identifier` is nullable on the API type, so the
display label is the fallback; and a legacy or plain-OKF base has no families at all, so the family
part is *dropped* rather than announced as "undefined". `announce()` in `graphKeyboard.ts` owns that,
and `familyOfType` (§6.1) is where the family comes from — the same inverted index the Type facet
uses, so the two can never disagree.

---

## 6. New tokens to generate, and where

### 6.1 The graph palette — generated, never hand-written

**Authoring file (new): `ui/desktop/themes/graph.mjs`.** It holds ~50 numbers — the seven family rows
(anchor hue, chroma, spread, ordered type list — **no `shape:` key; `NODE_SHAPES` and all seven of
them were deleted with the channel, R-04**), the two rung ladders, the fallback
chroma and rungs, and the seven credibility rows. It holds **no hex values and no ground**. It sits
beside `themes/*.theme.mjs` but is not a theme (it has no family id); the generator imports it
directly.

**Emitted into `ui/desktop/src/styles/themes.generated.ts`**, at **module scope** — *not* inside
`GENERATED_THEMES[family]`, because §5.1 proves one pair serves all three families:

```ts
export type GraphPalette = {
  types: Record<string, string>;                              // the 28, keyed by OKF type name
  families: Record<string, { members: string[] }>;            // members only — R-04 removed `shape`
  credibility: Record<CredibilityTier | 'retracted', string>; // the 7 ring hues
  ringArcs: Record<CredibilityTier | 'retracted', number | 'dashed' | 'solid'>;
  fallbackChroma: number;                                     // 0.055
  fallbackLightness: [number, number, number, number];        // light [0.7625, 0.7075, 0.6525, 0.5975]
  ground: string;                                             // RESOLVED from --background-muted (§5.1)
};

export const GRAPH_PALETTE: { light: GraphPalette; dark: GraphPalette } = { /* generated */ };
```

The precedent is exact: `syntax`, `terminal`, `codeGround` and `surface` are already emitted into this
file *because* "xterm paints to a canvas and react-syntax-highlighter takes a JS object, so neither
can read a custom property". A 2D canvas is the same category of consumer.

**Do not add anything to `SEMANTIC_TOKENS` in `scripts/lib/theme-contract.mjs`.** These are not
tokens, no CSS consumes them, and adding them would force every family to author 28 values it does not
need. **This design adds zero *theme* tokens** — no family authors anything new, which is the property
that keeps a fourth family cheap. (It does add three structural tokens, §6.2, and one runtime
dependency, §4.8.)

**Consumption.** `styles/graphPalette.ts` is the read side. It exports `typeFill(type, mode)` reading
`GRAPH_PALETTE[mode].types[type] ?? hashedFill(type, mode)`, and — since R-04 — `familyOfType(type,
mode)` in place of the deleted `typeShape`, returning the family name or `null` off a memoised
inverted index built once per mode. **There is no `typeShape` and no `NodeShape`**; the two readers
that need the mark are `nodeMark.ts` (`fillFor` / `isHollow` / `credibilityKey`, for the canvas) and
`NodeSwatch.tsx` (for the DOM), and neither asks for a silhouette.

⚠ **`familyOfType` exists because the inversion had a second reader.** It was hand-built inside
`GraphFacetStrip`, and §5.12's announcement needs the same map; a third private copy is how the
announcement and the picker come to disagree about which family a type is in.

`mode` comes from `useResolvedTheme()`, threaded through the same `useCanvasTheme` hook that carries
ink and ground — one theme-plumbing path, never two.

**`credColors.ts` is deleted.** `nodeFill()` is what DR-9b replaces; `kindColor` is a five-entry
pre-OKF map with no place in a 28-type world; `retractedColor` moves into
`GRAPH_PALETTE.credibility.retracted`. All 13 of its hardcoded hexes go with it.

**The knowledge-base identity dot.** A base's `manifest.color` currently defaults to `#5a6394` — a
slate-indigo baked into Rust that appears nowhere in the app's palette. This design does **not** change
the daemon. The renderer's shared `KbDot` component instead resolves its fill as:

```text
manifest.color, unless it is the legacy default, in which case
GRAPH_PALETTE[mode] hashed fill of the base id, by the §5.4 rule
```

`KbDot` is always a circle: it identifies a base, not a node family. That gives every base a stable,
theme-correct, contrast-audited colour with no migration and no backend change, and it keeps a colour
a user has deliberately set.

### 6.2 Structural CSS tokens

Three new, one comment amendment. All live in `main.css`'s hand-authored `:root` structural block —
they are identical across every family under every `data-theme`, so they are declared once and no
theme definition restates them.

| Token | Value | Why it is a token |
| --- | --- | --- |
| `--knowledge-rail-sources` | `300px` | Written three times otherwise: the xl grid column, the md grid column, and the `<md` tab panel width. |
| `--knowledge-rail-detail` | `340px` | Written four times otherwise: the xl column, the md overlay, the `<md` `Sheet`, and the change-log `Sheet` — which is deliberately the same object at the same width. |
| `--measure-graph` | `clamp(1440px, 96%, 2200px)` | §3.1. The widest column in the app must not be an un-tokenised arbitrary value beside `--measure-chat` and `--measure-page`. `ReadableContent`'s `graph` size reads the token. |
| `--dock-height` | `36px` **(unchanged)** | Comment amended from "terminal dock strip" to "a pane-docked control strip: the terminal dock, the knowledge facet strip, the knowledge legend dock". The value does not move; the name stops lying. |

**All three join every registry the eighteen existing geometry tokens join.** The draft added two and
deliberately skipped all of them, on the grounds that the call sites use `grid-cols-[var(…)]` and
`w-[var(…)]`, which is true of today's call sites and false of the next one:

1. **`@theme inline` mirrors** — `--spacing-knowledge-rail-sources`, `--spacing-knowledge-rail-detail`,
   `--spacing-measure-graph`. `main.css`'s own comment states the convention: authored in `:root`
   and mirrored so a utility "stays late-bound to a token a theme family can re-point".
2. **`src/utils.ts`'s `spacing` array** — `knowledge-rail-sources`, `knowledge-rail-detail`,
   `measure-graph`. Its warning is "one omission here breaks every one of those utilities for that
   name at once", because tailwind-merge otherwise fails to recognise them as the same class group.
3. **`STRUCTURAL_TOKENS` in `scripts/lib/theme-contract.mjs`** — the list that makes `validateMode`
   reject a family that tries to theme a structural value. Without it nothing stops a fourth family
   declaring its own rail width, which is precisely the "nothing enforces the sharing" argument §5.1
   makes for the ground.

With the mirrors in place the call sites are written as utilities — `w-knowledge-rail-detail`,
`max-w-measure-graph`, `h-row`, `h-dock` — and only the grid template keeps the `[var(…)]` form,
because `grid-cols` has no token shorthand.

> **This is also the §10 rule, not merely tidiness.** Under `BIOROUTER_NO_HMR` the renderer runs
> `watch: { ignored: ['**'] }`, which is the same signal Tailwind's scanner uses to notice new class
> strings, so a **newly written** arbitrary utility can silently never reach the stylesheet. A
> registered token consumed through a known utility is generated; a freshly-invented
> `w-[var(--knowledge-rail-detail)]` may not be. Nothing load-bearing may depend on class scanning
> having worked.

### 6.3 The swatch ring

**Every DOM palette swatch carries `box-shadow: 0 0 0 1px var(--background-default)`.** This is the
§5.5 orbit-ring gap transplanted into the DOM, and it exists for the same reason: to make the mark's
adjacent colour a known ground rather than whatever it happens to sit on.

The measurement that forces it: the palette is audited on flat opaque grounds, but §4.2 puts an 8px
`KbDot` on the primary row and §4.6 puts an 8px swatch on every Type-facet row, and those rows take
`tint-selected` / `tint-selected tint-interactive` — `color-mix(in oklab, var(--text-default) 14%/19%,
transparent)`, which is family-dependent because `--text-default` is. Compositing with the repo's own
`blend()` and re-measuring the worst fill (`Gene`) gives **2.89–2.93** at `.14` and **2.60–2.66** at
`.19` in the three light families, and **2.55–2.60** / **2.15–2.23** in the three dark ones: all
twelve family × mode × state combinations below SC 1.4.11's 3:1, worst 2.15.

With the ring, the adjacent colour is `--background-default` in every row state and the measured floor
returns to **3.86 light / 3.81 dark**.

> **Rejected: raising the base rung from 3.50 to ~4.05 so the fill survives a 19% ink wash.** It
> works arithmetically and it is the wrong trade. It re-solves all 28 fills — including the twenty
> that two independent reviewers reproduced hex-for-hex — to fix a *DOM composite* that a one-line
> `box-shadow` fixes outright, and it would darken every fill on the canvas, where the ground is flat
> and the existing floor is correct, to pay for a condition that only exists off it. The ring also
> generalises: it protects the swatch on any future ground, including `--background-strong`, which
> measures 2.80 / 2.52 and which §5.3.4 otherwise has to forbid by rule.

Swatches remain `aria-hidden` with the type name as the accessible label. The ring is presentation
only.

### 6.4 Guards

**The guard is split, because one script cannot see both halves.** The draft put four assertion
families in `scripts/check-contrast.mjs`, including "exact-string assertions that the generated hexes
equal the tables in §5.3 and §5.5" — called "the one that matters most". But `check-contrast.mjs`
reads exactly one file (`const CSS_PATH = join(here, '..', 'src', 'styles', 'main.css')`), parses CSS
declarations and resolves `var()` chains. It has no TypeScript loader and no path to
`themes.generated.ts`, where §6.1 correctly puts `GRAPH_PALETTE`. It could not have run.

The app already has the mechanism, and that script's own comment names it: *"The per-token syntax
stops are asserted in **codeTheme.test.ts**"* — a Vitest test over the generated TS module, which is
the same category of consumer.

| Guard | Addition |
| --- | --- |
| `scripts/generate-themes.mjs` | The `--background-muted` six-scope identity assertion (§5.1), run before emitting `GRAPH_PALETTE`, dying with the stated message; and resolving the ground pair into `GraphPalette.ground`. |
| `scripts/lib/theme-tokens.mjs` | `deltaE00(hexA, hexB)` — CIEDE2000, Sharma–Wu–Dalal, on CIELAB D65 — and `simulateCvd(hex, 'protan' \| 'deutan' \| 'tritan')` — Viénot/Brettel–Mollon 1999 in linear-light sRGB. **One implementation each**, beside `contrast` and `blend`, for the reason that module exists at all. |
| `scripts/check-contrast.mjs` | Keeps only what is a **CSS fact**: the `--background-muted` six-scope identity, now also asserted from the stylesheet side. |
| **`src/styles/graphPalette.test.ts` (new, beside `codeTheme.test.ts`)** | Everything that reads `GRAPH_PALETTE`: (a) 56 type-contrast assertions at `min = 3.0` against `GRAPH_PALETTE[mode].ground`; (b) 14 ring assertions at `min = 3.0`; (c) the CVD audit — all 378 pairs × 4 vision types × 2 modes, asserting **within-family ΔE00 ≥ 3.0** and **normal-trichromacy ΔE00 ≥ 5.0**, and *reporting* all 21 cross-family minima with no floor (the three shape-coverage assertions that stood here were deleted with the channel — R-04); (d) **exact-string assertions** that the generated hexes equal the tables in §5.3 and §5.5; (e) the eight §5.4 hash vectors. |
| `npm run themes -- --check` | Already wired into `lint:check`; fails CI if `GRAPH_PALETTE` is stale relative to `themes/graph.mjs`. |

**Two honesty clauses about what the guards do and do not prove:**

- **Assertion (d) is circular on first landing.** §5.2 already concedes that "the table is corrected
  from the measurement", so on the commit that generates the palette the exact-hex pins record the
  output rather than validate it. They protect *afterwards* — a future tweak to the solver cannot
  silently repaint every graph in the app — and that is their whole value. Say so in the file, or a
  reader will take (d) as validating the tables.
- **A wrong `deltaE00` makes (c) pass vacuously**, and CIEDE2000 is the easiest function here to get
  subtly wrong: the mean-hue discontinuity at ±180° and the `R_T` rotation term are both easy to
  mis-transcribe and neither shows up on well-separated colours. **`deltaE00` therefore ships with the
  published Sharma–Wu–Dalal 34-pair reference table checked in as a fixture
  (`scripts/lib/__fixtures__/ciede2000-sharma.json`, CIELAB inputs and expected ΔE00, cited to the
  source) and asserted to 4 decimal places in the same commit that adds the function** — exactly as
  §5.4 pins `fnv1a` vectors. That table is the correctness gate; the palette figures below are
  regression vectors against this palette and prove nothing about the implementation on their own:

  | Vector | Expected |
  | --- | --- |
  | `deltaE00('#433847', '#3c2b34')` (Concept/Other, light) | 5.54 |
  | `deltaE00('#d8cadc', '#f2dae7')` (Concept/Other, dark) | 5.65 |
  | `deltaE00('#6965be', '#546fa5')` (Variant/MaterialSample, light) | 7.77 |
  | `deltaE00(simulateCvd('#4a5263','deutan'), simulateCvd('#474557','deutan'))` (Population/GeographicLocation) | 3.55 |
  | `deltaE00(simulateCvd('#005963','protan'), simulateCvd('#605364','protan'))` (the draft palette's collision) | 0.35 |

  The last row is the regression test for the defect this revision exists to fix, written against the
  hexes that had it.

---

## 7. File plan

The draft had none: 1,457 lines, no per-file table, no landing order, and a §7 that exhaustively
listed what was *not* changing while nothing listed what was. Reviewers had to derive it. This is the
derivation, checked against the tree.

`components/knowledge/` is **4,742 lines across 22 non-test files** today. The estimate below is the
one number in this document that is a judgement rather than a measurement, and it is flagged as such.

### 7.1 Renderer — the Knowledge section

| File | Change | Governed by | Slice | Existing tests it touches |
| --- | --- | --- | --- | --- |
| `KnowledgeView.tsx` (139) | rewrite | §3 | A | `KnowledgeView.test.tsx` — the tab case breaks by design |
| `KBSelector/KBSelectorTrigger.tsx` | rewrite | §4.1 | A | `KBSelectorTrigger.test.tsx` |
| `KBSelector/KBSelectorPalette.tsx` (493) | **split, then delete** | §4.1, §4.2 | A | `KBSelectorPalette.test.tsx` — all 10 cases ported, not dropped |
| `KBSelector/KBSelectorMenu.tsx` | **new** | §4.0, §4.1 | A | receives 6 ported cases (DR-12 primary repair) |
| `KBSelector/KBManagerDialog.tsx` | **new** | §4.2 | A | receives 4 ported cases (list + search) |
| `KBSelector/KbFormatChooser.tsx` | **new** | §4.3 | **B** | — |
| `KbDot.tsx` | **new** (shared) | §4.2, §6.1, §6.3 | A | — |
| `SourcesRail.tsx` | **new** (shell only) | §4.4 | A | — |
| `IngestPanel/IngestPanel.tsx` (511) | edit — presentation only | §4.4 | A | `IngestPanel.test.tsx` (16), `IngestPanel.streamFailure.test.tsx` (3) — **must not break** |
| `IngestPanel/Dropzone.tsx` (217) | edit | §4.4 | A | — |
| `IngestPanel/PasteTextBox.tsx` | edit | §4.4 | A | — |
| `IngestPanel/StagedList.tsx` | edit | §4.4 | A | — |
| `IngestPanel/IngestWarnings.tsx` (89) | edit | §4.4 | A | — |
| `IngestPanel/IngestModelPicker.tsx` (247) | rewrite (menu → combobox) | §4.0, §4.4 | A | `resolveIngestModel.test.ts` (4) — **must not break** |
| `DispatchProgress.tsx` (172) | rewrite | §4.4 state 3 | A | — |
| `KbTierControl.tsx` (232) | edit — 4 class swaps + placement | §4.11 | A | `KbTierControl.test.tsx` (4) — **must not break** |
| `changelog/ChangeKindChip.tsx` | edit | §4.10 | A | **new** `ChangeKindChip.test.tsx` in the same commit |
| `changelog/ChangeLogDrawer.tsx` (149) | edit | §4.10 | A | — |
| `graph/KnowledgeGraphPanel.tsx` (198) | rewrite | §4.5 | A | — |
| `graph/ForceGraphCanvas.tsx` (327) | **replace** | §5.4–§5.12 | A, then B | see §7.4 |
| `graph/graphStyle.ts` (120) | rewrite | §5.11, §6.1 | A | `graphStyle.test.ts` — 4 cases become 1 parameterised table over 7 fields |
| `graph/nodeShapes.ts` | ~~new~~ **deleted** — R-04 removed the shape channel; the mark lives in `graph/nodeMark.ts` (fill + solid/hollow) and `graph/NodeSwatch.tsx` (the DOM key) | §5.3.1 | — | `nodeMark.test.ts` |
| `graph/graphKeyboard.ts` | **new** — the pure half of §5.12 (cone traversal, degree-ordered walk, `announce`) | §5.12 | A | `graphKeyboard.test.ts` (18) |
| `graph/layout.ts` | **new** | §5.10 | A (forces) / C (seeding) | — |
| `graph/labelText.ts` (109) | edit — delete `wrapLabel` | §5.8 | A | `labelText.test.ts` — 3 cases deleted with it |
| `graph/credColors.ts` | **delete** | §6.1 | B | — |
| `graph/NodePreview.tsx` (123) | **replace** | §4.8 | B | `NodePreview.test.tsx` — 2 contracts ported |
| `inspector/InspectorShell.tsx` | **new** | §4.8 | A | receives the 2 ported contracts |
| `inspector/NodeInspector.tsx` | **new** | §4.8 | **B** | — |
| `inspector/FrontmatterRows.tsx` | **new** (needs `js-yaml`) | §4.8 | **B** | — |
| `inspector/EdgeInspector.tsx` | **new** | §4.8 | **B** head/headline, **C** payload | — |
| `graph/FacetRail.tsx` | **new** | §4.6 | **B** | — |
| `graph/GraphLegend.tsx` | **new** | §4.7 | **B** | — |

### 7.2 Shared primitives and app-wide files

| File | Change | Governed by | Slice | Notes |
| --- | --- | --- | --- | --- |
| `components/ui/badge.tsx` | edit — add `asChild` | §4.7 | A | P4: the variant lives in the primitive. Used by the legend, the facets and the change log. |
| `components/icons/app-icons.tsx` | edit — re-export `Filter`, `Inbox` | §4.12 | A | Two lines. |
| `components/Layout/ReadableContent.tsx` | edit — `graph` reads `--measure-graph` | §3.1, §6.2 | A | |
| `components/bottom_menu/BottomMenuKnowledgeSelection.tsx` | edit — adopt the new trigger | §4.2 | A | The chat-side KB chip. The draft never mentioned it. |
| `src/styles/main.css` | edit — 3 tokens, 3 mirrors, 1 comment, the swatch ring | §6.2, §6.3 | A | |
| `src/utils.ts` | edit — 3 names into `spacing` | §6.2 | A | |
| `package.json` | edit — `js-yaml@4` | §4.8 | **B** | |

### 7.3 Generation, guards and verification

| File | Change | Governed by | Slice |
| --- | --- | --- | --- |
| `themes/graph.mjs` | **new** — ~50 numbers, no hexes, no ground | §6.1 | B |
| `scripts/generate-themes.mjs` | edit — resolve the ground, six-scope assertion, emit `GRAPH_PALETTE` | §5.1, §6.1 | B |
| `scripts/lib/theme-tokens.mjs` | edit — `deltaE00`, `simulateCvd` | §6.4 | B |
| `scripts/lib/__fixtures__/ciede2000-sharma.json` | **new** — the 34-pair reference table | §6.4 | B |
| `scripts/lib/theme-contract.mjs` | edit — 3 names into `STRUCTURAL_TOKENS` | §6.2 | A |
| `scripts/check-contrast.mjs` | edit — keep only the CSS fact | §6.4 | B |
| `src/styles/graphPalette.test.ts` | **new** — every assertion that reads `GRAPH_PALETTE` | §6.4 | B |
| `src/styles/themes.generated.ts` | generated | §6.1 | B |
| `ui/desktop/.knowledge-harness/` | **new** — Vite page on the `.artifact-harness` pattern | §10 | **first** |
| `crates/biorouter-mcp/tests/knowledge_graph_fixture_dump.rs` | **new** — real graphs, not hand-written JSON | §10 | **first** |

### 7.4 The canvas is its own workstream, and the document should say so

`ForceGraphCanvas.tsx` is **327 lines today**. §5.4–§5.12 specify: component-BFS seeded layout with
concentric ring packing, hub selection via a four-term affinity score, golden-angle member spirals, a
hard post-tick `containNode` clamp with velocity clamping and NaN re-seeding, per-link distance and
strength accessors, a 16-row × 4-band force table, viewport culling in `onRenderFramePre`, a
density/LOD model, a two-state solid/hollow circular mark (**not** seven shape paths — R-04 removed
them), an arc-segmented credibility ring, an entirely new label
pass in `onRenderFramePost` with priority sorting and greedy AABB packing and two-level memoisation,
five edge render cases including a filled tapered quadrilateral and lane-canonicalised curvature, and
a keyboard focus model.

**Realistically 900–1,300 lines of new canvas code.** It is by a wide margin the largest single unit
of work in this document, it is a *replacement* rather than an edit, and it gets its own multi-week
workstream with its own harness gate — not one row in a design pass. Budget it as such or the slice
plan is fiction.

### 7.5 Existing tests, by name

14 test files and 94 `it()` cases live under `components/knowledge`. Three categories, and the
distinction is the point:

**Break, and should:**

- `KnowledgeView.test.tsx :: "lets compact windows give the digest and graph their own workspace"` —
  §3.4 renames the tabs Digest/Graph → Sources/Graph, moves the pair into the subject band and
  replaces the hand-rolled pill with `<Tabs>`, so `getByRole('tab', { name: 'Digest' })` fails. Update
  the case; keep both `data-testid`s.
- `labelText.test.ts` — the three `wrapLabel` cases go with the function (§5.8).

**Break, and the contract must survive the break:**

- `NodePreview.test.tsx :: "has dialog semantics and dismisses with Escape"` and
  `:: "dismisses when another control is clicked without swallowing that click"` — ported onto
  `InspectorShell` in the same commit (§4.8). The second encodes a real fixed bug.
- `graphStyle.test.ts` — the four resolver cases become one parameterised table over all seven fields
  (§5.11), which *widens* the contract rather than replacing it.
- `KBSelectorPalette.test.tsx` — all ten cases land on the two new files (§4.2). The six
  "following the default again" cases encode DR-12 and must not be re-derived.

**Must not break — if they do, the change overreached:**

`IngestPanel.test.tsx` (16), `IngestPanel.streamFailure.test.tsx` (3), `useIngestStream.test.ts` (5),
`resolveIngestModel.test.ts` (4), `knowledgeRequest.test.ts` (3), `KnowledgeContext.test.tsx` (22),
`KbTierControl.test.tsx` (4). §9 says these are presentation-only changes; these tests are how that
claim is checked.

**Unguarded in both directions today:** no test file exists for `ChangeKindChip` or `ChangeLogDrawer`,
so §4.10's status-hue-to-taxonomy fix has nothing stopping it regressing and nothing that would have
caught it going in. §7.1 requires the test in the same commit.

### 7.6 Landing order within a slice

**`npm run lint:check` chains `check:themes` and `check:contrast`, so §6 must land in one commit, not
two.** The generator, `themes/graph.mjs`, the emitted `GRAPH_PALETTE`, the split guards and
`graphPalette.test.ts` land together; the consumers (`graphStyle.ts`, the legend, the facets) land in
the next. Sequenced the other way, `lint:check` is red across a merge, which is the state in which
people start passing `--no-verify`.

Within Slice A: the harness and the fixture dump first, then `main.css` + `utils.ts` +
`theme-contract.mjs` (tokens), then `badge.tsx` (the primitive), then the shell, then the surfaces,
then the canvas.

---

## 8. What changes for the user

- **Pick a format when you create a knowledge base.** OKF (permissive, the default) or BioOKF (the
  strict biomedical profile), each with guidance on when to pick it — and a plain warning that the
  choice cannot be changed yet.
- **The graph shows what things are.** Every page is a circle, and its *colour* says which type it
  is — related types sit in one hue family, so genes look like genes and diseases look like diseases.
  Pages that are *evidence* rather than science — publications, studies, datasets — are drawn as open
  rings instead of filled dots, and a page that is only referenced and not yet written is a dashed
  ring. The ring around a source says how well-sourced it is by how many arcs it has. Arbitrary types
  in an OKF base get a stable colour of their own that never changes between sessions.
- **If two colours look the same to you, the graph will tell you in words.** Hover or focus a page
  and its name, type and family are announced and shown; the legend and the inspector name every
  type. Colour is the fast channel, never the only one.
- **Links carry meaning.** A tapered line shows which way a claim points; a dashed red line with a
  struck-through label is a negation; a faint dotted line is provenance the system derived rather than
  something you asserted. Hovering a link names its predicate.
- **Click a link.** Edges are selectable and have their own inspector: the claim as a sentence and the
  source it came from.
- **The inspector reads the page instead of dumping it.** Frontmatter renders as labelled rows with
  clickable cross-references, and — new — the page's outgoing links grouped by predicate and its
  incoming links as "Referenced by". Clicking either moves you there without losing the canvas.
- **The graph works from the keyboard.** Tab into the canvas, arrow between pages, Enter to inspect,
  and the focused page is announced.
- **Filter the graph.** By node type, predicate, source or status, plus a live text filter. Filtering
  dims rather than removes, so the map you have learned stays where it was. The legend is now the type
  filter: click a swatch to isolate it.
- **Digest shows real progress.** A progress bar, announced to screen readers, instead of a spinner
  and an event count.
- **Everything else looks like the rest of BioRouter.** The base picker is a dropdown, not a
  full-screen dialog; every empty and error state has an icon, a title and a way forward; the primary
  base no longer un-highlights when you point at it; and the gradient behind the graph is gone.

**Not in this pass:** a lint pill. §2.3 records why, and what has to exist before it can come back.

---

## 9. What is NOT changing

Stated so the diff stays bounded. Anything below is out of scope for this spec and must not be
touched in the same change.

**Behaviour and data**

- The visible-set / primary-pointer model in `KnowledgeContext.tsx`, `.active-kb` and
  `.hidden-kb-sessions`, and every rule about primary repair and promotion. Untouched — and §4.2's
  file split is why `KBSelectorPalette.test.tsx`'s six DR-12 cases are ported rather than rewritten.
- The tier control's behaviour, copy and typed-phrase confirmation (issue #56 DR-18) — visual
  corrections only (§4.11).
- The ingest dispatch logic in `IngestPanel.tsx`: model resolution, the `modelOverride` stamping, the
  `kbPending`/`kbUnavailable` three-state precedence, the pre-flight `checkModel`, the abort path, and
  the auto-clear of succeeded items. Only presentation changes.
- K-04 — the one primary action stays full-opacity with a helper line, never half-lit.
- `useIngestStream`'s SSE contract and terminal-frame handling.
- Every `data-testid` in the section. Eight test files select on them; they survive the restyle.
- The `.brkb` export/import path and its provenance sidecar (DR-21).
- `PageKind` (`kind`). It survives Stage 2 and this document uses it for nothing (§5.5) — which is
  not the same as the app using it for nothing; see §5.5 for the two places it still renders.

**Deferred by DR-22, and therefore absent from this design**

- `kb_migrate_format` and any convert-format UI. The format chooser reserves the surface a future
  conversion would reuse and says plainly that conversion does not exist yet.
- `br_page_id` stamping (DR-3). Identity resolves through the ladder; nothing in the UI shows a page id.
- Attested Computations. No surface.
- A renderer swap (WebGL, sigma, cosmos). DR-9 defers it on measured graph sizes; this design keeps
  `react-force-graph-2d` and ports technique only.
- A plain-OKF bundle export (DR-21). `.brkb` stays the only transfer door.

**Design-system scope held elsewhere**

- `--row-height` is still 40px against the canonical 36px (**DR-62**). This design consumes
  `.biorouter-list-row` and inherits whatever the token says; fixing the token is an app-wide sweep
  with three hardcoded call sites and is not owned here.
- The `Input` primitive's resting edge does not reach 3:1 (§4.1's measured 1.62–1.66 light /
  2.04–2.10 dark). App-wide token change, tracked outside this document.
- `EmptyState`'s plate draws a 24px icon where `--icon-banner` is 20px and is commented "banners,
  empty states". §4.4 follows what ships; reconciling the two is a design-system item.
- `--color-block-teal`, the 11 hand-rolled `.biorouter-modal-surface` users, and the D-08 stage-2
  react-select → Radix swap. All app-wide, all out of scope.
- `.page-transition`, which has ten call sites and zero CSS rules. This section stops adding call
  sites (§3.2); defining it is a design-system change.
- Theme families, neutrals, accents, status hues, shadows and the focus treatment. Zero changes; zero
  new theme tokens. (Three *structural* tokens, §6.2.)

**Raw `<button>` elements — the honest count**

§9's predecessor claimed "new code in this section uses the primitives" while the specification
itself called for bare buttons in four places, which is a contradiction a reader would have had to
discover. Resolved as follows:

- Toggle chips (legend, facet, change-log filters) go through **`Badge asChild`** wrapping a
  `<Button>`, so they are primitives and take the D-15 focus fill (§4.7).
- **Two surfaces still take a raw element, deliberately**: the inspector's "Links out" / "Referenced
  by" rows (§4.8), which are full-width multi-column rows that no `Button` variant expresses without
  inventing one, and the canvas itself (§5.12), which is a `role="application"` region rather than a
  control.
- That adds roughly **two raw `<button>` patterns**, not a dozen, to **DR-24**'s open backlog (103
  elements in 58 files recorded; 149 in 88 files measured today). Naming the number is the point; a
  disclaimer that contradicts the body is not.

---

## 10. Verification

Stage 7's gate is a **real browser**, and most of this document cannot be checked anywhere else.
jsdom has no canvas 2D context, no layout, no viewport, does not run Tailwind and does not evaluate
`:has()`.

| Where | What it checks |
| --- | --- |
| `scripts/check-contrast.mjs` (Node, no browser) | The `--background-muted` identity across all six family × mode scopes. That and nothing else from this document — it reads only `main.css`. |
| `scripts/generate-themes.mjs` | The same identity from the generator side, before emitting; the resolved ground pair. |
| `src/styles/graphPalette.test.ts` (Vitest, over the generated TS module) | The 56 type-contrast assertions, the 14 ring assertions, the CVD audit over 378 pairs × 4 vision types × 2 modes (within-family floor asserted, cross-family collapse reported only — the shape assertions went with the channel, R-04), the exact-hex pins, and the eight hash vectors. |
| `scripts/lib/__fixtures__/ciede2000-sharma.json` | That `deltaE00` is CIEDE2000 at all. Without it the audit above can pass vacuously. |
| `npm run themes -- --check` | `GRAPH_PALETTE` is not stale relative to `themes/graph.mjs`. |
| Vitest (jsdom) — **pure functions only** | `fnv1a`; the density formulas over a table of `(visibleNodes, visibleEdges, globalScale)`; the AABB overlap predicate; the label priority ladder over a focus/hover/hub/neighbour/zoom matrix; the `w100 * fs / 100` width scaling; `prettyLabel`; the facet predicate (OR-within, AND-across); **and all of §5.12's pure half — `graphKeyboard.test.ts` exists and holds 18 tests** over cone traversal, the half-plane fallback, the degree-ordered `Tab` walk, `Home`, and the `announce()` string with each optional part missing. Each is deliberately free of React and the DOM, for the reason `utils/messageClamp.ts` records: a threshold you can only exercise by rendering a component is one nobody re-tests — and `ForceGraphCanvas` cannot be rendered in jsdom at all, because force-graph calls `canvas.getContext('2d')`. |
| **Browser harness** (new, on the `.artifact-harness` pattern: a plain Vite page mounting the real `ForceGraphCanvas` and the real panes against fixture graphs) | Everything else, enumerated below. |

### 10.1 What jsdom cannot see — the explicit list

**Canvas** (no 2D context, no layout, `getComputedStyle` returns nothing usable):

- Every paint behaviour in §5.3.1 and §5.5–§5.10: the solid-versus-hollow mark at three zooms —
  whether a 1.7-unit ring still reads as an open circle rather than a donut or a filled dot at the
  small end; the external node's dashed ring; the orbit ring and its 1px gap just above and
  just below `r × globalScale = 3.5`; **whether four arcs are countable at the LOD boundary**, which
  §5.5.1 explicitly declines to assert; the tapered quad reading directionally at full density fade;
  the `not_` dash plus struck-through label; the `[1,4]` synthesized dot pattern reading as distinct
  from the `[4,3]` negation dash at 1px; the focus radial gradient; the grid dot field; the retracted
  badge glyph painted in the resolved ground.
- §5.8's collision avoidance actually preventing overlap at three zooms on a 400-node fixture. The
  AABB predicate is unit-testable; that it works against real `measureText` is not.
- Frame time at 250 / 900 / 2,550 nodes.
- **§5.11 is the worst case.** In jsdom every resolver returns its fallback and the test passes
  whether the resolve works or not — identical in shape to the trap `styles/composerFocus.test.ts`
  documents, and the same bug class that produced `CANVAS_INK_FALLBACK` in the first place. That the
  resolved `ink` / `ground` / `danger` / `monoFamily` actually change when `data-theme` and `.dark`
  flip can only be seen in a browser.
- §4.5's gradient deletion. Only a real browser shows `getComputedStyle(el).backgroundColor`
  returning a colour instead of `rgba(0,0,0,0)`.

**Tailwind** (jsdom never runs it — the same blindness as the Prism `token table` collision):

- §4.7's legend chips, §4.6's facet menus and the inspector's heights.
- §3.4's three responsive steps and §3.5's yield ladder. jsdom has no viewport.
- §6.3's swatch ring, which is a `box-shadow` on a class jsdom will not generate.
- Every arbitrary utility this document introduces — which is exactly why §6.2 registers all three new
  tokens in `@theme inline`, `src/utils.ts` **and** `STRUCTURAL_TOKENS` instead of relying on
  `grid-cols-[var(…)]` and `w-[var(…)]` being scanned.

**Colour-vision simulation** is *not* on this list. It is arithmetic over generated hexes and belongs
in `graphPalette.test.ts`, where it runs on every CI job rather than in a harness someone has to
remember to open.

> **Superseded.** This paragraph continued: *"But **the shape channel is** — that seven silhouettes
> are actually distinguishable at the sizes §5.6 produces is a perceptual claim, and the harness is
> where it gets checked."* R-04 removed the channel, so there is no such claim left to check. What
> takes its place in the harness is narrower and still perceptual: that the **1.7-unit hollow ring**
> reads as an open circle at the radii §5.6 produces — neither a donut nor a filled dot — and that
> the **1.5px legend swatch** agrees with it, on all three families, light and dark, with contrast
> **measured** in the running app rather than asserted. The type-level claim that silhouettes used to
> answer is now answered by §5.12's live region, which is asserted where it lives.

### 10.2 Fixtures

One OKF base (arbitrary types, exercising §5.4) and one BioOKF base (all 28 types plus one
off-vocabulary string, negated edges, a synthesized provenance edge, a retracted source, an external
node), both emitted from the real Rust graph deriver on the `preview_fixture_dump` pattern —
**not hand-written JSON**, which would encode the schema someone believed in rather than the one
Stage 2 emits.

> **Warning.** Launch the dev GUI with `BIOROUTER_NO_HMR=1` for any interactive check, or a save
> anywhere under `ui/desktop/src/` full-reloads the renderer and destroys the session under test. But
> the same flag sets `watch: { ignored: ['**'] }`, which blinds Tailwind's class scanner, so a
> **newly written** utility class in the legend or facet markup may silently never reach the
> stylesheet. Nothing load-bearing may depend on class-scanning having worked — author such rules in
> `main.css`, as `.biorouter-composer-card:has(textarea:focus)` already is, or consume a registered
> token through a known utility, as §6.2 requires.

---

## Related documentation

- [OKF migration design and decision records](design.md) — DR-9, DR-9b, DR-10, DR-11 and DR-22, which this specification implements.
- [OKF migration stepwise plan](stages.md) — Stage 7, whose gate this document is the input to, and Stage 2, whose emitted contract §2.1 changes.
- [OKF migration progress](progress.md) — what has landed.
- [Multi-KB implementation plan](../multi-kb-implementation-plan.md) — the visible-set / primary-pointer model §4.1 and §4.2 must not disturb.
- [Theme system architecture](../../design/theming/theme-system-architecture.md) — the shared-neutrals rule §5.1 depends on, the per-family status-hue rule §5.11 obeys, and the generator contract §6.1 extends.
- [Privacy tiers](../../security/privacy-tiers.md) — the design the tier control in §4.11 implements.
