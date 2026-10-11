import { describe, expect, it, vi } from 'vitest';
import {
  isManagedAppNavigation,
  isManagedAppRequest,
  localAppLinkId,
  localAppPageId,
  managedAppLaunchUrl,
  managedAppPreviewScope,
} from './managedAppPreviewPolicy';

const origin = 'http://127.0.0.1:64005';
const controller = new AbortController();
const backend = { baseUrl: origin, signal: controller.signal };
const scope = managedAppPreviewScope(`${origin}/apps/queue-workbench/`, backend)!;

describe('main-owned managed app scope', () => {
  it('requires managed provenance supplied separately from the URL', () => {
    expect(managedAppPreviewScope(`${origin}/apps/queue-workbench/`)).toBeNull();
    expect(scope.appId).toBe('queue-workbench');
  });

  it.each([
    'https://127.0.0.1:64005/apps/queue-workbench/',
    'http://10.0.0.1:64005/apps/queue-workbench/',
    'http://evil.test:64005/apps/queue-workbench/',
    'http://[::1]:64005/apps/queue-workbench/',
    'http://0.0.0.0:64005/apps/queue-workbench/',
    'http://127.1:64005/apps/queue-workbench/',
    'http://2130706433:64005/apps/queue-workbench/',
    'http://127.0.0.1:64005@evil.test/apps/queue-workbench/',
    'http://user@127.0.0.1:64005/apps/queue-workbench/',
    `${origin}/apps/../apps/queue-workbench/`,
    `${origin}/apps/%71ueue-workbench/`,
    `${origin}/apps/queue-workbench%2fother/`,
    `${origin}/apps/queue-workbench/build`,
    `${origin}/sessions`,
    `${origin}/apps/${'a'.repeat(129)}/`,
    'file:///apps/queue-workbench/',
  ])('does not mint a scope for %s', (url) => {
    expect(managedAppPreviewScope(url, backend)).toBeNull();
  });

  it.each([
    'http://localhost:64005',
    'http://127.1:64005',
    'http://127.0.0.1',
    'http://127.0.0.1:64005/path',
    'https://remote.test',
  ])('rejects an invalid managed base %s', (baseUrl) => {
    expect(
      managedAppPreviewScope(`${origin}/apps/queue-workbench/`, { ...backend, baseUrl })
    ).toBeNull();
    // ...and the same as the app origin, whatever the base.
    expect(
      managedAppPreviewScope(`${origin}/apps/queue-workbench/`, {
        ...backend,
        appOrigin: baseUrl,
      })
    ).toBeNull();
  });
});

/**
 * The daemon's port is new on every start, and a shared daemon serves apps on its own listener
 * while the window talks to a proxy on another port. A link to an app opens that app on THIS
 * window's daemon whatever port it names; once open, the preview stays on exactly that origin.
 */
