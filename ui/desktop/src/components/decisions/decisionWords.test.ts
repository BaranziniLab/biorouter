import { describe, expect, it } from 'vitest';
import {
  AlertCircle,
  AlertTriangle,
  ArrowRight,
  Check,
  CircleIcon,
  Eye,
  Flag,
  Lock,
  X,
} from '../icons/app-icons';
import {
  CALIBRATION_COPY,
  GATE_STATE_ICONS,
  OUTCOME_FILTER_OPTIONS,
  RECORD_GLYPHS,
  RECORD_GLYPH_TONES,
  SOURCE_FILTER_OPTIONS,
  decisionModelDisplay,
  decisionModelPlace,
  formatDecisionTime,
  namedProbability,
  outcomeFilterChoice,
  outcomeFilterKey,
  outcomeKeptFor,
  recordGlyphState,
  recordSubtitle,
  recordTitle,
  recordVerdict,
  ruleInWords,
  sourceFilterOptionsFor,
  thresholdLine,
  verdictWord,
  watchedProbability,
} from './decisionWords';
import type {
  Decision,
  DecisionOrigin,
  DecisionOutcome,
  DecisionRecord,
  NamedQuestion,
  Question,
} from '../../utils/sessionDecisions';

const NOW = new Date('2026-11-10T21:31:49');

const binary: Question = { instructions: 'Has it run?', kind: { type: 'binary' }, threshold: null };
const score: Question = {
  instructions: 'How severe?',
  kind: { type: 'score', levels: ['None', 'Mild', 'Moderate', 'Severe'] },
  threshold: null,
};

function decision(overrides: Partial<Decision> = {}): Decision {
  return {
    question: 'q',
    answer: { type: 'binary', value: true },
    probabilities: [
      { key: 'yes', p: 0.9 },
      { key: 'no', p: 0.1 },
    ],
    confidence: 0.8,
    passed: true,
    passProbability: 0.9,
    source: { kind: 'native' },
    calibrated: true,
    model: null,
    tier: null,
    requestId: null,
    latencyMs: null,
    inputTokens: null,
    outputTokens: null,
    raw: null,
    ...overrides,
  };
}

function record(overrides: Partial<DecisionRecord> = {}): DecisionRecord {
  const questions: NamedQuestion[] = [{ ...binary, name: 'q' }];
  return {
    id: 'r1',
    sessionId: 's1',
    createdAt: '2026-11-10T21:31:02',
    origin: 'monitor',
    originId: 'm1',
    label: 'Done with shell',
    turnId: '0b1f6d1e-6c55-4c3e-9a1e-2c1d4e5f6a7b',
    batchId: 'batch-7',
    chunkIndex: null,
    chunkId: null,
    model: null,
    servedModel: null,
    tier: null,
    calibrated: true,
    questions,
    decisions: [decision()],
    outcome: 'passed',
    contextPreview: '',
    contextBytes: 0,
    contextTruncated: false,
    error: null,
    latencyMs: null,
    inputTokens: null,
    outputTokens: null,
    privacyEnforced: true,
    ...overrides,
  };
}

const twoMonitors = (firedFirst: boolean, firedSecond: boolean) =>
  record({
    originId: 'm1+m2',
    label: 'Shell seen + Needs a rest',
    questions: [
      { ...binary, name: 'm1' },
      { ...binary, name: 'm2' },
    ],
    decisions: [
      decision({ question: 'm1', passed: firedFirst }),
      decision({ question: 'm2', passed: firedSecond }),
    ],
    outcome: firedFirst || firedSecond ? 'passed' : 'failed',
  });

/** Every string a record's words can produce, for the never-leak checks. */
function everyLine(r: DecisionRecord): string[] {
  return [recordTitle(r), recordSubtitle(r, NOW), recordVerdict(r)];
}

describe('formatDecisionTime', () => {
  it('reads a bad time as nothing', () => {
    expect(formatDecisionTime('not a time')).toBe('');
  });
  it('gives seconds today and the day before', () => {
    expect(formatDecisionTime('2026-11-10T13:31:02', NOW)).toMatch(/31:02/);
    expect(formatDecisionTime('2026-11-08T13:31:02', NOW)).not.toMatch(/31:02/);
  });
});

