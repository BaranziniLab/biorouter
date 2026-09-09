import type { ProviderDetails } from '../../../api';
import {
  readProviderAffiliation,
  type AffiliationInstitution,
} from '../../privacy/providerAffiliation';
import { CODING_AGENT_ORDER } from '../../onboarding/codingAgentStatus';

/**
 * The three public providers a user is most likely to already hold a key for,
 * pinned to the top of the API section in this order.
 *
 * ⚠ This is a *habit* rule, not a taxonomy rule — everything here is Public and
 * carries the same disclosure. It exists because an alphabetical list buries
 * `openai` behind `openrouter` and `anthropic` behind nothing at all, and the
 * three names below are the ones the catalog is opened to find.
 */
export const PINNED_PUBLIC_PROVIDERS: readonly string[] = ['anthropic', 'openai', 'google'];

/**
 * The two local providers, in the order the product ranks them (Llama Server is
 * the zero-setup one and is offered first). Pinned by `providerOrdering.test.ts`.
 */
export const PINNED_LOCAL_PROVIDERS: readonly string[] = ['llamacpp', 'ollama'];

/**
 * The provider ids that drive a vendor CLI on the user's own subscription.
 *
 * ⚠ **Derived, never re-listed.** `codingAgentStatus.ts` owns both the set (the
 * keys of `AGENT_COMMAND_CONFIG`) and the order, and its own test asserts the
 * two agree. A second list here is how a third coding agent ends up with a
 * status pill and no section, or a section and no pill.
 */
export const AI_AGENT_PROVIDER_IDS: readonly string[] = CODING_AGENT_ORDER;

export type ProviderGroupKey = 'institutional' | 'local' | 'commercial';

/**
 * One ordered block inside a group's panel.
 *
 * A group is the *tab*; a section is a heading inside it. The institutional tab
 * has one section per institution (plus a fallback), the public tab has exactly
 * two (AI agents, then API providers), and the local tab has one unnamed
 * section — its heading is the group's own.
 */
export interface OrderedProviderSection {
  /**
   * `'agents'` | `'api'` | `'unaffiliated'` | `'local'` | an institution id.
   *
   * An institution id is a daemon-supplied slug, so a key here is not a closed
   * set and must never be `switch`ed on exhaustively.
   */
  key: string;
  /**
   * The heading, or `null` when the group's own heading is the only one — which
   * is what stops the local tab printing "Private · Local" twice.
   *
   * ⚠ An institution's label is the daemon's `display_name`, falling back to its
   * bare id. Never a literal: a renderer that hardcoded "UCSF" would print it for
   * an institution the registry renamed, and print nothing for the second one.
   */
  label: string | null;
  /** One line under the heading, when the section needs its own reason. */
  note?: string;
  providers: ProviderDetails[];
}

export interface OrderedProviderGroup {
  key: ProviderGroupKey;
  /**
   * §14.5: the group heading names the privacy taxonomy and the hosting
   * taxonomy in the same words, in the same place. A UCSF user whose Azure
   * OpenAI account is provisioned and paid for by UCSF IT reads
   * "Institutional"; under this design that account is **Public**, and the old
   * three headings never said so.
   *
   * ⚠ Consumed by THREE surfaces and hardcoded by none. `ProviderCatalog` prints
   * it as the tab's panel heading, the tab trigger takes its short form from
   * {@link tabLabel}, and `IngestModelPicker` has always read it. Do not inline
   * any of them back.
   */
  label: string;
  /**
   * The short word for the tab trigger — the hosting half of {@link label},
   * with the tier carried by the dot beside it and spelled out again on the
   * panel the trigger opens. Derived here rather than at the trigger so the two
   * cannot drift into naming different things.
   */
  tabLabel: string;
  /**
   * The one line of card copy §14.5 asks for — *why* this group has the tier it
   * has. Rendered under the heading, once per section, rather than per card:
   * the reason is a property of the group's classification rule, not of any one
   * vendor.
   */
  note: string;
  accentClassName: string;
  /**
   * Every provider in the group, flattened in section order.
   *
   * ⚠ Kept for consumers that want a flat list — `IngestModelPicker` renders one
   * `<optgroup>` per group and has no use for sub-headings. It is a *view* of
   * {@link sections}, derived here rather than recomputed there, so the two can
   * never disagree about membership or order.
   */
  providers: ProviderDetails[];
  sections: OrderedProviderSection[];
}

