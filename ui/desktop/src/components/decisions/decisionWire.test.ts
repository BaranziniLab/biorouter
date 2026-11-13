import { describe, expect, it } from 'vitest';
import {
  decideArguments,
  decideQuestionKind,
  isDecisionProvider,
  MARKER_PARTS_SHOWN,
  markerItemsByImportance,
  markerParts,
  markerSourceLine,
  markerStatus,
  orderedProbabilities,
  parseDecideBatchSummary,
} from './decisionWire';
import {
  parseDecision,
  parseQuestion,
  type DecisionTurnMarkerItem,
} from '../../utils/sessionDecisions';
import { wireDecideSummary } from './decisionFixtures';

const item = (overrides: Partial<DecisionTurnMarkerItem>): DecisionTurnMarkerItem => ({
  origin: 'monitor',
  originId: 'form',
  label: 'Form reached',
  count: 18,
  passed: 0,
  failed: 18,
  errors: 0,
  lastPassProbability: 0.31,
  fired: false,
  ...overrides,
});

describe('decision providers', () => {
  it('names the decision providers and nothing else', () => {
    expect(isDecisionProvider('baseten_decision')).toBe(true);
    expect(isDecisionProvider('ollama_decision')).toBe(true);
    expect(isDecisionProvider('ollama')).toBe(false);
    expect(isDecisionProvider('baseten')).toBe(false);
    expect(isDecisionProvider(null)).toBe(false);
    expect(isDecisionProvider(undefined)).toBe(false);
  });
});

const marker = (...items: DecisionTurnMarkerItem[]) => ({ turnId: 't1', items });
const texts = (...items: DecisionTurnMarkerItem[]) =>
  markerParts(marker(...items)).map((part) => part.text);
const monitor = (overrides: Partial<DecisionTurnMarkerItem>) => item(overrides);
const decide = (overrides: Partial<DecisionTurnMarkerItem> = {}) =>
  item({
    origin: 'tool',
    originId: 'call-1',
    label: 'decide',
    count: 30,
    passed: 10,
    failed: 20,
    lastPassProbability: null,
    ...overrides,
  });
const gate = (overrides: Partial<DecisionTurnMarkerItem> = {}) =>
  item({
    origin: 'gate',
    originId: 'probe',
    label: 'Probe write test',
    count: 2,
    passed: 0,
    failed: 2,
    lastPassProbability: 0.03,
    ...overrides,
  });

