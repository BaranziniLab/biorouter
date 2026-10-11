# The provider catalog

> **What this is.** The one surface that lists every provider the daemon serves — three tabs carrying the privacy taxonomy, institutions named from the daemon's own affiliation payload, AI agents first among the public ones — and how the first-run screen became the same component in a different mode.
> **Status:** Current.
> **Audience:** contributors working on the desktop renderer or on provider registration.

## What this replaced

There were two lists of the same providers, and they agreed about almost nothing.

Settings → Provider Configuration rendered `ProviderGrid`: three stacked sections in one
scroll, ordered by a `PRIORITY_ORDER` map that assigned twelve providers a rank and left
the other sixteen sorted by their internal id — so `custom_deepseek` sorted under "c" and
`zai` under "z", nowhere near where a reader looking for "DeepSeek" and "z.ai" would look.

The first-run screen (`ProviderGuard`) rendered five bespoke cards, one per setup path,
each ending in a "View all … providers →" link to a *third* rendering of the same grid.
Twenty-three of the twenty-eight providers were reachable only through that link, and the
screen had no way past it at all: a user who wanted to look at the application before
pasting an API key could not open it.

Both are now `components/settings/providers/ProviderCatalog.tsx`, in `mode="settings"` or
`mode="onboarding"`.

## Information architecture

Three tabs, in this fixed order — the order the product ranks these everywhere else
(onboarding, the settings provider grid, `biorouter configure`):

| Tab | Group label (§14.5) | What it holds |
|---|---|---|
| **Local** | Private · Local | Llama Server, then Ollama, then any other local provider by display name |
| **Institutional** | Private · Institutional | One section per institution, then "Unaffiliated private gateways", then the "Don't see your institution?" line |
| **Public** | Public · Commercial | "AI agents · your subscription", then "API providers" |

The group label, its one-line note and the tier dot all come from
`getOrderedProviderGroups` in `providerOrdering.ts` — the same function
`components/knowledge/IngestPanel/IngestModelPicker.tsx` reads, which prints the label as its
`<optgroup>`. The tab trigger's short label (`Local` / `Institutional` / `Public`) is derived there
too, so a trigger and the panel it opens cannot come to name different things.

**The panel prints the tier note, not the group label.** Since the 2026-10 redesign a panel opens
with one muted line, the group's note ("Private because inference runs on this machine.", "Public.
Biorouter can't verify…"), and no heading: the tab already names the group, and the note starts with
the same word the tier dot means, so the old "Private · Local" heading only repeated the tab. The
line is privacy information, so it is visible text and never an InfoTip
(`ProviderCatalog.privacy.test.tsx` holds both).

## The rows

A provider is a row on the shared settings-row geometry (`ProviderCard.tsx`, `providers.css`): a name
over a one-line description (truncated, with a tooltip for the rest only while it is cut), "Configured" as a success
dot and a word, and one action. There is no initial tile. The status and the action share one cell,
Crew's `meta` recipe (`.br-row-actions-slot`): "Configured" at rest on the right edge, and the action
(`Configure`, named for its row, as in "Configure Anthropic"; `Use`, named "Use Anthropic", for a
configured provider on first run, the verb the open panels use) in its place while the row is hovered or holds focus, always shown without a hover pointer
(`.br-row-actions`), so the edge never moves. **First run reveals its actions the same way** (vision round
1, V4-29): it used to keep the button on screen, which made it the one list in the app with permanent row
actions on a hover-capable display. Its rows act on a click as Settings' do, so the button is the keyboard
path and a shortcut for the mouse, never the only way in.

Two first-run refinements (first-run vision round 1):

- **An expandable row on first run has no row action** (FR-R1-5). Its open panel holds the primary for
  the same choice ("Use Claude Code", "Download and start", "Use Ollama", "Connect"), so a hover row action
  beside it was a second control for one decision. "Configured" stays its resting status and no longer
  gives way under the pointer. Settings keeps the action on its expandable rows.
- **A description is a Tab stop only while it is cut off** (FR-R1-9), measured as text wider than its
  box and re-measured on resize; the row's action (or an agent's toggle) is described by the full text,
  so a screen reader hears it without the stop. Every description used to be a stop, so the keyboard
  walk was row, description, action for every provider.

Four more from first-run vision round 2:

- **The tooltip opens only while the line is cut off** (FR-R2-7), by the same measure that makes it a
  Tab stop, and it shows what is cut: the line's own text, in the tooltip's one face. Hovering an agent's
  short line used to open the daemon's three sentences, `claude` in mono, over the next row.
