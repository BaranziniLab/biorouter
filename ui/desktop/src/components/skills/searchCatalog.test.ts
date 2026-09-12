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

const names = (entries: readonly SkillCatalogEntry[], query: string) =>
  rankCatalogEntries(entries, query).hits.map((hit) =>
    hit.entry.kind === 'single' ? hit.entry.skill.name : hit.entry.bundle.displayName
  );

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