describe('a turn marker’s closed line (F-19)', () => {
  it('counts one monitor’s checks, quiet or fired, with no probability', () => {
    expect(texts(monitor({ count: 5 }))).toEqual(['Monitor checked 5 times, quiet']);
    expect(texts(monitor({ count: 5, passed: 1, failed: 4, fired: true }))).toEqual([
      'Monitor checked 5 times, fired once',
    ]);
    expect(texts(monitor({ count: 1, failed: 1 }))).toEqual(['Monitor checked once, quiet']);
    for (const text of texts(monitor({}))) expect(text).not.toMatch(/P\b|0\.31/);
  });

  it('counts several monitors together: none fired, once, twice, 3 times', () => {
    const two = (passed: number) => [
      monitor({ originId: 'a', label: 'A', count: 5, passed, failed: 5 - passed }),
      monitor({ originId: 'b', label: 'B', count: 5, passed: 0, failed: 5 }),
    ];
    expect(texts(...two(0))).toEqual(['Monitors checked 10 times, none fired']);
    expect(texts(...two(1))).toEqual(['Monitors checked 10 times, fired once']);
    expect(texts(...two(2))).toEqual(['Monitors checked 10 times, fired twice']);
    expect(texts(...two(3))).toEqual(['Monitors checked 10 times, fired 3 times']);
    // One call that asked two monitors at once counts as two monitors.
    expect(texts(monitor({ label: 'Shell seen + Needs a rest', count: 4 }))).toEqual([
      'Monitors checked 4 times, none fired',
    ]);
  });

  it('counts decide calls and the chunks they read', () => {
    expect(texts(decide())).toEqual(['1 decide call read 30 chunks']);
    expect(texts(decide(), decide({ originId: 'call-2', count: 600 }))).toEqual([
      '2 decide calls read 630 chunks',
    ]);
    expect(texts(decide({ count: 12000 }))).toEqual(['1 decide call read 12,000 chunks']);
    // R3-F3: three calls on one question are one item that says how many calls made it.
    expect(texts(decide({ count: 600, calls: 3 }))).toEqual(['3 decide calls read 600 chunks']);
  });

  it('says a gate passed or did not, and counts several', () => {
    expect(texts(gate({ count: 1, passed: 1, failed: 0 }))).toEqual(['Gate checked once, passed']);
    expect(texts(gate())).toEqual(['Gate checked twice, not passed']);
    expect(
      texts(
        gate({ count: 2, passed: 2, failed: 0 }),
        gate({ originId: 'ship', label: 'Ship it', count: 1, passed: 0, failed: 1 })
      )
    ).toEqual(['Gates checked 3 times, passed twice, 1 not passed']);
  });

  it('names a decision chat’s and ad hoc calls, and ends any part with its errors', () => {
    expect(texts(item({ origin: 'chat', label: '', count: 3, passed: 0, failed: 0 }))).toEqual([
      'Decision chat answered 3 times',
    ]);
    expect(texts(item({ origin: 'adhoc', label: '', count: 3, passed: 0, failed: 0 }))).toEqual([
      '3 ad hoc calls',
    ]);
    expect(texts(decide({ errors: 2 }))).toEqual(['1 decide call read 30 chunks, 2 errors']);
    expect(texts(monitor({ errors: 1 }))).toEqual(['Monitor checked 18 times, quiet, 1 error']);
  });

  it('puts errors first, then a gate not passed, then a monitor that fired', () => {
    const quietMonitor = monitor({ count: 5 });
    const firedMonitor = monitor({ originId: 'f', count: 5, passed: 2, failed: 3, fired: true });
    // Ties keep monitors, decide calls, gates, other.
    expect(texts(item({ origin: 'chat', label: '', count: 1 }), decide(), quietMonitor)).toEqual([
      'Monitor checked 5 times, quiet',
      '1 decide call read 30 chunks',
      'Decision chat answered once',
    ]);
    // Three parts compete: the not-passed gate and the error are kept, never dropped.
    const ranked = markerParts(marker(firedMonitor, decide({ errors: 1 }), gate()));
    expect(ranked.map((part) => part.kind)).toEqual(['tool', 'gate', 'monitor']);
    expect(ranked.slice(0, MARKER_PARTS_SHOWN).map((part) => part.kind)).toEqual(['tool', 'gate']);
  });
});

describe('a turn marker’s source lines', () => {
  it('names each source by its display name, with the closed line’s words', () => {
    expect(markerSourceLine(monitor({ label: 'Done with shell', count: 3, passed: 2 }))).toBe(
      'Done with shell: checked 3 times, fired twice'
    );
    expect(markerSourceLine(monitor({ label: 'Needs a rest', count: 4 }))).toBe(
      'Needs a rest: checked 4 times, quiet'
    );
    expect(markerSourceLine(gate())).toBe('Probe write test: checked twice, not passed');
    expect(markerSourceLine(decide())).toBe('Decide call: 30 chunks');
    expect(markerSourceLine(decide({ errors: 2 }))).toBe('Decide call: 30 chunks, 2 errors');
  });

  it('never joins names with +', () => {
    expect(markerSourceLine(monitor({ label: 'Shell seen + Needs a rest', count: 2 }))).toBe(
      'Shell seen and Needs a rest: checked twice, quiet'
    );
  });

  it('orders the lines by importance, ties in their own order', () => {
    const lines = markerItemsByImportance(
      marker(
        monitor({ originId: 'a', label: 'Quiet one', count: 2 }),
        monitor({ originId: 'b', label: 'Fired one', count: 2, passed: 1, fired: true }),
        gate()
      )
    ).map((entry) => entry.label);
    expect(lines).toEqual(['Probe write test', 'Fired one', 'Quiet one']);
  });

  it('states the marker’s status: an error, then a gate not passed, then a monitor that fired', () => {
    expect(
      markerStatus(marker(monitor({ fired: true, passed: 1 }), gate(), decide({ errors: 1 })))
    ).toBe('error');
    expect(markerStatus(marker(monitor({ fired: true, passed: 1 }), gate()))).toBe('gate');
    expect(markerStatus(marker(monitor({ fired: true, passed: 1 })))).toBe('fired');
    expect(markerStatus(marker(monitor({})))).toBeNull();
  });
});