describe('path-based rebinding of app links', () => {
  const proxy = 'http://127.0.0.1:51852';
  const daemon = 'http://127.0.0.1:51851';
  const shared = { baseUrl: proxy, appOrigin: daemon, signal: controller.signal };

  it.each([
    `${daemon}/apps/queue-workbench/`,
    `${proxy}/apps/queue-workbench/`,
    'http://127.0.0.1:64006/apps/queue-workbench/',
    'http://127.0.0.1:7/apps/queue-workbench',
    `${daemon}/apps/queue-workbench/?theme=dark#queue`,
  ])('opens %s on the daemon’s own listener', (link) => {
    const rebound = managedAppPreviewScope(link, shared)!;
    expect(rebound).toMatchObject({
      origin: daemon,
      port: 51851,
      appId: 'queue-workbench',
      rootPath: '/apps/queue-workbench/',
    });
  });

  it('opens a link from an earlier run on a per-window daemon’s current port', () => {
    expect(
      managedAppPreviewScope('http://127.0.0.1:50001/apps/queue-workbench/', backend)
    ).toMatchObject({ origin, port: 64005, appId: 'queue-workbench' });
  });

  it('never lets the opened preview reach the port the link named, or the proxy', () => {
    const rebound = managedAppPreviewScope('http://127.0.0.1:64006/apps/queue-workbench/', shared)!;
    expect(isManagedAppNavigation(rebound, `${daemon}/apps/queue-workbench/`)).toBe(true);
    for (const other of ['http://127.0.0.1:64006', proxy, 'http://localhost:51851']) {
      expect(isManagedAppNavigation(rebound, `${other}/apps/queue-workbench/`)).toBe(false);
      expect(
        isManagedAppRequest(rebound, {
          url: `${other}/apps/queue-workbench/dist/app.js`,
          method: 'GET',
          resourceType: 'script',
        })
      ).toBe(false);
      expect(
        isManagedAppRequest(rebound, {
          url: `${other.replace('http:', 'ws:')}/apps/queue-workbench/agent`,
          method: 'GET',
          resourceType: 'webSocket',
        })
      ).toBe(false);
    }
    expect(
      isManagedAppRequest(rebound, {
        url: 'ws://127.0.0.1:51851/apps/queue-workbench/agent',
        method: 'GET',
        resourceType: 'webSocket',
      })
    ).toBe(true);
  });

  it('names the app a link to open in the browser points at, on any loopback spelling', () => {
    for (const link of [
      `${daemon}/apps/queue-workbench/`,
      'http://[::1]:9/apps/queue-workbench/dist/app.js',
      'http://127.0.0.1:9/apps/queue-workbench/?t=x',
    ]) {
      expect(localAppLinkId(link)).toBe('queue-workbench');
    }
    expect(localAppPageId(`${daemon}/apps/queue-workbench/dist/app.js`)).toBeNull();
    for (const link of [
      'http://127.1:9/apps/queue-workbench/',
      'http://127.0.0.1:9@evil.test/apps/queue-workbench/',
      'http://user@127.0.0.1:9/apps/queue-workbench/',
      'http://example.test/apps/queue-workbench/',
      'http://127.0.0.1:9/apps/../sessions',
      'http://127.0.0.1:9/apps/%71ueue/',
      'http://127.0.0.1:9/applications/x/',
      'http://127.0.0.1:9/apps/',
      'file:///apps/queue-workbench/',
      // Spellings the daemon never writes.
      'https://127.0.0.1:9/apps/queue-workbench/',
      'https://localhost:1/apps/queue-workbench',
    ]) {
      expect(localAppLinkId(link), link).toBeNull();
      expect(localAppPageId(link), link).toBeNull();
    }
  });

  /**
   * `localhost` is what a web development server prints. The agent running the person's own
   * project on Vite and pointing at its `/apps/dashboard/` route used to get the person's
   * Biorouter app named `dashboard` in the panel instead, or "This app no longer exists."
   */
  it.each([
    'http://localhost:5173/apps/dashboard/',
    'http://localhost:3000/apps/dashboard',
    'http://localhost/apps/dashboard/',
  ])('leaves a development server’s %s alone', (link) => {
    expect(managedAppPreviewScope(link, shared)).toBeNull();
    expect(localAppPageId(link)).toBeNull();
    expect(localAppLinkId(link)).toBeNull();
  });
});

