import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from '../state/useCrewChanges';
import { forgetUnconfirmedRevocations, useCrewGrants } from './useCrewGrants';

/**
 * A grant list follows Crew's change feed (WI-15): a chat's `grants_revoke` (or a revoke made in
 * another window or the terminal) takes the row away without a click here.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

const row = (sessionId: string) => ({
  session_id: sessionId,
  run_id: `run-${sessionId}`,
  connection_id: 'conn-1',
  channel_id: 'channel-1',
  source_channels: ['channel-1'],
  policy_epoch: 1,
  expired: false,
});

const listCalls = () =>
  mocks.crewHttp.mock.calls.filter(([path]) => String(path).endsWith('/grants'));

const revoked = (connectionId: string, revision = 1) => ({
  revision,
  truncated: false,
  changes: [
    {
      revision,
      kind: 'grants_revoke',
      connection_id: connectionId,
      channel_id: null,
      session_id: 'chat-a',
      run_id: null,
      job_id: null,
    },
  ],
});

const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe('useCrewGrants and the change feed', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('drops a grant a chat revoked, with one read for a burst of changes', async () => {
    let listed = [row('chat-a')];
    mocks.crewHttp.mockImplementation(async () => ({ grants: listed }));
    const { result } = renderHook(() => useCrewGrants(['conn-1']));
    await waitFor(() => expect(result.current.grants).toHaveLength(1));
    expect(listCalls()).toHaveLength(1);

    listed = [];
    dispatchCrewChanged(revoked('conn-1', 1));
    dispatchCrewChanged(revoked('conn-1', 2));
    await waitFor(() => expect(result.current.grants).toHaveLength(0));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(listCalls()).toHaveLength(2);
  });

  it('reads nothing for a change on another connection', async () => {
    mocks.crewHttp.mockResolvedValue({ grants: [row('chat-a')] });
    const { result } = renderHook(() => useCrewGrants(['conn-1']));
    await waitFor(() => expect(result.current.status).toBe('loaded'));
    dispatchCrewChanged(revoked('conn-9'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(listCalls()).toHaveLength(1);
  });

  it('reads nothing while it is turned off', async () => {
    mocks.crewHttp.mockResolvedValue({ grants: [] });
    renderHook(() => useCrewGrants(['conn-1'], { enabled: false }));
    dispatchCrewChanged(revoked('conn-1'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(listCalls()).toHaveLength(0);
  });
});