function displayName(provider: ProviderDetails): string {
  return provider.metadata.display_name || provider.name;
}

/** Case-insensitive by display name, with the provider id as the tiebreak. */
function byDisplayName(a: ProviderDetails, b: ProviderDetails): number {
  const byName = displayName(a).localeCompare(displayName(b), undefined, { sensitivity: 'base' });
  return byName !== 0 ? byName : a.name.localeCompare(b.name);
}

/**
 * `pinned` first, in the order given, then everything else by display name.
 *
 * The tail is alphabetical **by display name, not by id**, because the id is
 * not what the row prints: `custom_deepseek` and `zai` sort nowhere near
 * "DeepSeek" and "z.ai" on screen, and a list whose order the eye cannot follow
 * is the failure this replaces.
 */
function pinnedThenAlphabetical(
  providers: ProviderDetails[],
  pinned: readonly string[]
): ProviderDetails[] {
  const head = pinned
    .map((name) => providers.find((provider) => provider.name === name))
    .filter((provider): provider is ProviderDetails => provider !== undefined);
  const tail = providers.filter((provider) => !pinned.includes(provider.name)).sort(byDisplayName);
  return [...head, ...tail];
}

/**
 * The institutions a private, non-local provider is covered by — as this
 * catalog is allowed to read them.
 *
 * Two sources, and the precedence between them is the whole point:
 *
 * 1. `row.affiliation` is **instance-resolved** by the daemon
 *    (`ProviderAffiliation::of`, off a live provider) and always wins. A Versa
 *    module repointed at another host has already lost Private *and* `ucsf`
 *    there, and this catalog must lose the group with it.
 * 2. `row.metadata.institutions` is the **type-level** claim — where the
 *    provider *ships* pointed. It is read **only** when the daemon resolved
 *    nothing at all, which `resolved_tier === null` reports exactly:
 *    `GET /config/providers` resolves both axes together and only for a
 *    *configured* provider, so on a machine where nothing is set up yet every
 *    row's affiliation is `null`. That is precisely first-run onboarding — the
 *    screen where naming the institution matters most — and without the
 *    fallback the institutional tab would be one anonymous "unaffiliated" pile.
 *
 * ⚠ **Fallback, never preference.** Reading the metadata first would keep the
 * institution's name on a repointed instance the daemon has already demoted,
 * which is the exact defect `ProviderAffiliation`'s doc warns a name-keyed table
 * would cause. And nothing here is a privacy claim: the tab is a heading, the
 * badge that asserts a bound instance's tier reads elsewhere.
 */
export function institutionsForProvider(provider: ProviderDetails): AffiliationInstitution[] {
  const resolved = readProviderAffiliation(provider);
  if (resolved) {
    return resolved.kind === 'institutions' ? resolved.institutions : [];
  }
  // The daemon answered with an instance ("public", or one of the other two
  // kinds) — respect it, even though it named no institution.
  if (provider.resolved_tier != null) return [];
  return provider.metadata.institutions ?? [];
}

/** The registry's name for an institution, or its bare id — never nothing. */
function institutionLabel(institution: AffiliationInstitution): string {
  return institution.display_name?.trim() || institution.id;
}