- **"Use", not "Launch"** (FR-R2-4): a configured row that does not open in place (every configured API
  row, Versa API Azure) said "Launch" beside panels that say "Use Claude Code". Settings never shows it.
- **An accordion row's keyboard focus is the row's** (FR-R2-3): D-15's row focus (`--background-focus`,
  an inset 2px `--border-focus` edge, the row's radius) on `.biorouter-settings-row:has(> .br-provider-row-toggle:focus-visible)`,
  and none on the toggle, which used to take the base layer's square box flush against its text
  (`providerRowFocus.browser.test.tsx`).
- **Connect follows Detect's rule** (FR-R2-5): the institutional form's Connect is secondary until the
  keys its gateway needs are in, and coral once it can act. Disabled coral read as a pale coral in light
  and a muddy brown in dark.

Three details keep one list reading as one list (V4-26):

- **The disclosure chevron follows the name** ("Claude Code ⌄", as Codex draws a group), so every title
  starts on the same x. It points down at rest and turns over when the row is open.
- **One status per row.** A coding agent that is ready *and* configured is said by "Configured" alone; its
  pill ("Ready · signed in on your subscription") would be the same news twice. Every state that needs
  attention keeps its pill, and so does a ready agent that is not set up yet, because the pill is what
  invites the "Use" in its panel.
- **The daemon's backticks are drawn as inline code** (`InlineCodeText`): a description it serves as
  ``…installed `claude` command-line app`` shows `claude` in mono, not with the marks. Only a closed pair on
  one line counts, and nothing is interpreted as markup.

An open agent's panel keeps its buttons at the `sm` size and shows the CLI's version as a line; where the
CLI lives is a path, so it is the line's tooltip (principle 10), not a mono line on the row.

An accordion row (the coding agents, and the local providers on first run) and its panel are two
siblings in the list, never one wrapper: the shared rows block reads `:last-child`, and a wrapper made
every row its parent's last child.

**Rows are told apart by their air and a fill, not by a hairline** (owner message 13, R-27, design.md
9.11). A provider row is a two-line block with 8px above and below, so the rule the shared rows recipe
draws under every row is switched off for the catalog with the shared scope, `.br-rows-flat` on the
catalog root (the one definition lives in the rows block of `main.css`; the 1px transparent edge stays,
so nothing moves, and forced colours draw the line again, because every fill is repainted as Canvas
there and the line is all that tells one row from the next).

A hovered row is a washed row, and the shared rows block already steps a control in it back one ground
(the button rests on `--background-default` and hovers on the strong step). What `providers.css` adds is
the paint of the row. The shared wash is the page ink at 5%: in light it lands on the rest fill's value,
but in dark on the canvas it lands at rgb(30, 30, 29), where the Configure button resting on the page
ground (rgb(27, 27, 25)) is 1.03:1 off its row and has no box. The row therefore paints the card fill
(`--background-fill`), which leaves the button 1.10:1 off it (measured in Chromium,
`firstRunFill.browser.test.tsx`). First run stands on the same page ground, so one recipe serves both
surfaces. Settings paints its hovered rows with the same idea (`settings.css`); the two are separate
rules because the catalog root and Settings' reading column each carry `.br-rows-flat`, and the
catalog's selector (`.br-provider-catalog.br-rows-flat`, two classes on one element) out-ranks Settings'
one-class paint by specificity, so the order the style sheets are bundled in does not matter.

## Where a dialog puts focus when its controls load late

