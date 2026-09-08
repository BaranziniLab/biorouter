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

| Tab | Panel heading (§14.5) | What it holds |
|---|---|---|
| **Local** | Private · Local | Llama Server, then Ollama, then any other local provider by display name |
| **Institutional** | Private · Institutional | One section per institution, then "Unaffiliated private gateways", then the "Other institutions" explainer |
| **Public** | Public · Commercial | "AI agents · your subscription", then "API providers" |

The panel heading, its one-line note and the tier dot all come from
`getOrderedProviderGroups` in `providerOrdering.ts` — the same function
`components/knowledge/IngestPanel/IngestModelPicker.tsx` reads. The tab trigger's short
label (`Local` / `Institutional` / `Public`) is derived there too, so a trigger and the
panel it opens cannot come to name different things.

The taxonomy strings themselves are the design of record in
[`docs/security/privacy-tiers.md`](../security/privacy-tiers.md) §14.5 and are pinned by
`ProviderCatalog.privacy.test.tsx`.

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
  custom alike — by **display name**, case-insensitively; then "Add Custom Provider".

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
| `ui/desktop/src/components/settings/providers/ProviderSettingsPage.tsx` | The `configure-providers` and `welcome` routes |
| `ui/desktop/src/components/ProviderGuard.tsx` | The first-run wall, and the way past it |
| `ui/desktop/src/components/onboarding/OnboardingCardShell.tsx` | `card` / `bare` chrome around one body |
| `ui/desktop/src/components/onboarding/codingAgentControls.tsx` | The coding-agent probe, guidance and connect write |
| `ui/desktop/src/components/composerNoProvider.ts` | The no-model decision and copy |
| `crates/biorouter/src/providers/base.rs` | `ProviderMetadata::institutions`, `with_institution` |

Tests: `ProviderCatalog.test.tsx`, `ProviderCatalog.privacy.test.tsx`,
`ProviderCatalog.browserSurface.test.tsx`, `providerOrdering.test.ts`,
`ProviderGuard.test.tsx`, `ProviderGuard.browserSurface.test.tsx`,
`composerNoProvider.test.ts`, `ChatInput.noProvider.test.tsx`,
`ModelsBottomBar.noProvider.test.tsx`; and on the Rust side
`cargo test -p biorouter --lib -- providers::versa providers::base::type_level_institution`.

⚠ **Radix's `TabsTrigger` activates on `mousedown`, not on a synthetic `click`.** A
`fireEvent.click` alone leaves the panel untouched — and because an unopened panel renders
nothing, every assertion after it quietly tests whichever tab happened to be open. Every
suite here uses a `clickTab` helper that fires both.

## Related documentation

- [Privacy tiers](../security/privacy-tiers.md) — §14.5 owns the three group labels and their notes; SD-1 owns the browser surface's behaviour.
- [Coding-agent providers](../providers/coding-agents/README.md) — what `claude_code` and `codex` are, and why both are `ProviderTier::Public`.
- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) — how to put this screen in front of you.
