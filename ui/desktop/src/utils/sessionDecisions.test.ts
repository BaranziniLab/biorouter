import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../api';

const mocks = vi.hoisted(() => ({ get: vi.fn(), put: vi.fn(), headers: vi.fn() }));
vi.mock('../api/client.gen', () => ({ client: { get: mocks.get, put: mocks.put } }));
vi.mock('./userAction', () => ({ userActionHeaders: mocks.headers }));
/** The generated functions, watched: each still runs the real one (over the mocked client). */
const sdk = vi.hoisted(() => ({
  names: [
    'getSessionDecisionSummary',
    'getDecisionSettings',
    'putDecisionSettings',
    'listSessionDecisions',
    'getSessionDecision',
    'listDecisionModels',
  ],
  spies: new Map<string, ReturnType<typeof vi.fn>>(),
}));
vi.mock('../api/sdk.gen', async (original) => {
  const real = await original<Record<string, (...args: unknown[]) => unknown>>();
  const watched: Record<string, unknown> = { ...real };
  for (const name of sdk.names) {
    const spy = vi.fn(real[name]);
    sdk.spies.set(name, spy);
    watched[name] = spy;
  }
  return watched;
});

import {
  DECISION_ORIGINS,
  DECISION_OUTCOMES,
  MONITOR_ACTIONS,
  MONITOR_TIMINGS,
  MONITOR_WINDOW_CHARS_MAX,
  MONITOR_WINDOW_ENTRIES_MAX,
  answerProbability,
  calibrationText,
  decisionHistoryArtifact,
  decisionModelLabel,
  decisionQuery,
  decisionRevision,
  decisionSettingsToWire,
  decisionSourceRows,
  fetchDecisionDetail,
  fetchDecisionModels,
  fetchDecisionPage,
  fetchDecisionSettings,
  fetchDecisionSummary,
  firedMonitorIds,
  formatLatency,
  formatProbability,
  messageDecisionId,
  messageDecisionMarker,
  parseDecision,
  parseDecisionCatalog,
  parseDecisionDetail,
  parseDecisionModelEntries,
  parseDecisionPage,
  parseDecisionRecord,
  parseDecisionSettings,
  parsePhaseGateSnapshot,
  parseQuestion,
  parseSessionDecisionSummary,
  questionToWire,
  readableAnswer,
  refusalText,
  saveDecisionSettings,
  thresholdOnOption,
  thresholdSummary,
  type DecisionSettings,
} from './sessionDecisions';

const RS = (...parts: string[]) =>
  readFileSync(join(process.cwd(), '..', '..', 'crates', 'biorouter', 'src', ...parts), 'utf8');

