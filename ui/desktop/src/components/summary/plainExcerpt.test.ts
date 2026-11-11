import { describe, expect, it } from 'vitest';
import { plainExcerpt } from './plainExcerpt';

describe('plainExcerpt (R2-5)', () => {
  it('reads a table row as its cells', () => {
    expect(plainExcerpt('| Patient | sodium (Na) | potassium ...')).toBe(
      'Patient · sodium (Na) · potassium ...'
    );
    expect(plainExcerpt('| a | b |')).toBe('a · b');
  });

  it('has nothing for a line that is only markup, so the status word shows', () => {
    expect(plainExcerpt('|---|:---:|')).toBe('');
    expect(plainExcerpt('---')).toBe('');
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
  });
});
