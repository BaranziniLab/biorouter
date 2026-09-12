import { describe, expect, it } from 'vitest';
import { MARKETPLACE_EXTENSIONS, MARKETPLACE_SKILLS } from './marketplace.fixture';
import {
  FALLBACK_REGISTRY,
  rankExtensions,
  rankSkills,
  type RegistryExtension,
  type RegistrySkill,
} from './registry';
import {
  EXTENSION_NOISE,
  isBrowseQuery,
  namesOnlyTheLicense,
  parseQuery,
  rankEntries,
  scoreEntry,
  searchTerms,
  SKILL_NOISE,
  substantialInfix,
  Weight,
  writtenIn,
  type SearchField,
  type SearchResult,
} from './search';

/**
 * The marketplace matcher, ported from `crates/biorouter/src/catalog_search.rs`
 * (PR #242; moved there from `marketplace/search.rs`, and given the word-boundary
 * rule, by PR #266). The first half ports that file's own tests against the same
 * three synthetic entries, so the two languages are pinned by one set of cases;
 * the second half runs finding F5's queries against real registry rows.
 */

interface Entry {
  id: string;
  name: string;
  description: string;
  tags: string[];
}

const ENTRIES: Entry[] = [
  {
    id: 'complex-plots',
    name: 'Complex Plots',
    description: 'Draws annotated heat maps with the ComplexHeatmap package.',
    tags: ['ComplexHeatmap'],
  },
  {
    id: 'prose-only',
    name: 'Prose Only',
    description: 'Mentions scripting in passing.',
    tags: [],
  },
  {
    id: 'r-scripting',
    name: 'R Scripting',
    description: 'Tidyverse conventions for R code.',
    tags: ['R'],
  },
];

function fields(entry: Entry): SearchField[] {
  return [
    [entry.id, Weight.Name],
    [entry.name, Weight.Name],
    [entry.description, Weight.Prose],
    ...entry.tags.map((tag): SearchField => [tag, Weight.Label]),
  ];
}

function rank(query: string): SearchResult<Entry> {
  return rankEntries(query, [], ENTRIES, fields);
}

function ids<T extends { id: string }>(search: SearchResult<T>): string[] {
  return search.hits.map((hit) => hit.entry.id);
}

describe('marketplace search — reading a query', () => {
  it('splits a query at whitespace and punctuation, lowercased and de-duplicated', () => {
    expect(searchTerms('R scripting, ggplot/Visualization')).toEqual([
      'r',
      'scripting',
      'ggplot',
      'visualization',
    ]);
    expect(searchTerms('r-scripting')).toEqual(['r', 'scripting']);
    expect(searchTerms('ggplot2 ggplot2')).toEqual(['ggplot2']);
  });

  /// A letter is a letter in any script, as Rust's `char::is_alphanumeric`
  /// has it: an `\w`-style ASCII class would cut `naïve` in two.
  it('keeps letters and digits of any script inside a word', () => {
    expect(searchTerms('scRNA-seq, naïve B-cells')).toEqual([
      'scrna',
      'seq',
      'naïve',
      'b',
      'cells',
    ]);
  });

  it('drops filler words unless they are all there is', () => {
    expect(searchTerms('a skill about R scripting or ggplot', SKILL_NOISE)).toEqual([
      'r',
      'scripting',
      'ggplot',
    ]);
    expect(searchTerms('I need something for R', SKILL_NOISE)).toEqual(['something', 'r']);
    expect(searchTerms('skills', SKILL_NOISE)).toEqual(['skills']);
    expect(searchTerms('the', SKILL_NOISE)).toEqual(['the']);
    // A catalog's own noise words are its own.
    expect(searchTerms('skills', EXTENSION_NOISE)).toEqual(['skills']);
    expect(searchTerms('an extension for skills', EXTENSION_NOISE)).toEqual(['skills']);
  });

  it('treats an empty or all-whitespace query as browsing', () => {
    expect(isBrowseQuery('')).toBe(true);
    expect(isBrowseQuery('  \t ')).toBe(true);
    expect(isBrowseQuery(' r ')).toBe(false);
  });
});

