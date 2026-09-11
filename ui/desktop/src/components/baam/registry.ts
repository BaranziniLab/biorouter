// Types + loader for the BAAM marketplace registry consumed by the in-app
// Browse Skills / Browse Extensions modals. The registry is published at
// biorouter.ucsf.edu/registry.json (generated from baam.html). We fetch it live
// for freshness and fall back to a snapshot bundled with the app when offline.

import type { ProviderTier } from '../../api/types.gen';
import { nameToKey } from '../settings/extensions/utils';
import { rememberPrivateExtensionKeys } from './privateSet';
import fallback from './registry.fallback.json';
import { classifyExtension } from '../settings/extensions/extensionPrivacy';
import {
  EXTENSION_NOISE,
  rankEntries,
  SKILL_NOISE,
  Weight,
  type SearchField,
  type SearchResult,
} from './search';

export interface RegistryExtension {
  id: string;
  name: string;
  organization: string;
  version: string;
  description: string;
  tags: string[];
  github: string;
  download: string;
  filename: string;
  license?: string;
  /**
   * Registry v2. All three are optional so a v1 document cached before the
   * upgrade still parses — a stale snapshot must degrade, never throw.
   *
   * `extension_name` is the name the installed config entry carries, reduced to
   * its lookup **key**: whitespace stripped and lowercased, so the card's
   * `CDWAgent` is published here as `cdwagent`. That substitution is the
   * contract, not an accident of the generator — treat it as one.
   *
   * The key is always `/^[a-z0-9_-]+$/`. That character set is exactly where
   * `config::extensions::name_to_key` (which `classify_extension` applies) and
   * `agents::extension_manager::normalize` (which the manager applies to the
   * installed config name) provably agree; outside it they diverge, so the
   * generator refuses the name rather than publishing a key the running app
   * would never produce.
   *
   * It exists because `id` is derived from the download filename and agrees with
   * the installed name only by luck (`spokeagent-0.4.1` already diverges), and a
   * suffix-stripping heuristic in a security path is right until it isn't. The
   * generator hard-fails on a private entry without one.
   */
  extension_name?: string;
  /** Absent on a v1 document; the generator emits it for every v2 entry. */
  privacy?: 'private' | 'public';
  /**
   * DR-26. Institution ids from `BaamRegistry.institutions`. Absent means
   * unconstrained — any private model may use it. Never an empty array: the
   * generator rejects one, because "nothing is permitted" and "no constraint"
   * must not share a spelling.
   */
  affiliation?: string[];
}

export type SkillCategory = 'Core' | 'Developer' | 'Biomedical';

export interface RegistrySkill {
  id: string;
  name: string;
  category: SkillCategory;
  type: string;
  description: string;
  tags: string[];
  keywords: string[];
  download: string;
  filename: string;
  license?: string;
}

export interface BaamRegistry {
  version: number;
  source: string;
  /**
   * Registry v2, DR-26. Institution id → display name. Cross-institutional
   * warning copy renders names from here rather than hardcoding them, so an
   * `affiliation` naming an id absent from this map has no name to render with
   * — which is why the generator treats that as a build failure. Optional so a
   * cached v1 document still parses.
   */
  institutions?: Record<string, string>;
  extensions: RegistryExtension[];
  skills: RegistrySkill[];
}

export const FALLBACK_REGISTRY = fallback as BaamRegistry;

/**
 * How long the renderer waits for the main process to answer `registry:fetch`.
 *
 * Deliberately just ABOVE the main process's own 10 s `AbortController` (see
 * `REGISTRY_FETCH_TIMEOUT_MS` in `utils/registryCache`), so a healthy main
 * process always answers first and this budget only ever fires when the IPC
 * channel itself is the thing that is stuck — a wedged handler, a main process
 * busy elsewhere, a window reloaded mid-call. Without it a hung main process
 * hangs the modal forever, showing "Loading catalog…" with no way out.
 */
export const REGISTRY_LOAD_BUDGET_MS = 11_000;

export interface RegistryLoad {
  registry: BaamRegistry;
  /**
   * True only when this document came off a successful fetch just now. A
   * last-good copy and the bundled snapshot are both `false`: the catalogue on
   * screen may be out of date either way, and the user is told so.
   */
  live: boolean;
  /**
   * When the document on screen was fetched, ISO-8601. Absent for the bundled
   * snapshot, which has no fetch to date.
   */
  fetchedAt?: string;
}