describe('managed app default-deny requests', () => {
  it.each([
    '',
    'dist/app.js',
    'assets/chart-v2.css',
    'assets/icons/check.svg',
    'models',
    'runstate?session=qa',
  ])('allows a public app GET for %s', (tail) => {
    expect(
      isManagedAppRequest(scope, {
        url: `${origin}${scope.rootPath}${tail}`,
        method: 'GET',
        resourceType: 'xhr',
      })
    ).toBe(true);
  });

  it('allows only the exact app agent WebSocket', () => {
    const request = {
      url: `ws://127.0.0.1:64005${scope.rootPath}agent`,
      method: 'GET',
      resourceType: 'webSocket',
    };
    expect(isManagedAppRequest(scope, request)).toBe(true);
    expect(
      isManagedAppRequest(scope, { ...request, url: request.url.replace('/agent', '/models') })
    ).toBe(false);
    expect(isManagedAppRequest(scope, { ...request, resourceType: 'xhr' })).toBe(false);
  });

  it('allows the figure shell only as a subframe of this app on the same daemon', () => {
    const request = {
      url: `${origin}${scope.rootPath}figure-frame`,
      method: 'GET',
      resourceType: 'subFrame',
    };
    expect(isManagedAppRequest(scope, request)).toBe(true);
    for (const resourceType of ['mainFrame', 'xhr', 'script', 'webSocket', 'image']) {
      expect(isManagedAppRequest(scope, { ...request, resourceType })).toBe(false);
    }
    for (const url of [
      `${origin}/apps/other/figure-frame`,
      `http://127.0.0.1:64006${scope.rootPath}figure-frame`,
      `${origin}${scope.rootPath}figure-frame/`,
      `${origin}${scope.rootPath}figure-frame/anything`,
    ]) {
      expect(isManagedAppRequest(scope, { ...request, url })).toBe(false);
    }
  });

  it.each([
    '/sessions',
    '/config',
    '/apps',
    '/apps/other/',
    '/apps/other/dist/app.js',
    '/apps/queue-workbench/export',
    '/apps/queue-workbench/build',
    '/apps/queue-workbench/vault',
    '/apps/queue-workbench/future-admin',
    '/apps/queue-workbench/assets/../models',
    '/apps/queue-workbench/assets/%2e%2e/models',
    '/apps/queue-workbench/assets/%252e%252e/models',
    '/apps/queue-workbench/assets/foo%2fbar',
    '/apps/queue-workbench/assets/foo\\bar',
    '/apps/queue-workbench/assets//app.js',
  ])('blocks %s', (path) => {
    expect(
      isManagedAppRequest(scope, { url: `${origin}${path}`, method: 'GET', resourceType: 'xhr' })
    ).toBe(false);
  });

  it.each(['POST', 'PUT', 'DELETE', 'HEAD', 'OPTIONS'])('blocks method %s', (method) => {
    expect(
      isManagedAppRequest(scope, { url: `${origin}${scope.rootPath}`, method, resourceType: 'xhr' })
    ).toBe(false);
  });

  it.each([
    'http://127.0.0.1:64006',
    'http://10.0.0.1:64005',
    'http://169.254.169.254',
    'https://example.test',
  ])('blocks other origin %s', (other) =>
    expect(
      isManagedAppRequest(scope, {
        url: `${other}${scope.rootPath}`,
        method: 'GET',
        resourceType: 'xhr',
      })
    ).toBe(false)
  );

  it('permits data images/fonts but never data documents or script', () => {
    expect(
      isManagedAppRequest(scope, {
        url: 'data:image/png;base64,AA==',
        method: 'GET',
        resourceType: 'image',
      })
    ).toBe(true);
    expect(
      isManagedAppRequest(scope, {
        url: 'data:font/woff2;base64,AA==',
        method: 'GET',
        resourceType: 'font',
      })
    ).toBe(true);
    for (const resourceType of ['mainFrame', 'subFrame', 'script', 'xhr']) {
      expect(
        isManagedAppRequest(scope, { url: 'data:text/html,unsafe', method: 'GET', resourceType })
      ).toBe(false);
    }
  });

  it('keeps navigation within one app, including redirects and address-bar changes', () => {
    expect(isManagedAppNavigation(scope, `${origin}${scope.rootPath}?theme=dark#queue`)).toBe(true);
    expect(isManagedAppNavigation(scope, `${origin}/apps/other/`)).toBe(false);
    expect(isManagedAppNavigation(scope, `${origin}${scope.rootPath}assets/page.html`)).toBe(false);
    expect(
      isManagedAppRequest(scope, {
        url: `${origin}${scope.rootPath}models`,
        method: 'GET',
        resourceType: 'mainFrame',
      })
    ).toBe(false);
  });

  it('revokes an already-created scope immediately', () => {
    const lifetime = new AbortController();
    const scoped = managedAppPreviewScope(`${origin}${scope.rootPath}`, {
      baseUrl: origin,
      signal: lifetime.signal,
    })!;
    lifetime.abort();
    expect(isManagedAppNavigation(scoped, `${origin}${scope.rootPath}`)).toBe(false);
    expect(
      isManagedAppRequest(scoped, {
        url: `${origin}${scope.rootPath}`,
        method: 'GET',
        resourceType: 'xhr',
      })
    ).toBe(false);
  });
});