describe('marketplace search — matching and ranking', () => {
  /// `r` as a substring is in nearly every word of prose; as a term it must
  /// mean the R language.
  it('matches a term under three characters to whole words only', () => {
    const search = rank('R scripting');
    const prose = search.hits.find((hit) => hit.entry.id === 'prose-only');
    // `scripting` is in its description, and the `r` inside it is not the R language.
    expect(prose?.matchedTerms).toEqual(['scripting']);
    // Nor is the `r` inside `Draws`.
    expect(ids(search)).not.toContain('complex-plots');
  });

  it('matches a longer term inside a word, and a plural by its singular', () => {
    // `heatmap` inside `complexheatmap`.
    expect(ids(rank('heatmap'))).toEqual(['complex-plots']);
    // No field says `heatmaps`; its singular is inside `complexheatmap`.
    expect(ids(rank('heatmaps'))).toEqual(['complex-plots']);
    // `scripts` is not in `scripting`, but `script` starts it.
    expect(ids(rank('scripts'))).toEqual(['r-scripting', 'prose-only']);
  });

  /// An unanchored match has to be worth something. Three characters is enough to
  /// search from the START of a word — `gen` really does find `genomics` — and,
  /// inside a long one, is a morpheme: measured in Browse extensions against the
  /// shipped 37-entry registry, `lab` listed **37 of 37** through an infix of
  /// `baranzinilab`, and `gen` and `age` 36 each through an infix of `…Agent` in
  /// each extension's own name. `substantial_infix` in `catalog_search.rs` asserts
  /// these same words.
  it('matches a short term inside a word only when it is half of it', () => {
    // The three measured floods, at the word each of them came through.
    expect(substantialInfix('lab', 'baranzinilab'), '3 of 12').toBe(false);
    expect(substantialInfix('gen', 'cdwagent'), '3 of 8').toBe(false);
    expect(substantialInfix('age', 'language'), '3 of 8').toBe(false);
    // A short term inside a SHORT word is the search, not a morpheme — the hits a
    // flat four-character floor would have cost.
    expect(substantialInfix('rna', 'scrna'), '3 of 5').toBe(true);
    expect(substantialInfix('rna', 'rrna'), '3 of 4').toBe(true);
    expect(substantialInfix('sem', 'rsem'), '3 of 4').toBe(true);
    expect(substantialInfix('age', 'image'), '3 of 5').toBe(true);
    // At four characters a term is unanchored anywhere, however long the word,
    // which is what keeps a compound biomedical vocabulary findable.
    expect(substantialInfix('omics', 'transcriptomics')).toBe(true);
    expect(substantialInfix('flow', 'workflows')).toBe(true);
    // The case the infix rule was written for sits exactly on the short arm's
    // boundary, so it would pass on either arm.
    expect(substantialInfix('heatmap', 'complexheatmap'), '7 of 14').toBe(true);

    // And through the matcher: only the UNANCHORED match is refused.
    const entry = (name: string): Entry => ({ id: 'x', name, description: '', tags: [] });
    const found = (term: string, name: string) =>
      rankEntries(term, [], [entry(name)], fields).hits.length;
    expect(found('gen', 'Genomics'), 'a prefix still matches').toBe(1);
    expect(found('lab', 'Lab'), 'a whole word still matches').toBe(1);
    expect(found('gen', 'CDWAgent'), 'an infix of a long word does not').toBe(0);
    expect(found('rna', 'scRNA-seq'), 'an infix of a short word does').toBe(1);
  });

  it('returns the union, ranked by terms matched and then by where they matched', () => {
    const search = rank('R scripting');
    expect(ids(search)).toEqual(['r-scripting', 'prose-only']);
    expect(search.hits[0].matchedTerms).toEqual(['r', 'scripting']);
    expect(search.hits[1].matchedTerms).toEqual(['scripting']);

    // A match in the name outranks the same match in the description.
    expect(ids(rank('scripting'))).toEqual(['r-scripting', 'prose-only']);
  });

  /// The query as written is held to the same edges as a short term. Until PR
  /// #266 the whole-query check was a plain substring test, so a query that IS
  /// one short term came back in through every word containing it: `r` alone
  /// found `complex-plots` through "Draws" and `prose-only` through its own
  /// name, each with no matched term at all.
  it('reads a one-letter query as a whole word, not a letter inside one', () => {
    const search = rank('R');
    expect(ids(search)).toEqual(['r-scripting']);
    expect(search.hits[0].matchedTerms).toEqual(['r']);
  });

  /// The query as written outranks any count of separate words, so it has to be
  /// written there: `r scripting` inside "for scripting" is the tail of `for`
  /// and then a word. Read as a substring it ranked the entry matching one of
  /// the two words above the entry matching both.
  it('does not read a phrase found only inside longer words as written', () => {
    const entries: Entry[] = [
      {
        id: 'shell-snippets',
        name: 'Shell Snippets',
        description: 'Snippets for scripting the shell.',
        tags: [],
      },
      {
        id: 'tidy-style',
        name: 'Tidy Style',
        description: 'Scripting conventions for R.',
        tags: ['R'],
      },
    ];
    const search = rankEntries('R scripting', [], entries, fields);

    expect(ids(search)).toEqual(['tidy-style', 'shell-snippets']);
    expect(search.hits[0].matchedTerms).toEqual(['r', 'scripting']);
    expect(search.hits[1].matchedTerms).toEqual(['scripting']);
  });

  /// A fragment of a word is not the query as written, so a query of nothing but
  /// short terms finds nothing at all — `dy` is inside "Tidyverse", not a word of
  /// it. The `s p` half held under the substring rule; the `dy` half is the leak
  /// that rule asserted as a feature.
  it('finds nothing for a query written only inside longer words', () => {
    expect(rank('dy').hits).toEqual([]);
    expect(rank('s p').hits).toEqual([]);
  });

  it('browses every entry in registry order for an empty query', () => {
    const search = rank('   ');
    expect(search.terms).toEqual([]);
    expect(ids(search)).toEqual(['complex-plots', 'prose-only', 'r-scripting']);
  });

  /// Like the matcher this replaced, so an entry too malformed to read a field
  /// from is still listed while the user is only browsing.
  it('reads no field at all to browse', () => {
    const unreadable = (): SearchField[] => {
      throw new Error('a field was read');
    };
    expect(ids(rankEntries('', [], ENTRIES, unreadable))).toEqual([
      'complex-plots',
      'prose-only',
      'r-scripting',
    ]);
  });
});

