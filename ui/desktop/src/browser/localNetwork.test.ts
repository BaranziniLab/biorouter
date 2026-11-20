import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  canonicalHost,
  classifyAddress,
  clearLocalGrantsForTests,
  clearReservedLocalPortsForTests,
  configureBrowserTestHosts,
  decideLocalRequest,
  grantLocalDestination,
  hasLocalGrant,
  LOCAL_GRANT_TTL_MS,
  literalHostClass,
  noteLocalRefusal,
  proxyMayConnect,
  reserveLocalPort,
  resolveHostClass,
  setLocalNetworkClockForTests,
  wasRecentlyRefused,
  type ResolveHostClass,
} from './localNetwork';

/**
 * BB-06: which local destinations the Biorouter Browser may reach. The table the design states:
 * the person may open a private or loopback address; a page may reach one only from a page on
 * that same local host; a reserved loopback port (the daemon, the dev renderer, the proxies) is
 * never reachable. Each guard here is one a reverted line would let through.
 */

let now = 1_000_000;
beforeEach(() => {
  now = 1_000_000;
  setLocalNetworkClockForTests(() => now);
  clearLocalGrantsForTests();
  clearReservedLocalPortsForTests();
  configureBrowserTestHosts(undefined, false);
});
afterEach(() => setLocalNetworkClockForTests(null));

const resolveTo =
  (cls: 'public' | 'private' | 'loopback', addresses: string[]): ResolveHostClass =>
  async () => ({ cls, addresses });

describe('address classes', () => {
  it.each([
    ['127.0.0.1', 'loopback'],
    ['127.8.9.10', 'loopback'],
    ['0.0.0.0', 'loopback'],
    ['::1', 'loopback'],
    ['[::1]', 'loopback'],
    ['::ffff:127.0.0.1', 'loopback'],
    ['::ffff:7f00:1', 'loopback'],
    ['::127.0.0.1', 'loopback'],
    ['::ffff:c0a8:101', 'private'],
    ['::ffff:808:808', 'public'],
    ['10.1.2.3', 'private'],
    ['172.16.0.1', 'private'],
    ['192.168.1.1', 'private'],
    ['100.64.0.1', 'private'],
    ['169.254.169.254', 'private'],
    ['fd00::1', 'private'],
    ['fe80::1', 'private'],
    ['8.8.8.8', 'public'],
    ['2606:4700:4700::1111', 'public'],
  ])('%s is %s', (address, cls) => {
    expect(classifyAddress(address)).toBe(cls);
  });

  it('knows localhost without asking DNS and leaves other names to it', () => {
    expect(literalHostClass('localhost')).toBe('loopback');
    expect(literalHostClass('app.localhost')).toBe('loopback');
    expect(literalHostClass('LOCALHOST.')).toBe('loopback');
    expect(literalHostClass('example.com')).toBeNull();
  });

  it('spells a host one way for the filter and the proxy alike', () => {
    expect(canonicalHost('[0:0:0:0:0:0:0:1]')).toBe('::1');
    expect(canonicalHost('Example.COM.')).toBe('example.com');
  });
});

