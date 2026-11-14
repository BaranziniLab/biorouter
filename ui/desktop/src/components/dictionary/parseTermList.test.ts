import { describe, expect, it, vi } from 'vitest';
import {
  MAX_IMPORT_BYTES,
  MAX_IMPORT_ROWS,
  beforeComment,
  exceedsImportLimits,
  fileOverImportBytes,
  formatForFileName,
  importFormatFor,
  overImportBytes,
  parseTermList,
  splitDelimited,
} from './parseTermList';

/**
 * The page reads a list the way the daemon will (`store/transfer.rs`), so the
 * format it names is the one the daemon parses and the line numbers of skipped
 * rows match what the person pasted. The daemon still decides every row.
 */
describe('parseTermList', () => {
  it('reads lines, with spoken forms after = and comments after #', () => {
    const result = parseTermList(
      ['# my lab', 'UCSF', '', 'EPAS1 = epas one, e pass one  # gene', 'C#', 'F# = f sharp'].join(
        '\n'
      ),
      'lines'
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.terms).toEqual([
      { line: 2, written: 'UCSF', spokenForms: [], note: null },
      { line: 4, written: 'EPAS1', spokenForms: ['epas one', 'e pass one'], note: null },
      { line: 5, written: 'C#', spokenForms: [], note: null },
      { line: 6, written: 'F#', spokenForms: ['f sharp'], note: null },
    ]);
  });

  it('starts a comment only at the start of a line or after white space, as the daemon does', () => {
    expect(beforeComment('C#')).toBe('C#');
    expect(beforeComment('#all of it')).toBe('');
    expect(beforeComment('UCSF\t# the campus')).toBe('UCSF\t');
  });

  it('reads CSV with a header naming the columns in any order, and spoken forms split on ;', () => {
    const result = parseTermList(
      'note,written,spoken_forms\n"a gene, hypoxia",EPAS1,epas one;e pass one\n,UCSF,\n\n',
      'csv'
    );
    expect(result.ok && result.terms).toEqual([
      {
        line: 2,
        written: 'EPAS1',
        spokenForms: ['epas one', 'e pass one'],
        note: 'a gene, hypoxia',
      },
      { line: 3, written: 'UCSF', spokenForms: [], note: null },
    ]);
  });

  it('reads headerless TSV as written, spoken forms, note', () => {
    const result = parseTermList('EPAS1\tepas one\tgene\nUCSF\n', 'tsv');
    expect(result.ok && result.terms).toEqual([
      { line: 1, written: 'EPAS1', spokenForms: ['epas one'], note: 'gene' },
      { line: 2, written: 'UCSF', spokenForms: [], note: null },
    ]);
  });

  it('keeps a quoted field whole, quotes, delimiters and line breaks included', () => {
    expect(splitDelimited('"say ""hi""","a,b"\nnext', ',')).toEqual([
      { line: 1, cells: ['say "hi"', 'a,b'] },
      { line: 2, cells: ['next'] },
    ]);
    expect(splitDelimited('"two\nlines",x\nthird', ',')).toEqual([
      { line: 1, cells: ['two\nlines', 'x'] },
      { line: 3, cells: ['third'] },
    ]);
  });

  it('answers unparsable for a quote that never closes', () => {
    expect(parseTermList('"EPAS1,epas one\nUCSF', 'csv')).toEqual({
      ok: false,
      format: 'csv',
      problem: 'unparsable',
    });
  });

  it('reads the export JSON and nothing else as JSON', () => {
    const exported = JSON.stringify({
      format: 'biorouter.dictionary',
      version: 1,
      entries: [
        { written: 'EPAS1', spoken_forms: ['epas one'], tier: 'public' },
        { written: 'Zeta Lab', tier: 'private', note: 'ours' },
      ],
    });
    const result = parseTermList(exported, 'json');
    expect(result.ok && result.terms).toEqual([
      { line: 1, written: 'EPAS1', spokenForms: ['epas one'], note: null, private: false },
      { line: 2, written: 'Zeta Lab', spokenForms: [], note: 'ours', private: true },
    ]);
    expect(parseTermList('{"entries": []}', 'json')).toMatchObject({
      ok: false,
      problem: 'not_an_export',
    });
    expect(parseTermList('not json', 'json')).toMatchObject({
      ok: false,
      problem: 'not_an_export',
    });
  });

  it('names the format from the file extension first, then from the text', () => {
    expect(formatForFileName('terms.TXT')).toBe('lines');
    expect(formatForFileName('terms.csv')).toBe('csv');
    expect(formatForFileName('terms.tsv')).toBe('tsv');
    expect(formatForFileName('biorouter-dictionary.json')).toBe('json');
    expect(formatForFileName('terms.md')).toBeNull();
    expect(importFormatFor('terms.csv', 'UCSF')).toBe('csv');
    expect(importFormatFor(null, '  {"format": "biorouter.dictionary"}')).toBe('json');
    expect(importFormatFor(null, 'EPAS1\tepas one\nUCSF\tyou see ess eff')).toBe('tsv');
    // A comma is how a pasted line lists its spoken forms, so pasted CSV reads as lines.
    expect(importFormatFor(null, 'EPAS1 = epas one, e pass one')).toBe('lines');
    expect(importFormatFor('notes.md', 'UCSF')).toBe('lines');
  });

  it('knows the daemon’s limits: 1 MiB of content and 5,000 rows', () => {
    expect(exceedsImportLimits('UCSF', 1)).toBe(false);
    expect(exceedsImportLimits('x'.repeat(MAX_IMPORT_BYTES + 1), 1)).toBe(true);
    // Counted in UTF-8 bytes, as the daemon counts them.
    expect(exceedsImportLimits('é'.repeat(MAX_IMPORT_BYTES / 2 + 1), 1)).toBe(true);
    expect(exceedsImportLimits('UCSF', MAX_IMPORT_ROWS + 1)).toBe(true);
  });

  it('answers the byte limit from the length when it can, and encodes only in between', () => {
    // At most a third of the limit is under it whatever the text holds.
    expect(overImportBytes('文'.repeat(Math.floor(MAX_IMPORT_BYTES / 3)))).toBe(false);
    // Between a third and the whole of it, the UTF-8 bytes decide.
    expect(overImportBytes('文'.repeat(Math.floor(MAX_IMPORT_BYTES / 3) + 1))).toBe(true);
    expect(overImportBytes('x'.repeat(MAX_IMPORT_BYTES))).toBe(false);
    // A surrogate pair is two units and four bytes.
    expect(overImportBytes('🧬'.repeat(MAX_IMPORT_BYTES / 4))).toBe(false);
    expect(overImportBytes('🧬'.repeat(MAX_IMPORT_BYTES / 4 + 1))).toBe(true);
    // Longer than the limit is over it, and nothing is encoded to say so.
    const encode = vi.spyOn(TextEncoder.prototype, 'encode');
    try {
      expect(overImportBytes('x'.repeat(MAX_IMPORT_BYTES + 1))).toBe(true);
      expect(overImportBytes('UCSF')).toBe(false);
      expect(encode).not.toHaveBeenCalled();
    } finally {
      encode.mockRestore();
    }
  });

  it('refuses a file by its size before anything reads it', () => {
    expect(fileOverImportBytes({ size: MAX_IMPORT_BYTES })).toBe(false);
    expect(fileOverImportBytes({ size: MAX_IMPORT_BYTES + 1 })).toBe(true);
  });
});
