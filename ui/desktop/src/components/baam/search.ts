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
 * the document's here, the id's in Rust. **A change to a rule below is a change
 * to both files**, which is how the word-boundary rule PR #266 added arrived
 * here; the types it returns are `CatalogSearch` / `CatalogSearchHit` there and
 * {@link SearchResult} / {@link SearchHit} here.
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
 * 1. an entry containing the query **verbatim** — what the old matcher found,
 *    so nothing it showed is lost, bar what only the license matched (no longer
 *    a searched field: see `rankSkills` in `registry.ts`);
 * 2. then by **how many terms** it matched, so an entry matching every term
 *    precedes one matching some;
 * 3. then by **where** each term matched — the id or name outweighs a tag,
 *    which outweighs the description — and how exactly (the whole word, the
 *    start of one, or inside one);
 * 4. then registry order, so a result never reshuffles.
 *
 * Two rules keep the union from drowning the useful hits, both needed by the
 * measured query itself:
 *
 * - **A term under three characters matches whole words only.** `r` has to find
 *   the R language; as a substring it matched nearly every entry.
 * - **Filler words are dropped** ("a skill about R" is `r`), because in a union
 *   a word like `for` or `and` inflates the term count of every entry whose
 *   prose happens to use it, which ranks noise above the real hit.
 */

/** How much a match in one field says about an entry. */
export const Weight = {
  /** Free prose: a description. */
  Prose: 1,
  /** Curated labels: tags, keywords, a category, an organization. */
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

/** The best a single term can score: a whole-word match (3) in a name (3). */
const MAX_TERM_QUALITY = 3 * Weight.Name;

/**
 * Anything that is not a letter or a digit, in any script — the complement of
 * Rust's `char::is_alphanumeric`, which is Unicode `Alphabetic` or `Numeric`.
 */
const WORD_BREAK = /[^\p{Alphabetic}\p{N}]+/u;

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

/** Length in characters rather than UTF-16 code units, like Rust's `chars().count()`. */
function charCount(text: string): number {
  return Array.from(text).length;
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
 * How well `term` matches one field word: 3 for the whole word, 2 for its
 * start, 1 for anywhere inside it (`heatmap` in `complexheatmap`), 0 for no
 * match. A short term matches whole words only.
 */
function strength(term: string, word: string): number {
  if (word === term) return 3;
  if (charCount(term) < MIN_PARTIAL_CHARS) return 0;
  if (word.startsWith(term)) return 2;
  if (word.includes(term)) return 1;
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
  /** The whole query, trimmed and lowercased: what a verbatim match looks for. */
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
   * 0 is no match; otherwise higher ranks first. It packs the ranking —
   * verbatim, then terms matched, then where and how well they matched — into
   * one number whose scale depends on the query's term count, so it compares
   * entries scored against the SAME query and means nothing across two.
   */
  score: number;
  /**
   * The terms this entry matched, in query order. Empty for an entry that only
   * the verbatim query found.
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
  const verbatim = present.some(([text]) => text.toLowerCase().includes(query.phrase));
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

  // Lexicographic (verbatim, terms matched, quality) as one number. `quality`
  // is at most MAX_TERM_QUALITY per term, so it stays below `scale`; the terms
  // matched never exceed the term count, so a verbatim match outranks them.
  const termCount = query.terms.length;
  const scale = MAX_TERM_QUALITY * termCount + 1;
  const tier = (verbatim ? termCount + 1 : 0) + matchedTerms.length;
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
