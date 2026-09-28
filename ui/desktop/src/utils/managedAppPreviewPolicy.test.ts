import { describe, expect, it, vi } from 'vitest';
import {
  isManagedAppNavigation,
  isManagedAppRequest,
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
    'http://127.0.0.1:64006/apps/queue-workbench/',
    'http://localhost:64005/apps/queue-workbench/',
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
      'will not open'
    );
  });

  it('says when the app is gone or the backend refuses', async () => {
    const { launchable } = withSecret();
    await expect(managedAppLaunchUrl(launchable, answer('no such app', 404))).rejects.toThrow(
      'This app no longer exists.'
    );
    await expect(managedAppLaunchUrl(launchable, answer('', 401))).rejects.toThrow(
      'would not open this app'
    );
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