/** The snake_case variant names of a Rust `pub enum`, as serde writes them. */
function rustVariants(source: string, name: string): string[] {
  const body = source.match(new RegExp(`pub enum ${name} \\{([\\s\\S]*?)\\n\\}`))?.[1];
  expect(body, name).toBeTruthy();
  return [
    ...body!
      .replace(/\/\/[^\n]*/g, '')
      .replace(/#\[[^\]]*\]/g, '')
      .matchAll(/^\s*([A-Z][A-Za-z]*)\b/gm),
  ].map((match) => match[1].replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase());
}

/** A `Decision` exactly as `decisions_serialize_camel_case_with_snake_case_enums` pins it. */
const wireDecision = (overrides: Record<string, unknown> = {}) => ({
  question: 'has_ms',
  answer: { type: 'binary', value: true },
  probabilities: { yes: 0.93, no: 0.07 },
  confidence: 0.86,
  passed: true,
  passProbability: 0.93,
  source: { kind: 'native' },
  calibrated: true,
  model: { provider: 'ollama_decision', model: 'tev1:0.8b', adapter: 'system_one' },
  tier: 'private',
  requestId: 'r1',
  latencyMs: 22,
  usage: { inputTokens: 412, outputTokens: 3 },
  raw: { answer: 0.93 },
  ...overrides,
});

const wireRecord = (overrides: Record<string, unknown> = {}) => ({
  id: 'd1',
  sessionId: 's1',
  createdAt: '2026-11-04T10:00:00Z',
  origin: 'tool',
  originId: 'call-1',
  label: 'decide',
  turnId: 't1',
  batchId: 'b1',
  chunkIndex: 0,
  chunkId: 'note-0001',
  model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
  servedModel: null,
  tier: 'private',
  calibrated: true,
  questions: {
    has_ms: {
      instructions: 'Does this note state the patient has multiple sclerosis?',
      kind: { binary: {} },
      threshold: { yes: 0.9 },
    },
  },
  decisions: [wireDecision()],
  outcome: 'passed',
  contextPreview: 'Neuro f/u. 34F, RRMS dx 2019',
  contextBytes: 4096,
  contextTruncated: false,
  error: null,
  latencyMs: 22,
  inputTokens: 412,
  outputTokens: 3,
  privacyEnforced: true,
  ...overrides,
});

beforeEach(() => {
  mocks.get.mockReset();
  mocks.put.mockReset();
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

describe('the contract, read from the Rust side', () => {
  it('knows every origin, outcome, monitor timing and action the daemon can send', () => {
    expect([...DECISION_ORIGINS].sort()).toEqual(
      rustVariants(RS('decision', 'types.rs'), 'DecisionOrigin').sort()
    );
    expect([...DECISION_OUTCOMES].sort()).toEqual(
      rustVariants(RS('decision', 'service.rs'), 'DecisionOutcome').sort()
    );
    const settings = RS('decision', 'settings.rs');
    expect([...MONITOR_TIMINGS].sort()).toEqual(rustVariants(settings, 'MonitorTiming').sort());
    expect([...MONITOR_ACTIONS].sort()).toEqual(rustVariants(settings, 'MonitorAction').sort());
  });
});

describe('parsing', () => {
  it('reads a decision with its full distribution, in the order sent', () => {
    const decision = parseDecision(wireDecision())!;
    expect(decision.answer).toEqual({ type: 'binary', value: true });
    expect(decision.probabilities).toEqual([
      { key: 'yes', p: 0.93 },
      { key: 'no', p: 0.07 },
    ]);
    expect(decision.passProbability).toBe(0.93);
    expect(decision.source).toEqual({ kind: 'native' });
    expect(decision.model).toEqual({
      provider: 'ollama_decision',
      model: 'tev1:0.8b',
      adapter: 'system_one',
    });
    expect(decision.inputTokens).toBe(412);
    expect(answerProbability(decision)).toBe(0.93);
    expect(readableAnswer(decision)).toBe('Yes');
  });

  it('degrades an unknown source to uncalibrated and an unknown outcome to an error, never a pass', () => {
    const decision = parseDecision(wireDecision({ source: { kind: 'oracle' }, calibrated: true }))!;
    expect(decision.source.kind).toBe('verbalized');
    const record = parseDecisionRecord(wireRecord({ outcome: 'sort_of' }))!;
    expect(record.outcome).toBe('error');
    expect(parseDecisionRecord(wireRecord({ id: '' }))).toBeNull();
  });

  it('reads externally tagged questions and thresholds, and writes them back the same way', () => {
    const choice = {
      instructions: 'Which MS course is described?',
      kind: { choice: { options: { rrms: 'Relapsing-remitting', ppms: null } } },
      threshold: { option: { name: 'rrms', p: 0.9 } },
    };
    const parsed = parseQuestion(choice);
    expect(parsed.kind).toEqual({
      type: 'choice',
      options: [
        { name: 'rrms', description: 'Relapsing-remitting' },
        { name: 'ppms', description: null },
      ],
    });
    expect(parsed.threshold).toEqual({ type: 'option', name: 'rrms', p: 0.9 });
    expect(questionToWire(parsed)).toEqual(choice);

    const score = parseQuestion({
      instructions: 'How severe?',
      kind: { score: { levels: ['mild', 'moderate', 'severe'] } },
      threshold: { at_least_level: { level: 1, p: 0.8 } },
    });
    expect(score.threshold).toEqual({ type: 'at_least_level', level: 1, p: 0.8 });
    expect(thresholdSummary(score.threshold!, score)).toBe('P(level ≥ moderate) ≥ 0.80');
    expect(questionToWire(parseQuestion({ instructions: 'q', kind: { binary: {} } }))).toEqual({
      instructions: 'q',
      kind: { binary: {} },
    });
  });

  it('reads a record, a page and a detail with its exact context', () => {
    const record = parseDecisionRecord(wireRecord())!;
    expect(record.questions[0].name).toBe('has_ms');
    expect(record.chunkId).toBe('note-0001');
    const page = parseDecisionPage({
      records: [wireRecord(), wireRecord(), wireRecord({ id: 'd2' })],
      nextCursor: 'c2',
      total: 5000,
    })!;
    // A duplicate row (a page boundary that moved) is listed once.
    expect(page.records.map((entry) => entry.id)).toEqual(['d1', 'd2']);
    expect(page.nextCursor).toBe('c2');
    expect(page.total).toBe(5000);

    const detail = parseDecisionDetail({
      record: wireRecord(),
      context: { messages: [{ role: 'user', content: 'Fill in the consent form' }] },
    })!;
    expect(detail.context).toEqual({
      type: 'messages',
      messages: [{ role: 'user', content: 'Fill in the consent form' }],
    });
    expect(
      parseDecisionDetail({ record: wireRecord(), context: { text: '  keep spaces ' } })!.context
    ).toEqual({
      type: 'text',
      text: '  keep spaces ',
    });
  });

  it('reads settings with their monitors, and writes the contract shape back', () => {
    const wire = {
      defaultModel: null,
      monitors: [
        {
          id: 'form-reached',
          name: 'Form reached',
          question: {
            instructions: 'Has the agent reached the consent form page?',
            kind: { binary: {} },
          },
          model: null,
          timing: 'post_iteration',
          window: { entries: 24, maxChars: 6000, toolCalls: true },
          threshold: 0.85,
          option: null,
          consecutive: 1,
          rearmBelow: 0.7,
          action: 'stop',
          onError: 'notify',
          maxEvalsPerTurn: 50,
          enabled: true,
        },
      ],
    };
    const settings = parseDecisionSettings(wire)!;
    expect(settings.monitors[0].action).toBe('stop');
    expect(settings.monitors[0].rearmBelow).toBe(0.7);
    expect(decisionSettingsToWire(settings)).toEqual(wire);
    // A wrapped answer reads the same.
    expect(parseDecisionSettings({ settings: wire })!.monitors).toHaveLength(1);
  });

  it('keeps a monitor window within what the window builder delivers', () => {
    // The daemon's ceilings are the window builder's own, so a larger value is
    // never offered, stored and shown while every evaluation cuts it.
    const window = RS('decision', 'window.rs');
    const rustConst = (name: string) =>
      Number(
        window.match(new RegExp(`pub const ${name}: usize = ([0-9_]+);`))?.[1].replace(/_/g, '')
      );
    expect(MONITOR_WINDOW_ENTRIES_MAX).toBe(rustConst('MAX_WINDOW_MESSAGES'));
    expect(MONITOR_WINDOW_CHARS_MAX).toBe(rustConst('MAX_WINDOW_CHARS'));
    const settings = parseDecisionSettings({
      monitors: [
        {
          id: 'form',
          name: 'Form',
          question: { instructions: 'Reached the form?', kind: { binary: {} } },
          window: { entries: 48, maxChars: 12000, toolCalls: true },
          threshold: 0.8,
        },
      ],
    })!;
    expect(settings.monitors[0].window).toEqual({ entries: 24, maxChars: 6000, toolCalls: true });
  });

  it('reads the catalog bare or wrapped, and a phase gate', () => {
    const entry = {
      provider: 'baseten_decision',
      model: 'decision-models/baseten-joey-2b-latest',
      displayName: 'joey-1',
      contextTokens: 16384,
      kinds: ['binary', 'choice', 'score'],
      tier: 'public',
      adapter: 'system_one',
      local: false,
    };
    expect(parseDecisionModelEntries([entry])![0].displayName).toBe('joey-1');
    expect(parseDecisionModelEntries({ models: [entry] })![0].tier).toBe('public');
    // The chat-wide reason rides beside the rows, for a choice no row stands for.
    const crew = 'This chat works under a Crew grant.';
    expect(parseDecisionCatalog({ models: [entry], sessionUnavailableReason: crew })).toMatchObject(
      { models: [{ displayName: 'joey-1' }], sessionUnavailableReason: crew }
    );
    expect(parseDecisionCatalog({ models: [entry] })!.sessionUnavailableReason).toBeNull();
    expect(parseDecisionCatalog([entry])!.sessionUnavailableReason).toBeNull();
    expect(parseDecisionCatalog({ sessionUnavailableReason: crew })).toBeNull();
    expect(
      parsePhaseGateSnapshot({
        threshold: 0.8,
        questionCount: 2,
        attempts: 2,
        maxAttempts: 3,
        lastPassed: false,
        lastMinP: 0.62,
        blocked: false,
      })
    ).toEqual({
      threshold: 0.8,
      questionCount: 2,
      attempts: 2,
      maxAttempts: 3,
      lastPassed: false,
      lastMinP: 0.62,
      blocked: false,
    });
    expect(parsePhaseGateSnapshot(undefined)).toBeNull();
  });

  it('reads a turn marker and a decision id off message metadata', () => {
    const message = {
      role: 'user',
      created: 0,
      content: [],
      metadata: {
        agentVisible: false,
        userVisible: true,
        decisionSummary: {
          turnId: 't1',
          items: [
            {
              origin: 'monitor',
              originId: 'form',
              label: 'Form',
              count: 18,
              lastPassProbability: 0.31,
              fired: true,
            },
          ],
        },
      },
    } as unknown as Message;
    expect(messageDecisionMarker(message)!.items[0].count).toBe(18);
    expect(firedMonitorIds([message])).toEqual(new Set(['form']));
    expect(
      messageDecisionId({ ...message, metadata: { decisionId: 'd9' } } as unknown as Message)
    ).toBe('d9');
  });
});

describe('words and figures', () => {
  it('formats probabilities and latency for a person', () => {
    expect(formatProbability(0.931)).toBe('0.93');
    expect(formatProbability(0.9999)).toBe('>0.99');
    expect(formatProbability(0.001)).toBe('<0.01');
    expect(formatProbability(1)).toBe('1.00');
    expect(formatLatency(22)).toBe('22 ms');
    expect(formatLatency(1400)).toBe('1.4 s');
  });

  it('names a Baseten decision model by its alias and an Ollama tag by its last segment', () => {
    const baseten = (model: string) => ({ provider: 'baseten_decision', model });
    expect(decisionModelLabel(baseten('decision-models/baseten-joey-2b-latest'))).toBe('joey-1');
    expect(decisionModelLabel(baseten('decision-models/baseten-elephant-9b-latest'))).toBe(
      'elephant-1'
    );
    expect(decisionModelLabel(baseten('decision-models/baseten-mammoth-27b-latest'))).toBe(
      'mammoth-1'
    );
    // A monitor saved with the alias the person typed reads the same.
    expect(decisionModelLabel(baseten('joey-1'))).toBe('joey-1');
    // A slug this build does not know reads without its prefix.
    expect(decisionModelLabel(baseten('decision-models/baseten-new-4b-latest'))).toBe(
      'baseten-new-4b-latest'
    );
    expect(decisionModelLabel({ provider: 'ollama_decision', model: 'tev1:0.8b' })).toBe(
      'tev1:0.8b'
    );
    expect(
      decisionModelLabel({ provider: 'ollama_decision', model: 'hf.co/acme/clef-flash:latest' })
    ).toBe('clef-flash:latest');
    expect(decisionModelLabel(null)).toBeNull();
  });

  it('marks the threshold only on the option it is about', () => {
    expect(thresholdOnOption({ type: 'yes', p: 0.8 }, 'yes')).toBe(0.8);
    expect(thresholdOnOption({ type: 'yes', p: 0.8 }, 'no')).toBeNull();
    expect(thresholdOnOption({ type: 'option', name: 'rrms', p: 0.9 }, 'rrms')).toBe(0.9);
    expect(thresholdOnOption({ type: 'at_least_level', level: 1, p: 0.8 }, '1')).toBeNull();
  });

  it('says uncalibrated in words', () => {
    expect(calibrationText({ kind: 'verbalized', reason: null }, false)).toMatch(/^Uncalibrated/);
    expect(calibrationText({ kind: 'logprob', mass: 0.6 }, false)).toMatch(/0\.60/);
    expect(calibrationText({ kind: 'native' }, true)).toMatch(/not validated on your data/);
  });

  it('reads a refusal from plain text or a JSON body', () => {
    expect(refusalText('This chat is private.')).toBe('This chat is private.');
    expect(refusalText({ message: 'Too many monitors' })).toBe('Too many monitors');
    expect(refusalText(null)).toBeNull();
  });
});

describe('the summary rows: one per source, never one per call', () => {
  const settings: DecisionSettings = parseDecisionSettings({
    monitors: [
      {
        id: 'form',
        name: 'Form reached',
        question: { instructions: 'q', kind: { binary: {} } },
        threshold: 0.85,
      },
      {
        id: 'idle',
        name: 'Idle',
        question: { instructions: 'q', kind: { binary: {} } },
        threshold: 0.9,
        enabled: false,
      },
    ],
  })!;

  it('draws as many rows as sources for 5,000 calls', () => {
    // 5,000 calls spread over 50 decide calls, two monitors and a gate.
    const aggregates = [
      ...Array.from({ length: 50 }, (_, i) => ({
        origin: 'tool',
        originId: `call-${i}`,
        label: 'decide',
        count: 96,
        passed: 40,
        failed: 56,
        errors: 0,
        lastAt: `2026-11-04T10:${String(i).padStart(2, '0')}:00Z`,
        lastAnswer: 'yes',
        lastPassProbability: i / 50,
      })),
      {
        origin: 'monitor',
        originId: 'form',
        label: 'Form reached',
        count: 180,
        passed: 2,
        failed: 178,
        errors: 0,
        lastAt: '2026-11-04T11:00:00Z',
        lastAnswer: 'no',
        lastPassProbability: 0.31,
      },
      {
        origin: 'gate',
        originId: 'verify',
        label: 'Check',
        count: 20,
        passed: 1,
        failed: 19,
        errors: 0,
        lastAt: '2026-11-04T11:05:00Z',
        lastAnswer: 'yes',
        lastPassProbability: 0.91,
      },
    ];
    const summary = parseSessionDecisionSummary({ total: 5000, aggregates, recentTurns: [] })!;
    expect(summary.total).toBe(5000);
    const rows = decisionSourceRows(summary, settings, {
      gates: [
        {
          phaseId: 'verify',
          title: 'Check the numbers',
          gate: parsePhaseGateSnapshot({
            threshold: 0.8,
            attempts: 1,
            maxAttempts: 3,
            lastPassed: true,
            lastMinP: 0.91,
          })!,
        },
      ],
    });
    expect(rows.map((row) => row.key)).toEqual([
      'monitor:form',
      'monitor:idle',
      'tool',
      'gate:verify',
    ]);
    const tool = rows.find((row) => row.key === 'tool')!;
    expect(tool.count).toBe(4800);
    // The newest decide call speaks for the row.
    expect(tool.lastPassProbability).toBeCloseTo(49 / 50);
    expect(tool.label).toBe('decide');
    expect(rows.find((row) => row.key === 'monitor:form')!.state).toBe('armed');
    expect(rows.find((row) => row.key === 'monitor:idle')!.state).toBe('off');
    expect(rows.find((row) => row.key === 'monitor:idle')!.count).toBe(0);
    const gate = rows.find((row) => row.key === 'gate:verify')!;
    expect(gate.state).toBe('passed');
    expect(gate.label).toBe('Check the numbers');
  });

  it('names a fired monitor and an erroring one by state', () => {
    const summary = parseSessionDecisionSummary({
      total: 12,
      aggregates: [
        {
          origin: 'monitor',
          originId: 'form',
          label: 'Form reached',
          count: 10,
          passed: 1,
          failed: 9,
          errors: 0,
          lastAt: '2026-11-04T11:00:00Z',
          lastAnswer: 'yes',
          lastPassProbability: 0.91,
        },
        {
          origin: 'monitor',
          originId: 'idle',
          label: 'Idle',
          count: 2,
          passed: 0,
          failed: 0,
          errors: 2,
          lastAt: '2026-11-04T11:00:00Z',
          lastAnswer: null,
          lastPassProbability: null,
        },
      ],
      recentTurns: [],
    })!;
    const enabled = {
      ...settings,
      monitors: settings.monitors.map((m) => ({ ...m, enabled: true })),
    };
    const rows = decisionSourceRows(summary, enabled);
    expect(rows.find((row) => row.key === 'monitor:form')!.state).toBe('fired');
    expect(rows.find((row) => row.key === 'monitor:idle')!.state).toBe('error');
  });
});

describe('when to refetch', () => {
  const call = (id: string, name: string, answered = true): Message =>
    ({
      role: 'assistant',
      created: 0,
      metadata: { agentVisible: true, userVisible: true },
      content: [
        { type: 'toolRequest', id, toolCall: { status: 'success', value: { name } } },
        ...(answered
          ? [
              {
                type: 'toolResponse',
                id,
                toolResult: { status: 'success', value: { content: [] } },
              },
            ]
          : []),
      ],
    }) as unknown as Message;

  it('moves when a decide call answers, not when it is only proposed, and never for other tools', () => {
    const empty = decisionRevision([]);
    expect(decisionRevision([call('a', 'decisions__decide', false)])).toBe(empty);
    expect(decisionRevision([call('a', 'developer__shell')])).toBe(empty);
    const one = decisionRevision([call('a', 'decisions__decide')]);
    expect(one).not.toBe(empty);
    expect(decisionRevision([call('a', 'decisions__decide')])).toBe(one);
    expect(
      decisionRevision([call('a', 'decide'), call('b', 'workspace__workspace_orchestrate')])
    ).not.toBe(one);
  });
});

describe('the history view’s filter and tab', () => {
  it('sends only the set filters, camelCase, with a capped page size', () => {
    expect(
      decisionQuery({ origin: 'monitor', originId: 'form', errorsOnly: false }, { limit: 999 })
    ).toEqual({
      origin: 'monitor',
      originId: 'form',
      limit: 200,
    });
    expect(decisionQuery({ minP: 0, maxP: 0.6, errorsOnly: true }, { before: 'c1' })).toEqual({
      minP: 0,
      maxP: 0.6,
      errorsOnly: true,
      before: 'c1',
      limit: 100,
    });
    // A decision card's link names its one call (`DecisionQuery.decisionId`).
    expect(decisionQuery({ decisionId: 'dec-1', origin: 'chat' })).toEqual({
      decisionId: 'dec-1',
      origin: 'chat',
      limit: 100,
    });
  });

  it('opens one decisions tab per chat', () => {
    const source = decisionHistoryArtifact('s1', { origin: 'gate', originId: 'verify' });
    expect(source).toMatchObject({ kind: 'decisions', sessionId: 's1', title: 'Decisions' });
  });
});

describe('requests carry the person’s proof', () => {
  const ok = (data: unknown) => ({ data, error: undefined, response: { status: 200 } });

  it('reads the summary through the decisions route with userActionHeaders()', async () => {
    mocks.get.mockResolvedValue(ok({ total: 0, aggregates: [], recentTurns: [] }));
    const { data, missing } = await fetchDecisionSummary('s1');
    expect(missing).toBe(false);
    expect(data!.total).toBe(0);
    expect(mocks.get).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/sessions/{session_id}/decisions/summary',
        path: { session_id: 's1' },
        headers: { 'X-User-Action': 'synthetic-proof' },
      })
    );
  });

  it('pages with the query and reads one call by id, both with the proof', async () => {
    mocks.get.mockResolvedValueOnce(ok({ records: [wireRecord()], nextCursor: null, total: 1 }));
    const page = await fetchDecisionPage('s1', { origin: 'tool' }, { before: 'c1' });
    expect(page.records).toHaveLength(1);
    expect(mocks.get.mock.calls[0][0]).toMatchObject({
      url: '/sessions/{session_id}/decisions',
      query: { origin: 'tool', before: 'c1', limit: 100 },
      headers: { 'X-User-Action': 'synthetic-proof' },
    });
    mocks.get.mockResolvedValueOnce(ok({ record: wireRecord(), context: { text: 'note' } }));
    const detail = await fetchDecisionDetail('s1', 'd1');
    expect(detail!.context).toEqual({ type: 'text', text: 'note' });
    expect(mocks.get.mock.calls[1][0]).toMatchObject({
      url: '/sessions/{session_id}/decisions/{decision_id}',
      path: { session_id: 's1', decision_id: 'd1' },
      headers: { 'X-User-Action': 'synthetic-proof' },
    });
  });

  /**
   * Every request goes through its generated function (`api/sdk.gen.ts`), which owns the URL and
   * the security scheme, never through the client's low-level methods by URL; and each carries
   * the person's proof.
   */
  it('calls the generated functions, each with the proof', async () => {
    for (const spy of sdk.spies.values()) spy.mockClear();
    const proof = { 'X-User-Action': 'synthetic-proof' };
    mocks.get.mockResolvedValueOnce(ok({ total: 0, aggregates: [], recentTurns: [] }));
    await fetchDecisionSummary('s1');
    mocks.get.mockResolvedValueOnce(ok({ defaultModel: null, monitors: [] }));
    await fetchDecisionSettings('s1');
    mocks.get.mockResolvedValueOnce(ok({ records: [], nextCursor: null, total: 0 }));
    await fetchDecisionPage('s1', { origin: 'monitor', originId: 'form' });
    mocks.get.mockResolvedValueOnce(ok({ record: wireRecord(), context: { text: 'note' } }));
    await fetchDecisionDetail('s1', 'd1');
    mocks.get.mockResolvedValueOnce(ok([]));
    await fetchDecisionModels(undefined, 's1');
    mocks.put.mockResolvedValueOnce(ok({ defaultModel: null, monitors: [] }));
    await saveDecisionSettings('s1', { defaultModel: null, monitors: [] });

    const called = (name: string) => sdk.spies.get(name)!.mock.calls[0]?.[0];
    expect(called('getSessionDecisionSummary')).toMatchObject({
      path: { session_id: 's1' },
      headers: proof,
    });
    expect(called('getDecisionSettings')).toMatchObject({
      path: { session_id: 's1' },
      headers: proof,
    });
    expect(called('listSessionDecisions')).toMatchObject({
      path: { session_id: 's1' },
      query: { origin: 'monitor', originId: 'form', limit: 100 },
      headers: proof,
    });
    expect(called('getSessionDecision')).toMatchObject({
      path: { session_id: 's1', decision_id: 'd1' },
      headers: proof,
    });
    expect(called('listDecisionModels')).toMatchObject({
      query: { sessionId: 's1' },
      headers: proof,
    });
    expect(called('putDecisionSettings')).toMatchObject({
      path: { session_id: 's1' },
      body: { defaultModel: null, monitors: [] },
      headers: proof,
    });
    // And the PUT still says what its body is.
    expect(mocks.put.mock.lastCall![0].headers).toMatchObject({
      'Content-Type': 'application/json',
      ...proof,
    });
  });

  it('keeps no low-level request by URL in the decision fetchers', () => {
    for (const file of ['sessionDecisions.ts', '../components/decisions/decisionChatApi.ts']) {
      const source = readFileSync(join(__dirname, file), 'utf8');
      expect(source, file).not.toMatch(/\bclient\.(get|put|post|delete)\(/);
      expect(source, file).not.toMatch(/url:\s*'\//);
    }
  });

  it('treats a daemon without the route as nothing to show, and a refusal as its sentence', async () => {
    mocks.get.mockResolvedValue({ data: undefined, error: 'Not Found', response: { status: 404 } });
    await expect(fetchDecisionSummary('s1')).resolves.toEqual({ data: null, missing: true });
    mocks.put.mockResolvedValue({
      data: undefined,
      error: 'A private chat may not use a public decision model.',
      response: { status: 400 },
    });
    await expect(saveDecisionSettings('s1', { defaultModel: null, monitors: [] })).rejects.toThrow(
      'A private chat may not use a public decision model.'
    );
    expect(mocks.put).toHaveBeenCalledWith(
      expect.objectContaining({
        url: '/sessions/{session_id}/decision-settings',
        body: { defaultModel: null, monitors: [] },
        headers: expect.objectContaining({ 'X-User-Action': 'synthetic-proof' }),
      })
    );
  });
});
