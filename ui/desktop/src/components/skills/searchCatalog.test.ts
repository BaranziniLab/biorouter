import { describe, expect, it } from 'vitest';
import type { CatalogBundle, CatalogSkill } from '../../api';
import type { SkillCatalogEntry } from './useSkillCatalog';
import { catalogSearchFields, rankCatalogEntries } from './searchCatalog';
import { Weight } from '../baam/search';

const state = {
  machineEnabled: true,
  session: 'default' as const,
  sessionViaBundle: false,
  hiddenContext: false,
  effective: true,
};

function single(name: string, description = `${name} does things`): SkillCatalogEntry {
  const skill: CatalogSkill = {
    name,
    description,
    slug: name,
    directory: `/skills/${name}`,
    sourceRoot: '/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    bundle: null,
    builtin: false,
    state,
  };
  return { kind: 'single', key: name, skill, enabled: true };
}

function pack(name: string, members: string[], displayName = name): SkillCatalogEntry {
  const bundle: CatalogBundle = {
    name,
    displayName,
    directory: `/skills/${name}`,
    sourceRoot: '/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    skills: members,
    package: null,
    builtin: false,
    state,
  };
  return { kind: 'bundle', key: name, bundle, enabled: true };
}

const label = (entry: SkillCatalogEntry) =>
  entry.kind === 'single' ? entry.skill.name : entry.bundle.displayName;

const names = (entries: readonly SkillCatalogEntry[], query: string) =>
  rankCatalogEntries(entries, query).hits.map((hit) => label(hit.entry));

/** The Settings list's reading: the word being typed is not a finished word. */
const typed = (entries: readonly SkillCatalogEntry[], query: string) =>
  rankCatalogEntries(entries, query, { typeahead: true }).hits.map((hit) => label(hit.entry));

describe('installed-catalog search fields', () => {
  /**
   * A single row's fields are the Rust ones, so the picker and
   * `skills__searchSkills` read the same text at the same weights
   * (`search_fields` in `agents/skills_extension.rs`).
   */
  it('weights a single skill the way the model-facing search does', () => {
    expect(catalogSearchFields(single('ggplot', 'plots'))).toEqual([
      ['ggplot', Weight.Name],
      ['plots', Weight.Prose],
      [undefined, Weight.Label],
    ]);
  });

  /**
   * ⚠ The decision this file exists to record: a member name is a LABEL on the
   * row that contains it, not that row's name. See the note on `searchCatalog.ts`.
   */
  it('reads a bundle row as its own names plus its members as labels', () => {
    expect(catalogSearchFields(pack('tidyverse', ['ggplot', 'dplyr'], 'Tidyverse'))).toEqual([
      ['Tidyverse', Weight.Name],
      ['tidyverse', Weight.Name],
      ['ggplot', Weight.Label],
      ['dplyr', Weight.Label],
    ]);
  });
});