/// The ranking is packed into one number, and the packing is where a subtle
/// defect would hide: a scale too small lets a better placement leak into the
/// tier above it. Each case pits a stronger lower criterion against a weaker
/// higher one.
describe('marketplace search — the score', () => {
  it('is 0 for an entry that matches nothing, with no terms matched', () => {
    expect(scoreEntry(parseQuery('ggplot'), [['Data Visualization', Weight.Name]])).toEqual({
      score: 0,
      matchedTerms: [],
    });
  });

  it('ranks more terms matched above better placement', () => {
    const query = parseQuery('heatmap volcano');
    const oneTermInTheName = scoreEntry(query, [['Heatmap', Weight.Name]]);
    const twoTermsInProse = scoreEntry(query, [['complexheatmap and volcanoes', Weight.Prose]]);

    expect(oneTermInTheName.matchedTerms).toEqual(['heatmap']);
    expect(twoTermsInProse.matchedTerms).toEqual(['heatmap', 'volcano']);
    expect(twoTermsInProse.score).toBeGreaterThan(oneTermInTheName.score);
  });

  /// The query as written is a tier of its own, not one more term. It can no
  /// longer be pitted against a HIGHER term count, as the substring rule allowed
  /// — a phrase written in a field has every one of its words in that field as a
  /// whole word, so it always matches every term — so the weaker criterion it is
  /// pitted against here is placement: prose against two names.
  it('ranks the query as written above the same words scattered in weightier fields', () => {
    const query = parseQuery('tidy code');
    const writtenInProse = scoreEntry(query, [['Writes tidy code.', Weight.Prose]]);
    const scatteredInNames = scoreEntry(query, [
      ['Code Tidy', Weight.Name],
      ['code-tidy', Weight.Name],
    ]);

    expect(writtenInProse.matchedTerms).toEqual(['tidy', 'code']);
    expect(scatteredInNames.matchedTerms).toEqual(['tidy', 'code']);
    expect(writtenInProse.score).toBeGreaterThan(scatteredInNames.score);

    // The same pair ranked, as `catalog_search.rs` asserts it.
    const entries: Entry[] = [
      { id: 'code-tidy', name: 'Code Tidy', description: 'Formatting rules.', tags: [] },
      { id: 'styler', name: 'Styler', description: 'Writes tidy code.', tags: [] },
    ];
    expect(ids(rankEntries('tidy code', [], entries, fields))).toEqual(['styler', 'code-tidy']);
  });

  it('skips a field with no text rather than throwing', () => {
    expect(
      scoreEntry(parseQuery('ggplot'), [
        [undefined, Weight.Label],
        ['ggplot2', Weight.Label],
      ]).matchedTerms
    ).toEqual(['ggplot']);
  });
});

/// Finding F5, measured by the 2026-09-10 composer QA run against the live
/// registry and reproduced here against seven of its rows. Before this port,
/// the desktop modal showed no skill for the phrase, and none for `r-scripting`
/// either: the id was not a searched field.
describe('rankSkills — the QA queries (finding F5)', () => {
  it('returns the union for `R scripting ggplot visualization`, ggplot-visualization first', () => {
    const search = rankSkills(MARKETPLACE_SKILLS, 'R scripting ggplot visualization');

    expect(search.terms).toEqual(['r', 'scripting', 'ggplot', 'visualization']);
    // Three of the four terms, then two, then one.
    expect(ids(search)).toEqual([
      'ggplot-visualization',
      'r-scripting',
      'data-visualization',
      'python-scripting',
      'clinical-biostatistics',
    ]);
    expect(search.hits[0].matchedTerms).toEqual(['r', 'ggplot', 'visualization']);
    // `visual` is not `visualization`: a long term must be found in the entry,
    // not the other way round.
    expect(ids(search)).not.toContain('scientific-visual-communication');
  });

  it('finds exactly the two ggplot skills for `ggplot`', () => {
    expect(ids(rankSkills(MARKETPLACE_SKILLS, 'ggplot'))).toEqual([
      'ggplot-visualization',
      'data-visualization',
    ]);
  });

  it('puts the skill `r-scripting` names first', () => {
    const search = rankSkills(MARKETPLACE_SKILLS, 'r-scripting');

    expect(search.hits[0].entry.id).toBe('r-scripting');
    expect(search.hits[0].matchedTerms).toEqual(['r', 'scripting']);
    // Then one term each: in a name before in a tag, and a tie in registry order.
    expect(ids(search)).toEqual([
      'r-scripting',
      'python-scripting',
      'ggplot-visualization',
      'clinical-biostatistics',
    ]);
  });
});

