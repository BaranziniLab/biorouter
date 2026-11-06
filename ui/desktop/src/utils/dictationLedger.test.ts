import { describe, expect, it } from 'vitest';
import {
  correctionCandidates,
  locateEdit,
  mapDictatedSpan,
  mapPosition,
  MAX_DICTATED_SPANS,
  recordDictatedSpan,
  type DictatedSpan,
  type TextEdit,
} from './dictationLedger';

describe('locateEdit', () => {
  const cases: Array<[string, string, string, number | undefined, TextEdit | null]> = [
    // [name, before, after, caret after, expected]
    ['equal texts', 'abc', 'abc', undefined, null],
    ['insertion at the end', 'abc', 'abcd', 4, { start: 3, endBefore: 3, endAfter: 4 }],
    ['insertion at the start', 'abc', 'xabc', 1, { start: 0, endBefore: 0, endAfter: 1 }],
    ['insertion in the middle', 'abc', 'abXc', 3, { start: 2, endBefore: 2, endAfter: 3 }],
    ['deletion', 'abcdef', 'abef', 2, { start: 2, endBefore: 4, endAfter: 2 }],
    ['replacement', 'the gene', 'the GENE', 8, { start: 4, endBefore: 8, endAfter: 8 }],
    ['clear all', 'abc', '', 0, { start: 0, endBefore: 3, endAfter: 0 }],
    // A run of repeats: without a hint the change sits as far right as it can,
    // with one it ends at the caret.
    ['repeat, no hint', 'aa', 'aaa', undefined, { start: 2, endBefore: 2, endAfter: 3 }],
    ['repeat, typed at 0', 'aa', 'aaa', 1, { start: 0, endBefore: 0, endAfter: 1 }],
    ['repeat, typed at 1', 'aa', 'aaa', 2, { start: 1, endBefore: 1, endAfter: 2 }],
    ['repeat deletion at 0', 'aaa', 'aa', 0, { start: 0, endBefore: 1, endAfter: 0 }],
    ['word repeat', 'is is', 'is is is', 3, { start: 0, endBefore: 0, endAfter: 3 }],
  ];
  it.each(cases)('%s', (_name, before, after, caret, expected) => {
    const edit = locateEdit(before, after, caret);
    expect(edit).toEqual(expected);
    if (edit) {
      // Applying the edit to `before` yields `after`.
      expect(
        before.slice(0, edit.start) +
          after.slice(edit.start, edit.endAfter) +
          before.slice(edit.endBefore)
      ).toBe(after);
    }
  });

  it('a hint that would break equality outside the region is ignored', () => {
    expect(locateEdit('abc', 'abXc', 0)).toEqual({ start: 2, endBefore: 2, endAfter: 3 });
  });
});

describe('mapPosition', () => {
  const insertion: TextEdit = { start: 3, endBefore: 3, endAfter: 5 };
  const replacement: TextEdit = { start: 2, endBefore: 6, endAfter: 3 };
  it.each([
    [1, insertion, 'after', 1],
    [3, insertion, 'before', 3],
    [3, insertion, 'after', 5],
    [4, insertion, 'before', 6],
    [2, replacement, 'after', 2],
    [4, replacement, 'before', 2],
    [4, replacement, 'after', 3],
    [6, replacement, 'before', 3],
    [9, replacement, 'after', 6],
  ] as const)('%i through %o (%s) is %i', (position, edit, assoc, expected) => {
    expect(mapPosition(position, edit, assoc)).toBe(expected);
  });
});

function span(
  _prose: string,
  start: number,
  inserted: string,
  extra: Partial<DictatedSpan> = {}
): DictatedSpan {
  return {
    recordId: 'rec-1',
    start,
    end: start + inserted.length,
    current: inserted,
    raw: inserted.trim(),
    inserted,
    replaced: '',
    changes: [],
    state: 'pristine',
    model: 'whisper-small.en-q8_0',
    dictionaryRevision: 'etag-1',
    biasScope: 'public',
    ...extra,
  };
}

