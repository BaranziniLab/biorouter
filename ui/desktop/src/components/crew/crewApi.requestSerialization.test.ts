import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../api/client.gen';
import { crewHttp } from './crewApi';

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
});
