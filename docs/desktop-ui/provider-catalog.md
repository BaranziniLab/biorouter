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
over a one-line description (truncated, with a focusable tooltip for the rest), "Configured" as a success
dot and a word, and one action. There is no initial tile. The status and the action share one cell,
Crew's `meta` recipe (`.br-row-actions-slot`): "Configured" at rest on the right edge, and the action
(`Configure`, named for its row, as in "Configure Anthropic"; `Launch` for a configured provider on first
run) in its place while the row is hovered or holds focus, always shown without a hover pointer
(`.br-row-actions`), so the edge never moves. **First run reveals its actions the same way** (vision round
1, V4-29): it used to keep the button on screen, which made it the one list in the app with permanent row
actions on a hover-capable display. Its rows act on a click as Settings' do, so the button is the keyboard
path and a shortcut for the mouse, never the only way in.

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
(`--background-fill`, the strong step on the first-run ground), which leaves the button 1.10:1 off it
(measured in Chromium, `firstRunFill.browser.test.tsx`). On the first-run ground the row is already the
strong step, so a button in it hovers one step deeper, on `--background-strong`, as it does anywhere else
on that ground; the strong step there would be the row's own value and erase the button under the
pointer. Settings paints its hovered rows with the same idea (`settings.css`); the two are separate
rules because the catalog root and Settings' reading column each carry `.br-rows-flat`, and the
catalog's selector (`.br-provider-catalog.br-rows-flat`, two classes on one element) out-ranks Settings'
one-class paint by specificity, so the order the style sheets are bundled in does not matter.

## The page band

`ProviderSettingsPage.tsx` (`#/configure-providers`, and `#/welcome` on first run) is one 44px `PageHeader`
band, as every other page is: Back as the band's 32px icon action, a 14px title ("Provider configuration",
or "Choose a provider" on first run) whose InfoTip says where keys are kept (first run adds "You can
switch providers any time in Settings."), and Local, Institutional and Public on the title's line. The
catalog draws its tabs into the band's slot (`ProviderCatalog`'s `tabsHost`, a portal; omitted, as in
`ProviderGuard`, the list sits at the top of the catalog). There is no paragraph under the title.

Both routes are **outside the app shell** (`routeOwnsTopBand`), so `App.tsx`'s 32px titlebar drag strip
stays over the top of the page and the band starts below it, clear of the window controls
(`.br-provider-page-strip`). The shared band's `--sidebar` ground is there to meet the sidebar's
titlebar band, and this page has no sidebar, so the band is drawn on the page's own ground. The mouse's
Back button still leaves the page: `PageHeader` calls the shared `useMouseBackButton` for any band that
has an `onBack`, so the page carries no listener of its own.

First run's setup forms (`InstitutionalSetupCard`, `CommercialSetupCard`) sit on the rows' 12px text inset
(`.br-provider-inset`), so a field starts where a row's name does and ends where a row's button does.

A section is a `SettingSection`: a caps heading with, where there is a reason to give, an InfoTip beside
it (`OrderedProviderSection.help`: the agents' subscription billing, a gateway with no named
institution). An institution's own note only lists its rows, so it gets no tip. "Don't see your
institution?" is one quiet line under the Institutional tab with the answer in an InfoTip.

The taxonomy strings themselves are the design of record in
[`docs/security/privacy-tiers.md`](../security/privacy-tiers.md) §14.5 and are pinned by
`ProviderCatalog.privacy.test.tsx`.

## First run's ground and boxes

`ProviderGuard` paints first run on `--background-muted` (the boot splash hands off to it), where the
rest fill IS the ground (1.00:1): a field, a select or a secondary button resting on `--background-fill`
there would have no box at all. The page root is a recessed ground, so it carries the shared class
`.br-ground-recessed` (the grounds note in `main.css`, [shared-controls.md](shared-controls.md)): the
control ground steps to `--background-fill-strong` (1.10:1 off the page, design.md 9.11 rule 2), a filled
button hovers one step deeper again, and a neutral track (the Institutional gateway switch, a progress
bar) steps in, so every filled control inside follows with no class of its own and no rule of first run's.
The root also carries `.br-first-run-ground`, the name of the page itself, which the two rules that are
first run's own are scoped to (`onboarding/onboarding.css` and `providers/providers.css`).

A box inside an open row (the selected model's facts, a progress line, the embedded terminal) is
`.br-first-run-box`: a subtle fill with a transparent 1px edge that rests where a field would on the same
ground, so it is `--background-fill` on the canvas (the provider page in onboarding mode) and the strong
step on the first-run page. A box that holds a control also carries `.br-fill-surface`, so the button in
it rests on the page ground and never sits fill on fill. A progress track inside a box is the
strong step by the shared rule (`.br-progress-track` reads the track ground a filled box hands its
children); on the first-run page the box is itself the strong step, so the track rests one step deeper,
on `--background-strong`, through the one rule `onboarding.css` keeps. The standalone card chrome
(`OnboardingCardShell` with `chrome="card"`, which only tests render today) is `.br-first-run-card`, the
same fill at the container radius. No first-run box draws an outline; the header's rule is the scroll
edge of the body under it, and the first-run CTAs are on the control ladder (`size="lg"`, 36px).

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

The setup bodies are not duplicated. Each onboarding card gained an
`OnboardingCardShell` — `chrome="card"` for the standalone card, `chrome="bare"` for the
catalog row — around one body, and the coding-agent card's entire state machine moved into
`onboarding/codingAgentControls.tsx`, which the card and the catalog both render. A fix to
the `signed_in_with_api_key` copy therefore cannot land on one screen and not the other.

Two setup surfaces are not per-row:

- The **institutional** form (`InstitutionalSetupCard`) covers a family of gateways behind
  its own Azure/Bedrock toggle, so it renders once under an institution section — and only
  where it actually configures one of that section's providers, which the catalog asks the
  card itself via `INSTITUTIONAL_SETUP_PROVIDER_IDS`. A form written for UCSF's Versa
  endpoints must never appear under another institution's heading.
- The **paste-a-key detector** (`CommercialSetupCard`) sits at the top of the API-provider
  section: it is the fastest path for someone who already holds a key and does not know
  which of twenty-odd rows is theirs.

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

"Explore Biorouter first →" (under the header and repeated at the foot of the first-run
screen) writes `BIOROUTER_ONBOARDING_SKIPPED = true`, and the guard renders the application
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
| `ui/desktop/src/components/ProviderGuard.tsx` | The first-run wall, and the way past it |
| `ui/desktop/src/components/onboarding/OnboardingCardShell.tsx` | `card` / `bare` chrome around one body |
| `ui/desktop/src/components/onboarding/onboarding.css` | The first-run ground, the fill boxes and the progress track on them |
| `ui/desktop/src/components/onboarding/codingAgentControls.tsx` | The coding-agent probe, guidance and connect write |
| `ui/desktop/src/components/composerNoProvider.ts` | The no-model decision and copy |
| `crates/biorouter/src/providers/base.rs` | `ProviderMetadata::institutions`, `with_institution` |

Tests: `ProviderCatalog.test.tsx`, `ProviderCatalog.privacy.test.tsx`, `firstRunFill.browser.test.tsx` (Chromium),
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
