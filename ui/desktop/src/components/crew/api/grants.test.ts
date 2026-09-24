import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CrewHttpError } from '../crewApi';
import {
  CREW_GRANT_OTHER_CONNECTION,
  CREW_REVOCATION_UNCONFIRMED,
  CREW_UNEXPECTED_RESPONSE,
  isRevocationUnconfirmed,
} from './errors';
import {
  findSessionGrant,
  grantDestinationLabel,
  listSessionGrants,
  revokeSessionGrant,
  sessionGrantState,
  type CrewSessionGrant,
} from './grants';

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn() }));

// The same shape as CrewView.regression.test.tsx's mock: the actual module with crewHttp replaced.
// Every assertion below that sees a call proves the helpers go through the module's export, which
// is what lets that suite's mock intercept them.
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp };
});

function row(overrides: Record<string, unknown> = {}) {
  return {
    session_id: 'agent-1',
    run_id: 'run-1',
    connection_id: 'conn-1',
    channel_id: 'channel-1',
    source_channels: ['channel-2'],
    policy_epoch: 3,
    expired: false,
    ...overrides,
  };
}

describe('listSessionGrants', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('asks the daemon for one connection with a GET and no body', async () => {
    mocks.crewHttp.mockResolvedValue({ grants: [row({ connection_id: 'conn 1' })] });
    const signal = new AbortController().signal;

    const grants = await listSessionGrants('conn 1', signal);

    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn%201/grants',
      'GET',
      undefined,
      signal
    );
    expect(grants).toEqual([
      {
        session_id: 'agent-1',
        run_id: 'run-1',
        connection_id: 'conn 1',
        channel_id: 'channel-1',
        source_channels: ['channel-2'],
        policy_epoch: 3,
        expired: false,
      },
    ]);
  });

  it.each([
    ['an empty object', {}],
    ['null', null],
    ['a list that is not an array', { grants: { 'agent-1': row() } }],
    ['a bare array', [row()]],
  ])('reads %s as no grants', async (_label, answer) => {
    mocks.crewHttp.mockResolvedValue(answer);
    await expect(listSessionGrants('conn-1')).resolves.toEqual([]);
  });

  it('drops rows it cannot act on and rows for another connection', async () => {
    mocks.crewHttp.mockResolvedValue({
      grants: [
        row({ session_id: 'kept' }),
        row({ session_id: '' }),
        row({ session_id: undefined }),
        row({ run_id: 7 }),
        row({ channel_id: null }),
        row({ expired: 'no' }),
        row({ policy_epoch: 1.5 }),
        row({ source_channels: ['ok', 3] }),
        row({ session_id: 'elsewhere', connection_id: 'conn-2' }),
        'not a row',
        null,
      ],
    });

    const grants = await listSessionGrants('conn-1');

    expect(grants.map((grant) => grant.session_id)).toEqual(['kept']);
  });

  it('reads the RV-D2 fields and leaves malformed optional fields out', async () => {
    mocks.crewHttp.mockResolvedValue({
      grants: [
        row({ session_id: 'chat', kind: 'chat', session_name: 'Plot review', expires_at: 1700 }),
        row({ session_id: 'task', kind: 'task', session_name: null, expires_at: null }),
        row({ session_id: 'odd', kind: 'agent', session_name: 42, expires_at: 'soon' }),
        row({ session_id: 'old', source_channels: undefined }),
      ],
    });

    const [chat, task, odd, old] = await listSessionGrants('conn-1');

    expect(chat).toMatchObject({ kind: 'chat', session_name: 'Plot review', expires_at: 1700 });
    expect(task).toMatchObject({ kind: 'task', session_name: null, expires_at: null });
    expect(odd).not.toHaveProperty('kind');
    expect(odd).not.toHaveProperty('session_name');
    expect(odd).not.toHaveProperty('expires_at');
    expect(old.source_channels).toEqual([]);
  });

  it('reads the names recorded at grant time, dropping any field that is not text', async () => {
    mocks.crewHttp.mockResolvedValue({
      grants: [
        row({
          session_id: 'named',
          labels: {
            you: 'Alice Chen (@alice)',
            workspace: 'lab',
            destination: { channel_id: 'channel-1', label: '#methods', team: 'Analysis Lab' },
            sources: [
              { channel_id: 'channel-1', label: '#methods', team: 'Analysis Lab' },
              { channel_id: 'channel-2', label: 7, team: null },
              'not a label',
              { label: '   ' },
            ],
          },
        }),
        row({
          session_id: 'odd',
          labels: { workspace: 3, destination: { label: ['#x'] }, sources: 'all' },
        }),
        row({ session_id: 'none', labels: null }),
      ],
    });

    const [named, odd, none] = await listSessionGrants('conn-1');

    expect(named.labels).toEqual({
      workspace: 'lab',
      destination: { channel_id: 'channel-1', label: '#methods', team: 'Analysis Lab' },
      sources: [
        { channel_id: 'channel-1', label: '#methods', team: 'Analysis Lab' },
        { channel_id: 'channel-2' },
      ],
    });
    expect(odd).not.toHaveProperty('labels');
    expect(none).not.toHaveProperty('labels');
  });
});

