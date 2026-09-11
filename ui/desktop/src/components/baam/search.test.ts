import { describe, expect, it } from 'vitest';
import { MARKETPLACE_EXTENSIONS, MARKETPLACE_SKILLS } from './marketplace.fixture';
import { rankExtensions, rankSkills, type RegistryExtension, type RegistrySkill } from './registry';
import {
  EXTENSION_NOISE,
  isBrowseQuery,
  parseQuery,
  rankEntries,
  scoreEntry,
  searchTerms,
  SKILL_NOISE,
  Weight,
  type SearchField,
  type SearchResult,
} from './search';

/**
 * The marketplace matcher, ported from `crates/biorouter/src/marketplace/search.rs`
 * (PR #242). The first half ports that file's own tests against the same three
 * synthetic entries, so the two languages are pinned by one set of cases; the
 * second half runs finding F5's queries against real registry rows.
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

  it('returns the union, ranked by terms matched and then by where they matched', () => {
    const search = rank('R scripting');
    expect(ids(search)).toEqual(['r-scripting', 'prose-only']);
    expect(search.hits[0].matchedTerms).toEqual(['r', 'scripting']);
    expect(search.hits[1].matchedTerms).toEqual(['scripting']);

    // A match in the name outranks the same match in the description.
    expect(ids(rank('scripting'))).toEqual(['r-scripting', 'prose-only']);
  });

  /// Everything the substring matcher found is still found: a query that
  /// occurs verbatim in a field is a hit even when its terms are too short to
  /// match on their own.
  it('still finds a verbatim occurrence', () => {
    expect(rank('s p').hits).toEqual([]);

    const search = rank('dy');
    // `dy` inside `Tidyverse`.
    expect(ids(search)).toEqual(['r-scripting']);
    expect(search.hits[0].matchedTerms).toEqual([]);
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

  /// Verbatim is a tier of its own, not one more term: here it wins while
  /// matching FEWER terms, because `io` is too short to match inside `audio`.
  it('ranks a verbatim match above every term match, even one matching more terms', () => {
    const query = parseQuery('io pipe');
    const verbatimInProse = scoreEntry(query, [['audio pipeline', Weight.Prose]]);
    const bothTermsInNames = scoreEntry(query, [
      ['IO', Weight.Name],
      ['Pipe', Weight.Name],
    ]);

    expect(verbatimInProse.matchedTerms).toEqual(['pipe']);
    expect(bothTermsInNames.matchedTerms).toEqual(['io', 'pipe']);
    expect(verbatimInProse.score).toBeGreaterThan(bothTermsInNames.score);
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

  /// A one-letter query occurs verbatim inside most entries, and a verbatim hit
  /// is kept so nothing the old matcher showed is lost. The whole-word rule
  /// still decides the order: the three skills about R lead, and the verbatim-
  /// only hits follow in registry order.
  it('ranks the R skills first for `R` alone, above what merely contains an r', () => {
    const search = rankSkills(MARKETPLACE_SKILLS, 'R');

    expect(ids(search)).toEqual([
      'r-scripting',
      'ggplot-visualization',
      'clinical-biostatistics',
      'scientific-visual-communication',
      'python-scripting',
      'single-cell',
    ]);
    expect(search.hits.map((hit) => hit.matchedTerms)).toEqual([['r'], ['r'], ['r'], [], [], []]);
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
    ['category', { category: 'Biomedical' }, 'biomedical'],
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

describe('rankExtensions — the same matcher over the extensions catalog', () => {
  /// The phrase is in no field verbatim — SPOKEAgent says "SPOKE biomedical
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
