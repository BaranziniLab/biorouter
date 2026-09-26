import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../api/client.gen';
import { crewHttp } from './crewApi';
import { revokeSessionGrant } from './api/grants';
import { claimJoin } from './api/join';
import { resolve } from './api/names';

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'serialization-test-proof' }),
}));

describe('crewHttp request serialization', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    client.setConfig({
      baseUrl: 'http://crew-serialization.test',
      headers: { 'Content-Type': 'application/json' },
    });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { getSecretKey: vi.fn().mockResolvedValue('serialization-secret') },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('does not advertise or send JSON for an empty DELETE cleanup request', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ forgotten: true }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    );
    vi.stubGlobal('fetch', fetchMock);

    await crewHttp('/transfers/transfer-1', 'DELETE');

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(init.method).toBe('DELETE');
    expect(init.body).toBeUndefined();
    expect(new Headers(init.headers).has('Content-Type')).toBe(false);
  });

  it('keeps JSON headers and body for DELETE cleanup carrying a capability', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ forgotten: true }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    );
    vi.stubGlobal('fetch', fetchMock);

    await crewHttp('/transfers/transfer-1', 'DELETE', { file_capability: 'capability-1' });

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(new Headers(init.headers).get('Content-Type')).toBe('application/json');
    expect(init.body).toBe(JSON.stringify({ file_capability: 'capability-1' }));
  });

  it('revokes a chat grant as the person, with no body and no Content-Type', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          revoked: true,
          session_id: 'agent 1',
          run_id: 'run-1',
          remote_revocation_confirmed: true,
        }),
        { status: 200, headers: { 'content-type': 'application/json' } }
      )
    );
    vi.stubGlobal('fetch', fetchMock);

    await expect(revokeSessionGrant('conn/1', 'agent 1')).resolves.toEqual({
      revoked: true,
      remote_revocation_confirmed: true,
      session_id: 'agent 1',
      run_id: 'run-1',
    });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe(
      'http://crew-serialization.test/crew/connections/conn%2F1/sessions/agent%201/revoke'
    );
    expect(init.method).toBe('POST');
    expect(init.body).toBeUndefined();
    const headers = new Headers(init.headers);
    expect(headers.has('Content-Type')).toBe(false);
    expect(headers.get('X-User-Action')).toBe('serialization-test-proof');
    expect(headers.get('X-Secret-Key')).toBe('serialization-secret');
  });

  it('claims a join as the person, with no body and no Content-Type', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ joined: true }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    );
    vi.stubGlobal('fetch', fetchMock);

    await claimJoin('conn-1');

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://crew-serialization.test/crew/connections/conn-1/join');
    expect(init.method).toBe('POST');
    expect(init.body).toBeUndefined();
    const headers = new Headers(init.headers);
    expect(headers.has('Content-Type')).toBe(false);
    expect(headers.get('X-User-Action')).toBe('serialization-test-proof');
  });

  it('sends a name lookup as JSON with the person proof', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          results: [{ status: 'unknown_name', kind: 'person', text: '@bob' }],
        }),
        { status: 200, headers: { 'content-type': 'application/json' } }
      )
    );
    vi.stubGlobal('fetch', fetchMock);

    await resolve([{ kind: 'person', text: '@bob' }]);

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('http://crew-serialization.test/crew/resolve');
    expect(init.method).toBe('POST');
    expect(init.body).toBe(JSON.stringify({ selectors: [{ kind: 'person', text: '@bob' }] }));
    const headers = new Headers(init.headers);
    expect(headers.get('Content-Type')).toBe('application/json');
    expect(headers.get('X-User-Action')).toBe('serialization-test-proof');
  });
});
