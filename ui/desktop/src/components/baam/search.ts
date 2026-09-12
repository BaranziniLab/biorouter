/**
 * Free-text search over the marketplace catalog — the matcher behind the Browse
 * skills and Browse extensions modals.
 *
 * A port of the model-facing matcher, `crates/biorouter/src/catalog_search.rs`
 * (PR #242, moved there from `marketplace/search.rs` by PR #266 when the
 * installed-skill search became its second caller), and it has to stay one: a
 * user typing into the modal and a model calling
 * `skills__searchMarketplaceSkills` on that user's behalf read the same catalog,
 * and the same words must find the same entries, ranked the same way. Only a tie
 * can fall differently, because each side breaks ties by its own registry order —
 * the document's here, the id's in Rust. The types it returns are `CatalogSearch`
 * / `CatalogSearchHit` there and {@link SearchResult} / {@link SearchHit} here.
 *
 * ⚠ **A change to a rule below is a change to THREE files, not two.** The third
 * is the website's own copy, `landing/marketplace-search.js`, which the BAAM
 * shelves at biorouter.ucsf.edu call — and it is the copy that drifts, because
 * nothing imports it from here and its callers assemble their fields out of the
 * DOM. This header said "both files" while the three had already diverged over
 * which fields they search; the divergence is recorded where the fields are
 * chosen, in `registry.ts`.
 *
 * ⚠ **A query is a set of words, not a substring.** The matcher this replaced
 * asked whether the WHOLE lowercased query occurred inside a single field — the
 * defect the 2026-09-10 composer QA run measured as finding F5, present here in
 * TypeScript as well as in Rust. Measured in this modal against seven rows of
 * `landing/registry.json`: `R scripting ggplot visualization` showed no skill
 * at all while `ggplot` alone showed two, and `r-scripting` — a skill's own id —
 * showed none, because the id was not searched.
 *
 * So a query is split into terms and an entry is a hit when it matches ANY of
 * them. The union is deliberate: no single entry has to contain every word a
 * user happened to type, and an AND over a phrase is the same empty list with a
 * different cause. Precision comes from the ranking instead, best first:
 *
 * 1. an entry holding the query **as written** — its words, in that order, as
 *    whole words, so `r scripting` is in "R scripting" but not in "for
 *    scripting" — which no scatter of the same words outranks;
 * 2. then by **how many terms** it matched, so an entry matching every term
 *    precedes one matching some;
 * 3. then by **where** each term matched — the id or name outweighs a tag,
 *    which outweighs the description — and how exactly (the whole word, the
 *    start of one, or inside one);
 * 4. then registry order, so a result never reshuffles.
 *
 * Three rules keep the union from drowning the useful hits; the first two were
 * needed by the measured query itself, and the third — {@link substantialInfix} —
 * closes the same failure one step further in: a query that finds everything
 * says nothing, whether it got there through a repeated field or through a
 * three-letter morpheme.
 *
 * - **A term under three characters matches whole words only.** `r` has to find
 *   the R language; as a substring it matched nearly every entry. The query as
 *   written is held to the same edges (see {@link writtenIn}) — it counts only
 *   where it starts and ends at a word boundary. Tested as a plain substring,
 *   which is what this file did until PR #266, a query that IS one short term
 *   came back in through every word containing it: measured over the live
 *   `landing/registry.json`, `R` returned 125 of 129 skills, 117 of them
 *   matching no term at all, and ranked `empirical-paper-submission-rr` above
 *   `r-scripting`. Refusing that leak drops 198 hits across 76 searches and adds
 *   none, and only a query whose every term is short changes at all.
 * - **Filler words are dropped** ("a skill about R" is `r`), because in a union
 *   a word like `for` or `and` inflates the term count of every entry whose
 *   prose happens to use it, which ranks noise above the real hit.
 */

