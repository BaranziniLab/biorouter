import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import { STALE_DAEMON_MESSAGE } from '../api/errors';
import { listSessionGrants } from '../api/grants';
import { accessRows, formatExpiry, splitAccessRows } from './accessRows';
import { accessCopy } from './copy';
import { readPastAccess } from './pastAccess';
import {
  CREW_GRANTS_CHANGED_EVENT,
  announceGrantsChanged,
  forgetUnconfirmedRevocations,
  isUnconfirmedRevocation,
  revokeGrant,
  revokeOutcomeFrom,
  useCrewGrants,
} from './useCrewGrants';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

const row = (sessionId: string, overrides: Record<string, unknown> = {}) => ({
  session_id: sessionId,
  run_id: `run-${sessionId}`,
  connection_id: 'conn-1',
  channel_id: 'channel-1',
  source_channels: ['channel-1'],
  policy_epoch: 1,
  expired: false,
  ...overrides,
});

const listCalls = () =>
  mocks.crewHttp.mock.calls.filter(([path]) => String(path).endsWith('/grants'));

describe('revoke outcomes', () => {
  it('reads only a 503 crew_revocation_unconfirmed as stopped on this device', () => {
    expect(
      revokeOutcomeFrom(new CrewHttpError('Stopped here.', 503, 'crew_revocation_unconfirmed'))
    ).toEqual({ kind: 'unconfirmed', message: 'Stopped here.' });
    expect(revokeOutcomeFrom(new CrewHttpError('Busy.', 503, 'crew_other'))).toEqual({
      kind: 'not-revoked',
      message: 'Busy.',
    });
  });

  it('keeps the daemon’s words for every other failure', () => {
    for (const [status, code] of [
      [400, 'crew_profile_refused'],
      [403, 'crew_user_action_required'],
      [404, 'crew_grant_not_found'],
      [409, 'crew_grant_other_connection'],
    ] as const) {
      expect(revokeOutcomeFrom(new CrewHttpError(`said ${status}`, status, code))).toEqual({
        kind: 'not-revoked',
        message: `said ${status}`,
      });
    }
    expect(revokeOutcomeFrom(new TypeError('Failed to fetch'))).toEqual({
      kind: 'not-revoked',
      message: 'Failed to fetch',
    });
    expect(revokeOutcomeFrom('nope')).toEqual({
      kind: 'not-revoked',
      message: accessCopy.revokeFallback,
    });
  });

  it('names a daemon without the route instead of echoing a bare 404', () => {
    expect(revokeOutcomeFrom(new CrewHttpError('Crew request failed (404)', 404))).toEqual({
      kind: 'not-revoked',
      message: STALE_DAEMON_MESSAGE,
    });
  });
});

describe('revokeGrant', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });

  it('POSTs the revoke with no body, resolves revoked on a confirmed 200 and announces it', async () => {
    mocks.crewHttp.mockResolvedValue({ revoked: true, remote_revocation_confirmed: true });
    const heard = vi.fn();
    window.addEventListener(CREW_GRANTS_CHANGED_EVENT, heard);
    try {
      await expect(revokeGrant('conn-1', 'agent-1')).resolves.toEqual({ kind: 'revoked' });
    } finally {
      window.removeEventListener(CREW_GRANTS_CHANGED_EVENT, heard);
    }
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn-1/sessions/agent-1/revoke',
      'POST'
    );
    expect(heard).toHaveBeenCalledTimes(1);
    expect((heard.mock.calls[0][0] as CustomEvent).detail).toEqual({
      connectionId: 'conn-1',
      sessionId: 'agent-1',
      change: 'revoked',
    });
  });

  it('remembers an unconfirmed stop until a later revoke is confirmed', async () => {
    mocks.crewHttp.mockRejectedValueOnce(
      new CrewHttpError('Stopped here.', 503, 'crew_revocation_unconfirmed')
    );
    await expect(revokeGrant('conn-1', 'agent-1')).resolves.toMatchObject({ kind: 'unconfirmed' });
    expect(isUnconfirmedRevocation('conn-1', 'agent-1')).toBe(true);

    mocks.crewHttp.mockResolvedValueOnce({ revoked: true });
    await expect(revokeGrant('conn-1', 'agent-1')).resolves.toEqual({ kind: 'revoked' });
    expect(isUnconfirmedRevocation('conn-1', 'agent-1')).toBe(false);
  });

  it('never throws', async () => {
    mocks.crewHttp.mockRejectedValue(new Error('down'));
    await expect(revokeGrant('conn-1', 'agent-1')).resolves.toEqual({
      kind: 'not-revoked',
      message: 'down',
    });
  });
});