A row's action opens `ProviderConfigurationModal`, whose key fields load after the dialog mounts (the
form says "Loading…" until the saved values are in). Radix picks the first tabbable control at mount, which
was the footer's **Remove** on a configured provider, or Cancel on a new one, so a keyboard user opened
the dialog on a destructive button. The Lead/worker dialog (`LeadWorkerSettings`) loads its models the same
way and opened on its own × with the ring showing. Both now use the two halves in
`settings/models/dialogFocus.ts`: `restOnDialog` (the dialog's `onOpenAutoFocus`) rests focus on the dialog
box, which draws no ring and where a screen reader reads the title, and `useHandFocusOnReady` moves it to the
first enabled field (Configure) or the switch (Lead/worker) once the body is in, **only while the box still
holds it**: a person who has already tabbed to Cancel stays on Cancel. The opener is recorded before either
runs, so Escape and Cancel still return focus to the row's button. `ProviderConfigurationModal.focus.test.tsx`
and `LeadWorkerSettings.focus.test.tsx` hold it, and each line that makes it true was checked by removing it.

**After a pointer open, only a text field takes the hand-off** (V4-R4-1). A focus a script moves after a mouse
click can match `:focus-visible` in Chromium (it decides from the menu item the pointer just picked), so the
Lead/worker switch wore the inset grey ring after "Lead and worker..." was clicked. `restOnDialog` records
whether the last input was a key (`lastInputWasKey` from `ui/inputTrail`, the tracker `ui/dialog.tsx` already
reads for its own quiet open), and `useHandFocusOnReady` skips a control that is not a text entry when it was
not: focus stays on the dialog box, which draws none, and the next Tab reaches the first control. A Configure
dialog opened with the mouse still lands in its first key field, because a text field is always ringed and wants
the cursor. A keyboard open hands focus on as before. jsdom has no `:focus-visible`, so
`LeadWorkerSettings.focusRing.browser.test.tsx` drives the real menu with Playwright's mouse and keyboard in
Chromium and reads `:focus-visible` off the focused node; it fails against the previous `dialogFocus.ts`.

**The shared dialog now makes every pointer open quiet by itself, and this module is still needed** (V4-R5-1).
`useReturnFocusToOpener` in `ui/dialog.tsx` rests any dialog opened without a key on its box, unless Radix's first
stop is a text field, so a dialog that does nothing about focus gets the quiet open for free. These two dialogs
cannot rely on that, for two reasons. Their first control does not exist when the dialog mounts, so there is
nothing for the shared rule to look at: `restOnDialog` runs first (the shared hook calls the caller's
`onOpenAutoFocus` and stands down when it cancels the event), keeps the box as the resting place, and records the
modality for `useHandFocusOnReady`, which decides the hand-off once the body arrives. And a keyboard open must
still land on the first control after the late load, which only the hand-off does. The two rules agree about a
pointer open (the box, unless a text field) because one was written after the other; the shared dialog does not
hand focus on, so deleting `dialogFocus.ts` would leave a late-loading form with no first stop at all.
`dialogFocus.ts` keeps its own small text-entry test for the hand-off: both callers' selectors name a switch or a
plain input, never the shared `Select`'s typing input that the shared rule excludes.

## The Llama Server card's model picker is the shared select

`LlamaServerInlineCard` (the first row of the Local tab, open by default in onboarding, and the expanded row in
Settings) picks its model with `ui/Select` inside the shared `Field`, the same select the Switch models and Lead/worker dialogs use. It
was a native `<select>` until vision round 4 (V4-R4-3), which drew the OS chevron and opened the OS popup, the
only ones in the app. The options are the same strings as before ("Gemma 4 E4B · 9.6 GB · Needs download"), the
label still points at the field (`for` and `inputId` are both `llamacpp-model-select`), and the menu is portalled
so a card or a scrolling page cannot clip it. react-select forwards no `data-*` attribute, so
`data-testid="llamacpp-model-select"` sits on a wrapper around the select; read the chosen model from its text,
not from a `value`. `LlamaServerInlineCard.select.browser.test.tsx` measures the trigger (32px, a 16px chevron),
the menu (one row of at least 32px per model, a trailing check on the chosen one) and the disabled step in
Chromium, light and dark. The two other native selects in the tree, `components/parameter/NativeSelect.tsx` and
the enum branch of `components/ui/JsonSchemaForm.tsx`, are not this card's and were left alone.

Under the select the panel says one line, the Settings row's facts line (`settings/models/localModelFacts.ts`,
read by both surfaces: speed, context window, and "Large for this Mac" only when the daemon reports the
model above this machine's recommendation), as the closed summary of a "Details" disclosure. The panel's line
leaves out the size (`factsLine(model, { size: false })`), because the option and the button already say it;
three copies of "7.6 GB" in three lines was FR-R2-6. Details holds the model's description, the download note,
the speed hint, a Memory row that says the fit in words with the two numbers it rests on ("Fits this Mac (128
GiB unified memory, 32 GiB recommended)", `fitLine`, from the daemon's `suitability_status`; the daemon's own
sentence repeated its label and printed `apple_unified`), the Windows RAM caveat on Windows, the Ollama tag,
the fallback GGUF, the model file and the model store, as sans labels over values, the machine values in mono at
12px; no box. Then the one action on the 32px rung: "Start", "Download and start (9.6 GB)" or "Resume and start".
A status shows only with news (running, warm-up needed, binary missing). It used to be a box of seven lines and an
unconditional "above the 16 GB laptop tier" warning that fired on the model the app recommends for a 128 GiB
machine, and it pushed the only action below the fold (first-run vision FR-R1-3, FR-R1-4, FR-S-2).