/** How much a match in one field says about an entry. */
export const Weight = {
  /** Free prose: a description. */
  Prose: 1,
  /**
   * Curated labels: tags, keywords, an organization.
   *
   * ⚠ **Not a curation bucket the surface also offers as a filter control.** A
   * skill's `category` was here, and it is the licence's defect one field on:
   * `Core` names 57 of the registry's 129 skills and `Biomedical` 63, so `core`
   * listed 59 and `biomedical` 65 — half the modal, under a word the user did
   * not mean. The modal already answers the category with its own filter —
   * `All` / `Core skills` / `Developer & authoring` / `Biomedical analysis`, and
   * the Developer chip was measured showing exactly the 9 rows `developer` used to
   * return — the website shelf with three `data-facet="category"` chips, and the
   * website's matcher never searched the field at all. See `rankSkills` in
   * `registry.ts`.
   */
  Label: 2,
  /** What the entry is called: its registry id and names. */
  Name: 3,
} as const;

/** One of the {@link Weight}s. */
export type FieldWeight = (typeof Weight)[keyof typeof Weight];

/** One searchable field of an entry, and what a match there counts for. */
export type SearchField = readonly [text: string | undefined, weight: FieldWeight];

/**
 * Words that say how a request is phrased, not what it is for. Dropped from a
 * query unless nothing else is left, so a query made only of them (`agent`,
 * `or`) still searches for what it says. The Rust list, word for word.
 */
const FILLER: ReadonlySet<string> = new Set([
  'a',
  'about',
  'an',
  'and',
  'any',
  'are',
  'baam',
  'be',
  'by',
  'can',
  'do',
  'does',
  'find',
  'for',
  'from',
  'help',
  'how',
  'i',
  'in',
  'into',
  'is',
  'it',
  'its',
  'looking',
  'marketplace',
  'me',
  'my',
  'need',
  'of',
  'on',
  'or',
  'please',
  'search',
  'some',
  'that',
  'the',
  'this',
  'to',
  'use',
  'using',
  'via',
  'want',
  'what',
  'which',
  'with',
]);

/** Filler specific to the skills catalog: every entry in it is a skill. */
export const SKILL_NOISE: readonly string[] = ['skill', 'skills'];

/** Filler specific to the extensions catalog: every entry in it is an extension. */
export const EXTENSION_NOISE: readonly string[] = ['extension', 'extensions'];

/** Below this many characters a term matches whole words only. */
const MIN_PARTIAL_CHARS = 3;

/**
 * At or above this many characters a term may match anywhere inside a word,
 * however long the word. Below it, {@link substantialInfix} asks for half.
 */
const MIN_INFIX_CHARS = 4;

/** The best a single term can score: a whole-word match (3) in a name (3). */
const MAX_TERM_QUALITY = 3 * Weight.Name;

/**
 * Anything that is not a letter or a digit, in any script — the complement of
 * Rust's `char::is_alphanumeric`, which is Unicode `Alphabetic` or `Numeric`.
 */
const WORD_BREAK = /[^\p{Alphabetic}\p{N}]+/u;

/**
 * One letter or digit: Rust's `char::is_alphanumeric` itself. The complement of
 * {@link WORD_BREAK}, and it has to stay the complement — {@link words} splits a
 * field on one and {@link writtenIn} tests the other, so a term and the query
 * around it are held to the same edges.
 *
 * ⚠ **Not `\b`**, which JavaScript defines over `[A-Za-z0-9_]` alone. It reads
 * `_` as a word character where Rust does not, and every non-ASCII letter — `é`,
 * `π`, `中` — as a boundary where Rust reads a word character. On `-` and `.` the
 * two agree, which is exactly why an ASCII fixture would pass over the
 * disagreement.
 */
const WORD_CHAR = /[\p{Alphabetic}\p{N}]/u;

/**
 * Lowercase words, split at every character that is not a letter or digit —
 * whitespace and punctuation alike, so `r-scripting` is `r` + `scripting` and
 * `ggplot2` stays one word.
 */
function words(text: string): string[] {
  return text
    .split(WORD_BREAK)
    .filter((word) => word !== '')
    .map((word) => word.toLowerCase());
}