describe('useCrewGrants', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('reads each connection once when it opens, and never polls', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mocks.crewHttp.mockImplementation(async (path: string) =>
      path === '/connections/conn-1/grants'
        ? { grants: [row('a')] }
        : { grants: [row('b', { connection_id: 'conn-2' })] }
    );
    const { result } = renderHook(() => useCrewGrants(['conn-2', 'conn-1', 'conn-1']));
    expect(result.current.status).toBe('loading');
    await waitFor(() => expect(result.current.status).toBe('loaded'));
    expect(result.current.grants.map((grant) => grant.session_id).sort()).toEqual(['a', 'b']);
    expect(listCalls()).toHaveLength(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
    });
    expect(listCalls()).toHaveLength(2);
  });

  it('re-reads after a grant change is announced for one of its connections, and only then', async () => {
    mocks.crewHttp.mockResolvedValue({ grants: [row('a')] });
    const { result } = renderHook(() => useCrewGrants(['conn-1']));
    await waitFor(() => expect(result.current.status).toBe('loaded'));
    expect(listCalls()).toHaveLength(1);

    act(() => announceGrantsChanged({ connectionId: 'conn-9', sessionId: 'x', change: 'revoked' }));
    expect(listCalls()).toHaveLength(1);

    mocks.crewHttp.mockResolvedValue({ grants: [row('a', { expired: true })] });
    act(() => announceGrantsChanged({ connectionId: 'conn-1', sessionId: 'a', change: 'revoked' }));
    await waitFor(() => expect(result.current.grants[0]?.expired).toBe(true));
    expect(listCalls()).toHaveLength(2);
  });

  it('keeps a refetch’s rows on screen while it reads', async () => {
    mocks.crewHttp.mockResolvedValue({ grants: [row('a')] });
    const { result } = renderHook(() => useCrewGrants(['conn-1']));
    await waitFor(() => expect(result.current.status).toBe('loaded'));
    let answer: (value: unknown) => void = () => {};
    mocks.crewHttp.mockImplementation(() => new Promise((resolve) => (answer = resolve)));
    act(() => result.current.refetch());
    expect(result.current.status).toBe('loaded');
    expect(result.current.grants).toHaveLength(1);
    await act(async () => answer({ grants: [] }));
    await waitFor(() => expect(result.current.grants).toHaveLength(0));
  });

  it('reports a failed list, and names a daemon that predates the route', async () => {
    mocks.crewHttp.mockRejectedValue(new CrewHttpError('boom', 500, 'crew_internal'));
    const { result, unmount } = renderHook(() => useCrewGrants(['conn-1']));
    await waitFor(() => expect(result.current.status).toBe('failed'));
    expect(result.current.error).toBe(accessCopy.listFailed);
    expect(result.current.anyFailed).toBe(true);
    unmount();

    mocks.crewHttp.mockRejectedValue(new CrewHttpError('Crew request failed (404)', 404));
    const stale = renderHook(() => useCrewGrants(['conn-1']));
    await waitFor(() => expect(stale.result.current.status).toBe('failed'));
    expect(stale.result.current.error).toBe(STALE_DAEMON_MESSAGE);
  });

  it('keeps the answers it got when another connection fails', async () => {
    mocks.crewHttp.mockImplementation(async (path: string) => {
      if (path === '/connections/conn-2/grants') throw new Error('down');
      return { grants: [row('a')] };
    });
    const { result } = renderHook(() => useCrewGrants(['conn-1', 'conn-2']));
    await waitFor(() => expect(result.current.status).toBe('loaded'));
    expect(result.current.grants).toHaveLength(1);
    expect(result.current.anyFailed).toBe(true);
  });

  it('asks for nothing while disabled or without connections', async () => {
    const disabled = renderHook(() => useCrewGrants(['conn-1'], { enabled: false }));
    expect(disabled.result.current.status).toBe('idle');
    const none = renderHook(() => useCrewGrants([]));
    await waitFor(() => expect(none.result.current.status).toBe('loaded'));
    expect(mocks.crewHttp).not.toHaveBeenCalled();
  });

  it('starts from the last answer its scope saw, then reads the daemon anyway', async () => {
    const scope = {};
    mocks.crewHttp.mockResolvedValue({ grants: [row('a')] });
    const first = renderHook(() => useCrewGrants(['conn-1'], { cacheScope: scope }));
    await waitFor(() => expect(first.result.current.status).toBe('loaded'));

    const second = renderHook(() => useCrewGrants(['conn-1'], { cacheScope: scope }));
    expect(second.result.current.status).toBe('loaded');
    expect(second.result.current.grants.map((grant) => grant.session_id)).toEqual(['a']);
    await waitFor(() => expect(listCalls()).toHaveLength(2));

    const unscoped = renderHook(() => useCrewGrants(['conn-1'], { cacheScope: {} }));
    expect(unscoped.result.current.status).toBe('loading');
  });
});