describe('the decision for one request', () => {
  it('lets the person open a loopback page, and grants exactly that host and port', async () => {
    const verdict = await decideLocalRequest(
      { url: new URL('http://localhost:8888/lab'), person: true, localPageHost: null },
      resolveTo('public', [])
    );
    expect(verdict).toEqual({ allow: true, cls: 'loopback' });
    expect(hasLocalGrant('localhost', 8888)).toBe(true);
    // Chromium may hand the proxy the address, not the name.
    expect(hasLocalGrant('127.0.0.1', 8888)).toBe(true);
    expect(hasLocalGrant('localhost', 8889)).toBe(false);
  });

  it('lets the person open an intranet name that resolves private', async () => {
    const verdict = await decideLocalRequest(
      { url: new URL('https://wiki.intranet.example/'), person: true, localPageHost: null },
      resolveTo('private', ['10.0.0.7'])
    );
    expect(verdict).toEqual({ allow: true, cls: 'private' });
    expect(hasLocalGrant('wiki.intranet.example', 443)).toBe(true);
    expect(hasLocalGrant('10.0.0.7', 443)).toBe(true);
  });

  it('refuses a public page reaching a local address, and grants nothing', async () => {
    for (const url of ['http://127.0.0.1:8888/api', 'http://192.168.1.1/', 'http://localhost/']) {
      const verdict = await decideLocalRequest(
        { url: new URL(url), person: false, localPageHost: null },
        resolveTo('public', [])
      );
      expect(verdict).toMatchObject({ allow: false, reason: 'local' });
    }
    expect(hasLocalGrant('127.0.0.1', 8888)).toBe(false);
  });

  it('lets a local page reach its own host, on any port', async () => {
    for (const url of ['http://localhost:8888/api/kernels', 'ws://localhost:8000/socket']) {
      const verdict = await decideLocalRequest(
        { url: new URL(url), person: false, localPageHost: 'localhost' },
        resolveTo('public', [])
      );
      expect(verdict.allow).toBe(true);
    }
  });

  it('refuses a local page reaching a different local host', async () => {
    const verdict = await decideLocalRequest(
      { url: new URL('http://192.168.1.1/'), person: false, localPageHost: 'localhost' },
      resolveTo('public', [])
    );
    expect(verdict).toMatchObject({ allow: false, reason: 'local' });
  });

  it('refuses a public frame inside a local page', async () => {
    const verdict = await decideLocalRequest(
      {
        url: new URL('http://localhost:8888/api'),
        person: false,
        localPageHost: 'localhost',
        requesterHost: 'ads.example.com',
      },
      resolveTo('public', [])
    );
    expect(verdict).toMatchObject({ allow: false, reason: 'local' });
  });

  it('never reaches a reserved loopback port, not even for the person', async () => {
    reserveLocalPort(53111);
    for (const url of [
      'http://127.0.0.1:53111/',
      'http://localhost:53111/',
      'http://[::1]:53111/',
      'http://[::ffff:127.0.0.1]:53111/',
      'http://0.0.0.0:53111/',
      'http://127.1.2.3:53111/',
    ]) {
      const verdict = await decideLocalRequest(
        { url: new URL(url), person: true, localPageHost: null },
        resolveTo('public', [])
      );
      expect(verdict).toMatchObject({ allow: false, reason: 'reserved' });
    }
    // A name that resolves to loopback is caught too.
    const named = await decideLocalRequest(
      { url: new URL('http://sneaky.example:53111/'), person: true, localPageHost: null },
      resolveTo('loopback', ['127.0.0.1'])
    );
    expect(named).toMatchObject({ allow: false, reason: 'reserved' });
    expect(hasLocalGrant('127.0.0.1', 53111)).toBe(false);
  });

  it('does not resolve a name a public page asks for when nobody holds a grant for it', async () => {
    const resolve = vi.fn(resolveTo('private', ['10.0.0.9']));
    const verdict = await decideLocalRequest(
      { url: new URL('https://example.com/'), person: false, localPageHost: null },
      resolve
    );
    expect(verdict).toEqual({ allow: true, cls: null });
    expect(resolve).not.toHaveBeenCalled();
  });

  it('refuses a public page using a name another tab was granted', async () => {
    grantLocalDestination('jupyter.lan', 8888);
    const verdict = await decideLocalRequest(
      { url: new URL('http://jupyter.lan:8888/api'), person: false, localPageHost: null },
      resolveTo('private', ['192.168.1.20'])
    );
    expect(verdict).toMatchObject({ allow: false, reason: 'local' });
  });
});

describe('the proxy check', () => {
  it('connects public destinations and refuses local ones without a grant', () => {
    expect(proxyMayConnect('example.com', 443, 'public')).toBe(true);
    expect(proxyMayConnect('127.0.0.1', 8888, 'loopback')).toBe(false);
    expect(proxyMayConnect('10.0.0.1', 80, 'private')).toBe(false);
  });

  it('connects a granted local destination until the grant expires', () => {
    grantLocalDestination('127.0.0.1', 8888);
    expect(proxyMayConnect('127.0.0.1', 8888, 'loopback')).toBe(true);
    now += LOCAL_GRANT_TTL_MS + 1;
    expect(proxyMayConnect('127.0.0.1', 8888, 'loopback')).toBe(false);
  });

  it('never connects a reserved port, grant or not', () => {
    reserveLocalPort('47000');
    grantLocalDestination('127.0.0.1', 47000);
    expect(proxyMayConnect('127.0.0.1', 47000, 'loopback')).toBe(false);
  });

  it('remembers a refusal briefly, so a failed load can say why', () => {
    noteLocalRefusal('intranet.example', 80);
    expect(wasRecentlyRefused('intranet.example', 80)).toBe(true);
    now += 31_000;
    expect(wasRecentlyRefused('intranet.example', 80)).toBe(false);
  });
});

describe('fixture host names for the Electron tests', () => {
  it('are ignored unless enabled', async () => {
    configureBrowserTestHosts('site-a.test=127.0.0.1:public', false);
    expect(literalHostClass('site-a.test')).toBeNull();
  });

  it('map to loopback with the class they stand for when enabled', async () => {
    configureBrowserTestHosts(
      'site-a.test=127.0.0.1:public, intranet.test=127.0.0.1:private,bad entry,x.test=nope:public',
      true
    );
    expect(literalHostClass('site-a.test')).toBe('public');
    expect(literalHostClass('intranet.test')).toBe('private');
    expect(literalHostClass('x.test')).toBeNull();
    await expect(resolveHostClass('site-a.test')).resolves.toEqual({
      cls: 'public',
      addresses: ['127.0.0.1'],
      family: 4,
    });
    configureBrowserTestHosts(undefined, false);
  });
});

describe('resolving a host', () => {
  it('reads one local answer as local', async () => {
    const lookup = async () => [
      { address: '93.184.216.34', family: 4 },
      { address: '10.0.0.5', family: 4 },
    ];
    await expect(resolveHostClass('mixed.example', lookup)).resolves.toMatchObject({
      cls: 'private',
    });
  });

  it('refuses an answer whose family is wrong', async () => {
    const lookup = async () => [{ address: '10.0.0.5', family: 6 }];
    await expect(resolveHostClass('odd.example', lookup)).resolves.toBeNull();
  });
});