/**
 * Does `label` say nothing that `license` does not — is every word of it a word
 * of the licence? A catalog whose entries carry a licence drops such a label when
 * it assembles an entry's searchable text (see `labelFields` in `registry.ts`).
 *
 * The licence itself is not a searchable field: every entry in the BAAM registry
 * is Apache-2.0, so it separates nothing, and both catalog searches leave the
 * field out for that reason.
 *
 * ⚠ **Leaving the FIELD out was not enough.** A registry republishes the licence
 * as one of the entry's own tag chips — and, for a skill, again among its
 * keywords — and labels are searched, rightly: `MCP`, `ELN` and `Imaging` are
 * exactly what a tag is for. Measured in the Browse-extensions modal on
 * 2026-09-12 against the live 37-entry registry, with the field already gone:
 * `PACS` → 31 of 37, `pac` → 31, `apache` → 31, and not one of the 31 about PACS.
 * The three counts agreeing is the identification — `PACS` reaches `pac` through
 * the plural fallback in {@link termStrength}, `pac` is inside `apache`, and 31
 * rows wear an `Apache-2.0` chip. Removing the field had moved the defect one
 * field over, where a test asserting "the licence is not searched" still passed.
 *
 * Compared by WORDS rather than by equality, because the second spelling is not
 * the first: the tag is `Apache-2.0` and the keyword is `apache`. An equality
 * test drops the tag and keeps the keyword, which is the same half-fix again.
 *
 * What this deliberately does not do: drop every label (`MCP`, `Imaging`, `ELN`,
 * `Registry` are real search value), or name a licence in the matcher (`Apache`,
 * `MIT` — the next licence reopens the hole). The cost of the word test is a
 * licence id built from a topical word — `Python-2.0`, `Ruby`, `PostgreSQL` — on
 * an entry that also tags itself with that word; the tag is then dropped for
 * saying only what the licence says. No entry in the registry is such a case
 * (measured over all 166: the rule drops the 129 licence labels and nothing
 * else), and an equality test pays a smaller version of the same cost.
 */
export function namesOnlyTheLicense(label: string, license: string | undefined): boolean {
  const labelWords = words(label);
  // An empty label says nothing at all, which is not the same as saying only the
  // licence: leave it, so the rule stays about the licence.
  if (labelWords.length === 0) return false;
  const licenseWords = words(license ?? '');
  return labelWords.every((word) => licenseWords.includes(word));
}

/** Length in characters rather than UTF-16 code units, like Rust's `chars().count()`. */
function charCount(text: string): number {
  return Array.from(text).length;
}

/** Is `char` a letter or digit? The text's edge — `undefined` — is not. */
function isWordChar(char: string | undefined): boolean {
  return char !== undefined && WORD_CHAR.test(char);
}

/**
 * Does `text` hold `phrase` as written — starting and ending at a word boundary,
 * not inside a longer word? `r scripting` is in "R scripting" but not in "for
 * scripting", where its `r` is the tail of `for`. Both are lowercase already.
 *
 * An edge of `phrase` that is not a letter or digit needs no boundary: it is
 * one, so `++` is written in "c++".
 *
 * Every character position is tried, not only the occurrences `indexOf` would
 * step through, because a refused occurrence can overlap an accepted one:
 * `a a` in "ba a a" is written only from the second `a`.
 *
 * Compared as code points rather than UTF-16 units, like Rust's `char_indices`,
 * so an astral character is one character on both sides of the port.
 *
 * Exported only so the boundary cases can be asserted directly, the way Rust
 * asserts them from inside the module.
 */
export function writtenIn(text: string, phrase: string): boolean {
  const chars = Array.from(text);
  const wanted = Array.from(phrase);
  const startsWord = isWordChar(wanted[0]);
  const endsWord = isWordChar(wanted[wanted.length - 1]);
  for (let start = 0; start < chars.length; start += 1) {
    if (!wanted.every((want, offset) => chars[start + offset] === want)) continue;
    const opens = !startsWord || !isWordChar(chars[start - 1]);
    const closes = !endsWord || !isWordChar(chars[start + wanted.length]);
    if (opens && closes) return true;
  }
  return false;
}

/**
 * An empty or all-whitespace query is the browse case: every entry, in
 * registry order.
 */