describe('mapDictatedSpan', () => {
  // "Check  the a pass one gene" with " a pass one" dictated at 9.
  const base = 'Check the a pass one gene';
  const dictated = span(base, 9, ' a pass one');

  it('a change before the span shifts it', () => {
    const after = `Please ${base}`;
    const mapped = mapDictatedSpan(dictated, base, after, 7);
    expect(mapped).toMatchObject({ start: 16, end: 27, state: 'pristine', current: ' a pass one' });
    expect(after.slice(mapped.start, mapped.end)).toBe(' a pass one');
  });

  it('a change after the span leaves it, and typing right at its end stays outside', () => {
    const after = `${base}s`;
    expect(mapDictatedSpan(dictated, base, after, after.length)).toMatchObject({
      start: 9,
      end: 20,
      state: 'pristine',
    });
    const atEnd = 'Check the a pass oneX gene';
    expect(mapDictatedSpan(dictated, base, atEnd, 21)).toMatchObject({ start: 9, end: 20 });
  });

  it('a correction inside the span marks it edited and reads the new text', () => {
    const after = 'Check the EPAS1 gene';
    const mapped = mapDictatedSpan(dictated, base, after, 15);
    expect(mapped.state).toBe('edited');
    expect(mapped.current).toBe(' EPAS1');
    expect(after.slice(mapped.start, mapped.end)).toBe(' EPAS1');
  });

  it('a change that straddles an edge, or covers more than the span, taints it', () => {
    const straddle = 'Check thX pass one gene';
    expect(mapDictatedSpan(dictated, base, straddle, 9).state).toBe('tainted');
    expect(mapDictatedSpan(dictated, base, 'something else entirely', 23).state).toBe('tainted');
  });

  it('an undo back to the text before the insert marks it undone', () => {
    const replacedSpan = span('say THIS now', 4, 'that', { replaced: 'THIS' });
    const mapped = mapDictatedSpan(replacedSpan, 'say that now', 'say THIS now', 8);
    expect(mapped).toMatchObject({ state: 'undone', current: 'THIS', end: 8 });
    const undone = mapDictatedSpan(dictated, base, 'Check the gene', 9);
    expect(undone.state).toBe('undone');
  });

  it('tainted and undone are final', () => {
    const tainted = { ...dictated, state: 'tainted' as const };
    expect(mapDictatedSpan(tainted, base, `x${base}`, 1)).toBe(tainted);
  });
});

describe('recordDictatedSpan', () => {
  it(`keeps at most ${MAX_DICTATED_SPANS} spans, dropping the oldest`, () => {
    let spans: DictatedSpan[] = [];
    for (let i = 0; i < MAX_DICTATED_SPANS + 3; i++) {
      spans = recordDictatedSpan(spans, span('', 0, `word${i}`, { recordId: `rec-${i}` }));
    }
    expect(spans).toHaveLength(MAX_DICTATED_SPANS);
    expect(spans[0].recordId).toBe('rec-3');
  });
});

describe('correctionCandidates', () => {
  const edited = (inserted: string, current: string, extra: Partial<DictatedSpan> = {}) =>
    span('', 0, inserted, { current, state: 'edited', ...extra });

  it('offers the replaced run of words', () => {
    expect(
      correctionCandidates(
        edited(
          ' the gene a pass one is expressed in the lung',
          ' the gene EPAS1 is expressed in the lung'
        )
      )
    ).toEqual([{ record_id: 'rec-1', kind: 'correction', from: 'a pass one', to: 'EPAS1' }]);
  });

  it('reads a span whose words mostly changed as a rewrite, as the plan words it', () => {
    // Three of five inserted words changed: more than half, so nothing is offered.
    expect(correctionCandidates(edited(' the a pass one gene', ' the EPAS1 gene'))).toEqual([]);
  });

  it('offers nothing for a pristine, tainted or undone span', () => {
    for (const state of ['pristine', 'tainted', 'undone'] as const) {
      expect(correctionCandidates(edited(' a b', ' a c', { state }))).toEqual([]);
    }
  });

  it('drops a rewrite, more than half of the words changed', () => {
    expect(correctionCandidates(edited(' one two three four', ' five six seven four'))).toEqual([]);
  });

  it('offers nothing for a pure deletion or a pure addition', () => {
    expect(correctionCandidates(edited(' the big gene', ' the gene'))).toEqual([]);
    expect(correctionCandidates(edited(' the gene', ' the big gene'))).toEqual([]);
  });

  it('a run that puts back a correction the engine made is a rejected pair for that change', () => {
    const span = edited(' the SPOKE graph', ' the spoke graph', {
      changes: [
        { start: 4, end: 9, from: 'spoke', to: 'SPOKE', stage: 'exact', entry_id: 'dct_1' },
      ],
    });
    expect(correctionCandidates(span)).toEqual([
      { record_id: 'rec-1', kind: 'rejected', from: 'spoke', to: 'SPOKE', entry_id: 'dct_1' },
    ]);
  });

  it('applies the cheap size checks: at most three words a side, no comma in the source', () => {
    expect(correctionCandidates(edited(' a b c d e f g h', ' a b c d X Y Z W h'))).toEqual([]);
    expect(
      correctionCandidates(
        edited(' the alpha, beta gene is here now', ' the alphabeta gene is here now')
      )
    ).toEqual([]);
  });

  it('offers at most three pairs from one span', () => {
    const inserted = ' w1 a w2 b w3 c w4 d w5 e w6 f w7 g w8 h';
    const current = ' w1 A w2 B w3 C w4 D w5 E w6 f w7 g w8 h';
    expect(correctionCandidates(edited(inserted, current))).toHaveLength(3);
  });
});
