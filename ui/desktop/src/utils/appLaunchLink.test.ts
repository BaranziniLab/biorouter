import { describe, expect, it, vi } from 'vitest';
import {
  browserLaunchLink,
  mintAppLaunchLink,
  openAppInThisBrowser,
  requestAppLaunch,
} from './appLaunchLink';

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

  // Each cause its own sentence: they used to collapse into "would not open this app".
  it.each([401, 403])('says the background service refused this window on a %i', async (status) => {
    await expect(
      mintAppLaunchLink(base, 'cohort-explorer', 'daemon-secret', daemonAnswering('', status))
    ).rejects.toThrow(
      "Biorouter's background service refused this window's connection. Quit and reopen Biorouter."
    );
  });

  it.each(['daemon_restarted', 'daemon_unavailable'])(
    'shows the proxy’s own sentence for %s',
    async (code) => {
      const message = `The proxy says ${code}.`;
      await expect(
        mintAppLaunchLink(
          base,
          'cohort-explorer',
          'daemon-secret',
          daemonAnswering({ code, error: message, message }, 502)
        )
      ).rejects.toThrow(message);
    }
  );

  it('does not repeat words from a 502 that is not the proxy’s', async () => {
    await expect(
      mintAppLaunchLink(
        base,
        'cohort-explorer',
        'daemon-secret',
        daemonAnswering({ code: 'other', message: '<script>' }, 502)
      )
    ).rejects.toThrow('The app backend would not open this app.');
  });

  it('says the backend did not answer when nothing did', async () => {
    const unreachable = vi.fn(async () => {
      throw new TypeError('fetch failed');
    });
    await expect(
      mintAppLaunchLink(base, 'cohort-explorer', 'daemon-secret', unreachable)
    ).rejects.toThrow('did not answer');
  });
});

/**
 * The desktop reaches a shared daemon through a proxy that refuses every request without the
 * desktop's secret, so the browser is sent to the daemon's own listener, which the daemon names
 * in `url`. A link on the proxy's origin was the "Access to 127.0.0.1 was denied" the person saw.
 */
describe('requestAppLaunch: the daemon’s own listener', () => {
  const proxy = 'http://127.0.0.1:51852';
  const daemon = 'http://127.0.0.1:51851';
  const path = `/apps/cohort-explorer/?t=${token}`;

  it('answers the link on the daemon’s listener beside the one on the caller’s address', async () => {
    const launch = await requestAppLaunch(
      proxy,
      'cohort-explorer',
      'daemon-secret',
      daemonAnswering({ path, url: `${daemon}${path}` })
    );
    expect(launch).toEqual({ path, viaBase: `${proxy}${path}`, onDaemon: `${daemon}${path}` });
    // Asked through the proxy, sent to the daemon: the proxy never reaches the browser.
    const link = browserLaunchLink(launch, { daemonListener: true });
    expect(link).toBe(`${daemon}${path}`);
    expect(link.startsWith(proxy)).toBe(false);
  });

  it.each([`http://[::1]:51851${path}`, `http://localhost:51851${path}`])(
    'takes a loopback listener spelled %s',
    async (url) => {
      const launch = await requestAppLaunch(
        proxy,
        'cohort-explorer',
        'daemon-secret',
        daemonAnswering({ path, url })
      );
      expect(launch.onDaemon).toBe(url);
    }
  );

  it.each([
    // Not an address this machine's browser is sent to: the caller's own address is used.
    `https://127.0.0.1:51851${path}`,
    `http://10.0.0.5:51851${path}`,
    `http://evil.test:51851${path}`,
    `http://127.0.0.1${path}`,
    `http://user@127.0.0.1:51851${path}`,
    null,
  ])('names no listener for %s', async (url) => {
    const launch = await requestAppLaunch(
      proxy,
      'cohort-explorer',
      'daemon-secret',
      daemonAnswering({ path, url })
    );
    expect(launch.onDaemon).toBeUndefined();
  });

  it.each([
    `${daemon}/apps/other-app/?t=${token}`,
    `${daemon}/apps/cohort-explorer/?t=${'0'.repeat(64)}`,
    `${daemon}${path}&next=/sessions`,
    `${daemon}${path}#x`,
    `${daemon}/apps/cohort-explorer/agent?t=${token}`,
    `${daemon}/apps/cohort-explorer/?t=${token.toUpperCase()}`,
    'not a url',
    42,
  ])('opens nothing when the listener’s link is %s', async (url) => {
    await expect(
      requestAppLaunch(proxy, 'cohort-explorer', 'daemon-secret', daemonAnswering({ path, url }))
    ).rejects.toThrow('unexpected app address');
  });
});

describe('browserLaunchLink', () => {
  const path = `/apps/cohort-explorer/?t=${token}`;
  const viaBase = `http://127.0.0.1:51852${path}`;

  it('sends the browser to the caller’s address for a daemon Biorouter does not run', () => {
    // An external backend may be a tunnel, whose far end's listener is not this machine's.
    expect(
      browserLaunchLink(
        { path, viaBase, onDaemon: `http://127.0.0.1:3000${path}` },
        { daemonListener: false, appOrigin: 'http://127.0.0.1:3000' }
      )
    ).toBe(viaBase);
  });

  it('falls back to the listener learned at attach for a daemon too old to name one', () => {
    expect(
      browserLaunchLink(
        { path, viaBase },
        { daemonListener: true, appOrigin: 'http://127.0.0.1:51851' }
      )
    ).toBe(`http://127.0.0.1:51851${path}`);
    expect(browserLaunchLink({ path, viaBase }, { daemonListener: true })).toBe(viaBase);
    for (const appOrigin of ['http://10.0.0.5:1', 'http://127.0.0.1:1/x', 'https://127.0.0.1:1']) {
      expect(browserLaunchLink({ path, viaBase }, { daemonListener: true, appOrigin })).toBe(
        viaBase
      );
    }
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

  it('keeps its own address when the daemon names its listener', async () => {
    // A `biorouter serve` browser may reach the daemon at a LAN or proxied address, from where
    // the daemon's loopback listener is somebody else's machine.
    const tab = fakeTab();
    await openAppInThisBrowser(base, 'cohort-explorer', 'daemon-secret', {
      openTab: () => tab,
      fetchImpl: daemonAnswering({
        path: `/apps/cohort-explorer/?t=${token}`,
        url: `http://127.0.0.1:9/apps/cohort-explorer/?t=${token}`,
      }),
    });
    expect(tab.location.replace).toHaveBeenCalledWith(link);
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
