/**
 * Wire-shaped fixtures for `GET /sessions/{id}/subagents`: camelCase fields and
 * `null` for Rust's `None`, as the daemon serializes `SessionSubagents`. Shared
 * by the parser, hook and component tests, so their rows cannot drift apart.
 */

/** One `SubagentRow` as the daemon sends it. */
export function wireRow(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    sessionId: 'child-1',
    name: 'Search PubMed for trials',
    role: null,
    phaseId: null,
    status: 'running',
    loopPhase: 'tool_batch',
    quietSeconds: 4,
    createdAt: '2026-11-04T10:00:00Z',
    updatedAt: '2026-11-04T10:01:00Z',
    endedAt: null,
    elapsedSeconds: 75,
    providerName: 'anthropic',
    modelName: 'claude-sonnet-4-5-20250929',
    extensions: ['developer', 'webdocuments'],
    skills: ['scientific-research'],
    knowledgeBases: ['soul'],
    privacyTier: 'public',
    totalTokens: 12345,
    messageCount: 7,
    toolCallCount: 3,
    resultLine: null,
    ...overrides,
  };
}

/** One `PhaseSnapshot` as the daemon sends it. */
export function wirePhase(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: 'research',
    title: 'Research',
    goal: null,
    kind: 'research',
    status: 'pending',
    dependsOn: [],
    agentIds: [],
    counts: { total: 0, done: 0, running: 0, blocked: 0, failed: 0, quiet: 0 },
    handoff: null,
    startedAt: null,
    finishedAt: null,
    // The gated phase this one waits for; `null` when none holds it. A daemon
    // before held phases sends no key at all (`olderPhase`).
    heldBy: null,
    ...overrides,
  };
}

/** One `PhaseSnapshot` as a daemon before held phases and the person's gate controls sent it. */
export function olderPhase(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  const { heldBy: _heldBy, ...phase } = wirePhase(overrides);
  return phase;
}

/** A whole `SessionSubagents` body. */
export function wireSubagents(
  rows: Record<string, unknown>[],
  plan: Record<string, unknown> | null = null,
  parentSessionId = 'parent'
): Record<string, unknown> {
  return { parentSessionId, subagents: rows, plan };
}