/**
 * W2-HRD-1: the daemon serves an app's page only to a browser holding that app's
 * access cookie, so the preview asks for a launch link with the secret, and
 * opens it only if it is exactly this app's page with a token.
 */
describe('managed app launch link', () => {
  const token = 'ab'.repeat(32);
  const secret = 'daemon-secret';
  const withSecret = () => {
    const lifetime = new AbortController();
    const launchable = managedAppPreviewScope(`${origin}/apps/queue-workbench/`, {
      baseUrl: origin,
      signal: lifetime.signal,
      secretKey: secret,
    })!;
    return { lifetime, launchable };
  };
  const answer = (body: unknown, status = 200) =>
    vi.fn(
      async () =>
        ({
          ok: status >= 200 && status < 300,
          status,
          json: async () => body,
        }) as Response
    );

  it('asks the backend with its secret and opens exactly the page it mints', async () => {
    const { launchable } = withSecret();
    const fetchImpl = answer({ path: `/apps/queue-workbench/?t=${token}` });
    await expect(managedAppLaunchUrl(launchable, fetchImpl)).resolves.toBe(
      `${origin}/apps/queue-workbench/?t=${token}`
    );
    expect(fetchImpl).toHaveBeenCalledOnce();
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe(`${origin}/apps/queue-workbench/launch`);
    expect(init.method).toBe('POST');
    expect(init.headers).toEqual({ 'X-Secret-Key': secret });
    expect(init.redirect).toBe('error');
  });

  it.each([
    `/apps/other-app/?t=${token}`,
    `/apps/queue-workbench/?t=${token}&next=/sessions`,
    `/apps/queue-workbench/?t=${token.toUpperCase()}`,
    `/apps/queue-workbench/?t=${token.slice(2)}`,
    `/apps/queue-workbench/agent?t=${token}`,
    `//evil.test/apps/queue-workbench/?t=${token}`,
    `/sessions`,
    42,
  ])('refuses the answer %s', async (path) => {
    const { launchable } = withSecret();
    await expect(managedAppLaunchUrl(launchable, answer({ path }))).rejects.toThrow(
      'unexpected app address'
    );
  });

  it('says when the app is gone or the backend refuses', async () => {
    const { launchable } = withSecret();
    await expect(managedAppLaunchUrl(launchable, answer('no such app', 404))).rejects.toThrow(
      'This app no longer exists.'
    );
    await expect(managedAppLaunchUrl(launchable, answer('', 401))).rejects.toThrow(
      "refused this window's connection"
    );
  });

  it('mints through the proxy and opens on the shared daemon’s own listener', async () => {
    const proxy = 'http://127.0.0.1:51852';
    const daemon = 'http://127.0.0.1:51851';
    const shared = managedAppPreviewScope(`${daemon}/apps/queue-workbench/`, {
      baseUrl: proxy,
      appOrigin: daemon,
      signal: new AbortController().signal,
      secretKey: secret,
    })!;
    const path = `/apps/queue-workbench/?t=${token}`;
    for (const body of [{ path }, { path, url: `${daemon}${path}` }]) {
      const fetchImpl = answer(body);
      await expect(managedAppLaunchUrl(shared, fetchImpl)).resolves.toBe(`${daemon}${path}`);
      expect((fetchImpl.mock.calls[0] as unknown as [string])[0]).toBe(
        `${proxy}/apps/queue-workbench/launch`
      );
    }
    // A daemon that names another listener is not the one this preview was opened for.
    await expect(
      managedAppLaunchUrl(shared, answer({ path, url: `http://127.0.0.1:50000${path}` }))
    ).rejects.toThrow('The app backend stopped');
  });

  it('asks nothing without a secret or once the backend is gone', async () => {
    const fetchImpl = answer({ path: `/apps/queue-workbench/?t=${token}` });
    await expect(managedAppLaunchUrl(scope, fetchImpl)).rejects.toThrow('cannot be opened');
    const { lifetime, launchable } = withSecret();
    lifetime.abort();
    await expect(managedAppLaunchUrl(launchable, fetchImpl)).rejects.toThrow('backend stopped');
    expect(fetchImpl).not.toHaveBeenCalled();
  });
});
