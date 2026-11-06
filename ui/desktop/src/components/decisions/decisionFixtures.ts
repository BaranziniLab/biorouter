/**
 * Decision-log rows as the daemon sends them (`DecisionDetail` in
 * `crates/biorouter/src/decision/log.rs`, camelCase), for the chat surfaces'
 * tests: a yes or no decision with a threshold, a verbalized choice, and a
 * failed call.
 */

export function wireBinaryDetail(overrides: Record<string, unknown> = {}) {
  return {
    record: {
      id: 'dec-1',
      sessionId: 'chat-1',
      createdAt: '2026-11-04T20:00:00Z',
      origin: 'chat',
      originId: null,
      label: null,
      turnId: 'turn-7',
      batchId: null,
      chunkIndex: null,
      chunkId: null,
      model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
      servedModel: 'tev1:0.8b',
      tier: 'private',
      calibrated: true,
      questions: {
        has_ms: {
          instructions: 'Does this note state the patient has multiple sclerosis?',
          kind: { binary: {} },
          threshold: { yes: 0.8 },
        },
      },
      decisions: [
        {
          question: 'has_ms',
          answer: { type: 'binary', value: true },
          probabilities: { yes: 0.93, no: 0.07 },
          confidence: 0.86,
          passed: true,
          passProbability: 0.93,
          source: { kind: 'native' },
          calibrated: true,
          model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
          tier: 'private',
          requestId: 'req-1',
          latencyMs: 22,
          usage: { inputTokens: 120, outputTokens: 1 },
          raw: {},
        },
      ],
      outcome: 'passed',
      contextPreview: 'Neuro f/u. 34F, RRMS dx 2019',
      contextBytes: 28,
      contextTruncated: false,
      error: null,
      latencyMs: 22,
      inputTokens: 120,
      outputTokens: 1,
      privacyEnforced: true,
      ...overrides,
    },
    context: { text: 'Neuro f/u. 34F, RRMS dx 2019' },
  };
}

export function wireChoiceDetail() {
  return {
    record: {
      ...wireBinaryDetail().record,
      id: 'dec-2',
      tier: 'public',
      calibrated: false,
      servedModel: null,
      model: { provider: 'baseten_decision', model: 'decision-models/baseten-joey-2b-latest' },
      questions: {
        course: {
          instructions: 'Which MS course is described?',
          kind: { choice: { options: { rrms: 'Relapsing-remitting', spms: null, ppms: null } } },
          threshold: { option: { name: 'rrms', p: 0.6 } },
        },
      },
      decisions: [
        {
          question: 'course',
          answer: { type: 'choice', option: 'spms' },
          probabilities: { rrms: 0.2, spms: 0.5, ppms: 0.3 },
          confidence: 0.25,
          passed: false,
          passProbability: 0.2,
          source: { kind: 'verbalized', reason: null },
          calibrated: false,
          model: { provider: 'baseten_decision', model: 'decision-models/baseten-joey-2b-latest' },
          tier: 'public',
          requestId: 'req-2',
          latencyMs: 1400,
          usage: {},
          raw: {},
        },
      ],
      outcome: 'failed',
      latencyMs: 1400,
    },
    context: { text: 'x' },
  };
}

export function wireErrorDetail() {
  return {
    record: {
      ...wireBinaryDetail().record,
      id: 'dec-3',
      decisions: [],
      outcome: 'error',
      error: 'The decision model did not answer in 10 s',
    },
    context: { text: 'x' },
  };
}

/**
 * A `decisions__decide` call's `structuredContent`, as `DecideSummary` in
 * `crates/biorouter/src/agents/decisions_extension.rs` serializes it.
 */
export function wireDecideSummary() {
  return {
    kind: 'decide_batch',
    batchId: 'batch-42',
    chunks: 600,
    decided: 598,
    errors: 2,
    model: { provider: 'ollama_decision', model: 'tev1:0.8b', adapter: 'system_one' },
    tier: 'private',
    calibrated: true,
    elapsedMs: 13_200,
    questions: [
      {
        name: 'has_ms',
        type: 'yes_no',
        threshold: 0.9,
        passOn: 'yes',
        decided: 598,
        passed: 41,
        yes: 41,
        review: 6,
        maxP: 0.98,
        answerCounts: null,
        passingIds: ['note-0003', 'note-0017'],
        morePassing: 39,
        reviewIds: ['note-0100'],
        moreReview: 5,
      },
      {
        name: 'edss_stated',
        type: 'yes_no',
        threshold: 0.8,
        passOn: 'yes',
        decided: 598,
        passed: 17,
        yes: 19,
        review: 0,
        maxP: 0.91,
        answerCounts: null,
        passingIds: [],
        morePassing: 0,
        reviewIds: [],
        moreReview: 0,
      },
      {
        name: 'course',
        type: 'choice',
        threshold: 0.6,
        passOn: 'rrms',
        decided: 598,
        passed: 280,
        yes: null,
        review: 12,
        maxP: 0.71,
        answerCounts: { rrms: 300, spms: 200, none: 98 },
        passingIds: ['note-0004'],
        morePassing: 279,
        reviewIds: ['note-0200'],
        moreReview: 11,
      },
    ],
    errorGroups: [
      {
        message: 'The decision model did not answer in 10 s',
        chunkIds: ['note-0450', 'note-0451'],
        more: 0,
      },
    ],
  };
}
