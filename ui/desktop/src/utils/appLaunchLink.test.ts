import { describe, expect, it, vi } from 'vitest';
import { mintAppLaunchLink, openAppInThisBrowser } from './appLaunchLink';

const base = 'http://127.0.0.1:64005';
const token = 'ef'.repeat(32);
const link = `${base}/apps/cohort-explorer/?t=${token}`;

function daemonAnswering(body: unknown, status = 200) {
  return vi.fn(
    async () =>
      ({
        ok: status >= 200 && status < 300,
        status,
        json: async () => body,
      }) as Response
  );
}

describe('mintAppLaunchLink', () => {
  it('asks the daemon with the secret and answers its one-time link', async () => {
    const daemon = daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}` });
    await expect(mintAppLaunchLink(base, 'cohort-explorer', 'daemon-secret', daemon)).resolves.toBe(
      link
    );
    const [url, init] = daemon.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe(`${base}/apps/cohort-explorer/launch`);
    expect(init).toMatchObject({
      method: 'POST',
      headers: { 'X-Secret-Key': 'daemon-secret' },
      redirect: 'error',
    });
  });

  it.each([
    `/apps/other-app/?t=${token}`,
    `/apps/cohort-explorer/?t=${token}&next=/sessions`,
    `/apps/cohort-explorer/?t=${token.slice(4)}`,
    `/apps/cohort-explorer/?t=${token.toUpperCase()}`,
    `/apps/cohort-explorer/`,
    `//evil.test/apps/cohort-explorer/?t=${token}`,
    null,
  ])('opens nothing when the daemon answers %s', async (answer) => {
    await expect(
      mintAppLaunchLink(base, 'cohort-explorer', 'daemon-secret', daemonAnswering({ path: answer }))
    ).rejects.toThrow('unexpected app address');
  });

  it.each(['../sessions', 'a/b', '', 'x'.repeat(129)])(
    'asks nothing for an app named %j',
    async (appId) => {
      const daemon = daemonAnswering({});
      await expect(mintAppLaunchLink(base, appId, 'daemon-secret', daemon)).rejects.toThrow(
        'not an app name'
      );
      expect(daemon).not.toHaveBeenCalled();
    }
  );

  it.each([
    'http://127.0.0.1:64005/?next=/sessions',
    'http://127.0.0.1:64005/#x',
    'file:///tmp/x',
    'http://u:p@127.0.0.1:1',
    'not a url',
  ])('asks nothing of a backend at %s', async (baseUrl) => {
    const daemon = daemonAnswering({});
    await expect(
      mintAppLaunchLink(baseUrl, 'cohort-explorer', 'daemon-secret', daemon)
    ).rejects.toThrow('no usable address');
    expect(daemon).not.toHaveBeenCalled();
  });

  it('says the app is gone on a 404', async () => {
    await expect(
      mintAppLaunchLink(base, 'cohort-explorer', 'daemon-secret', daemonAnswering('', 404))
    ).rejects.toThrow('no longer exists');
  });
});

it('keeps the path of a daemon served below one', async () => {
  const daemon = daemonAnswering({ path: `/apps/cohort-explorer/?t=${token}` });
  await expect(
    mintAppLaunchLink('https://host.test/api/', 'cohort-explorer', 'daemon-secret', daemon)
  ).resolves.toBe(`https://host.test/api/apps/cohort-explorer/?t=${token}`);
  expect(daemon).toHaveBeenCalledWith(
    'https://host.test/api/apps/cohort-explorer/launch',
    expect.anything()
  );
});

describe('openAppInThisBrowser', () => {
  function fakeTab() {
    return { opener: {} as unknown, close: vi.fn(), location: { replace: vi.fn() } };
  }

  it('opens the tab first, then sends it, cut off from this page, to the link', async () => {
    const tab = fakeTab();
    const order: string[] = [];
    const daemon = vi.fn(async () => {
      order.push('asked');
      return {
        ok: true,
        status: 200,
        json: async () => ({ path: `/apps/cohort-explorer/?t=${token}` }),
      } as Response;
    });
    await openAppInThisBrowser(base, 'cohort-explorer', 'daemon-secret', {
      openTab: () => {
        order.push('tab');
        return tab;
      },
      fetchImpl: daemon,
    });
    expect(order).toEqual(['tab', 'asked']);
    expect(tab.opener).toBeNull();
    expect(tab.location.replace).toHaveBeenCalledWith(link);
    expect(tab.close).not.toHaveBeenCalled();
  });

  it('closes the tab when the daemon will not open the app', async () => {
    const tab = fakeTab();
    await expect(
      openAppInThisBrowser(base, 'cohort-explorer', 'daemon-secret', {
        openTab: () => tab,
        fetchImpl: daemonAnswering('', 404),
      })
    ).rejects.toThrow('no longer exists');
    expect(tab.close).toHaveBeenCalledOnce();
    expect(tab.location.replace).not.toHaveBeenCalled();
  });

  it('asks nothing when the browser blocks the tab', async () => {
    const daemon = daemonAnswering({});
    await expect(
      openAppInThisBrowser(base, 'cohort-explorer', 'daemon-secret', {
        openTab: () => null,
        fetchImpl: daemon,
      })
    ).rejects.toThrow('did not open a new tab');
    expect(daemon).not.toHaveBeenCalled();
  });
});