describe('the order a card draws its bars in', () => {
  it('follows the question’s option order, not the order the probabilities came in', () => {
    const question = parseQuestion({
      instructions: 'Which?',
      kind: { choice: { options: { rrms: null, spms: null, ppms: null } } },
    });
    const decision = parseDecision({
      question: 'q',
      answer: { type: 'choice', option: 'spms' },
      probabilities: { ppms: 0.2, spms: 0.5, rrms: 0.3 },
      source: { kind: 'native' },
      calibrated: true,
    })!;
    expect(orderedProbabilities(decision, question).map((bar) => bar.key)).toEqual([
      'rrms',
      'spms',
      'ppms',
    ]);
  });

  it('puts yes before no for a yes or no question', () => {
    const decision = parseDecision({
      question: 'q',
      answer: { type: 'binary', value: false },
      probabilities: { no: 0.7, yes: 0.3 },
      source: { kind: 'native' },
    })!;
    expect(orderedProbabilities(decision, undefined)).toEqual([
      { key: 'yes', p: 0.3 },
      { key: 'no', p: 0.7 },
    ]);
  });
});

describe('a decide batch summary', () => {
  it('is read from structuredContent, counts only', () => {
    const parsed = parseDecideBatchSummary({
      status: 'success',
      value: {
        content: [{ type: 'text', text: 'note-0003 has_ms P(yes) 0.97 ...' }],
        structuredContent: wireDecideSummary(),
      },
    });
    expect(parsed).toEqual({
      batchId: 'batch-42',
      chunks: 600,
      decided: 598,
      errors: 2,
      model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
      tier: 'private',
      calibrated: true,
      elapsedMs: 13_200,
      questions: [
        {
          name: 'has_ms',
          kind: 'yes_no',
          threshold: 0.9,
          decided: 598,
          passed: 41,
          yes: 41,
          review: 6,
          maxP: 0.98,
          answerCounts: [],
          groups: null,
          vote: null,
        },
        {
          name: 'edss_stated',
          kind: 'yes_no',
          threshold: 0.8,
          decided: 598,
          passed: 17,
          yes: 19,
          review: 0,
          maxP: 0.91,
          answerCounts: [],
          groups: null,
          vote: null,
        },
        {
          name: 'course',
          kind: 'choice',
          threshold: 0.6,
          decided: 598,
          passed: 280,
          yes: null,
          review: 12,
          maxP: 0.71,
          answerCounts: [
            { answer: 'rrms', count: 300 },
            { answer: 'spms', count: 200 },
            { answer: 'none', count: 98 },
          ],
          groups: null,
          vote: null,
        },
      ],
      errorGroups: [{ message: 'The decision model did not answer in 10 s', chunks: 2 }],
      voteModels: null,
      escalation: null,
    });
  });

  it('reads what escalate did: the second model, its counts and each chunk it was asked', () => {
    const parsed = parseDecideBatchSummary({
      status: 'success',
      value: {
        content: [],
        structuredContent: {
          ...wireDecideSummary(),
          escalation: {
            model: { provider: 'ollama_decision', model: 'clef-flash:latest' },
            label: 'clef-flash:latest',
            tier: 'private',
            firstLabel: 'tev1:0.8b',
            below: 0.8,
            firstDecided: 598,
            escalated: 2,
            answers: 2,
            decided: 1,
            changed: 1,
            errors: 1,
            items: [
              {
                chunkId: 'note-0003',
                answers: [
                  {
                    question: 'has_ms',
                    label: 'no',
                    p: 0.91,
                    by: 'clef-flash:latest',
                    firstLabel: 'yes',
                    firstP: 0.55,
                  },
                ],
              },
              { chunkId: 'note-0004', answers: [{ question: 'has_ms' }] },
              { answers: [] },
            ],
            errorGroups: [],
          },
        },
      },
    });
    expect(parsed?.escalation).toEqual({
      label: 'clef-flash:latest',
      tier: 'private',
      firstLabel: 'tev1:0.8b',
      below: 0.8,
      firstDecided: 598,
      escalated: 2,
      answers: 2,
      decided: 1,
      changed: 1,
      errors: 1,
      tooLong: 0,
      // The dropped item held the failure, and no group names it.
      failed: 0,
      // An answer with no label, or an item with no id, is dropped rather than drawn empty.
      items: [
        {
          chunkId: 'note-0003',
          answers: [
            {
              question: 'has_ms',
              label: 'no',
              p: 0.91,
              by: 'clef-flash:latest',
              firstLabel: 'yes',
              firstP: 0.55,
              error: null,
              tooLarge: false,
            },
          ],
        },
      ],
    });
  });

  it('tells the chunks too long for the second model from the ones it failed', () => {
    const kept = (chunkId: string, error: string, tooLarge?: boolean) => ({
      chunkId,
      answers: [
        {
          question: 'has_ms',
          label: 'yes',
          p: 0.6,
          by: 'tev1:0.8b',
          firstLabel: 'yes',
          firstP: 0.6,
          error,
          ...(tooLarge === undefined ? {} : { tooLarge }),
        },
      ],
    });
    const parsed = parseDecideBatchSummary({
      status: 'success',
      value: {
        content: [],
        structuredContent: {
          ...wireDecideSummary(),
          escalation: {
            model: { provider: 'baseten_decision', model: 'elephant-1' },
            label: 'elephant-1',
            tier: 'public',
            firstLabel: 'tev1:0.8b',
            below: 0.8,
            firstDecided: 30,
            escalated: 3,
            answers: 3,
            decided: 0,
            changed: 0,
            errors: 3,
            items: [
              kept('n01', 'Chunk 1 is about 20000 tokens, more than …', true),
              // Named only in `tooLargeIds`: still too long, not failed.
              kept('n02', 'Chunk 2 is about 20000 tokens, more than …'),
              kept('n03', 'the decision server failed'),
            ],
            errorGroups: [{ message: 'the decision server failed', chunkIds: ['n03'], more: 0 }],
            tooLargeIds: ['n01', 'n02'],
          },
        },
      },
    });
    expect(parsed?.escalation).toMatchObject({ errors: 3, tooLong: 2, failed: 1 });
    expect(parsed?.escalation?.items.map((item) => item.answers[0].tooLarge)).toEqual([
      true,
      true,
      false,
    ]);
  });

  it('does not count a chunk too long for the second model as failed when no chunk is listed', () => {
    // `errors` includes the chunks too long for the second model, so a result that lists
    // no chunk and no group must not say the same two as "too long" and "failed" both.
    const escalationWith = (errors: number) =>
      parseDecideBatchSummary({
        status: 'success',
        value: {
          content: [],
          structuredContent: {
            ...wireDecideSummary(),
            escalation: {
              model: { provider: 'baseten_decision', model: 'elephant-1' },
              label: 'elephant-1',
              tier: 'public',
              firstLabel: 'tev1:0.8b',
              below: 0.8,
              firstDecided: 30,
              escalated: 3,
              answers: 3,
              decided: 0,
              changed: 0,
              errors,
              items: [],
              errorGroups: [],
              tooLargeIds: ['n01', 'n02'],
            },
          },
        },
      })?.escalation;
    expect(escalationWith(2)).toMatchObject({ errors: 2, tooLong: 2, failed: 0 });
    expect(escalationWith(3)).toMatchObject({ errors: 3, tooLong: 2, failed: 1 });
    // Never below zero, whatever the counts say.
    expect(escalationWith(1)).toMatchObject({ errors: 1, tooLong: 2, failed: 0 });
  });

  it('reads the four answer groups that add up to the chunks decided (G-3)', () => {
    const wire = wireDecideSummary();
    const parsed = parseDecideBatchSummary({
      status: 'success',
      value: {
        content: [],
        structuredContent: {
          ...wire,
          questions: [
            {
              ...wire.questions[0],
              groups: {
                yesConfident: 38,
                yesReview: 3,
                noReview: 3,
                noConfident: 554,
                passFrom: 0.9,
                confidentFrom: 0.9,
                reviewFrom: 0.2,
              },
            },
          ],
        },
      },
    });
    expect(parsed?.questions[0].groups).toEqual({
      yesConfident: 38,
      yesReview: 3,
      noReview: 3,
      noConfident: 554,
    });
  });

  it('reads a vote of several models: its models by label, the majority and the agreement (G-1)', () => {
    const parsed = parseDecideBatchSummary({
      status: 'success',
      value: {
        content: [],
        structuredContent: {
          kind: 'decide_vote',
          batchId: 'batch-vote',
          chunks: 20,
          elapsedMs: 900,
          models: [
            {
              model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
              label: 'tev1:0.8b',
              tier: 'private',
              decided: 20,
              errors: 0,
              error: null,
            },
            {
              model: { provider: 'baseten_decision', model: 'decision-models/baseten-joey' },
              label: 'joey-1',
              tier: 'public',
              decided: 0,
              errors: 20,
              error: 'Baseten did not answer.',
            },
          ],
          questions: [
            {
              name: 'has_ms',
              type: 'yes_no',
              threshold: null,
              perModel: [{ yes: 7, no: 13 }, {}],
              majority: { yes: 7, no: 12 },
              ties: 1,
              compared: 20,
              agreed: 18,
            },
          ],
        },
      },
    });
    expect(parsed?.voteModels).toEqual([
      { label: 'tev1:0.8b', decided: 20, errors: 0, error: null },
      { label: 'joey-1', decided: 0, errors: 20, error: 'Baseten did not answer.' },
    ]);
    expect(parsed?.questions[0].vote).toEqual({
      majority: [
        { answer: 'yes', count: 7 },
        { answer: 'no', count: 12 },
      ],
      ties: 1,
      compared: 20,
      agreed: 18,
    });
  });

  it('is read from a text block that is the summary as JSON, plain or fenced', () => {
    const json = JSON.stringify(wireDecideSummary());
    for (const text of [json, `Batch done.\n\`\`\`json\n${json}\n\`\`\``]) {
      const parsed = parseDecideBatchSummary({
        status: 'success',
        value: { content: [{ type: 'text', text }] },
      });
      expect(parsed?.batchId).toBe('batch-42');
      expect(parsed?.questions).toHaveLength(3);
    }
  });

  it('reads the review count under its own name, never the retired uncertain field', () => {
    const summary = wireDecideSummary();
    const retired = {
      ...summary,
      questions: summary.questions.map(({ review, reviewIds, moreReview, ...rest }) => ({
        ...rest,
        uncertain: review,
        uncertainIds: reviewIds,
        moreUncertain: moreReview,
      })),
    };
    const parsed = parseDecideBatchSummary({
      status: 'success',
      value: { content: [], structuredContent: retired },
    });
    expect(parsed?.questions.map((question) => question.review)).toEqual([null, null, null]);
  });

  it('is null for an error result, prose with no summary, or a summary with no batch id', () => {
    expect(parseDecideBatchSummary({ status: 'error', error: 'refused' })).toBeNull();
    expect(
      parseDecideBatchSummary({
        status: 'success',
        value: { content: [{ type: 'text', text: 'done' }] },
      })
    ).toBeNull();
    expect(
      parseDecideBatchSummary({
        status: 'success',
        value: { content: [], structuredContent: { ...wireDecideSummary(), batchId: '' } },
      })
    ).toBeNull();
  });
});