export function isBrowseQuery(query: string): boolean {
  return query.trim() === '';
}

/** The distinct terms of `query`, in the order written, without filler. */
export function searchTerms(query: string, noise: readonly string[] = []): string[] {
  const all: string[] = [];
  for (const word of words(query)) {
    if (!all.includes(word)) all.push(word);
  }
  const meaningful = all.filter((term) => !FILLER.has(term) && !noise.includes(term));
  return meaningful.length > 0 ? meaningful : all;
}

/**
 * Is `term`, found inside `word` without touching its start, enough of that word
 * to be a search rather than a morpheme?
 *
 * The matcher already grades an anchored match above an unanchored one — a prefix
 * scores 2, an infix 1 — and {@link MIN_PARTIAL_CHARS} was the only admission
 * gate, so three characters bought a match anywhere inside any word. Measured in
 * the Browse-extensions modal against the shipped 37-entry registry: `lab` → **37
 * of 37**, `gen` → 36, `age` → 36. Per hit, 32 of `lab`'s matched only as an
 * infix of `baranzinilab` (the organization) and 33 each of `gen`'s and `age`'s
 * only as an infix of `…Agent` in the extension's own NAME — so this is a rule,
 * not a field: dropping `organization` fixes one of the three, and nothing can
 * drop a name. In Browse skills the same rule had `ing` matching 88 of 129
 * skills, `ica` 85, `ion` 84, `cal` 80 and `tio` 77 — every one of them a
 * morpheme.
 *
 * So an unanchored match needs either {@link MIN_INFIX_CHARS} characters or half
 * the word it sits in. Two arms rather than one number, because each closes a
 * case the other gets wrong, and each was checked against the registry's own
 * vocabulary.
 *
 * ⚠ The counts that used to sit here were NOT reproducible and are gone.
 * Measured 2026-09-12 from `landing/registry.json`, over exactly the fields
 * these matchers search: 1,445 distinct words with the prose descriptions, 773
 * without. No field combination yields the 807 this comment claimed. Quote a
 * corpus size only with the field set that produces it.
 *
 * - A flat four-character floor drops what a short term earns inside a SHORT
 *   word — `rna` in `scRNA`/`rRNA`/`miRNA`/`piRNA`, `sem` in `RSEM` — costing
 *   `rna` the `single-cell` and `microbiome` skills.
 * - A flat half-the-word ratio drops what a LONG term earns inside a longer
 *   compound, which is most of a biomedical vocabulary: `omics` stopped finding
 *   `transcriptomics`, `flow` stopped finding `workflows`.
 *
 * What the two arms remove together is dominated by the `lab` flood. Half is
 * also the proportion this rule's own documented case sits at —
 * `heatmap` is 7 of `complexheatmap`'s 14 — so the arm that admits a short term
 * is calibrated to the example the infix rule exists for.
 *
 * Exported so the two arms can be asserted directly, the way Rust asserts them
 * from inside the module.
 */
export function substantialInfix(term: string, word: string): boolean {
  const termChars = charCount(term);
  return termChars >= MIN_INFIX_CHARS || termChars * 2 >= charCount(word);
}

/**
 * How well `term` matches one field word: 3 for the whole word, 2 for its
 * start, 1 for anywhere inside it (`heatmap` in `complexheatmap`), 0 for no
 * match. A short term matches whole words only, and a term that is short
 * relative to the word matches only at its start — see {@link substantialInfix}.
 */
function strength(term: string, word: string): number {
  if (word === term) return 3;
  if (charCount(term) < MIN_PARTIAL_CHARS) return 0;
  if (word.startsWith(term)) return 2;
  if (word.includes(term)) return substantialInfix(term, word) ? 1 : 0;
  return 0;
}

/**
 * `term`'s strength against `word`, falling back to its singular so
 * `visualizations` finds `visualization` and `heatmaps` finds `heatmap`.
 * `class` and `gis` are left alone.
 */
function termStrength(term: string, word: string): number {
  const direct = strength(term, word);
  if (direct > 0 || !term.endsWith('s')) return direct;
  const stem = term.slice(0, -1);
  return charCount(stem) >= MIN_PARTIAL_CHARS && !stem.endsWith('s') ? strength(stem, word) : 0;
}