describe('rankSkills — the rules that keep a union useful', () => {
  /// Without the filler rule `and` is a term, and it is a whole word in the
  /// description of every skill below that matches nothing else: single-cell,
  /// python-scripting, scientific-visual-communication.
  it('drops filler, so a skill that only says `and` is not a hit', () => {
    const search = rankSkills(MARKETPLACE_SKILLS, 'skills for R and ggplot');

    expect(search.terms).toEqual(['r', 'ggplot']);
    expect(ids(search)).toEqual([
      'ggplot-visualization',
      'r-scripting',
      'clinical-biostatistics',
      'data-visualization',
    ]);
  });

  it('reads `R` in a phrase as the R language, not the r inside `writing`', () => {
    // python-scripting's description says `error`, `structure` and `writing`.
    expect(ids(rankSkills(MARKETPLACE_SKILLS, 'R ggplot'))).toEqual([
      'ggplot-visualization',
      'r-scripting',
      'clinical-biostatistics',
      'data-visualization',
    ]);
  });

  /// A one-letter query is the letter as a word: the three skills that are about
  /// R, and nothing that merely contains an r. Until PR #266 this returned six —
  /// these three, then scientific-visual-communication, python-scripting and
  /// single-cell, each matching no term at all and kept only by the substring
  /// test. Measured against the live registry rather than this fixture, that rule
  /// returned 125 of 129 skills, 117 of them with no matched term.
  it('finds only the R skills for `R` alone, not what merely contains an r', () => {
    const search = rankSkills(MARKETPLACE_SKILLS, 'R');

    expect(ids(search)).toEqual(['r-scripting', 'ggplot-visualization', 'clinical-biostatistics']);
    expect(search.hits.map((hit) => hit.matchedTerms)).toEqual([['r'], ['r'], ['r']]);
  });

  it('finds a singular field word for a plural term', () => {
    // No field says `visualizations`, and `visuals` is not a visualization.
    expect(ids(rankSkills(MARKETPLACE_SKILLS, 'visualizations'))).toEqual([
      'ggplot-visualization',
      'data-visualization',
    ]);
    expect(ids(rankSkills(MARKETPLACE_SKILLS, 'scripts'))).toEqual([
      'r-scripting',
      'python-scripting',
    ]);
  });

  it('browses every skill in registry order for an empty query', () => {
    expect(ids(rankSkills(MARKETPLACE_SKILLS, ''))).toEqual(
      MARKETPLACE_SKILLS.map((skill) => skill.id)
    );
  });

  /// `isRegistryDocument` admits any object as an entry, so a cached v1 or
  /// hand-edited document can omit fields. The ranker must not be what throws on
  /// one: browsing reads no field at all, and a search skips what is missing.
  /// The matcher this replaced threw on the missing `name` the moment a query
  /// was typed.
  it('lists a malformed entry and searches past it without throwing', () => {
    const malformed = { id: 'r-scripting' } as unknown as RegistrySkill;

    expect(ids(rankSkills([malformed], ''))).toEqual(['r-scripting']);
    expect(ids(rankSkills([malformed], 'R scripting'))).toEqual(['r-scripting']);
  });
});