/**
 * Grouping is the backend's answer, never a list kept here. `runs_locally` is
 * the display-only fact that splits the private tier into the two sections this
 * catalog has always had. A renderer-side copy of either field is a second source
 * of truth that drifts silently the moment a provider is added, renamed, or
 * re-pointed.
 *
 * ⚠ `metadata.tier` is the *type-level* claim — the tier computed from the
 * endpoint a provider ships with — NOT the tier of the instance actually bound
 * to a session. `GET /config/providers` serves `ProviderMetadata` verbatim, and
 * for a built-in that struct is static, so an `ollama` re-pointed off this
 * machine by `OLLAMA_HOST` still arrives here as `private` while its instance
 * `Provider::tier()` resolves `public`. The two can only ever disagree in that
 * direction, which is why this module may read it.
 *
 * ⚠ The headings below say the word "Private", which the old ones
 * ("Local Models" / "Institutional Models") only implied — so the residual
 * inaccuracy is louder than it was, and it is written down here rather than
 * discovered later. Exactly one configuration reaches it: a genuine built-in
 * whose endpoint was re-pointed off this machine by an environment variable
 * (`OLLAMA_HOST`) still ships `tier: 'private'` in its type-level metadata and
 * therefore still sits under "Private · Local". A provider *declared* public by
 * the daemon is grouped Commercial, which `providerOrdering.test.ts` pins.
 *
 * It is still **not** a licence to hang a `PrivacyBadge` on this field. A badge
 * asserts the tier of a *bound* instance; hung here it would read Private in
 * exactly the demotion case the tier exists to catch. The composer chip's badge
 * therefore reads the session's own ratcheted classification, never this. See
 * the `tier` field's doc comment in `crates/biorouter/src/providers/base.rs`.
 */
function classifyProvider(provider: ProviderDetails): ProviderGroupKey {
  if (provider.metadata.tier !== 'private') {
    return 'commercial';
  }
  return provider.metadata.runs_locally ? 'local' : 'institutional';
}

/**
 * The institutional tab's sections: one per institution, then the honest
 * fallback.
 *
 * ⚠ **A provider covered by two institutions appears under both.** That is what
 * `ProviderAffiliation.institutions` being a *set* means — an endpoint two
 * institutions' agreements both cover is reachable from either — and hiding it
 * from one of them would tell a user of that institution their gateway is not
 * theirs.
 */