## How the Configure dialog names a field

One function, `providerFieldName` in `DefaultProviderSetupForm.tsx`, names a key for its label, its
placeholder fallback, the reveal toggle ("Show Anthropic API key") and the "is required" error, so the four
can never disagree. The names are sentence case ("Anthropic API key", "Azure OpenAI deployment name",
"Llama Server context size"), the same as the custom provider form's "API key" and every other label in the
app. The curated names in `utils/configUtils.ts` stay Title Case in the table, and each reader sentence-cases
them on the way out: this function and the Configuration editor both use `sentenceCase` from
`settings/config/configLabels.ts`, which keeps acronyms and product names. A key with no curated name falls
back to its role ("API key", "API host") or to the key without the provider's prefix ("Access key ID").

## A disabled field dims once

With Lead/worker mode off, the two model selects and the three number fields are disabled, and each control
dims itself to the one 50% step every disabled control takes (the Select trigger's `opacity-50`, the Input's
`disabled:opacity-50`). The group around them used to dim to 50% as well, so a field was painted at 25%: the
placeholder was barely legible and, in dark, a number field's fill faded into the dialog. `.br-model-fields`
in `model-dialogs.css` now carries no opacity, and the labels and InfoTips stay at full ink so a person can
still read what a field is for. `LeadWorkerSettings.fill.browser.test.tsx` reads the product of opacities in
Chromium (0.5 for every field, 1 for every label, light and dark) and puts the old rule back to prove it
would see the defect.

## The page band

`ProviderSettingsPage.tsx` (`#/configure-providers`, and `#/welcome` on first run) is one 44px `PageHeader`
band, as every other page is: Back as the band's 32px icon action, a 14px title ("Provider configuration",
or "Choose a provider" on first run) whose InfoTip says where keys are kept (first run adds "You can
switch providers any time in Settings."), and Local, Institutional and Public on the title's line. The
catalog draws its tabs into the band's slot (`ProviderCatalog`'s `tabsHost`, a portal; omitted, the list
sits at the top of the catalog). There is no paragraph under the title.

The first-run page (`ProviderGuard`, what a person sees on a fresh install) is the same page with two
differences and no others (R-34, first-run vision round 1, FR-R1-2 and FR-S-4). Its band leads with the
wordmark, the logo (R-29), at the sidebar's 20px scale and at rest (no entrance animation); and its
trailing edge holds the two ways out, "Explore Biorouter first" and "Open Crew", as ghost `sm` buttons
in one ink, said once, where the page has Back. The band is the shared band's markup written out in
`ProviderGuard`, because `PageHeader` has no slot for a leading mark, and the root carries
`.br-provider-page` so the two rules below hold for it too. It used to be a 235px hero that never
scrolled (a 36px wordmark, a 24px "Welcome to Biorouter", a two-line paragraph, two arrow links in two
inks, the skip repeated at the foot) on the recessed grey ground. Its loader (the frame or two while the
provider is read) is now empty and on the same page ground: it used to draw the boot splash's 84px BR
monogram on the muted grey, a second logo and a second ground before the wordmark (FR-R1-7, R-29). The
splash itself (`index.html`) now draws the wordmark at rest on the page canvas, with no mark and no
entrance (FR-R1-7, round 3; design.md, "The boot splash"), so the frame before this page is the same logo on
the same ground. On a browser served by `biorouter serve` the band is
the wordmark and "Get started", over the host-managed panel, which carries the reason itself.

The catalog opens once, at rest (FR-R1-1). With `holdUntilSettled` (both pages pass it) it draws a quiet
skeleton at row height, with the three tab labels and none selected, until the provider list, the bound
provider, the API section's disclosure copy and, when it can decide the tab (no route hint, no bound
provider), the coding-agent probe are in; it waits for the probe and the copy at most
`AGENT_PROBE_WAIT_MS` (2.5 s). The copy is fetched when the catalog mounts, whichever tab it opens on: the
note used to fetch it when it mounted, so Public grew its two-line disclosure 300 ms after it was drawn and
moved every row under the pointer (FR-R2-1). A tab the person picks during the hold is kept. It then freezes that tab, so a late probe or a refreshed list never
switches it; a person's click still does. `loading` alone draws the skeleton too, so a list that is still
in flight never reads as "Nothing here yet.". First run first painted Local with that line, filled it a
second later and switched itself to Public when the probe answered.