describe('grantDestinationLabel', () => {
  const base = { channel_id: 'channel-1' };

  it('names the channel the grant posts in', () => {
    expect(
      grantDestinationLabel({
        ...base,
        labels: { destination: { channel_id: 'channel-1', label: '#methods' } },
      })
    ).toBe('#methods');
    expect(grantDestinationLabel({ ...base, labels: { destination: { label: '#methods' } } })).toBe(
      '#methods'
    );
  });

  it('never borrows the name of a different channel, and is null without one', () => {
    expect(
      grantDestinationLabel({
        ...base,
        labels: { destination: { channel_id: 'channel-9', label: '#elsewhere' } },
      })
    ).toBeNull();
    expect(grantDestinationLabel({ ...base, labels: { workspace: 'lab' } })).toBeNull();
    expect(grantDestinationLabel(base)).toBeNull();
  });
});

describe('findSessionGrant', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  function grantsBy(byConnection: Record<string, unknown>) {
    mocks.crewHttp.mockImplementation(async (path: string) => {
      const id = decodeURIComponent(path.split('/')[2]);
      const answer = byConnection[id];
      if (answer instanceof Error) throw answer;
      return answer;
    });
  }

  it('asks every saved connection once and returns the first match in connection order', async () => {
    grantsBy({
      'conn-1': { grants: [row({ session_id: 'other' })] },
      'conn-2': { grants: [row({ connection_id: 'conn-2', channel_id: 'from-2' })] },
      'conn-3': { grants: [row({ connection_id: 'conn-3', channel_id: 'from-3' })] },
    });

    const grant = await findSessionGrant(['conn-1', 'conn-2', 'conn-3', 'conn-2'], 'agent-1');

    expect(grant?.connection_id).toBe('conn-2');
    expect(grant?.channel_id).toBe('from-2');
    expect(mocks.crewHttp.mock.calls.map(([path]) => path)).toEqual([
      '/connections/conn-1/grants',
      '/connections/conn-2/grants',
      '/connections/conn-3/grants',
    ]);
  });

  it('returns null when every connection answered and none holds the session', async () => {
    grantsBy({ 'conn-1': { grants: [] }, 'conn-2': {} });
    await expect(findSessionGrant(['conn-1', 'conn-2'], 'agent-1')).resolves.toBeNull();
  });

  it('returns null without a request when there are no saved connections', async () => {
    await expect(findSessionGrant([], 'agent-1')).resolves.toBeNull();
    expect(mocks.crewHttp).not.toHaveBeenCalled();
  });

  it('finds the grant even when another connection failed to answer', async () => {
    grantsBy({
      'conn-1': new CrewHttpError('Crew request failed (500)', 500),
      'conn-2': { grants: [row({ connection_id: 'conn-2' })] },
    });
    await expect(findSessionGrant(['conn-1', 'conn-2'], 'agent-1')).resolves.toMatchObject({
      connection_id: 'conn-2',
    });
  });

  it('does not report "no grant" when a connection that might hold it failed', async () => {
    const failure = new CrewHttpError('Crew request failed (500)', 500);
    grantsBy({ 'conn-1': failure, 'conn-2': { grants: [] } });
    await expect(findSessionGrant(['conn-1', 'conn-2'], 'agent-1')).rejects.toBe(failure);
  });
});