describe('a decide call’s own arguments', () => {
  it('counts the chunks and lists the questions without their chunks', () => {
    const args = decideArguments({
      questions: {
        has_ms: { instructions: 'Does this note state MS?', type: 'yes_no', threshold: 0.9 },
        course: { instructions: 'Which course?', type: 'choice', options: { rrms: 'RR' } },
      },
      chunks: Array.from({ length: 600 }, (_, i) => ({ id: `note-${i}`, text: 'PHI text' })),
    });
    expect(args.chunks).toBe(600);
    expect(args.chunkIds).toHaveLength(600);
    expect(args.chunkIds[3]).toBe('note-3');
    expect(args.questions).toEqual([
      { name: 'has_ms', instructions: 'Does this note state MS?', kind: 'yes_no', threshold: 0.9 },
      { name: 'course', instructions: 'Which course?', kind: 'choice', threshold: null },
    ]);
  });

  it('counts only the chunks `only_ids` names, and keeps every id in call order', () => {
    const chunks = Array.from({ length: 60 }, (_, i) => ({ id: ` n${i} `, text: 'PHI text' }));
    for (const key of ['only_ids', 'onlyIds']) {
      const args = decideArguments({
        questions: { q: { instructions: 'Is it?' } },
        chunks,
        [key]: ['n3', 'n9'],
      });
      expect(args.chunks).toBe(2);
      // Every chunk keeps its place, so the card numbers one as the history does.
      expect(args.chunkIds.indexOf('n9')).toBe(9);
    }
    // An empty or malformed `only_ids` is no narrowing: the tool refuses it anyway.
    expect(decideArguments({ chunks, only_ids: [] }).chunks).toBe(60);
    expect(decideArguments({ chunks, only_ids: 'n3' }).chunks).toBe(60);
  });

  it('reads `question` as the instructions and infers the type the way the tool does', () => {
    const args = decideArguments({
      questions: {
        // No type: options make it a choice, levels a score, neither a yes or no.
        course: { question: 'Which MS course is described?', options: { rrms: 'RR', spms: 'SP' } },
        edss: { question: 'How disabled?', levels: ['none', 'mild', 'severe'] },
        has_ms: { question: 'Is MS stated?' },
        // The synonyms the tool accepts, however they are cased or separated.
        picked: { instructions: 'Which one?', type: 'Multiple-Choice', options: { a: 'A' } },
        rated: { instructions: 'How sure?', type: ' rating ', levels: ['low', 'high'] },
        multi: { instructions: 'Which?', type: 'multi/choice', options: { a: 'A' } },
        flag: { instructions: 'Is it?', type: 'Boolean' },
        // The written type wins over the fields, as in the tool.
        typed: { instructions: 'Is it?', type: 'yes_no', options: { a: 'A' } },
        // A word the tool refuses is shown as written, never as a yes or no.
        odd: { instructions: 'Rank them', type: 'Ranking' },
      },
    });
    expect(Object.fromEntries(args.questions.map((q) => [q.name, q.kind]))).toEqual({
      course: 'choice',
      edss: 'score',
      has_ms: 'yes_no',
      picked: 'choice',
      rated: 'score',
      multi: 'choice',
      flag: 'yes_no',
      typed: 'yes_no',
      odd: 'Ranking',
    });
    expect(args.questions[0].instructions).toBe('Which MS course is described?');
    // A null field is an absent one, as serde reads an `Option`.
    expect(decideQuestionKind({ type: null, options: null, levels: ['a'] })).toBe('score');
  });
});