/// Which fields a catalog searches is part of the port. In the registry's own
/// rows, keywords repeat the id and the tags, so a field dropped from the list
/// changes no ranking the QA queries pin; each field is asserted on its own
/// instead, carrying a word no other field holds.
describe('the fields each catalog searches, and what a match in each is worth', () => {
  const blankSkill: RegistrySkill = {
    id: 'blank',
    name: 'Blank',
    category: 'Core',
    type: '',
    description: '',
    tags: [],
    keywords: [],
    download: '',
    filename: '',
  };
  const blankExtension: RegistryExtension = {
    id: 'blank',
    name: 'Blank',
    organization: '',
    version: '',
    description: '',
    tags: [],
    github: '',
    download: '',
    filename: '',
  };

  it.each<[string, Partial<RegistrySkill>, string]>([
    // The matcher this replaced never searched the id.
    ['id', { id: 'zebrafish-imaging' }, 'zebrafish'],
    ['name', { name: 'Zebrafish Imaging' }, 'zebrafish'],
    ['description', { description: 'Segments zebrafish embryos.' }, 'zebrafish'],
    ['tag', { tags: ['Zebrafish'] }, 'zebrafish'],
    ['keyword', { keywords: ['zebrafish'] }, 'zebrafish'],
  ])('finds a skill by its %s', (_field, override, query) => {
    expect(rankSkills([{ ...blankSkill, ...override }], query).hits).toHaveLength(1);
  });

  /// The one field the whole-phrase matcher searched and the Rust catalog does
  /// not. Every registry entry is Apache-2.0, so it separates nothing — and
  /// under word matching it listed every skill for `PACS`, whose singular `pac`
  /// is inside `apache`.
  it('does not search the license, in either catalog', () => {
    expect(rankSkills([{ ...blankSkill, license: 'Apache-2.0' }], 'PACS').hits).toEqual([]);
    expect(rankExtensions([{ ...blankExtension, license: 'Apache-2.0' }], 'PACS').hits).toEqual([]);
  });

  /// ⚠ And not through a LABEL either, which is where the licence went on
  /// holding the whole catalog after the field was dropped — the case the test
  /// above cannot see, because an entry with no tags has nowhere for it to hide.
  /// The registry publishes the licence a second time as one of the entry's own
  /// tag chips and, for a skill, a third time among its keywords, and both are
  /// searched. `Apache-2.0` and `apache` are both dropped, because they are the
  /// same licence spelled two ways and an equality test would keep the second.
  it('does not search the license republished as a tag or a keyword', () => {
    const licensed = { license: 'Apache-2.0' };
    for (const query of ['PACS', 'pac', 'apache', 'Apache-2.0']) {
      expect(
        rankSkills([{ ...blankSkill, ...licensed, tags: ['Apache-2.0'] }], query).hits,
        `skill tag, ${query}`
      ).toEqual([]);
      expect(
        rankSkills([{ ...blankSkill, ...licensed, keywords: ['apache'] }], query).hits,
        `skill keyword, ${query}`
      ).toEqual([]);
      expect(
        rankExtensions([{ ...blankExtension, ...licensed, tags: ['Apache-2.0'] }], query).hits,
        `extension tag, ${query}`
      ).toEqual([]);
    }

    // Only the licence goes. A label that says anything else stays searchable,
    // including one that merely CONTAINS a word of the licence.
    expect(
      rankSkills([{ ...blankSkill, ...licensed, tags: ['Apache Spark'] }], 'spark').hits
    ).toHaveLength(1);
    expect(
      rankExtensions([{ ...blankExtension, ...licensed, tags: ['ELN'] }], 'eln').hits
    ).toHaveLength(1);
    // An entry with no licence has no licence label to drop.
    expect(
      rankExtensions([{ ...blankExtension, tags: ['Apache-2.0'] }], 'apache').hits
    ).toHaveLength(1);
  });

  /// ⚠ **The category is a filter CONTROL, not a searched field.** It was one, and
  /// it is the licence's defect one field further on: `Core` names 57 of the
  /// registry's 129 skills and `Biomedical` 63, so `core` listed 59 of them here
  /// and `biomedical` 65 — half the modal, under a word nobody typed for a topic.
  /// This modal already answers the question with its own filter — `All` /
  /// `Core skills` / `Developer & authoring` / `Biomedical analysis`, whose
  /// Developer chip was measured showing exactly the 9 rows `developer` returned —
  /// and the website's copy of this matcher never searched the field at all, so
  /// the three were not in step.
  /// `type` is the same shape — the `Auto-applied` / `User-invocable` facet — and
  /// was never searched here.
  it('does not search a skill category or type, in either spelling', () => {
    for (const query of ['core', 'cor', 'ore', 'biomedical', 'developer']) {
      expect(
        rankSkills([{ ...blankSkill, category: 'Biomedical' }], query).hits,
        `category, ${query}`
      ).toEqual([]);
    }
    expect(
      rankSkills([{ ...blankSkill, type: 'User-invocable · /blank' }], 'invocable').hits
    ).toEqual([]);
    // Neither is dropped from the CATALOG: the modal still groups by category and
    // the card still shows the type. Only the matcher stops reading them.
    expect(
      rankSkills(
        [{ ...blankSkill, category: 'Biomedical', description: 'Biomedical imaging.' }],
        'biomedical'
      ).hits
    ).toHaveLength(1);
  });

  it('ranks a skill matched by id or name above a label, and a label above prose', () => {
    const skills: RegistrySkill[] = [
      { ...blankSkill, id: 'in-the-description', description: 'Segments zebrafish embryos.' },
      { ...blankSkill, id: 'in-a-keyword', keywords: ['zebrafish'] },
      { ...blankSkill, id: 'in-a-tag', tags: ['Zebrafish'] },
      { ...blankSkill, id: 'in-the-name', name: 'Zebrafish Imaging' },
    ];

    expect(ids(rankSkills(skills, 'zebrafish'))).toEqual([
      'in-the-name',
      // Two labels tie, and a tie keeps registry order.
      'in-a-keyword',
      'in-a-tag',
      'in-the-description',
    ]);
  });

  it.each<[string, Partial<RegistryExtension>]>([
    ['id', { id: 'zebrafish-0.1.0' }],
    // The installed config name, which can differ from the id and the display name.
    ['extension name', { extension_name: 'zebrafishagent' }],
    ['name', { name: 'Zebrafish Agent' }],
    ['organization', { organization: 'Zebrafish Lab' }],
    ['description', { description: 'Segments zebrafish embryos.' }],
    ['tag', { tags: ['Zebrafish'] }],
  ])('finds an extension by its %s', (_field, override) => {
    expect(rankExtensions([{ ...blankExtension, ...override }], 'zebrafish').hits).toHaveLength(1);
  });
});

