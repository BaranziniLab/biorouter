import { describe, expect, it } from 'vitest';
import { DICTIONARY_LABELS } from '../dictionary/copy';
import { EXTENSIONS_COPY } from '../extensions/copy';
import { scheduleCopy } from '../schedule/copy';
import { SKILLS_COPY } from '../skills/copy';
import { workflowsCopy } from '../workflows/copy';

/**
 * One wording for every page band's one primary (owner, 2026-11-19): a verb and its object, in
 * sentence case. "New <thing>" when the button makes one from scratch on this page, "Add <thing>"
 * when it brings one in from somewhere else (a marketplace, a file, a repository, a pasted list).
 *
 * The bands used to read "New schedule" and "New workflow" beside a bare "Add" on Extensions,
 * Skills and Dictionary, so two of the same coral button named their object and three did not,
 * and the widths jumped between pages. Applications, Knowledge and History have no primary.
 */
const PRIMARIES: ReadonlyArray<[page: string, label: string, verb: 'New' | 'Add']> = [
  ['Scheduler', scheduleCopy.newSchedule, 'New'],
  ['Workflows', workflowsCopy.newWorkflow, 'New'],
  ['Extensions', EXTENSIONS_COPY.add, 'Add'],
  ['Skills', SKILLS_COPY.add, 'Add'],
  ['Dictionary', DICTIONARY_LABELS.add, 'Add'],
];

describe('the page band primaries', () => {
  it.each(PRIMARIES)('%s: "%s" is "%s" and its object, in sentence case', (_page, label, verb) => {
    expect(label).toMatch(new RegExp(`^${verb} [a-z]+$`));
  });
});
