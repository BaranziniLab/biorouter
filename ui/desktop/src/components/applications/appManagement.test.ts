import { afterEach, describe, expect, it, vi } from 'vitest';
import { appLaunchUrl } from './appManagement';

const base = 'http://127.0.0.1:64005';
const token = 'ef'.repeat(32);

function daemonAnswering(body: unknown, status = 200) {
  const fetchMock = vi.fn(
    async () =>
      ({
        ok: status >= 200 && status < 300,
        status,
        json: async () => body,
        text: async () => (typeof body === 'string' ? body : JSON.stringify(body)),
      }) as Response
  );
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

/**
 * W2-HRD-1: an app's page answers 401 in a browser that has not opened its
 * one-time launch link, so Applications asks the daemon for that link, with the
 * secret, and opens it.
 */
describe('appLaunchUrl', () => {
  it('asks the daemon with the secret and returns its one-time link', async () => {
    window.electron = { getSecretKey: vi.fn().mockResolvedValue('daemon-secret') } as never;
    const daemon = daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}` });
    await expect(appLaunchUrl('cohort-explorer', base)).resolves.toBe(
      `${base}/apps/cohort-explorer/?t=${token}`
    );
    const [url, init] = daemon.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe(`${base}/apps/cohort-explorer/launch`);
    expect(init.method).toBe('POST');
    expect(init.headers).toEqual({ 'X-Secret-Key': 'daemon-secret' });
  });

  it.each([
    `/apps/other-app/?t=${token}`,
    `/apps/cohort-explorer/?t=${token}&next=/sessions`,
    `/apps/cohort-explorer/?t=${token.slice(4)}`,
    `/apps/cohort-explorer/`,
    `//evil.test/apps/cohort-explorer/?t=${token}`,
    null,
  ])('refuses the answer %s', async (path) => {
    window.electron = { getSecretKey: vi.fn().mockResolvedValue('daemon-secret') } as never;
    daemonAnswering({ path });
    await expect(appLaunchUrl('cohort-explorer', base)).rejects.toThrow('unexpected app address');
  });

  it('reports a refusal instead of opening anything', async () => {
    window.electron = { getSecretKey: vi.fn().mockResolvedValue('daemon-secret') } as never;
    daemonAnswering('no such app', 404);
    await expect(appLaunchUrl('cohort-explorer', base)).rejects.toThrow('HTTP 404');
  });
});