/**
 * Final polish NEW-3: "Show past access" dated only a revoke the daemon confirmed at once (a 200).
 * One answered 503 and confirmed later by the daemon itself (F3's path) got no time — its row read
 * plain "Revoked" — and once the chat was granted again the daemon's list replaced it, so the row
 * was gone (live, runs f5c6dc10 and 2425afd9). Now the first list read that says `confirmed` for a
 * run this window saw waiting records it, dated then, and the row outlives a new grant.
 */
describe('a revoke the daemon confirms later, in Show past access (NEW-3)', () => {
  const T0 = Date.UTC(2026, 8, 25, 21, 13, 10);
  const CONFIRMED_AT = T0 + 12_800;
  const snapshot = {
    teams: [{ id: 'team-1', name: 'Polish Lab' }],
    channels: [{ id: 'channel-1', team_id: 'team-1', name: 'general' }],
  };
  const chat = (runId: string, overrides: Record<string, unknown> = {}) =>
    row('chat-1', {
      run_id: runId,
      kind: 'chat',
      session_name: 'Crew context check',
      expires_at: Math.floor(T0 / 1000) + 3600,
      ...overrides,
    });
  let answer: { grants: unknown[]; replaced_grants?: unknown[] };

  beforeEach(() => {
    vi.clearAllMocks();
    forgetUnconfirmedRevocations();
    window.localStorage.clear();
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(T0);
    answer = { grants: [chat('run-1')] };
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path === '/connections/conn-1/sessions/chat-1/revoke' && method === 'POST') {
        answer = { grants: [chat('run-1', { expired: true, revocation: 'unconfirmed' })] };
        throw new CrewHttpError(
          'Stopped on this device. The workspace hasn’t confirmed yet.',
          503,
          'crew_revocation_unconfirmed'
        );
      }
      if (path === '/connections/conn-1/grants') return answer;
      return {};
    });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const pastRows = (grants: Awaited<ReturnType<typeof listSessionGrants>>) =>
    splitAccessRows(
      accessRows(grants, {
        snapshot,
        now: Date.now(),
        pastAccess: { connectionId: 'conn-1', entries: readPastAccess('conn-1') },
      })
    ).old;

  it('dates the revoke when a list first says confirmed, and keeps the row after a new grant', async () => {
    const listed = await listSessionGrants('conn-1');
    await expect(revokeGrant('conn-1', 'chat-1', listed[0])).resolves.toMatchObject({
      kind: 'unconfirmed',
    });
    // While it waits, the list says so; nothing is recorded yet.
    await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')).toEqual([]);

    // The daemon re-dials and the workspace confirms; the next read (the pane's watch) sees it.
    answer = { grants: [chat('run-1', { expired: true, revocation: 'confirmed' })] };
    vi.setSystemTime(CONFIRMED_AT);
    const confirmed = await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')).toMatchObject([
      { session_id: 'chat-1', run_id: 'run-1', channel_id: 'channel-1', revoked_at: CONFIRMED_AT },
    ]);
    const time = formatExpiry(CONFIRMED_AT / 1000, CONFIRMED_AT);
    expect(pastRows(confirmed).map((item) => [item.runId, item.statusLabel])).toEqual([
      ['run-1', accessCopy.status.revokedAt(time)],
    ]);

    // Later reads do not move the time.
    vi.setSystemTime(CONFIRMED_AT + 60_000);
    await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')[0].revoked_at).toBe(CONFIRMED_AT);

    // The chat is granted again: the daemon lists only the new grant, and the old row stays.
    answer = { grants: [chat('run-2')] };
    const regranted = await listSessionGrants('conn-1');
    expect(pastRows(regranted).map((item) => [item.runId, item.statusLabel])).toEqual([
      ['run-1', accessCopy.status.revokedAt(time)],
    ]);
  });

  it('dates one the daemon confirms after the chat was granted again (`replaced_grants`)', async () => {
    const listed = await listSessionGrants('conn-1');
    await revokeGrant('conn-1', 'chat-1', listed[0]);
    // Granted again before the workspace answered: the earlier stop is kept as replaced.
    answer = {
      grants: [chat('run-2')],
      replaced_grants: [chat('run-1', { expired: true, revocation: 'unconfirmed' })],
    };
    await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')).toEqual([]);

    answer = {
      grants: [chat('run-2')],
      replaced_grants: [chat('run-1', { expired: true, revocation: 'confirmed' })],
    };
    vi.setSystemTime(CONFIRMED_AT);
    const grants = await listSessionGrants('conn-1');
    // What the list resolves with is still only the chats' grants.
    expect(grants.map((grant) => grant.run_id)).toEqual(['run-2']);
    expect(readPastAccess('conn-1')).toMatchObject([{ run_id: 'run-1', revoked_at: CONFIRMED_AT }]);
    expect(pastRows(grants).map((item) => item.runId)).toEqual(['run-1']);
  });

  it('dates a revoke made elsewhere that this window saw waiting', async () => {
    // A terminal revoked it while offline: this window only ever read the list.
    answer = { grants: [chat('run-1', { expired: true, revocation: 'unconfirmed' })] };
    await listSessionGrants('conn-1');
    answer = { grants: [chat('run-1', { expired: true, revocation: 'confirmed' })] };
    vi.setSystemTime(CONFIRMED_AT);
    await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')).toMatchObject([{ run_id: 'run-1', revoked_at: CONFIRMED_AT }]);
  });

  it('never dates a revoke it did not see waiting, or one the workspace ended itself', async () => {
    // Confirmed before this window ever looked: when is unknown, so no time is invented.
    answer = { grants: [chat('run-1', { expired: true, revocation: 'confirmed' })] };
    await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')).toEqual([]);

    // Seen waiting, then ended by the workspace (D-1): nobody's revoke was confirmed.
    answer = { grants: [chat('run-2', { expired: true, revocation: 'unconfirmed' })] };
    await listSessionGrants('conn-1');
    answer = { grants: [chat('run-2', { expired: true, revocation: 'ended_by_workspace' })] };
    await listSessionGrants('conn-1');
    answer = { grants: [chat('run-2', { expired: true, revocation: 'confirmed' })] };
    await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')).toEqual([]);
  });

  it('keeps a 200’s own time: a later list that says confirmed does not move it', async () => {
    mocks.crewHttp.mockImplementation(async (path: string, method = 'GET') => {
      if (path.endsWith('/revoke') && method === 'POST')
        return { revoked: true, remote_revocation_confirmed: true, run_id: 'run-1' };
      if (path === '/connections/conn-1/grants') return answer;
      return {};
    });
    answer = { grants: [chat('run-1', { expired: true, revocation: 'unconfirmed' })] };
    const listed = await listSessionGrants('conn-1');
    // Retry answered 200 at T0: recorded then.
    await expect(revokeGrant('conn-1', 'chat-1', listed[0])).resolves.toEqual({
      kind: 'revoked',
    });
    expect(readPastAccess('conn-1')).toMatchObject([{ run_id: 'run-1', revoked_at: T0 }]);
    answer = { grants: [chat('run-1', { expired: true, revocation: 'confirmed' })] };
    vi.setSystemTime(CONFIRMED_AT);
    await listSessionGrants('conn-1');
    expect(readPastAccess('conn-1')).toMatchObject([{ run_id: 'run-1', revoked_at: T0 }]);
  });
});
