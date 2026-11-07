import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeConnection } from '../onboarding/testCrew';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from './useCrewChanges';
import {
  connectionVerifiedThisSession,
  noteConnectionVerified,
  resetConnectionMemoryForTests,
  useCrewConnections,
} from './useCrewConnections';

/**
 * The connection list follows Crew's change feed (WI-15): a workspace a chat set up or joined shows
 * up, selected, without a reload or a focus change, and a connection removed anywhere has what this
 * window remembers about it forgotten.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn(), listed: [] as unknown[] }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

const lab = fakeConnection({ id: 'conn-1', name: 'lab' });
const leo = fakeConnection({ id: 'conn-2', name: 'leo' });

const listReads = () =>
  mocks.crewHttp.mock.calls.filter(([path, method]) => path === '/connections' && method === 'GET')
    .length;

const changed = (kind: string, connectionId: string | null, revision = 1) => ({
  revision,
  truncated: false,
  changes: [
    {
      revision,
      kind,
      connection_id: connectionId,
      channel_id: null,
      session_id: null,
      run_id: null,
      job_id: kind.startsWith('setup') ? 'job-1' : null,
    },
  ],
});

const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function mounted() {
  const generation = { current: 0 };
  const hook = renderHook(() => useCrewConnections(generation));
  await act(async () => {
    await hook.result.current.loadConnections();
  });
  return hook;
}

beforeEach(() => {
  mocks.crewHttp.mockReset();
  resetConnectionMemoryForTests();
  mocks.listed = [lab];
  mocks.crewHttp.mockImplementation(async (path: string) =>
    path === '/connections' ? { connections: mocks.listed } : {}
  );
});

describe('useCrewConnections and the change feed', () => {
  it('shows and selects a connection a chat saved, with one read for a burst', async () => {
    const { result } = await mounted();
    expect(result.current.connectionId).toBe('conn-1');
    const before = listReads();

    mocks.listed = [lab, leo];
    dispatchCrewChanged(changed('setup_running', null, 1));
    dispatchCrewChanged(changed('setup_finished', 'conn-2', 2));
    await waitFor(() => expect(result.current.connections).toHaveLength(2));
    await waitFor(() => expect(result.current.connectionId).toBe('conn-2'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(listReads() - before).toBe(1);
  });

  it('keeps the selection for a change to a connection it already listed', async () => {
    mocks.listed = [lab, leo];
    const { result } = await mounted();
    expect(result.current.connectionId).toBe('conn-1');
    dispatchCrewChanged(changed('connections_update', 'conn-2'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    await waitFor(() => expect(listReads()).toBeGreaterThan(1));
    expect(result.current.connectionId).toBe('conn-1');
  });

  it('forgets what it kept about a connection removed elsewhere', async () => {
    mocks.listed = [lab, leo];
    const { result } = await mounted();
    noteConnectionVerified('conn-1');
    mocks.listed = [leo];
    dispatchCrewChanged(changed('connections_remove', 'conn-1'));
    await waitFor(() => expect(result.current.connections).toEqual([leo]));
    expect(result.current.connectionId).toBe('conn-2');
    expect(connectionVerifiedThisSession('conn-1')).toBe(false);
  });

  it('keeps the list it has when the read fails', async () => {
    const { result } = await mounted();
    mocks.crewHttp.mockRejectedValue(new Error('daemon restarting'));
    dispatchCrewChanged(changed('route', null));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(result.current.connections).toEqual([lab]);
  });
});