/**
 * The word-boundary test the whole-query rank runs on, ported case for case from
 * `written_in` in `catalog_search.rs`, and then pushed at the characters where
 * JavaScript's own `\b` disagrees with Rust's `char::is_alphanumeric`. Those
 * cases are why this is a hand-written scan over code points and not a regular
 * expression — and every one of them is invisible to an ASCII fixture.
 */
describe('marketplace search — the query as written', () => {
  it('reads a phrase as written only between word boundaries', () => {
    expect(writtenIn('r scripting', 'r scripting')).toBe(true);
    // The second `r`, the one that is a word of its own.
    expect(writtenIn('tidy code for r.', 'r')).toBe(true);
    expect(writtenIn('snippets for scripting', 'r scripting')).toBe(false);
    expect(writtenIn('tidyverse', 'dy')).toBe(false);
    // Written at the second `a`, which overlaps the refused first occurrence —
    // so every position is tried, not only the first one a search would find.
    expect(writtenIn('ba a a', 'a a')).toBe(true);
    // An edge that is not a letter or digit is a boundary itself.
    expect(writtenIn('c++ code', '++')).toBe(true);
  });

  it('reads a word character as Rust does, not as `\b` does', () => {
    // `_` is a boundary here and a word character to `\b`, which would refuse
    // this.
    expect(writtenIn('snake_case', 'case')).toBe(true);
    // `ï` is a word character here and a boundary to `\b`, which would accept
    // this.
    expect(writtenIn('naïve', 'naï')).toBe(false);
    expect(writtenIn('naïve bayes', 'naïve')).toBe(true);
    // A letter outside the BMP is ONE character, so it closes a word. Compared
    // as UTF-16 units its trailing half is a lone surrogate, which matches no
    // letter, and the phrase would be read as written.
    expect(writtenIn('𝐚rna', 'rna')).toBe(false);
    // A digit is a word character, the same way `words` keeps `ggplot2` whole.
    expect(writtenIn('ggplot2 plots', 'ggplot')).toBe(false);
  });
});

/// The same cases `catalog_search.rs` asserts for
/// `names_only_the_license`, so the rule is pinned in both languages. Both
/// spellings the registry publishes go — `Apache-2.0` is the tag and `apache` is
/// the keyword, and an equality test would keep the second and leave `PACS`
/// matching 49 skills through it.
describe('namesOnlyTheLicense — which labels a catalog stops searching', () => {
  it.each(['Apache-2.0', 'apache', 'APACHE', 'apache 2.0', '2.0', 'Apache/2.0'])(
    'reads `%s` as saying nothing `Apache-2.0` does not',
    (label) => {
      expect(namesOnlyTheLicense(label, 'Apache-2.0')).toBe(true);
    }
  );

  it.each(['Apache Spark', 'MCP', 'ELN', 'Imaging', 'Registry', 'apachex'])(
    'keeps `%s`, which says more than the licence',
    (label) => {
      expect(namesOnlyTheLicense(label, 'Apache-2.0')).toBe(false);
    }
  );

  it('says no for a label with no words, and for an entry with no licence', () => {
    // Saying nothing at all is not the same as saying only the licence.
    expect(namesOnlyTheLicense('', 'Apache-2.0')).toBe(false);
    expect(namesOnlyTheLicense('  -  ', 'Apache-2.0')).toBe(false);
    expect(namesOnlyTheLicense('Apache-2.0', '')).toBe(false);
    expect(namesOnlyTheLicense('Apache-2.0', undefined)).toBe(false);
  });
});

