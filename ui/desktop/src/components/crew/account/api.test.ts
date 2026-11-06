import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CREW_DAEMON_OUTDATED, CREW_UNEXPECTED_RESPONSE } from '../api/errors';
import {
  acknowledgeNewDevices,
  listPersonReads,
  rejoinConnection,
  revokeDevice,
  revokePersonRead,
} from './api';

/** The account-ownership routes, on a mocked transport: paths, methods and what is let through. */

const mocks = vi.hoisted(() => ({ crewHttp: vi.fn(), crewRequest: vi.fn() }));
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: mocks.crewHttp, crewRequest: mocks.crewRequest };
});

beforeEach(() => {
  mocks.crewHttp.mockReset();
  mocks.crewRequest.mockReset().mockResolvedValue({});
});

describe('account ownership routes', () => {
  it('adds this computer again, reading the role and a device the cap removed', async () => {
    mocks.crewHttp.mockResolvedValue({
      role: 'member',
      status: 'joined',
      evicted_device: { fingerprint: 'AAAA BBBB', added_at: 1 },
    });
    await expect(rejoinConnection('conn 1')).resolves.toEqual({
      role: 'member',
      evictedFingerprint: 'AAAA BBBB',
    });
    expect(mocks.crewHttp).toHaveBeenCalledWith('/connections/conn%201/rejoin', 'POST');
  });

  it('never reads a body that is not a rejoin answer as one', async () => {
    mocks.crewHttp.mockResolvedValue(null);
    await expect(rejoinConnection('conn-1')).rejects.toMatchObject({ code: CREW_DAEMON_OUTDATED });
    mocks.crewHttp.mockResolvedValue({ status: 'joined' });
    await expect(rejoinConnection('conn-1')).rejects.toMatchObject({
      code: CREW_UNEXPECTED_RESPONSE,
    });
  });

  it('acknowledges the new devices it was shown, and revokes one as a signed mutation', async () => {
    mocks.crewHttp.mockResolvedValue(null);
    await acknowledgeNewDevices('conn-1', ['AAAA BBBB', 'CCCC DDDD']);
    // The route requires the body: only the devices the person saw stop being reported.
    expect(mocks.crewHttp).toHaveBeenCalledWith('/connections/conn-1/devices/acknowledge', 'POST', {
      fingerprints: ['AAAA BBBB', 'CCCC DDDD'],
    });
    await revokeDevice('conn-1', 'AAAA BBBB');
    expect(mocks.crewRequest).toHaveBeenCalledWith(
      'conn-1',
      'device.revoke',
      { fingerprint: 'AAAA BBBB' },
      true
    );
  });

  it('lists a connection’s read consents and revokes one', async () => {
    mocks.crewHttp.mockResolvedValue({
      grants: [],
      replaced_grants: [],
      person_reads: [
        { session_id: 'chat-1', connection_id: 'conn-1', granted_at: 5, session_name: 'Notes' },
        { session_id: 'chat-2', connection_id: 'conn-1', granted_at: 'soon' },
        { session_id: '', connection_id: 'conn-1', granted_at: 5 },
        { session_id: 'chat-3', connection_id: 'conn-2', granted_at: 5 },
      ],
    });
    await expect(listPersonReads('conn-1')).resolves.toEqual([
      { sessionId: 'chat-1', connectionId: 'conn-1', grantedAt: 5, sessionName: 'Notes' },
      { sessionId: 'chat-2', connectionId: 'conn-1', grantedAt: null, sessionName: null },
    ]);
    mocks.crewHttp.mockResolvedValue(null);
    await revokePersonRead('chat/1', 'conn-1');
    expect(mocks.crewHttp).toHaveBeenLastCalledWith('/sessions/chat%2F1/reads/conn-1', 'DELETE');
  });
});