describe('revokeSessionGrant', () => {
  beforeEach(() => {
    mocks.crewHttp.mockReset();
  });

  it('posts with no body and resolves on a confirmed revoke', async () => {
    mocks.crewHttp.mockResolvedValue({
      revoked: true,
      session_id: 'agent-1',
      run_id: 'run-1',
      remote_revocation_confirmed: true,
      run: { id: 'run-1', revoked: true },
    });

    await expect(revokeSessionGrant('conn-1', 'agent-1')).resolves.toEqual({
      revoked: true,
      remote_revocation_confirmed: true,
      session_id: 'agent-1',
      run_id: 'run-1',
    });
    expect(mocks.crewHttp).toHaveBeenCalledWith(
      '/connections/conn-1/sessions/agent-1/revoke',
      'POST'
    );
    expect(mocks.crewHttp.mock.calls[0]).toHaveLength(2);
  });

  it('treats the run a daemon before RV-D1 returned as a confirmed revoke', async () => {
    // That daemon revoked on the workspace first and answered 200 only once it had.
    mocks.crewHttp.mockResolvedValue({ id: 'run-1', owner_id: 'person-1', revoked: true });
    await expect(revokeSessionGrant('conn-1', 'agent-1')).resolves.toEqual({
      revoked: true,
      remote_revocation_confirmed: true,
    });
  });

  it('never resolves a revoke the workspace did not confirm', async () => {
    mocks.crewHttp.mockResolvedValue({ revoked: true, remote_revocation_confirmed: false });

    const failure = await revokeSessionGrant('conn-1', 'agent-1').catch((error) => error);

    expect(failure).toBeInstanceOf(CrewHttpError);
    expect(failure.code).toBe(CREW_REVOCATION_UNCONFIRMED);
    expect(isRevocationUnconfirmed(failure)).toBe(true);
  });

  it.each([
    ['an empty object', {}],
    ['null', null],
    ['a false revoke', { revoked: false, remote_revocation_confirmed: true }],
  ])('refuses to report %s as revoked', async (_label, answer) => {
    mocks.crewHttp.mockResolvedValue(answer);

    const failure = await revokeSessionGrant('conn-1', 'agent-1').catch((error) => error);

    expect(failure).toBeInstanceOf(CrewHttpError);
    expect(failure.code).toBe(CREW_UNEXPECTED_RESPONSE);
    expect(isRevocationUnconfirmed(failure)).toBe(false);
  });

  it('passes the daemon refusal through unchanged', async () => {
    const unconfirmed = new CrewHttpError(
      'Stopped on this device.',
      503,
      CREW_REVOCATION_UNCONFIRMED
    );
    mocks.crewHttp.mockRejectedValueOnce(unconfirmed);
    await expect(revokeSessionGrant('conn-1', 'agent-1')).rejects.toBe(unconfirmed);

    const elsewhere = new CrewHttpError('Other connection', 409, CREW_GRANT_OTHER_CONNECTION);
    mocks.crewHttp.mockRejectedValueOnce(elsewhere);
    await expect(revokeSessionGrant('conn-1', 'agent-1')).rejects.toBe(elsewhere);
  });
});

describe('sessionGrantState', () => {
  const base: CrewSessionGrant = {
    session_id: 'agent-1',
    run_id: 'run-1',
    connection_id: 'conn-1',
    channel_id: 'channel-1',
    source_channels: [],
    policy_epoch: 1,
    expired: false,
  };
  const now = 1_700_000_000_000;

  it('reads a grant stopped on this device as revoked, even before it would expire', () => {
    expect(sessionGrantState({ ...base, expired: true, expires_at: now / 1000 + 60 }, now)).toBe(
      'revoked'
    );
  });

  it('reads a grant past its expiry as expired', () => {
    expect(sessionGrantState({ ...base, expires_at: now / 1000 }, now)).toBe('expired');
    expect(sessionGrantState({ ...base, expires_at: now / 1000 - 1 }, now)).toBe('expired');
  });

  it('reads a live grant, or one with no known expiry, as active', () => {
    expect(sessionGrantState({ ...base, expires_at: now / 1000 + 1 }, now)).toBe('active');
    expect(sessionGrantState({ ...base, expires_at: null }, now)).toBe('active');
    expect(sessionGrantState(base, now)).toBe('active');
  });
});
