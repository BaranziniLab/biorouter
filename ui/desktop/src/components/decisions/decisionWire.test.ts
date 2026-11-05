import { describe, expect, it } from 'vitest';
import {
  decideArguments,
  isDecisionProvider,
  markerItemText,
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

describe('a turn marker line', () => {
  it('counts a monitor and states its last probability', () => {
    expect(markerItemText(item({}))).toBe('monitor Form reached 18 checks, last P 0.31');
  });

  it('counts a decide batch in chunks, with how many passed and errored', () => {
    expect(
      markerItemText(
        item({
          origin: 'tool',
          originId: 'call-1',
          label: 'decide',
          count: 600,
          passed: 41,
          failed: 557,
          errors: 2,
          lastPassProbability: null,
        })
      )
    ).toBe('decide 600 chunks, 41 passed, 2 errors');
  });

  it('says a monitor fired, and uses the singular for one check', () => {
    expect(markerItemText(item({ count: 1, fired: true, lastPassProbability: 0.92 }))).toBe(
      'monitor Form reached 1 check, last P 0.92, fired'
    );
  });

  it('writes large counts with separators, never one entry per call', () => {
    expect(
      markerItemText(
        item({
          origin: 'tool',
          label: '',
          count: 12000,
          passed: 0,
          failed: 0,
          lastPassProbability: null,
        })
      )
    ).toBe('decide 12,000 chunks');
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
          uncertain: 6,
          maxP: 0.98,
          answerCounts: [],
        },
        {
          name: 'edss_stated',
          kind: 'yes_no',
          threshold: 0.8,
          decided: 598,
          passed: 17,
          yes: 19,
          uncertain: 0,
          maxP: 0.91,
          answerCounts: [],
        },
        {
          name: 'course',
          kind: 'choice',
          threshold: null,
          decided: 598,
          passed: null,
          yes: null,
          uncertain: null,
          maxP: 0.71,
          answerCounts: [
            { answer: 'rrms', count: 300 },
            { answer: 'spms', count: 200 },
            { answer: 'none', count: 98 },
          ],
        },
      ],
      errorGroups: [{ message: 'The decision model did not answer in 10 s', chunks: 2 }],
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
    expect(args.questions).toEqual([
      { name: 'has_ms', instructions: 'Does this note state MS?', kind: 'yes_no', threshold: 0.9 },
      { name: 'course', instructions: 'Which course?', kind: 'choice', threshold: null },
    ]);
  });
});