/** A query as the matcher reads it. */
export interface SearchQuery {
  /** The whole query, trimmed and lowercased: what {@link writtenIn} looks for. */
  phrase: string;
  /** Its distinct words, without filler. See {@link searchTerms}. */
  terms: string[];
}

export function parseQuery(query: string, noise: readonly string[] = []): SearchQuery {
  return { phrase: query.trim().toLowerCase(), terms: searchTerms(query, noise) };
}

/** How one entry matched one query. */
export interface EntryMatch {
  /**
   * 0 is no match; otherwise higher ranks first. It packs the ranking — the
   * query as written, then terms matched, then where and how well they matched —
   * into one number whose scale depends on the query's term count, so it
   * compares entries scored against the SAME query and means nothing across two.
   */
  score: number;
  /**
   * The terms this entry matched, in query order. Empty only when the query holds
   * no word at all (`++`), so that nothing but the query as written could have
   * found the entry — a query written in a field has every one of its words in
   * that field as a whole word, and so matches every term.
   */
  matchedTerms: string[];
}

/**
 * Score one entry's fields against a query. Pure, and total over absent field
 * text. Under the browse query every entry matches, equally.
 */
export function scoreEntry(query: SearchQuery, fields: readonly SearchField[]): EntryMatch {
  if (query.phrase === '') return { score: 1, matchedTerms: [] };

  const present = fields.filter(
    (field): field is readonly [string, FieldWeight] => typeof field[0] === 'string'
  );
  const written = present.some(([text]) => writtenIn(text.toLowerCase(), query.phrase));
  const entryWords = present.flatMap(([text, weight]) =>
    words(text).map((word) => ({ word, weight }))
  );

  const matchedTerms: string[] = [];
  let quality = 0;
  for (const term of query.terms) {
    let best = 0;
    for (const { word, weight } of entryWords) {
      best = Math.max(best, termStrength(term, word) * weight);
    }
    if (best > 0) {
      matchedTerms.push(term);
      quality += best;
    }
  }

  // Lexicographic (written, terms matched, quality) as one number — the same key
  // Rust sorts on. `quality` is at most MAX_TERM_QUALITY per term, so it stays
  // below `scale`; the terms matched never exceed the term count, so the query
  // as written outranks any scatter of its words.
  const termCount = query.terms.length;
  const scale = MAX_TERM_QUALITY * termCount + 1;
  const tier = (written ? termCount + 1 : 0) + matchedTerms.length;
  return { score: tier * scale + quality, matchedTerms };
}

/** One entry a search returned. */
export interface SearchHit<T> extends EntryMatch {
  entry: T;
}

/** A ranked search: what the query was read as, and every entry that matched it, best first. */
export interface SearchResult<T> {
  /** The query's terms, after filler was dropped. Empty for the browse query. */
  terms: string[];
  /** Best first; equal scores keep registry order. The browse query returns every entry. */
  hits: SearchHit<T>[];
}

/**
 * Rank `entries` against `query`. `fields` names the text of one entry that is
 * searched, and what a match there counts for; `noise` is the catalog's own
 * filler (every entry in the skills catalog is a skill).
 */
export function rankEntries<T>(
  query: string,
  noise: readonly string[],
  entries: readonly T[],
  fields: (entry: T) => readonly SearchField[]
): SearchResult<T> {
  const parsed = parseQuery(query, noise);
  if (parsed.phrase === '') {
    // Browsing reads no field at all — nor did the matcher this replaced — so
    // an entry too malformed to search is still listed.
    return { terms: [], hits: entries.map((entry) => ({ entry, score: 1, matchedTerms: [] })) };
  }
  const hits = entries
    .map((entry) => ({ entry, ...scoreEntry(parsed, fields(entry)) }))
    .filter((hit) => hit.score > 0);
  // `Array.prototype.sort` is stable, so equal scores keep registry order.
  hits.sort((left, right) => right.score - left.score);
  return { terms: parsed.terms, hits };
}
