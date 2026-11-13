import { describe, expect, it } from 'vitest';
import { plainExcerpt } from './plainExcerpt';
import { subagentLine } from './SummarySubagents';
import { SUBAGENT_STATUS_WORDS } from '../subagent/subagentStatus';
import type { SubagentRow } from '../../utils/sessionSubagents';

describe('plainExcerpt (R2-5)', () => {
  // Live QA, 2026-11-11: a review's row read "Abstract · Flaw type(s) · Evidence
  // · Bri…", its answer's table header. A table row is not a summary.
  it('has nothing for a table row, with or without the outer pipes', () => {
    expect(plainExcerpt('| Abstract | Flaw type(s) | Evidence | Bri…')).toBe('');
    expect(plainExcerpt('| Patient | sodium (Na) | potassium ...')).toBe('');
    expect(plainExcerpt('| a | b |')).toBe('');
    expect(plainExcerpt('Abstract | Flaw type(s) | Evidence')).toBe('');
  });

  it('has nothing for a code fence', () => {
    expect(plainExcerpt('```')).toBe('');
    expect(plainExcerpt('```json')).toBe('');
    expect(plainExcerpt('~~~ python')).toBe('');
  });

  it('has nothing for a line that is only markup, so the status word shows', () => {
    expect(plainExcerpt('|---|:---:|')).toBe('');
    expect(plainExcerpt('---')).toBe('');
    expect(plainExcerpt('***')).toBe('');
    expect(plainExcerpt('   ')).toBe('');
    expect(plainExcerpt(null)).toBe('');
  });

  it('drops heading, list and quote markers, emphasis, code ticks and link targets', () => {
    expect(plainExcerpt('## Summary of **10** notes')).toBe('Summary of 10 notes');
    expect(plainExcerpt('- Found `3` matches in [the registry](https://example.org)')).toBe(
      'Found 3 matches in the registry'
    );
    expect(plainExcerpt('> _Task complete._ I reviewed the table')).toBe(
      'Task complete. I reviewed the table'
    );
  });

  it('leaves plain prose and snake_case names as they are', () => {
    expect(plainExcerpt('Task complete. I reviewed the 10-patient table')).toBe(
      'Task complete. I reviewed the 10-patient table'
    );
    expect(plainExcerpt('values_listed passed at 2 * 3 = 6')).toBe(
      'values_listed passed at 2 * 3 = 6'
    );
    // One pipe is prose, and a pipe inside code is code.
    expect(plainExcerpt('Read 12 notes | 3 flagged')).toBe('Read 12 notes | 3 flagged');
    expect(plainExcerpt('Ran `cut -f1 | sort | uniq` over the export')).toBe(
      'Ran cut -f1 | sort | uniq over the export'
    );
  });
});

describe("a finished subagent's roster line", () => {
  const row = (status: SubagentRow['status'], resultLine: string | null) =>
    ({ status, resultLine, quietSeconds: 0 }) as unknown as SubagentRow;

  it('falls back to the status word when the result line has nothing readable', () => {
    expect(subagentLine(row('completed', '| Abstract | Flaw type(s) | Evidence | Bri…'))).toBe(
      SUBAGENT_STATUS_WORDS.completed
    );
    expect(subagentLine(row('failed', '```'))).toBe(SUBAGENT_STATUS_WORDS.failed);
    expect(subagentLine(row('completed', null))).toBe(SUBAGENT_STATUS_WORDS.completed);
  });

  it('is the plain words of a readable result line', () => {
    expect(subagentLine(row('completed', '## Reviewed **4** abstracts'))).toBe(
      'Reviewed 4 abstracts'
    );
  });
});