Both routes are **outside the app shell** (`routeOwnsTopBand`), so they have no sidebar and none of the
shell's top-band bookkeeping, and the page does that bookkeeping itself. The band starts at **y=0** and is
the window-drag region, as every band is (principle 1); there is no spacer under the strip. It used to
start at y=32 under a separate 32px strip, which made 76px of chrome where Settings has 44 (vision round
2, V4-14). Three rules in `providers.css` hold it:

- `App.tsx`'s 32px `.titlebar-drag-region` is `position: fixed` at z-index 50, so over the band's top 32px
  it would take the clicks meant for Back, the InfoTip and the tabs. It takes no pointer events while the
  page is mounted (`body:has(.br-provider-page) .titlebar-drag-region`), which is what
  `body.biorouter-chat-route-active` does for a route inside the shell, keyed on the page instead of a
  body class that `AppLayout` owns. The strip's drag rect comes earlier in tree order than the band's
  and Electron folds drag rects in tree order, so the band's `no-drag` controls still subtract from it.
- The band's first control starts at the titlebar reserve, `--br-provider-band-lead`, which the page sets
  from `getTitlebarControlInset` (100px on macOS, where the traffic lights sit over the band's left end,
  16px elsewhere and in a browser). The chat header's collapsed sidebar starts its controls at the same
  x. It is padding rather than margin: nothing earlier in tree order is `no-drag` on this route.
- The band keeps the shared `--sidebar` paint, the same recessed ground as Settings' band. It used to be
  overridden to transparent, so the page showed an unpainted white bar.

`providerBand.browser.test.tsx` (Chromium) measures all of it, and strips each rule to see the defect.
The mouse's Back button still leaves the page: `PageHeader` calls the shared `useMouseBackButton` for any
band that has an `onBack`, so the page carries no listener of its own.

