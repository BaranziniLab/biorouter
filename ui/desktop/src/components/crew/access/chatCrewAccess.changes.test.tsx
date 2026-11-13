import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CREW_CHANGED_DEBOUNCE_MS, dispatchCrewChanged } from '../state/useCrewChanges';
import { forgetChannelLabels, useChatCrewAccess } from './chatCrewAccess';
import { connection, grantRow } from './testing';
import { forgetUnconfirmedRevocations } from './useCrewGrants';

/**
 * An ordinary chat's Crew access follows Crew's change feed (WI-15): a grant revoked from another
 * chat, the Crew view in another window or the terminal stops showing in this chat without a click
 * or a focus change here.
 */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

let daemon: { connections: Record<string, unknown>[]; grants: Record<string, unknown>[] };

const grantReads = () =>
  mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections/conn-1/grants').length;

const changed = (kind: string, sessionId: string | null, revision = 1) => ({
  revision,
  truncated: false,
  changes: [
    {
      revision,
      kind,
      connection_id: 'conn-1',
      channel_id: null,
      session_id: sessionId,
      run_id: null,
      job_id: null,
    },
  ],
});

const quietFor = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
  vi.clearAllMocks();
  forgetUnconfirmedRevocations();
  forgetChannelLabels();
  daemon = { connections: [connection], grants: [grantRow({ session_id: 'chat-1' })] };
  mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
    if (path === '/connections') return { connections: daemon.connections };
    if (path === '/connections/conn-1/grants' && method === 'GET') return { grants: daemon.grants };
    return {};
  });
});

describe('useChatCrewAccess and the change feed', () => {
  it("drops this chat's grant once it was revoked elsewhere, with one read for a burst", async () => {
    const { result } = renderHook(() => useChatCrewAccess('chat-1'));
    await waitFor(() => expect(result.current.state).toBe('active'));
    const before = grantReads();

    daemon.grants = [];
    dispatchCrewChanged(changed('grants_revoke', 'chat-1', 1));
    dispatchCrewChanged(changed('grants_revoke', 'chat-1', 2));
    await waitFor(() => expect(result.current.state).toBe('none'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(grantReads() - before).toBe(1);
  });

  it('reads again for a change that names no chat, such as a disconnect', async () => {
    const { result } = renderHook(() => useChatCrewAccess('chat-1'));
    await waitFor(() => expect(result.current.state).toBe('active'));
    daemon.connections = [{ ...connection, status: 'disconnected' }];
    dispatchCrewChanged(changed('disconnect', null));
    await waitFor(() => expect(result.current.state).toBe('offline'));
  });

  it('follows the feed before this computer has a connection, reading only the list', async () => {
    daemon = { connections: [], grants: [] };
    const { result } = renderHook(() => useChatCrewAccess('chat-1'));
    await waitFor(() => expect(result.current.state).toBe('none'));
    const listReads = () =>
      mocks.crewHttp.mock.calls.filter(([path]) => path === '/connections').length;
    const before = listReads();

    // A change while there is still no connection reads the list again, and nothing else.
    dispatchCrewChanged(changed('setup_running', null, 1));
    await waitFor(() => expect(listReads()).toBe(before + 1));
    expect(grantReads()).toBe(0);

    // This chat's own setup saves the first connection, and the person grants it access.
    daemon = { connections: [connection], grants: [grantRow({ session_id: 'chat-1' })] };
    dispatchCrewChanged(changed('setup_finished', null, 2));
    await waitFor(() => expect(result.current.state).toBe('active'));
  });

  it('reads nothing for a change that names only another chat', async () => {
    const { result } = renderHook(() => useChatCrewAccess('chat-1'));
    await waitFor(() => expect(result.current.state).toBe('active'));
    const before = grantReads();
    dispatchCrewChanged(changed('grants_revoke', 'chat-2'));
    await quietFor(CREW_CHANGED_DEBOUNCE_MS * 2);
    expect(grantReads()).toBe(before);
  });
});