describe('rankExtensions — the same matcher over the extensions catalog', () => {
  /// No field holds the phrase as written — SPOKEAgent says "SPOKE biomedical
  /// knowledge graph" and "spoke-knowledge-graph" — so the matcher this replaced
  /// showed nothing for it.
  it('ranks a multi-word query by the terms each extension matches', () => {
    const search = rankExtensions(MARKETPLACE_EXTENSIONS, 'SPOKE knowledge graph');

    expect(ids(search)).toEqual(['spokeagent', 'primekgagent', 'codegraphagent']);
    expect(search.hits[0].matchedTerms).toEqual(['spoke', 'knowledge', 'graph']);
  });

  it('drops its own catalog noise: every entry is an extension', () => {
    expect(rankExtensions(MARKETPLACE_EXTENSIONS, 'a knowledge graph extension').terms).toEqual([
      'knowledge',
      'graph',
    ]);
  });
});

/**
 * The finding this fix answers, measured by driving the real Browse-extensions
 * modal on 2026-09-12 against the live 37-entry registry — with the licence
 * FIELD already excluded by PR #242 and its port PR #255:
 *
 * | query        | matches |
 * | ------------ | ------- |
 * | *(empty)*    | 37      |
 * | `PACS`       | **31**  |
 * | `pac`        | **31**  |
 * | `apache`     | **31**  |
 * | `Apache-2.0` | 32      |
 * | `zzzznope`   | 0       |
 *
 * The three counts agreeing identifies the path: `PACS` → its singular `pac` →
 * inside `apache` → the `Apache-2.0` TAG chip on 31 rows, none of them about
 * PACS (BenchlingAgent, DNAnexusAgent, OMEROAgent…). The field had been removed
 * and the same string kept matching through a different field, so a test
 * asserting "the license is not searched" passed while the defect survived.
 *
 * Run against the bundled snapshot, not a fixture: a fixture without the licence
 * label cannot fail, which is precisely how this got through.
 */
describe('a licence republished as a label is not searchable through it', () => {
  const { extensions, skills } = FALLBACK_REGISTRY;

  /** A word of `license`, as a whole label — spelled out so this cannot be satisfied by the fix's own mistake. */
  const isALicenseWord = (license: string | undefined, label: string) =>
    (license ?? '')
      .split(/[^0-9A-Za-z]+/)
      .filter(Boolean)
      .some((word) => word.toLowerCase() === label.toLowerCase());

  it('still carries the overlap this pins, or proves nothing', () => {
    const tagged = (entries: readonly { tags: string[]; license?: string }[]) =>
      entries.filter((entry) =>
        entry.tags.some((tag) => tag.toLowerCase() === (entry.license ?? '').toLowerCase())
      ).length;
    expect(tagged(extensions), 'extensions tagged with their own licence').toBeGreaterThan(1);
    expect(tagged(skills), 'skills tagged with their own licence').toBeGreaterThan(1);
    expect(
      skills.filter((skill) => skill.keywords.some((k) => isALicenseWord(skill.license, k))).length,
      'skills with a licence word among their keywords'
    ).toBeGreaterThan(1);
  });

  it('finds nothing for the licence, in either catalog', () => {
    // `apache` occurs nowhere in the snapshot except each entry's own licence.
    // Measured before the fix: 31 extensions and 49 skills.
    expect(ids(rankExtensions(extensions, 'apache'))).toEqual([]);
    expect(ids(rankSkills(skills, 'apache'))).toEqual([]);
  });

  it('keeps the plural fallback and the substring rule, and loses only the licence', () => {
    const pacsSkills = ids(rankSkills(skills, 'PACS'));
    // Real hits: a skill whose keywords say `pacs`, and `pac` inside `PacBio`.
    expect(pacsSkills).toContain('biomedical-imaging-pathology');
    expect(pacsSkills).toContain('long-read-sequencing');
    // Licence-only, measured among the 51 before the fix.
    expect(pacsSkills).not.toContain('empirical-research-router');
    expect(pacsSkills).not.toContain('causal-identification-gates');

    const pacsExtensions = ids(rankExtensions(extensions, 'PACS'));
    for (const licenceOnly of ['benchlingagent', 'dnanexusagent', 'omeroagent']) {
      expect(pacsExtensions, `${licenceOnly} is not about PACS`).not.toContain(licenceOnly);
    }
  });

  /**
   * The fix does exactly one thing: it reads an entry as if the licence label
   * were not there. Asserted against a copy of the snapshot with those labels
   * removed from the DATA, so an over-broad rule — dropping every tag, or every
   * label containing a licence word — fails here even though it would satisfy the
   * assertions above. The queries are the legitimate ones the port's differential
   * harness measured, plus the licence ones.
   */
  it('changes nothing else about any query', () => {
    const withoutLicenceLabels = (labels: string[], license: string | undefined) =>
      labels.filter((label) => !isALicenseWord(license, label) && label !== license);
    const strippedExtensions = extensions.map((entry) => ({
      ...entry,
      tags: withoutLicenceLabels(entry.tags, entry.license),
    }));
    const strippedSkills = skills.map((entry) => ({
      ...entry,
      tags: withoutLicenceLabels(entry.tags, entry.license),
      keywords: withoutLicenceLabels(entry.keywords, entry.license),
    }));

    let matched = 0;
    for (const query of [
      'R scripting ggplot visualization',
      'r-scripting',
      'SPOKE knowledge graph',
      'ggplot',
      'heatmap',
      'python',
      'PACS',
      'pac',
      'apache',
      'Apache-2.0',
      'zzzznope',
      '',
    ]) {
      const skillHits = ids(rankSkills(skills, query));
      const extensionHits = ids(rankExtensions(extensions, query));
      expect(skillHits, `skills, ${query || '(empty)'}`).toEqual(
        ids(rankSkills(strippedSkills, query))
      );
      expect(extensionHits, `extensions, ${query || '(empty)'}`).toEqual(
        ids(rankExtensions(strippedExtensions, query))
      );
      matched += skillHits.length + extensionHits.length;
    }
    // The browse query alone contributes 166, so a run that read no entry at all
    // cannot pass this by matching empty against empty.
    expect(matched).toBeGreaterThan(166);
  });
});