/** Resolves to `null` rather than rejecting once the budget is spent. */
function withBudget<T>(promise: Promise<T>): Promise<T | null> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(null), REGISTRY_LOAD_BUDGET_MS);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      () => {
        clearTimeout(timer);
        resolve(null);
      }
    );
  });
}

/**
 * Load the marketplace registry. Tries the live registry.json published with the
 * website; the main process falls back to its last-good copy and this falls back
 * again to the snapshot bundled with the app, so Browse always works offline.
 *
 * Every document that arrives — live or last-good — teaches the persisted
 * private set (§10.2). That is the "raises" half of the rule; nothing here can
 * lower a badge, because `effectivePrivacy` unions rather than reads.
 */
export async function loadRegistry(): Promise<RegistryLoad> {
  let result: Awaited<ReturnType<typeof window.electron.fetchRegistry>> | null = null;
  try {
    result = await withBudget(window.electron.fetchRegistry());
  } catch {
    // Never rejects. Every caller renders a modal, and a modal that throws on
    // load shows nothing at all — strictly worse than the bundled snapshot.
    result = null;
  }
  if (
    result &&
    'registry' in result &&
    result.registry &&
    Array.isArray(result.registry.skills) &&
    Array.isArray(result.registry.extensions)
  ) {
    const registry = withUsableEntriesOnly(result.registry);
    rememberPrivateExtensions(registry);
    return {
      registry,
      live: result.stale !== true,
      fetchedAt: result.fetchedAt,
    };
  }
  return { registry: FALLBACK_REGISTRY, live: false };
}