First run's setup forms sit on the rows' 12px text inset (an open row's panel, `.br-provider-panel`, and
the detector's `.br-provider-inset`), so a field starts where a row's name does and ends where a row's button
does.

A section is a `SettingSection`: a caps heading with, where there is a reason to give, an InfoTip beside
it (`OrderedProviderSection.help`: the agents' subscription billing, a gateway with no named
institution). An institution's own note only lists its rows, so it gets no tip. "Don't see your
institution?" is one quiet line under the Institutional tab with the answer in an InfoTip.

The taxonomy strings themselves are the design of record in
[`docs/security/privacy-tiers.md`](../security/privacy-tiers.md) §14.5 and are pinned by
`ProviderCatalog.privacy.test.tsx`.

## First run's boxes

First run stands on the page ground, as the provider page does, so a box rests where a field would
there: a box inside an open row (a progress line, the embedded terminal) is `.br-first-run-box`, the
rest fill (`--background-fill`) with a transparent 1px edge at the element radius. A box that holds a
control also carries `.br-fill-surface`, so the button in it rests on the page ground and never sits fill
on fill, and a progress track in it rests one step in by the shared rule (`.br-progress-track` reads the
track ground a filled box hands its children). The host-managed panel on a browser surface is
`.br-first-run-card`, the same fill at the container radius. No first-run box draws an outline. Until
R-34 the page was `--background-muted`, where the rest fill is the ground itself, and it needed the
recessed-ground class and two rules of its own; they went with the muted ground.

## Ordering rules

All of them live in `providerOrdering.ts`. None of them is a `switch` on a provider name
in the catalog component.

- **Local**: `llamacpp`, `ollama`, then the rest by display name.
- **Institutional**: grouped by institution, sections sorted by the institution's own
  label. A provider covered by two institutions appears under **both** — that is what a
  covering *set* means, and hiding it from one would tell that institution's users their
  gateway is not theirs. A private, non-local provider with no institution falls into
  "Unaffiliated private gateways" rather than disappearing.
- **Public**: the two coding-agent providers first, in the order
  `codingAgentStatus.ts` declares (`CODING_AGENT_ORDER`); then `anthropic`, `openai`,
  `google` pinned; then every remaining public provider — built-in, declarative and
  custom alike — by **display name**, case-insensitively; then "Add custom provider".

The set of AI-agent provider ids is imported from `codingAgentStatus.ts`, which already
owns the map of each agent's config key. `providerOrdering.test.ts` asserts the two agree,
so a third coding agent cannot end up with a status pill and no section.

## Naming an institution

⚠ **An institution's name comes only from the daemon.** There is no literal `"UCSF"`
anywhere in the catalog. `institutionsForProvider` reads two sources, in this order:

1. `row.affiliation` — **instance-resolved** by the daemon from a live provider
   (`ProviderAffiliation::of`). Always wins. A Versa module repointed at another host has
   already lost Private *and* `ucsf` there, and the catalog loses the group with it.
2. `row.metadata.institutions` — the **type-level** claim: where the provider *ships*
   pointed. Read only when the daemon resolved nothing at all, which
   `resolved_tier === null` reports exactly.

The second source exists because of a measured gap: `GET /config/providers` resolves both
axes **only for a configured provider** (`resolve_provider_axes` in
`crates/biorouter-server/src/routes/config_management.rs` — an unconfigured provider has
no keys, cannot be constructed, and must not have every provider module's constructor run
on a plain GET). So on a machine where nothing is set up yet — first-run onboarding, the
screen where naming the institution matters most — **every** row's `affiliation` is
`null`, and without the fallback the institutional tab would be one anonymous pile.

⚠ The fallback is never a *preference*. Reading the shipped metadata first would keep an
institution's name on a gateway the daemon has already demoted, which is the precise
defect `ProviderAffiliation`'s own doc warns a name-keyed table would cause.

### The `versa_bedrock` backend note

`versa_bedrock` was reported as serving `affiliation: null` while `versa_azure` served
UCSF, and diagnosed as a missing `affiliation()` implementation. **It is not.**
`versa_bedrock.rs` has had one since DR-26, identical to `versa_azure`'s and keyed on the
same resolved endpoint through the same `ucsf_gateway_affiliation` host check. The null on
the wire was the `is_configured` gate above: the machine had a Versa Azure key and no
Bedrock credentials.

What shipped instead is `ProviderMetadata::institutions` — the type-level institution
claim, declared with `.with_institution("ucsf")` in each Versa module beside the existing
`.with_tier(ProviderTier::Private)`, with its display name looked up in the same
institution registry every warning and badge reads. Same warning as `tier`'s applies: it
is not an enforcement input and no badge may hang on it.

## The default tab

Computed from state, never hardcoded — `defaultCatalogTab` in `ProviderCatalog.tsx`:

1. A route hint (`#/configure-providers?tab=public`) wins outright, **even at an empty
   tab**: a CTA that says "set up an institutional model" must land there and show that
   there is nothing yet.
2. Otherwise, the tab holding the provider in `BIOROUTER_PROVIDER`.
3. Otherwise, **Public** when a coding-agent CLI reports `signed_in_subscription` — the
   machine already has a plan ready to use, and that path needs no key and no download.
4. Otherwise **Local**.
5. …unless the tab chosen by 2–4 is empty, in which case the first tab that is not.

Rule 2 outranks rule 3 deliberately: a rule that checked the agent first would drag a user
who works on Versa every day onto Public because a `claude` binary happens to be signed in
on their machine.

## Onboarding mode

Each row opens **in place** into that provider's own setup UI, one at a time, with Llama
Server open by default.

The setup bodies are not duplicated. Each one is a plain column the catalog opens under its
row (the standalone card chrome, `OnboardingCardShell` and its section label, and the unused
`CodingAgentInlineCard` were deleted, FR-S-12), and the coding agents' whole state machine is
`onboarding/codingAgentControls.tsx`, which the catalog and the configure dialog's recovery
panel both render (its suite is `codingAgentControls.test.tsx`). A fix to the
`signed_in_with_api_key` copy therefore cannot land on one screen and not the other. The
words of first run are in `onboarding/copy.ts`, which the tests import.

- **An institutional gateway's row is the selector** (FR-R1-6, FR-S-6). A Versa row that is not
  set up opens `InstitutionalSetupCard` in place for its own gateway (`INSTITUTIONAL_SETUP_FLAVOR`,
  the form's own table of the rows it sets up); a configured one is chosen on a click like any
  configured row. The tab used to list the two rows (which opened the Configure dialog) and, under
  them, a second unheaded form whose segmented switch chose between the same two. A form written
  for UCSF's Versa endpoints must never open from another institution's gateway.
- The **paste-a-key detector** (`CommercialSetupCard`) sits at the top of the API-provider
  section: it is the fastest path for someone who already holds a key and does not know
  which of twenty-odd rows is theirs. It is a labelled field ("API key", with what it detects
  in the label's InfoTip) and a "Detect provider" button that stays secondary until there is
  a key (FR-S-5); a failure is the Note alone.
- **A coding-agent row's second line is short** ("Your Claude subscription · no API key", from
  `AGENT_SUBSCRIPTION`); the daemon's long sentence, which the row cut mid-word, stays its
  accessible description (FR-R1-8), and is no longer a hover tooltip over a line that fits (FR-R2-7). Each auth state says one instruction with its
  reason in an InfoTip beside it, then the copyable command and the actions; a ready agent's
  plan and account sit on its version line (FR-S-3).
- Every status line on first run is the one `StatusPill` on the shared `StatusDot` (FR-S-10), and
  every panel action is on the 32px rung, never stretched to the panel's width (FR-R1-11, FR-S-11).

The coding-agent rows are accordions in **both** modes. Their content is a live auth status
and the guidance for it, which a modal cannot carry and which a settings user needs exactly
as much as a first-run user does.

⚠ `GET /coding_agents/status` **spawns both vendor CLIs**. The probe is mounted once, at
the catalog, and never per row; it runs on mount and on an explicit "Check again", never on
a timer. `ProviderCatalog.test.tsx` asserts the call count across a tab change and a minute
of fake timers.

### One row, one answer about "installed"

An agent row carries two statements from two routes: the status pill from
`/coding_agents/status`, and the **Configured** check from `is_configured` on
`/config/providers`. They used to disagree on one line — *"Codex · Not installed ✓ Configured"*
(the 2026-09-10 provider QA run, F6) — because `is_configured` asked only whether the command key
was saved. Three things now keep them together:

- **The daemon asks the same question twice.** `check_provider_configured`
  (`routes/utils.rs`) grants a coding agent `is_configured` only when
  `discovery::resolve_configured` finds its CLI, which is the lookup the status probe uses. A
  saved key naming a missing CLI is served `is_configured: false` **with**
  `unavailable_reason` — the sentence a turn would have failed with.
- **"Check again" re-reads the provider list.** The catalog reads that list once, when the page
  opens; a re-check that changed "installed" has changed `is_configured` too, so the hook's
  `onRechecked` hands it the page's `refreshProviders`. The mount probe does not — the page
  fetched the list at the same moment.
- **The model picker disables the row instead of dropping it.** `SwitchModelModal` lists
  every usable provider plus every row with an `unavailable_reason`, the latter as react-select's
  own `aria-disabled` option with the reason as its detail line — the private-chat pre-flight's
  shape, one level up. A dialog that opens *on* such a provider (the bound one) says why and
  will not submit.

Sign-in is not folded into `is_configured`: learning it spawns the CLI, and
`/config/providers` runs for every provider on every settings open. The pill carries it.

## Entering without a provider

"Explore Biorouter first" (a ghost button in the first-run band) writes `BIOROUTER_ONBOARDING_SKIPPED = true`, and the guard renders the application
when `BIOROUTER_PROVIDER` is empty **and** that key is true.

- It is a **config key**, not `localStorage`: a user who skipped setup yesterday must not
  meet the wall again after a renderer reset.
- Choosing a provider **clears** it. Left set, it would suppress the first-run screen on a
  machine that later lost its provider — the one situation the wall exists for, silently
  disabled by a flag set months earlier for an unrelated reason.
- A skip that **fails to save** keeps the wall up. Letting the user through anyway would
  put them in an app whose first-run screen returns on the next launch with no explanation.
- The skip is **not offered on a browser-served surface**. There, the host owns the choice
  (SD-1), so "continue without a provider" would lead to a chat this tab can never
  configure — a second dead end wearing the clothes of an escape.

### Settings that belong to the host

On a browser-served surface, the provider form shows the settings that decide where a provider
sends its requests and key (host, endpoint, base path, region, CA or client certificate path, a
coding agent's command) read only, with a note that they are changed with `biorouter configure`
on the host computer. The save leaves them out, and a required one the host has not set says so
instead of "is required". A custom provider's form asks for the key again before a browser moves
its saved key to a new URL, the one such move the daemon allows there. The daemon's side of this
is [SD-1](../deployment/serve-decisions.md#sd-1--a-browser-session-cannot-change-its-model-or-provider-and-that-is-the-point).

### The in-app no-provider state

This is what makes "get in first" honest rather than a trapdoor. `composerNoProvider.ts`
holds the decision and the copy, and both consumers read it:

- the composer's model chip reads **"Choose a model"** and opens the catalog
  (`setView('ConfigureProviders')`) instead of a dropdown whose items — "Change Model",
  "Lead/Worker Settings" — are adjustments to a model that does not exist;
- the composer shows one line above the input, **"No model yet — choose a provider to start
  chatting"**, with the link, and refuses to send by button *and* by Enter.

⚠ `hasNoModelConfigured` requires `modelConfigStatus === 'ready'`, and that is the whole
function. `currentProvider` is `null` until the config has been read, so the obvious
`!provider` check announces "no model yet" over every correctly configured install for the
first frames after launch — and disables Send there. An unknown status says nothing and
blocks nothing.

Everything that does not need a model — Home, sessions, Knowledge, settings, extensions —
renders normally.

## Files

| File | Role |
|---|---|
| `ui/desktop/src/components/settings/providers/ProviderCatalog.tsx` | The tabbed catalog; both modes; `defaultCatalogTab` |
| `ui/desktop/src/components/settings/providers/providerOrdering.ts` | Groups, sections, ordering, `institutionsForProvider` |
| `ui/desktop/src/components/settings/providers/subcomponents/ProviderCard.tsx` | One row definition, click-to-configure or accordion |
| `ui/desktop/src/components/settings/providers/copy.ts`, `providers.css` | The catalog's words (tests import them) and its row geometry |
| `ui/desktop/src/components/settings/providers/ProviderSettingsPage.tsx` | The `configure-providers` and `welcome` routes |
| `ui/desktop/src/components/ProviderGuard.tsx` | The first-run page (its band, the catalog under it) and the way past it |
| `ui/desktop/src/components/onboarding/copy.ts` | The words of first run (tests import them) |
| `ui/desktop/src/components/onboarding/StatusPill.tsx` | Every first-run status line, on the shared `StatusDot` |
| `ui/desktop/src/components/settings/models/localModelFacts.ts` | A local model's state word and facts line, for Settings and the Llama Server panel |
| `ui/desktop/src/components/onboarding/onboarding.css` | The first-run boxes and the band's wordmark |
| `ui/desktop/src/components/onboarding/codingAgentControls.tsx` | The coding-agent probe, guidance and connect write |
| `ui/desktop/src/components/composerNoProvider.ts` | The no-model decision and copy |
| `crates/biorouter/src/providers/base.rs` | `ProviderMetadata::institutions`, `with_institution` |

Tests: `ProviderCatalog.test.tsx`, `ProviderCatalog.privacy.test.tsx`, `firstRunFill.browser.test.tsx` and
`providerBand.browser.test.tsx` (Chromium),
`ProviderCatalog.browserSurface.test.tsx`, `providerOrdering.test.ts`,
`ProviderGuard.test.tsx`, `ProviderGuard.browserSurface.test.tsx`,
`composerNoProvider.test.ts`, `ChatInput.noProvider.test.tsx`,
`ModelsBottomBar.noProvider.test.tsx`, `SwitchModelModal.unavailable.test.tsx`; and on the Rust
side `cargo test -p biorouter --lib -- providers::versa providers::base::type_level_institution`
and `cargo test -p biorouter-server --lib -- routes::utils routes::config_management`.
⚠ Several filters go **after** `--`: cargo's own `TESTNAME` is a single positional, so
`cargo test … routes::utils routes::config_management` stops at
`unexpected argument 'routes::config_management'` without running anything.

⚠ **Radix's `TabsTrigger` activates on `mousedown`, not on a synthetic `click`.** A
`fireEvent.click` alone leaves the panel untouched — and because an unopened panel renders
nothing, every assertion after it quietly tests whichever tab happened to be open. Every
suite here uses a `clickTab` helper that fires both.

## Related documentation

- [Privacy tiers](../security/privacy-tiers.md) — §14.5 owns the three group labels and their notes; SD-1 owns the browser surface's behaviour.
- [Settings visual vocabulary](settings-visual-vocabulary.md) — the row, section and InfoTip anatomy the catalog and the Models tab now share.
- [Coding-agent providers](../providers/coding-agents/README.md) — what `claude_code` and `codex` are, and why both are `ProviderTier::Public`.
- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) — how to put this screen in front of you.
