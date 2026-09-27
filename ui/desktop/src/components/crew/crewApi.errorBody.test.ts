import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../api/client.gen';
import { CrewHttpError, crewHttp } from './crewApi';
import {
  CREW_CONNECTION_EXISTS,
  CREW_INVITATION_CONFLICT,
  CREW_NOT_CONNECTED,
  previewInvitation,
  refusalConnectionId,
  saveFromInvitation,
} from './api/join';

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'error-body-test-proof' }),
}));

/** Answer every request with `body` at `status`, as the daemon's refusal would arrive. */
function answer(status: number, body: unknown) {
  const fetchMock = vi.fn().mockImplementation(
    async () =>
      new Response(JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
      })
  );
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

async function refusalOf(request: Promise<unknown>): Promise<CrewHttpError> {
  const failure = await request.then(
    () => {
      throw new Error('expected a refusal');
    },
    (error: unknown) => error
  );
  expect(failure).toBeInstanceOf(CrewHttpError);
  return failure as CrewHttpError;
}

// The daemon's 409 for a join (routes/crew_authentication.rs `invitation_refusal`): `code` and
// `error`, plus a top-level `connection_id` naming the saved connection it concerns.
describe('crewHttp refusal body', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    client.setConfig({
      baseUrl: 'http://crew-error-body.test',
      headers: { 'Content-Type': 'application/json' },
    });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { getSecretKey: vi.fn().mockResolvedValue('error-body-secret') },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('carries the connection a conflicting invitation concerns, so Join can offer to open it', async () => {
    answer(409, {
      code: CREW_INVITATION_CONFLICT,
      error: 'This computer pins a different key for this workspace.',
      connection_id: 'c-1',
    });

    const error = await refusalOf(previewInvitation('brcrew1:x'));
    expect(error.status).toBe(409);
    expect(error.code).toBe(CREW_INVITATION_CONFLICT);
    expect(error.message).toBe('This computer pins a different key for this workspace.');
    expect(error.connectionId).toBe('c-1');
    expect(refusalConnectionId(error)).toBe('c-1');
    // The refusal's own id wins over the preview's.
    expect(refusalConnectionId(error, 'c-preview')).toBe('c-1');
  });

  it('carries the connection a save finds already saved', async () => {
    const id = '0b6f4c1e-7a2d-4e59-9c3b-2f1d8e6a5b40';
    answer(409, {
      code: CREW_CONNECTION_EXISTS,
      error: 'This workspace is already saved with other settings.',
      connection_id: id,
    });

    const error = await refusalOf(saveFromInvitation('brcrew1:x'));
    expect(error.connectionId).toBe(id);
    expect(refusalConnectionId(error, null)).toBe(id);
  });

  it('keeps no id that is not one', async () => {
    for (const malformed of [
      '../x\n',
      'c-1\n',
      '..',
      'conn 1',
      'conn/1',
      '',
      'a'.repeat(200),
      42,
      null,
      { id: 'c-1' },
    ]) {
      answer(409, {
        code: CREW_CONNECTION_EXISTS,
        error: 'Already saved.',
        connection_id: malformed,
      });
      const error = await refusalOf(crewHttp('/connections/from-invitation', 'POST', {}));
      expect(error.connectionId, JSON.stringify(malformed)).toBeUndefined();
      expect(refusalConnectionId(error)).toBeNull();
      // With no id of its own, the preview's is offered instead.
      expect(refusalConnectionId(error, 'c-preview')).toBe('c-preview');
    }
  });

  it('names no connection for a refusal that concerns none', async () => {
    answer(409, { code: CREW_NOT_CONNECTED, error: 'Connect first.', connection_id: 'c-1' });

    const error = await refusalOf(crewHttp('/connections/c-1/join', 'GET'));
    expect(error.code).toBe(CREW_NOT_CONNECTED);
    expect(refusalConnectionId(error)).toBeNull();
    expect(refusalConnectionId(error, 'c-preview')).toBeNull();
  });
});