function institutionalSections(providers: ProviderDetails[]): OrderedProviderSection[] {
  const byInstitution = new Map<string, { label: string; providers: ProviderDetails[] }>();
  const unaffiliated: ProviderDetails[] = [];

  for (const provider of providers) {
    const institutions = institutionsForProvider(provider);
    if (institutions.length === 0) {
      unaffiliated.push(provider);
      continue;
    }
    for (const institution of institutions) {
      const entry = byInstitution.get(institution.id) ?? {
        label: institutionLabel(institution),
        providers: [],
      };
      entry.providers.push(provider);
      byInstitution.set(institution.id, entry);
    }
  }

  const sections: OrderedProviderSection[] = [...byInstitution.entries()]
    .sort(([, a], [, b]) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base' }))
    .map(([id, entry]) => {
      // ⚠ SORT FIRST. The note is derived from this list, and an object literal
      // evaluates its properties in order — deriving the note before the sort
      // printed the gateways in whatever order the daemon happened to serve
      // them, so the sub-line and the rows below it disagreed. Measured in the
      // dev GUI: "Versa API Bedrock · Versa API Azure" above rows reading
      // Azure, then Bedrock.
      const providers = [...entry.providers].sort(byDisplayName);
      return {
        key: id,
        label: entry.label,
        // Built from the providers' own display names, so the sub-line names
        // what this institution actually hosts without a literal here naming a
        // gateway.
        note: providers.map((provider) => displayName(provider)).join(' · '),
        providers,
      };
    });

  if (unaffiliated.length > 0) {
    sections.push({
      key: 'unaffiliated',
      label: 'Unaffiliated private gateways',
      note: 'Private because Biorouter recognizes the endpoint, but no institution is named for it.',
      providers: [...unaffiliated].sort(byDisplayName),
    });
  }

  return sections;
}

/**
 * The public tab's two sections, in this order and never the other way round.
 *
 * AI agents first because they are the commonest path on a machine that already
 * pays for a coding-agent plan: they need no key at all, so offering them before
 * the section that asks for a secret is the cheaper question asked first. The
 * same reasoning already put `CodingAgentInlineCard` above `CommercialSetupCard`
 * in the old onboarding stack.
 */
function commercialSections(providers: ProviderDetails[]): OrderedProviderSection[] {
  const agents = AI_AGENT_PROVIDER_IDS.map((id) =>
    providers.find((provider) => provider.name === id)
  ).filter((provider): provider is ProviderDetails => provider !== undefined);
  const rest = providers.filter((provider) => !AI_AGENT_PROVIDER_IDS.includes(provider.name));

  const sections: OrderedProviderSection[] = [];
  if (agents.length > 0) {
    sections.push({
      key: 'agents',
      label: 'AI agents · your subscription',
      note: 'Biorouter drives a vendor CLI already installed and signed in on this machine; turns bill to that plan, not to an API key.',
      providers: agents,
    });
  }
  sections.push({
    key: 'api',
    label: 'API providers',
    providers: pinnedThenAlphabetical(rest, PINNED_PUBLIC_PROVIDERS),
  });
  return sections;
}

/**
 * Every provider the daemon serves is shown. A hide-list used to live here for
 * `claude-code`, `codex` and `cursor-agent` — soft-disabled shims that drove
 * another vendor's installed CLI as a subprocess — and it is worth being precise
 * about why it has not come back now that two of them have. `claude_code` and
 * `codex` are registered providers the daemon serves on purpose, so a filter
 * here would mean this catalog silently contradicting the daemon it renders.
 * They reach a *group* the same way every other provider does, from the
 * `metadata.tier` and `runs_locally` the daemon sends over the wire; the only
 * thing this module recognises about them is which **section** of the public tab
 * they belong in, and that set is imported from `codingAgentStatus.ts` rather
 * than written here. A new entry in this catalog is a decision made where the
 * provider is registered, and so is removing one.
 */
export function getOrderedProviderGroups(providers: ProviderDetails[]): OrderedProviderGroup[] {
  const grouped: Record<ProviderGroupKey, ProviderDetails[]> = {
    institutional: [],
    local: [],
    commercial: [],
  };

  for (const provider of providers) {
    grouped[classifyProvider(provider)].push(provider);
  }

  const sectionsByKey: Record<ProviderGroupKey, OrderedProviderSection[]> = {
    local: [
      {
        key: 'local',
        // The group's own heading is the only one this tab needs.
        label: null,
        providers: pinnedThenAlphabetical(grouped.local, PINNED_LOCAL_PROVIDERS),
      },
    ],
    institutional: institutionalSections(grouped.institutional),
    commercial: commercialSections(grouped.commercial),
  };

  const withSections = (
    group: Omit<OrderedProviderGroup, 'providers' | 'sections'>
  ): OrderedProviderGroup => {
    const sections = sectionsByKey[group.key];
    return {
      ...group,
      sections,
      // Flattened from the sections, so a provider under two institutions is
      // listed once per section here too — `IngestModelPicker` renders a
      // `<select>`, where a repeated option would be a repeated choice, so it
      // de-duplicates on read rather than this view lying about the sections.
      providers: sections.flatMap((section) => section.providers),
    };
  };

  return [
    withSections({
      key: 'local',
      label: 'Private · Local',
      tabLabel: 'Local',
      note: 'Private because inference runs on this machine. Nothing leaves it.',
      accentClassName: 'bg-background-success',
    }),
    withSections({
      key: 'institutional',
      label: 'Private · Institutional',
      tabLabel: 'Institutional',
      note: 'Private because Biorouter recognizes this institutional gateway endpoint.',
      accentClassName: 'bg-background-info',
    }),
    withSections({
      key: 'commercial',
      // ⚠ §14.5's note, and the wording matters. The obvious copy — "a direct
      // cloud account, even if your institution pays for it" — is NOT accurate
      // as shipped: `azure.rs` defaults `AZURE_OPENAI_ENDPOINT` to
      // `https://unified-api.ucsf.edu/general`, the same UCSF gateway
      // `versa_azure` uses. A name-keyed tier calls `azure_openai` Public even
      // when it in fact resolves to that gateway — conservative and fail-safe,
      // but the copy must not claim something the configuration contradicts.
      label: 'Public · Commercial',
      tabLabel: 'Public',
      note: "Public. Biorouter can't verify where this account's endpoint points, even one your institution pays for.",
      accentClassName: 'bg-background-warning',
    }),
  ];
}