describe('installed-catalog search', () => {
  /**
   * QA finding F5. The whole phrase is in no single field, so the filter this
   * replaces returned nothing; a query is a union of its words, ranked.
   */
  it('finds every skill a multi-word phrase names, best match first', () => {
    const entries = [single('ggplot'), single('pdf'), single('r-scripting')];
    expect(names(entries, 'R scripting ggplot visualization')).toEqual(['r-scripting', 'ggplot']);
  });

  it('holds a one-letter query to whole words', () => {
    const entries = [single('markdown-render'), single('r-scripting')];
    expect(names(entries, 'R')).toEqual(['r-scripting']);
  });

  /**
   * V3-23. The Settings list is typed into: `gg` on the way to `ggplot` listed
   * nothing (and read as a broken list) while `ggp` found it. With `typeahead`,
   * every prefix of a name finds it, and the whole word still ranks first. The
   * composer's picker does not pass the option (see `searchCatalog.ts`), which
   * the first test below pins as the default.
   */
  describe('while the last word is still being typed', () => {
    it('is off unless asked for, so the composer picker keeps its whole-word floor', () => {
      const entries = [single('bb-cli'), single('ggplot')];
      expect(names(entries, 'gg')).toEqual([]);
      expect(names(entries, 'ggp')).toEqual(['ggplot']);
    });

    it('finds a skill from the first one or two letters of its name', () => {
      const entries = [single('bb-cli'), single('ggplot'), single('pdf')];
      expect(typed(entries, 'g')).toEqual(['ggplot']);
      expect(typed(entries, 'gg')).toEqual(['ggplot']);
      expect(typed(entries, 'ggp')).toEqual(['ggplot']);
      expect(typed(entries, 'ggplot')).toEqual(['ggplot']);
    });

    it('matches the start of any word of the name, not only the first', () => {
      expect(typed([single('hpc-ucsf')], 'uc')).toEqual(['hpc-ucsf']);
    });

    it('ranks the whole word above a prefix, and reads no description by prefix', () => {
      const entries = [
        single('ralph'),
        single('markdown-render'),
        single('r-scripting'),
        // `r` starts a word of the sentence but of no name: whole words only there.
        single('pdf', 'Read and rewrite documents'),
        // Inside a word, not at its start.
        single('carbon'),
      ];
      expect(typed(entries, 'r')).toEqual(['r-scripting', 'ralph', 'markdown-render']);
    });

    it('finds a package by the start of a member it contains', () => {
      const entries = [single('pdf'), pack('tidyverse', ['ggplot', 'dplyr'], 'Tidyverse')];
      expect(typed(entries, 'gg')).toEqual(['Tidyverse']);
    });

    it('holds a finished word to whole words again', () => {
      const entries = [single('ggplot'), single('gg')];
      expect(typed(entries, 'gg')).toEqual(['gg', 'ggplot']);
      // A space after it says the word is done, so it is no longer a prefix.
      expect(typed(entries, 'gg ')).toEqual(['gg']);
    });

    it('gives the allowance to the last word only', () => {
      const entries = [single('r-scripting'), single('ralph'), single('ggplot')];
      // `r` is finished (a whole word, so `ralph` is out); `gg` is being typed.
      expect(typed(entries, 'r gg')).toEqual(['r-scripting', 'ggplot']);
    });
  });

  it('finds a package by a skill it contains', () => {
    const entries = [single('pdf'), pack('tidyverse', ['ggplot', 'dplyr'], 'Tidyverse')];
    expect(names(entries, 'ggplot')).toEqual(['Tidyverse']);
  });

  /**
   * ⚠ **What makes {@link Weight.Label} the right weight for a member name, and
   * the assertion that fails if it is changed to `Name`.**
   *
   * A skill called `ggplot` and a package that merely contains one both hold the
   * query as written and both match its only term, so the two are separated by
   * the field weight alone: 3 (a whole-word match) × 3 (`Name`) = 9 against
   * 3 × 2 (`Label`) = 6. The scores are asserted, not just the order, because at
   * `Name` the two would TIE at 39 — and a tie keeps catalog order, which lists
   * every bundle before every single skill, so the package would silently win
   * every such query.
   */
  it('ranks a skill above a package that merely contains one by that name', () => {
    const skill = single('ggplot');
    const bundle = pack('tidyverse', ['ggplot', 'dplyr'], 'Tidyverse');
    // Catalog order, which `useSkillCatalog` builds bundles-first.
    const hits = rankCatalogEntries([bundle, skill], 'ggplot').hits;

    expect(hits.map((hit) => hit.score)).toEqual([39, 36]);
    expect(hits[0].entry).toBe(skill);
    expect(hits[1].entry).toBe(bundle);
  });

  it('returns every row in catalog order when nothing is typed', () => {
    const entries = [pack('tidyverse', ['ggplot']), single('pdf')];
    expect(names(entries, '   ')).toEqual(['tidyverse', 'pdf']);
  });
});