/**
 * The two modals against the catalog the app actually ships — `registry.fallback.json`,
 * the snapshot `build-registry.mjs` writes from `landing/baam.html` beside
 * `landing/registry.json`, so these are the counts a user sees offline and (bar a
 * newer fetch) online.
 *
 * Bounds rather than equalities: a new entry whose prose says "lab" must not fail
 * this. What it pins is that a short query answers with a handful and not the
 * shelf, and that what the shelf is browsed BY still answers in full.
 */
describe('the shipped catalog: a short query is not the whole shelf', () => {
  const { extensions, skills } = FALLBACK_REGISTRY;

  it('answers a three-letter extensions query with a handful, not all 37', () => {
    // Guard: the words the flood came through are still in the catalog, so a pass
    // here means the rule refused them and not that the registry stopped saying
    // them.
    expect(
      extensions.filter((entry) => /agent/i.test(entry.name ?? '')).length,
      'cards whose name says Agent'
    ).toBeGreaterThan(19);
    expect(
      extensions.filter((entry) => /baranzinilab/i.test(entry.organization ?? '')).length,
      'cards whose organization says BaranziniLab'
    ).toBeGreaterThan(19);

    // Measured on this registry before the rule: 37 of 37, 36, 36.
    for (const [query, was] of [
      ['lab', 37],
      ['gen', 36],
      ['age', 36],
    ] as const) {
      const hits = rankExtensions(extensions, query).hits;
      expect(hits.length, `${query} (was ${was} of ${extensions.length})`).toBeLessThan(9);
    }

    // What the shelf is browsed BY: all three reach their rows as whole words.
    expect(ids(rankExtensions(extensions, 'SPOKEAgent'))).toEqual(['spokeagent']);
    expect(rankExtensions(extensions, 'BaranziniLab').hits.length).toBeGreaterThan(19);
    const ucsf = rankExtensions(extensions, 'UCSF').hits;
    expect(ucsf.length).toBeGreaterThan(4);
    expect(ucsf.length).toBeLessThan(extensions.length);
    expect(ids(rankExtensions(extensions, 'UCSF'))).toContain('ucsfhpcagent');
  });

  it('answers a curation-bucket query with the skills that say the word, not the bucket', () => {
    // Guard: the buckets are still most of the catalog, which is what made
    // searching them return most of it.
    for (const bucket of ['Core', 'Biomedical']) {
      const rows = skills.filter((entry) => entry.category === bucket).length;
      expect(rows * 3, `${bucket} names ${rows} of ${skills.length} skills`).toBeGreaterThan(
        skills.length
      );
    }

    // Measured on this registry before the field was dropped: 59, 59, 61, 65, 9.
    for (const [query, was] of [
      ['core', 59],
      ['cor', 59],
      ['ore', 61],
      ['biomedical', 65],
      ['developer', 9],
    ] as const) {
      const hits = rankSkills(skills, query).hits;
      expect(hits.length * 4, `${query} (was ${was} of ${skills.length})`).toBeLessThan(
        skills.length
      );
      // Every row still returned says the word itself, somewhere the matcher reads.
      for (const hit of hits) {
        const said = [
          hit.entry.id,
          hit.entry.name,
          hit.entry.description,
          ...(hit.entry.tags ?? []),
          ...(hit.entry.keywords ?? []),
        ].some((text) => String(text).toLowerCase().includes(query));
        expect(said, `${hit.entry.id} matched ${query} through nothing but its category`).toBe(
          true
        );
      }
    }

    // Browsing is untouched: the category is dropped from what is SEARCHED, not
    // from the catalog, and it is still what the modal groups by.
    expect(rankSkills(skills, '').hits.length).toBe(skills.length);
  });
});
