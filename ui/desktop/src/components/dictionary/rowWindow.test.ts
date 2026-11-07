import { describe, expect, it } from 'vitest';
import { entry } from './dictionaryTestKit';
import { byWritten, groupEntries } from './rows';
import {
  DICTIONARY_OVERSCAN,
  FIRST_DRAW,
  linesInView,
  nextDrawRange,
  rowsLines,
  sliceRows,
  type DrawnLine,
} from './rowWindow';

const drawn = <T>(lines: DrawnLine<T>[]) =>
  lines.flatMap((line) => (line.kind === 'row' ? [line.entry] : []));
const held = <T>(lines: DrawnLine<T>[]) =>
  lines.reduce((sum, line) => sum + (line.kind === 'gap' ? line.rows : 1), 0);

describe('the lines of the page', () => {
  it('counts one header line per group, then one per row', () => {
    expect(rowsLines([{ entries: [1, 2] }, { entries: [] }, { entries: [3] }])).toEqual([1, 4, 5]);
  });
});

describe('which rows are drawn', () => {
  const rows = Array.from({ length: 5_000 }, (_, index) => index);

  it('draws the rows in range and folds every run outside it into one gap', () => {
    const lines = sliceRows(rows, 1, { from: 2_001, to: 2_041 });
    expect(drawn(lines)).toEqual(rows.slice(2_000, 2_040));
    expect(lines[0]).toEqual({ kind: 'gap', rows: 2_000, key: 'gap:0' });
    expect(lines[lines.length - 1]).toEqual({ kind: 'gap', rows: 2_960, key: 'gap:2040' });
    // The gaps stand for exactly the rows they replace, so the scroll height never changes.
    expect(held(lines)).toBe(5_000);
  });

  it('keeps the focused row drawn wherever the list is scrolled, in its place', () => {
    const lines = sliceRows(rows, 1, { from: 0, to: 11 }, (row) => row === 4_000);
    expect(drawn(lines)).toEqual([...rows.slice(0, 10), 4_000]);
    expect(lines.slice(-3).map((line) => line.kind)).toEqual(['gap', 'row', 'gap']);
    expect(held(lines)).toBe(5_000);
  });

  it('draws only the first lines before the list is measured', () => {
    const groups = [{ entries: rows }];
    const [line] = rowsLines(groups);
    expect(drawn(sliceRows(rows, line, FIRST_DRAW))).toHaveLength(FIRST_DRAW.to - 1);
  });
});

describe('the lines in view', () => {
  // Two groups: 100 rows of 57px under a 24px header, then 10 rows below a 24px gap.
  const boxes = [
    { rowsLine: 1, rows: 100, top: 16, rowsTop: 40 },
    { rowsLine: 102, rows: 10, top: 40 + 100 * 57 + 24, rowsTop: 40 + 100 * 57 + 48 },
  ];

  it('reads a line from its group’s first row and the pitch', () => {
    expect(linesInView(boxes, 0, 600, 57)).toEqual({ first: 0, last: 1 + Math.floor(560 / 57) });
    expect(linesInView(boxes, 40 + 57 * 50, 40 + 57 * 60, 57)).toEqual({ first: 51, last: 61 });
  });

  it('reads a header line for a point above a group’s rows, and the next group below', () => {
    const second = boxes[1];
    expect(linesInView(boxes, second.top + 1, second.rowsTop + 57 * 3 + 1, 57)).toEqual({
      first: 101,
      last: 105,
    });
  });

  it('measures nothing for a list with no layout', () => {
    expect(linesInView([], 0, 600, 57)).toBeNull();
    expect(linesInView(boxes, 0, 0, 57)).toBeNull();
  });
});

describe('the range to draw', () => {
  it('adds the overscan on each side, and keeps the same range while the view stays inside it', () => {
    const first = nextDrawRange(FIRST_DRAW, { first: 40, last: 50 });
    expect(first).toEqual({ from: 40 - DICTIONARY_OVERSCAN, to: 51 + DICTIONARY_OVERSCAN });
    expect(nextDrawRange(first, { first: 42, last: 52 })).toBe(first);
    expect(nextDrawRange(first, { first: 60, last: 70 })).not.toBe(first);
    expect(nextDrawRange(first, null)).toBe(first);
  });
});

describe('groups at the 5,000 row cap', () => {
  it('sorts as the per call localeCompare did, with one collator', () => {
    const words = ['zeta', 'Alpha', 'alpha', 'EPAS1', 'epas10', 'epas2', 'Éclair', 'eclair', 'b'];
    const sample = words.map((written) => entry({ written }));
    const old = [...sample].sort(
      (a, b) =>
        a.written.localeCompare(b.written, undefined, { sensitivity: 'base', numeric: true }) ||
        a.written.localeCompare(b.written) ||
        a.id.localeCompare(b.id)
    );
    expect([...sample].sort(byWritten).map((row) => row.id)).toEqual(old.map((row) => row.id));
  });

  it('buckets every row once', () => {
    const rows = Array.from({ length: 5_000 }, (_, index) =>
      entry({ written: `T${index}`, source: index % 2 ? 'import' : 'user' })
    );
    const groups = groupEntries(rows);
    expect(groups.map((group) => [group.key, group.entries.length])).toEqual([
      ['user', 2_500],
      ['imported', 2_500],
    ]);
  });
});