/** A usable catalogue entry is a plain object; everything else is not one. */
function isEntry(value: unknown): boolean {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Drop entries that are not objects, once, at the boundary.
 *
 * Every consumer downstream — the classifier, the two Browse lists, the
 * provenance line — reads fields off these entries, so a single `null` in the
 * array is a `TypeError` in whichever of them runs first. Sanitising here makes
 * all of them total at one seam instead of asking each to remember, and it is
 * the right seam: this is where an untrusted document stops being untrusted.
 *
 * The main process rejects such a document before caching it, so in practice
 * this only ever fires on a cache written by an older build. Both layers exist
 * because either alone would leave the other trusting its input.
 */
function withUsableEntriesOnly(registry: BaamRegistry): BaamRegistry {
  const extensions = registry.extensions.filter(isEntry);
  const skills = registry.skills.filter(isEntry);
  if (
    extensions.length === registry.extensions.length &&
    skills.length === registry.skills.length
  ) {
    return registry;
  }
  return { ...registry, extensions, skills };
}

/**
 * The lookup key an entry claims, or `null` if it claims none.
 *
 * `extension_name` ONLY, and by contract rather than by luck: `id` is derived
 * from the download filename and agrees with the installed name by coincidence
 * (`spokeagent-0.4.1` already diverges), and a suffix-stripping heuristic in a
 * security path is right until it isn't. The generator hard-fails on a private
 * entry without an `extension_name`, so a private entry always has one.
 */
function privacyKeyOf(entry: RegistryExtension): string | null {
  return entry.extension_name ? nameToKey(entry.extension_name) : null;
}

/**
 * Every key this document marks private.
 *
 * Total by construction, and deliberately so even though `loadRegistry`
 * sanitises: this is the classifier, it is called during React render, and a
 * classifier that throws on a hostile input has failed in the one direction it
 * exists to prevent. It must never depend on a caller having tidied up first.
 */
function privateKeysIn(registry: BaamRegistry): string[] {
  return (registry.extensions ?? [])
    .filter((e): e is RegistryExtension => isEntry(e) && e.privacy === 'private')
    .map(privacyKeyOf)
    .filter((k): k is string => k !== null);
}

function rememberPrivateExtensions(registry: BaamRegistry): void {
  rememberPrivateExtensionKeys(privateKeysIn(registry));
}

/**
 * **The union rule, as one function rather than four inline ORs** (§10.2):
 *
 * ```
 * private_set = PRIVATE_EXTENSIONS ∪ private(last_good_fetch)
 * ```
 *
 * A live document can RAISE an extension to private and can never lower one.
 * The natural implementation — trusting whatever the fetched document says — is
 * the bug this exists to prevent: a compromised or merely stale `registry.json`
 * would strip the private badge off `ucsfomopagent` on every machine that
 * fetched it, and the badge is the only warning a user gets before pasting a
 * cohort into a public model's chat.
 *
 * The three sources, all of which raise and none of which lower:
 *   1. `PRIVATE_EXTENSION_KEYS` — the compiled-in mirror of the Rust baseline.
 *   2. the persisted last-good set — every private key ever fetched.
 *   3. `registry`, the document in hand.
 *
 * ⚠ **The first two are `classifyExtension` and are delegated to it, not
 * restated here.** A card renders a badge from one and a provenance sentence
 * from the other; two copies of "is this private" on one row is a row that can
 * contradict itself, and only one of the two would be edited when the rule
 * changes.
 *
 * The third term is additive and, in practice, already redundant:
 * `loadRegistry` folds every private key in a fetched document into the learned
 * set before returning it, so for any registry that came from `loadRegistry`
 * this function and `classifyExtension` agree by construction. It is kept
 * because this function is also callable with a document that never went
 * through that path, and dropping it there would be the one direction the rule
 * forbids.
 */
export function effectivePrivacy(registry: BaamRegistry, name: string): ProviderTier {
  if (classifyExtension(name) === 'private') return 'private';
  return privateKeysIn(registry).includes(nameToKey(name)) ? 'private' : 'public';
}

/**
 * The marketplace entry for an installed extension, or `null` if the catalogue
 * does not list it.
 *
 * Looser than `privacyKeyOf` on purpose, and only ever consulted for prose. A
 * public entry carries no `extension_name` (the generator emits it for private
 * entries, where the key is load-bearing), so provenance falls back to the id
 * and the display name — `SPOKEAgent` reduces to the installed `spokeagent`
 * even though its id is `spokeagent-0.4.1`. Nothing about the privacy decision
 * goes through here.
 */
export function marketplaceEntryFor(
  registry: BaamRegistry,
  name: string
): RegistryExtension | null {
  const key = nameToKey(name);
  return (
    (registry.extensions ?? []).find(
      (e) =>
        isEntry(e) &&
        (privacyKeyOf(e) === key ||
          nameToKey(e.id ?? '') === key ||
          nameToKey(e.name ?? '') === key)
    ) ?? null
  );
}

/**
 * §13.5's strings, verbatim, as one total function.
 *
 * Two naming consequences, known rather than discovered:
 *
 *   - a hand-installed extension *named* `ucsfomopagent` inherits the private
 *     badge. Fail-closed, and fine.
 *   - a genuinely private extension renamed locally becomes public.
 *
 * ⚠ **The second is NOT "already the accepted direction under R11(ii)."** That
 * clause was here and is withdrawn by DR-19. R11(ii) rules that an extension
 * **not on BAAM** is Public — a statement about *unknown* extensions. A
 * **known** private extension losing its tier because a config entry was
 * renamed is a different fact, and it was never ruled on.
 *
 * Nor is it merely a badge. `classify_extension` stamps `Extension.tier`, which
 * Gates C (dispatch), E (discovery) and F (enable) all read, so the rename
 * removes **enforcement**: a public model becomes able to *call* the extension.
 * `config.yaml` is agent-writable with `text_editor`, and an extension is the
 * one object in this feature whose tier has no single lowering writer — a
 * session has `privacy::declassify`, a knowledge base has
 * `tier_user.rs::set_unlocked`, an extension has anyone who can rename a line.
 *
 * "Unavoidable because the install records no provenance at all" is the accurate
 * half, and is exactly why this is open question 28 rather than a fix here: the
 * repair is to derive the tier from install provenance (registry id, source URL,
 * hash) instead of from a mutable local string, and none of that is recorded to
 * be read. Two repairs are ruled OUT in advance — letting `config.yaml` declare
 * a tier (that is R11(i) inverted, and Task 8's OpenAPI-diff gate exists to
 * catch it) and widening the private set with aliases (a rename can pick any
 * string).
 */
export function extensionProvenance(registry: BaamRegistry, name: string): string {
  const listed = marketplaceEntryFor(registry, name);
  if (effectivePrivacy(registry, name) === 'private') {
    // ⚠ "Published" is a claim about the CATALOGUE, so it is only made when the
    // catalogue in hand backs it. The union has branches that do not: a key
    // learned from an earlier document that no longer lists it, and the
    // compiled baseline read against any document that does not. The previous
    // version of this asserted publication unconditionally, on the reasoning
    // that "every branch of the union is a marketplace source" — which is true
    // of where the KEY came from and false of the sentence it produced.
    //
    // What neither string can fix is that the whole lookup is by name: a
    // hand-installed bundle that adopts a published private name is
    // indistinguishable from the real one here, because the install records no
    // provenance to compare against. That is open question 28, not a wording
    // problem.
    return listed
      ? 'Private: published on the Biorouter marketplace'
      : 'Private: the Biorouter marketplace publishes this name as private';
  }
  if (listed) {
    return 'Public: published on the Biorouter marketplace';
  }
  return 'Public: installed from a file, not on the marketplace. Any model can call it.';
}

/**
 * The staleness line (§10.2), or `null` when the catalogue is fresh — a "this is
 * current" note on every screen is noise, and noise is what makes the stale case
 * invisible.
 */
export function catalogFreshnessLine(load: { live: boolean; fetchedAt?: string }): string | null {
  if (load.live) return null;
  // No date means the bundled snapshot, and that is an invariant rather than an
  // assumption: `readLastGoodRegistry` in `main.ts` rejects a cache entry whose
  // `fetchedAt` is missing or unparseable, so a dateless non-live load can only
  // be the fallback. Guess wrong here and the line names the wrong catalogue —
  // and this sentence is what the user reads to decide whether to trust the
  // entries under it.
  if (!load.fetchedAt) return 'showing bundled catalog (offline)';
  const when = new Date(load.fetchedAt);
  if (Number.isNaN(when.getTime())) return 'showing a cached catalog of unknown age';
  return `catalog last updated ${when.toLocaleDateString()}`;
}

/**
 * Every label in a list, as a search field. Total, because an entry can omit
 * any field — `isRegistryDocument` checks only that an entry is an object — and
 * a search that throws in render takes the whole modal with it.
 */
function labelFields(labels: readonly string[] | undefined): SearchField[] {
  return Array.isArray(labels) ? labels.map((label): SearchField => [label, Weight.Label]) : [];
}

/**
 * Rank skills against what the user typed in Browse skills, best first; an
 * empty query returns them all in registry order. How a query is matched — and
 * why a phrase is a union of its words rather than one substring (finding F5) —
 * is documented in `search.ts`.
 *
 * The fields are exactly the Rust catalog's (`MarketplaceCatalog::search_skills`),
 * so this modal and the model's search tool agree. ⚠ That excludes the license,
 * which the whole-phrase matcher searched: every skill and extension in the
 * registry is Apache-2.0, so it separates nothing, and under word matching it
 * made `PACS` list every skill — a plural's singular, `pac`, is inside `apache`.
 */
export function rankSkills(
  skills: readonly RegistrySkill[],
  query: string
): SearchResult<RegistrySkill> {
  return rankEntries(query, SKILL_NOISE, skills, (skill) => [
    [skill.id, Weight.Name],
    [skill.name, Weight.Name],
    [skill.category, Weight.Label],
    [skill.description, Weight.Prose],
    ...labelFields(skill.tags),
    ...labelFields(skill.keywords),
  ]);
}

/**
 * Rank extensions against what the user typed in Browse extensions, matched as
 * {@link rankSkills} is, over exactly the Rust catalog's fields
 * (`MarketplaceCatalog::search_extensions`).
 */
export function rankExtensions(
  extensions: readonly RegistryExtension[],
  query: string
): SearchResult<RegistryExtension> {
  return rankEntries(query, EXTENSION_NOISE, extensions, (ext) => [
    [ext.id, Weight.Name],
    [ext.extension_name, Weight.Name],
    [ext.name, Weight.Name],
    [ext.organization, Weight.Label],
    [ext.description, Weight.Prose],
    ...labelFields(ext.tags),
  ]);
}
