/**
 * Free-text search over the INSTALLED skill catalog — the matcher behind
 * Settings → Skills and the composer's skill picker.
 *
 * It is not a matcher. The rules all live in `baam/search.ts`, the port of
 * `crates/biorouter/src/catalog_search.rs`, and this file says only what text of
 * an installed row is searched and what a match there is worth. Both surfaces
 * used to carry their own copy of a whole-phrase `includes(query)` test — QA
 * finding F5's third and fourth copies, after the Rust catalog search (PR #266)
 * and the Browse modals (PR #255). Measured on this tree against the rows the
 * two pickers render, before the change: `R scripting ggplot visualization`
 * listed **0 of 3** rows on both surfaces while `ggplot` alone listed 1, and
 * `R` listed **2 of 2** — `markdown-render`, which holds the letter twice and
 * means nothing by it, and `r-scripting`.
 *
 * ⚠ **`baam/search.ts` stays where it is, and this file does not copy it.** The
 * matcher is shared by import; only the name of its directory is now narrower
 * than its callers. Moving it would rewrite four BAAM files for a rename, and
 * the header that has to stay accurate — the one warning that a rule change is
 * a change to three files, `landing/marketplace-search.js` included — is that
 * file's, not this one's.
 *
 * # The two decisions this file had to make
 *
 * **1. A bundle row has no counterpart in Rust.** `skills__searchSkills` ranks
 * individual skills (`search_fields` in `agents/skills_extension.rs`): the
 * skill's own name at {@link Weight.Name}, its description at
 * {@link Weight.Prose}, the bundle it ships in at {@link Weight.Label}. No row
 * stands for a whole package there, so a member's name is the `Name` field of a
 * different entry. In these two pickers the package IS a row — the member is
 * reached through it, and in the composer only the package can be toggled — so
 * member names have to be searchable somewhere, or a package is unfindable by
 * what it contains.
 *
 * They are searched at {@link Weight.Label}, which is a TypeScript-only rule and
 * is argued rather than ported: on a bundle row a member's name is not what the
 * row is called, it is one of the labels saying what the row contains — the same
 * role a tag plays on a marketplace card. The weight is load-bearing, and
 * `searchCatalog.test.ts` pins it with the scores measured on this tree: for the
 * query `ggplot`, a skill of its own by that name scores 39 and a package merely
 * containing a member called `ggplot` scores 36, so the skill itself ranks
 * first. At `Name` both score 39 and the tie falls to catalog order — which puts
 * every bundle above every single skill, so the package would always win.
 *
 * **2. `SkillsView` groups by provenance and would throw the ranking away.** It
 * renders Biorouter / per-extension / other-agent / project headings, so under a
 * query a Biorouter skill matching one word would sit above a project skill
 * matching all of them. It now does what `BrowseSkillsModal` does — one ranked
 * "Matches (n)" list under a query, the headings when browsing — and reads
 * `isBrowseQuery` from the same module to decide which. The composer's list is
 * flat, so the ranking reaches it directly.
 */

import {
  rankEntries,
  SKILL_NOISE,
  Weight,
  type SearchField,
  type SearchResult,
} from '../baam/search';
import type { SkillCatalogEntry } from './useSkillCatalog';

export { isBrowseQuery } from '../baam/search';

/**
 * What one installed row is searched over.
 *
 * For a single skill these are exactly the Rust fields, so a user typing here
 * and a model calling `skills__searchSkills` read the same text: the name, the
 * description, and the bundle when the row is a member of one. (`slug` is
 * searched on neither side, and was not searched by the filter this replaces.)
 *
 * For a bundle row, see the note on this file about member names.
 */
export function catalogSearchFields(entry: SkillCatalogEntry): SearchField[] {
  if (entry.kind === 'single') {
    return [
      [entry.skill.name, Weight.Name],
      [entry.skill.description, Weight.Prose],
      // Total over the absent case rather than conditional: a picker row is a
      // standalone skill today, but the field is the Rust one and stays.
      [entry.skill.bundle ?? undefined, Weight.Label],
    ];
  }
  return [
    [entry.bundle.displayName, Weight.Name],
    [entry.bundle.name, Weight.Name],
    ...entry.bundle.skills.map((member): SearchField => [member, Weight.Label]),
  ];
}

/**
 * Rank installed catalog rows against what the user typed, best first. An empty
 * query returns every row in catalog order, which is what both surfaces showed
 * before a character was typed.
 */
export function rankCatalogEntries(
  entries: readonly SkillCatalogEntry[],
  query: string
): SearchResult<SkillCatalogEntry> {
  return rankEntries(query, SKILL_NOISE, entries, catalogSearchFields);
}