describe('decisionModelDisplay (R3)', () => {
  it('names the alias and where it runs', () => {
    expect(
      decisionModelDisplay({ provider: 'ollama_decision', model: 'tev1:0.8b' }, 'private', false)
    ).toBe('tev1:0.8b · this computer');
    expect(
      decisionModelDisplay(
        { provider: 'baseten_decision', model: 'decision-models/joey-1' },
        'public',
        false
      )
    ).toMatch(/ · Baseten$/);
  });
  it('reads a public Ollama as Ollama, not this computer', () => {
    expect(
      decisionModelDisplay({ provider: 'ollama_decision', model: 'tev1:0.8b' }, 'public', false)
    ).toBe('tev1:0.8b · Ollama');
  });
  it('gives any other provider the alias alone, never its key', () => {
    expect(decisionModelDisplay({ provider: 'other_decision', model: 'x/y-2' }, null, false)).toBe(
      'y-2'
    );
  });
  it('never prints a provider key or slug', () => {
    for (const provider of ['ollama_decision', 'baseten_decision']) {
      for (const tier of ['public', 'private', null] as const) {
        const text = decisionModelDisplay(
          { provider, model: 'decision-models/joey-1' },
          tier,
          false
        );
        expect(text).not.toMatch(/ollama_decision|baseten_decision|decision-models\//);
      }
    }
  });
  it('is null without a model', () => {
    expect(decisionModelDisplay(null)).toBeNull();
  });
  it('says where a browser surface runs it', () => {
    expect(decisionModelPlace('ollama_decision', { local: true, browser: true })).toBe(
      'the computer Biorouter runs on'
    );
    expect(decisionModelPlace('ollama_decision', { local: false, browser: false })).toBe('Ollama');
    expect(decisionModelPlace('unknown', { browser: false })).toBeNull();
  });
});

describe('probabilities (R2)', () => {
  it('names the probability beside the answer, yes and no lowercased', () => {
    expect(namedProbability('No', 0.88)).toBe('P(no) 0.88');
    expect(namedProbability('Yes', 0.12)).toBe('P(yes) 0.12');
    expect(namedProbability('rrms', 0.62)).toBe('P(rrms) 0.62');
    expect(namedProbability('Moderate', 0.4)).toBe('P(Moderate) 0.40');
    expect(namedProbability('No', null)).toBeNull();
    expect(namedProbability('Moderate', 0.4)).not.toContain('≥');
  });

  it('writes the threshold line for every rule', () => {
    expect(thresholdLine({ type: 'yes', p: 0.85 }, binary, 0.12)).toBe(
      'P(yes) 0.12 · threshold 0.85'
    );
    expect(thresholdLine({ type: 'yes', p: 0.3, below: true }, binary, 0.02)).toBe(
      'P(yes) 0.02 · threshold below 0.30'
    );
    expect(thresholdLine({ type: 'option', name: 'rrms', p: 0.8 }, null, 0.62)).toBe(
      'P(rrms) 0.62 · threshold 0.80'
    );
    expect(thresholdLine({ type: 'at_least_level', level: 2, p: 0.8 }, score, 0.4)).toBe(
      'P(level ≥ Moderate) 0.40 · threshold 0.80'
    );
  });

  it('omits the line without a threshold, never "No threshold"', () => {
    expect(thresholdLine(null, binary, 0.5)).toBeNull();
  });

  it('keeps the threshold when no probability came', () => {
    expect(thresholdLine({ type: 'no', p: 0.5 }, binary, null)).toBe('threshold 0.50');
    expect(watchedProbability({ type: 'no', p: 0.5 }, binary, 0.31)).toBe('P(no) 0.31');
  });
});

describe('ruleInWords', () => {
  it('says a rule in words, with no ≥ or <', () => {
    expect(ruleInWords({ type: 'yes', p: 0.85 })).toBe('when P(yes) is at or above 0.85');
    expect(ruleInWords({ type: 'yes', p: 0.3, below: true })).toBe('when P(yes) is below 0.30');
    expect(ruleInWords({ type: 'no', p: 0.85 })).toBe('when P(no) is at or above 0.85');
    expect(ruleInWords({ type: 'option', name: 'rrms', p: 0.7 })).toBe(
      'when P(rrms) is at or above 0.70'
    );
    expect(ruleInWords({ type: 'at_least_level', level: 1, p: 0.85 }, score)).toBe(
      'when P(Mild or a higher level) is at or above 0.85'
    );
    for (const text of [
      ruleInWords({ type: 'at_least_level', level: 1, p: 0.85, below: true }, score),
      ruleInWords({ type: 'yes', p: 0.3, below: true }),
    ]) {
      expect(text).not.toMatch(/[≥<]/);
    }
  });
});

describe('verdictWord (R1)', () => {
  const cases: [DecisionOrigin, DecisionOutcome, string | null][] = [
    ['monitor', 'passed', 'Fired'],
    ['monitor', 'failed', 'Quiet'],
    ['gate', 'passed', 'Passed'],
    ['gate', 'failed', 'Not passed'],
    ['tool', 'passed', null],
    ['tool', 'failed', null],
    ['chat', 'decided', null],
    ['adhoc', 'failed', null],
  ];
  it.each(cases)('%s %s reads %s', (origin, outcome, word) => {
    expect(verdictWord(origin, outcome)).toBe(word);
  });
  it('keeps Error and Refused for every origin', () => {
    for (const origin of ['chat', 'monitor', 'tool', 'gate', 'adhoc'] as const) {
      expect(verdictWord(origin, 'error')).toBe('Error');
      expect(verdictWord(origin, 'refused')).toBe('Refused');
    }
  });
  it('never says Failed', () => {
    for (const origin of ['chat', 'monitor', 'tool', 'gate', 'adhoc'] as const) {
      for (const outcome of ['passed', 'failed', 'decided', 'error', 'refused'] as const) {
        expect(verdictWord(origin, outcome) ?? '').not.toMatch(/fail/i);
      }
    }
  });
});

describe('record lines', () => {
  it('titles a monitor by its name and its verdict Fired or Quiet', () => {
    expect(recordTitle(record())).toBe('Done with shell');
    expect(recordVerdict(record())).toBe('Fired');
    const quiet = record({ outcome: 'failed', decisions: [decision({ passed: false })] });
    expect(recordVerdict(quiet)).toBe('Quiet');
    expect(recordVerdict(quiet)).not.toMatch(/fail/i);
  });

  it('titles a several-monitor call "2 monitors", names on line 2, never "+"', () => {
    const r = twoMonitors(true, false);
    expect(recordTitle(r)).toBe('2 monitors');
    expect(recordSubtitle(r, NOW)).toMatch(/ · Shell seen, Needs a rest$/);
    expect(recordVerdict(r)).toBe('1 of 2 fired');
    expect(recordVerdict(twoMonitors(false, false))).toBe('Quiet');
    for (const line of everyLine(r)) expect(line).not.toContain('+');
  });

  it('gives a one-monitor row only the time on line 2', () => {
    expect(recordSubtitle(record(), NOW)).toBe(formatDecisionTime(record().createdAt, NOW));
  });

  it('titles a gate by its phase and reads Passed or Not passed', () => {
    const gate = record({ origin: 'gate', label: 'Research', outcome: 'failed' });
    expect(recordTitle(gate)).toBe('Research');
    expect(recordVerdict(gate)).toBe('Not passed');
    expect(recordVerdict({ ...gate, outcome: 'passed' })).toBe('Passed');
  });

  it('titles a decide chunk by its chunk id, else Chunk N', () => {
    const tool = record({ origin: 'tool', label: 'decide', chunkId: 'note-30', chunkIndex: 29 });
    expect(recordTitle(tool)).toBe('note-30');
    expect(recordTitle({ ...tool, chunkId: null })).toBe('Chunk 30');
    expect(recordTitle({ ...tool, chunkId: null, chunkIndex: null })).toBe('Decide call');
  });

  it('reads a decide "no" with outcome failed as its answer, never Failed, drawn as a ring', () => {
    const tool = record({
      origin: 'tool',
      outcome: 'failed',
      chunkId: 'note-30',
      decisions: [decision({ answer: { type: 'binary', value: false }, passed: false })],
    });
    expect(recordVerdict(tool).toLowerCase()).toBe('no');
    expect(recordGlyphState(tool)).toBe('answer');
    expect(RECORD_GLYPHS[recordGlyphState(tool)]).not.toBe(X);
  });

  it('adds +N when a call asked more questions', () => {
    const chat = record({
      origin: 'chat',
      outcome: 'decided',
      decisions: [decision(), decision({ question: 'other' })],
    });
    expect(recordTitle(chat)).toBe('Decision chat');
    expect(recordVerdict(chat)).toBe('Yes +1');
    expect(recordTitle({ ...chat, origin: 'adhoc' })).toBe('Ad hoc call');
  });

  it('says Error and Refused whatever the origin', () => {
    expect(recordVerdict(record({ outcome: 'error' }))).toBe('Error');
    expect(recordVerdict(record({ origin: 'tool', outcome: 'refused' }))).toBe('Refused');
  });

  it('never shows a question name, batch id, turn id or provider key', () => {
    const records = [
      record(),
      twoMonitors(true, true),
      record({ origin: 'tool', chunkIndex: 3 }),
      record({ origin: 'chat', label: null }),
      record({ origin: 'gate', label: null }),
    ];
    for (const r of records) {
      for (const line of everyLine(r)) {
        expect(line).not.toContain(r.batchId as string);
        expect(line).not.toContain(r.turnId as string);
        expect(line).not.toMatch(/ollama_decision|baseten_decision|decision-models\//);
        expect(line).not.toMatch(/\bm1\b|\bm2\b/);
      }
    }
  });
});

describe('glyphs (R1, by shape)', () => {
  it('draws each state with its shape', () => {
    expect(RECORD_GLYPHS).toEqual({
      monitorFired: Flag,
      monitorQuiet: Eye,
      gatePassed: Check,
      gateNotPassed: X,
      answer: CircleIcon,
      error: AlertTriangle,
      refused: Lock,
    });
    expect(RECORD_GLYPH_TONES.monitorFired).toBe('accent');
    expect(RECORD_GLYPH_TONES.monitorQuiet).toBe('muted');
    expect(RECORD_GLYPH_TONES.gatePassed).toBe('success');
    expect(RECORD_GLYPH_TONES.error).toBe('warning');
  });

  it('never draws a quiet monitor with a cross', () => {
    const quiet = record({ outcome: 'failed', decisions: [decision({ passed: false })] });
    expect(recordGlyphState(quiet)).toBe('monitorQuiet');
    expect(RECORD_GLYPHS.monitorQuiet).not.toBe(X);
    expect(recordGlyphState(twoMonitors(false, true))).toBe('monitorFired');
    expect(recordGlyphState(record({ origin: 'gate', outcome: 'failed' }))).toBe('gateNotPassed');
    expect(recordGlyphState(record({ outcome: 'refused' }))).toBe('refused');
  });

  it('gives the gate dialog the same shapes', () => {
    expect(GATE_STATE_ICONS).toEqual({
      passed: Check,
      failed: X,
      blocked: AlertCircle,
      pending: CircleIcon,
      skipped: ArrowRight,
    });
  });
});

describe('calibration copy', () => {
  it('pins the sentences', () => {
    expect(CALIBRATION_COPY.estimate).toBe("This model's probabilities are its own estimate.");
    expect(CALIBRATION_COPY.uncalibratedDetail).toBe(
      'Uncalibrated: the model wrote this number itself.'
    );
    expect(CALIBRATION_COPY.calibrated).toBe(
      "The decision model's own probabilities, not validated on your data."
    );
    expect(CALIBRATION_COPY.estimateTip).toMatch(/rough guide\.$/);
  });
});

describe('Source filter', () => {
  it('lists every source in order, Any source first', () => {
    expect(SOURCE_FILTER_OPTIONS.map((o) => [o.origin, o.label])).toEqual([
      [null, 'Any source'],
      ['monitor', 'Monitors'],
      ['tool', 'Decide calls'],
      ['gate', 'Phase gates'],
      ['chat', 'Decision chat'],
      ['adhoc', 'Ad hoc calls'],
    ]);
  });
  it('offers only the sources a chat has, plus the current one', () => {
    expect(sourceFilterOptionsFor(['gate', 'monitor']).map((o) => o.origin)).toEqual([
      null,
      'monitor',
      'gate',
    ]);
    expect(sourceFilterOptionsFor(['monitor'], 'adhoc').map((o) => o.origin)).toEqual([
      null,
      'monitor',
      'adhoc',
    ]);
  });
});

describe('Outcome filter, the source-to-choices table (history spec 1.2)', () => {
  const rows = (origin: DecisionOrigin | null) =>
    OUTCOME_FILTER_OPTIONS(origin).map((o) => [o.label, o.filter]);

  it('Any source: Error or refused only', () => {
    expect(rows(null)).toEqual([
      ['Any outcome', { outcome: null, errorsOnly: false }],
      ['Error or refused', { outcome: null, errorsOnly: true }],
    ]);
  });
  it('Monitors: Fired, Quiet, Error or refused', () => {
    expect(rows('monitor')).toEqual([
      ['Any outcome', { outcome: null, errorsOnly: false }],
      ['Fired', { outcome: 'passed', errorsOnly: false }],
      ['Quiet', { outcome: 'failed', errorsOnly: false }],
      ['Error or refused', { outcome: null, errorsOnly: true }],
    ]);
  });
  it('Phase gates: Passed, Not passed, Error or refused', () => {
    expect(rows('gate')).toEqual([
      ['Any outcome', { outcome: null, errorsOnly: false }],
      ['Passed', { outcome: 'passed', errorsOnly: false }],
      ['Not passed', { outcome: 'failed', errorsOnly: false }],
      ['Error or refused', { outcome: null, errorsOnly: true }],
    ]);
  });
  it.each(['tool', 'chat', 'adhoc'] as const)('%s: Error or refused only', (origin) => {
    expect(rows(origin)).toEqual(rows(null));
  });
  it('never offers Passed to monitors, Fired to gates, or a threshold word elsewhere', () => {
    const labels = (origin: DecisionOrigin | null) =>
      OUTCOME_FILTER_OPTIONS(origin).map((o) => o.label);
    expect(labels('monitor')).not.toContain('Passed');
    expect(labels('gate')).not.toContain('Fired');
    for (const origin of ['tool', 'chat', 'adhoc', null] as const) {
      for (const label of labels(origin)) {
        expect(label).not.toMatch(/threshold|passed|fired|quiet|failed/i);
      }
    }
  });

  it('maps a filter to its row and says an outside outcome by its own word', () => {
    expect(outcomeFilterKey({ outcome: null, errorsOnly: false })).toBe('any');
    expect(outcomeFilterKey({ outcome: 'passed' })).toBe('passed');
    expect(outcomeFilterKey({ outcome: 'error' })).toBeNull();
    expect(outcomeFilterKey({ errorsOnly: true })).toBe('errors');
    expect(outcomeFilterChoice('monitor', { outcome: 'passed' })).toEqual({
      key: 'passed',
      label: 'Fired',
      listed: true,
    });
    expect(outcomeFilterChoice('monitor', { outcome: 'error' })).toEqual({
      key: null,
      label: 'Error',
      listed: false,
    });
    expect(outcomeFilterChoice('tool', { outcome: 'failed' }).label).toBe('Not passed');
    expect(outcomeFilterChoice('tool', { outcome: 'refused' }).label).toBe('Refused');
  });

  it('keeps an outcome only where the new source lists it', () => {
    expect(outcomeKeptFor('gate', { outcome: 'passed' })).toBe(true);
    expect(outcomeKeptFor('tool', { outcome: 'passed' })).toBe(false);
    expect(outcomeKeptFor('tool', { errorsOnly: true })).toBe(true);
    expect(outcomeKeptFor('chat', {})).toBe(true);
  });
});
