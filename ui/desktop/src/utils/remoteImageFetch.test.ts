// @vitest-environment node
import { describe, expect, it, vi } from 'vitest';
import {
  fetchRemoteImage,
  pinnedRequestOptions,
  REMOTE_IMAGE_MAX_REDIRECTS,
} from './remoteImageFetch';
import { remoteImageTarget } from './remoteImage';

/**
 * The main process's half of click-to-load remote images (OP-P0-09, owner
 * answer D15). A person clicked one image in a chat; this fetches exactly that
 * address, carrying nothing that could tie it to anything else the app has
 * loaded: no cookie, no referrer, no cache, and never a private address.
 */

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);

type Pinned = { address: string; family: 4 | 6 };
type FetchCall = { url: string; init: RequestInit; pinned: Pinned };

function fakeFetch(responses: Array<Response | (() => Response) | Error>) {
  const calls: FetchCall[] = [];
  const queue = [...responses];
  const fetch = vi.fn(async (url: string, init: RequestInit, pinned: Pinned) => {
    calls.push({ url, init, pinned });
    const next = queue.shift();
    if (!next) throw new Error('no response queued');
    if (next instanceof Error) throw next;
    return typeof next === 'function' ? next() : next;
  });
  return { fetch, calls };
}

const publicHost = vi.fn(async () => ({ address: '93.184.216.34', family: 4 as const }));

function image(bytes: Uint8Array<ArrayBuffer> = PNG, headers: Record<string, string> = {}) {
  return new Response(bytes, {
    status: 200,
    headers: { 'content-type': 'image/png', ...headers },
  });
}

function redirect(location: string, status = 302) {
  return new Response(null, { status, headers: { location } });
}

describe('remoteImageTarget', () => {
  it('names the host of an https image and keeps the address', () => {
    expect(remoteImageTarget('https://cdn.example.org/fig/plot.png?x=1')).toEqual({
      url: 'https://cdn.example.org/fig/plot.png?x=1',
      host: 'cdn.example.org',
    });
  });

  it('asks for an http image over https, the upgrade the page policy always made', () => {
    expect(remoteImageTarget('http://cdn.example.org/a.png')).toEqual({
      url: 'https://cdn.example.org/a.png',
      host: 'cdn.example.org',
    });
  });

  it('reads an address with odd slashes the way a URL parser does', () => {
    expect(remoteImageTarget('https:/cdn.example.org/a.png')).toEqual({
      url: 'https://cdn.example.org/a.png',
      host: 'cdn.example.org',
    });
    expect(remoteImageTarget('HTTPS:\\\\cdn.example.org\\a.png')).toEqual({
      url: 'https://cdn.example.org/a.png',
      host: 'cdn.example.org',
    });
  });

  it('keeps a port in the host it shows', () => {
    expect(remoteImageTarget('https://images.example.org:8443/a.png')?.host).toBe(
      'images.example.org:8443'
    );
  });

  it('shows an internationalised host in its ASCII form, so a look-alike cannot pass', () => {
    expect(remoteImageTarget('https://exämple.org/a.png')?.host).toBe('xn--exmple-cua.org');
  });

  it.each([
    'https://user:secret@cdn.example.org/a.png',
    'https://user@cdn.example.org/a.png',
    'javascript:alert(1)',
    'data:image/png;base64,AAAA',
    'file:///etc/passwd',
    'ftp://cdn.example.org/a.png',
    './fig.png',
    '//cdn.example.org/a.png',
    '',
    `https://cdn.example.org/${'a'.repeat(9 * 1024)}.png`,
  ])('offers nothing to load for %s', (raw) => {
    expect(remoteImageTarget(raw)).toBeNull();
  });
});

describe('fetchRemoteImage', () => {
  it('returns the bytes and type of the one image asked for', async () => {
    const { fetch, calls } = fakeFetch([image()]);
    const result = await fetchRemoteImage('https://cdn.example.org/plot.png', {
      fetch,
      resolveHost: publicHost,
    });
    expect(result).toEqual({ ok: true, mimeType: 'image/png', bytes: PNG });
    expect(calls.map((call) => call.url)).toEqual(['https://cdn.example.org/plot.png']);
  });

  it('carries no cookie, no referrer and no cache, and follows no redirect by itself', async () => {
    const { fetch, calls } = fakeFetch([image()]);
    await fetchRemoteImage('https://cdn.example.org/plot.png', { fetch, resolveHost: publicHost });
    const init = calls[0].init;
    expect(init.credentials).toBe('omit');
    expect(init.referrerPolicy).toBe('no-referrer');
    expect(init.cache).toBe('no-store');
    expect(init.redirect).toBe('manual');
    expect(init.method).toBe('GET');
    const headers = new Headers(init.headers);
    expect(headers.has('cookie')).toBe(false);
    expect(headers.has('referer')).toBe(false);
    expect(headers.has('authorization')).toBe(false);
  });

  it('requests an http address over https', async () => {
    const { fetch, calls } = fakeFetch([image()]);
    await fetchRemoteImage('http://cdn.example.org/a.png', { fetch, resolveHost: publicHost });
    expect(calls[0].url).toBe('https://cdn.example.org/a.png');
  });

  it.each([
    42,
    null,
    'javascript:alert(1)',
    'https://user:pw@cdn.example.org/a.png',
    'file:///etc/hosts',
  ])('refuses %s without a request', async (raw) => {
    const { fetch } = fakeFetch([]);
    const result = await fetchRemoteImage(raw, { fetch, resolveHost: publicHost });
    expect(result).toEqual({ ok: false, reason: 'invalid' });
    expect(fetch).not.toHaveBeenCalled();
  });

  it('refuses a private or loopback address before any request', async () => {
    const { fetch } = fakeFetch([image()]);
    const resolveHost = vi.fn(async () => {
      throw new Error('Local addresses are not available to embedded pages.');
    });
    const result = await fetchRemoteImage('https://intranet.example.org/a.png', {
      fetch,
      resolveHost,
    });
    expect(result).toEqual({ ok: false, reason: 'not-public' });
    expect(resolveHost).toHaveBeenCalledWith('intranet.example.org');
    expect(fetch).not.toHaveBeenCalled();
  });

  it('checks every redirect hop, so a public address cannot bounce to a private one', async () => {
    const { fetch, calls } = fakeFetch([redirect('https://10.0.0.7/a.png')]);
    const resolveHost = vi.fn(async (host: string) => {
      if (host === '10.0.0.7') throw new Error('private');
      return { address: '93.184.216.34', family: 4 as const };
    });
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', { fetch, resolveHost });
    expect(result).toEqual({ ok: false, reason: 'not-public' });
    expect(calls).toHaveLength(1);
  });

  it('follows a redirect to another public image, over https', async () => {
    const { fetch, calls } = fakeFetch([redirect('http://img.example.net/b.png', 301), image()]);
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', {
      fetch,
      resolveHost: publicHost,
    });
    expect(result.ok).toBe(true);
    expect(calls.map((call) => call.url)).toEqual([
      'https://cdn.example.org/a.png',
      'https://img.example.net/b.png',
    ]);
  });

  it('gives up after a bounded number of redirects', async () => {
    const hops = Array.from({ length: REMOTE_IMAGE_MAX_REDIRECTS + 2 }, (_, index) =>
      redirect(`https://cdn.example.org/hop-${index + 1}.png`)
    );
    const { fetch, calls } = fakeFetch(hops);
    const result = await fetchRemoteImage('https://cdn.example.org/hop-0.png', {
      fetch,
      resolveHost: publicHost,
    });
    expect(result).toEqual({ ok: false, reason: 'unavailable' });
    expect(calls).toHaveLength(REMOTE_IMAGE_MAX_REDIRECTS + 1);
  });

  it('refuses a response that is not an image', async () => {
    const { fetch } = fakeFetch([
      new Response('<html>hi</html>', { status: 200, headers: { 'content-type': 'text/html' } }),
    ]);
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', {
      fetch,
      resolveHost: publicHost,
    });
    expect(result).toEqual({ ok: false, reason: 'not-an-image' });
  });

  it('keeps the type without its parameters', async () => {
    const svg = new TextEncoder().encode('<svg xmlns="http://www.w3.org/2000/svg"/>');
    const { fetch } = fakeFetch([image(svg, { 'content-type': 'Image/SVG+XML; charset=utf-8' })]);
    const result = await fetchRemoteImage('https://cdn.example.org/a.svg', {
      fetch,
      resolveHost: publicHost,
    });
    expect(result).toEqual({ ok: true, mimeType: 'image/svg+xml', bytes: svg });
  });

  it('refuses an image whose declared size is over the limit before reading it', async () => {
    const { fetch } = fakeFetch([image(PNG, { 'content-length': String(64 * 1024 * 1024) })]);
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', {
      fetch,
      resolveHost: publicHost,
      maxBytes: 1024,
    });
    expect(result).toEqual({ ok: false, reason: 'too-large' });
  });

  it('stops reading a body that grows past the limit without a declared size', async () => {
    const big = new Uint8Array(4096);
    const { fetch } = fakeFetch([image(big)]);
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', {
      fetch,
      resolveHost: publicHost,
      maxBytes: 1024,
    });
    expect(result).toEqual({ ok: false, reason: 'too-large' });
  });

  it('reports an error status as unavailable', async () => {
    const { fetch } = fakeFetch([new Response('gone', { status: 404 })]);
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', {
      fetch,
      resolveHost: publicHost,
    });
    expect(result).toEqual({ ok: false, reason: 'unavailable' });
  });

  it('reports a network failure as unavailable', async () => {
    const { fetch } = fakeFetch([new TypeError('fetch failed')]);
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', {
      fetch,
      resolveHost: publicHost,
    });
    expect(result).toEqual({ ok: false, reason: 'unavailable' });
  });

  // The vetted address is the one connected to. Were the name looked up again
  // for the connection, a host whose answer changes between the two lookups (DNS
  // rebinding) could turn a click into a request to a private address.
  it('connects to the address it checked, on every hop', async () => {
    const { fetch, calls } = fakeFetch([redirect('https://img.example.net/b.png'), image()]);
    const resolveHost = vi.fn(async (host: string) =>
      host === 'cdn.example.org'
        ? { address: '93.184.216.34', family: 4 as const }
        : { address: '2606:2800:220:1:248:1893:25c8:1946', family: 6 as const }
    );
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', { fetch, resolveHost });
    expect(result.ok).toBe(true);
    expect(calls.map((call) => call.pinned)).toEqual([
      { address: '93.184.216.34', family: 4 },
      { address: '2606:2800:220:1:248:1893:25c8:1946', family: 6 },
    ]);
  });

  it('gives up on a server that never answers', async () => {
    const fetch = vi.fn(
      (_url: string, init: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          init.signal?.addEventListener('abort', () => reject(new Error('aborted')));
        })
    );
    const result = await fetchRemoteImage('https://cdn.example.org/a.png', {
      fetch,
      resolveHost: publicHost,
      timeoutMs: 20,
    });
    expect(result).toEqual({ ok: false, reason: 'unavailable' });
  });
});

describe('pinnedRequestOptions', () => {
  const pinned = { address: '93.184.216.34', family: 4 as const };
  const init: RequestInit = { method: 'GET', headers: { Accept: 'image/*' } };

  function lookUp(
    options: ReturnType<typeof pinnedRequestOptions>,
    hostname: string,
    lookupOptions: { all?: boolean }
  ): Promise<unknown[]> {
    return new Promise((resolve, reject) => {
      const lookup = options.lookup as unknown as (
        host: string,
        opts: { all?: boolean },
        callback: (error: Error | null, ...rest: unknown[]) => void
      ) => void;
      lookup(hostname, lookupOptions, (error, ...rest) => (error ? reject(error) : resolve(rest)));
    });
  }

  it('answers every lookup with the checked address, so the name is never looked up again', async () => {
    const options = pinnedRequestOptions('https://cdn.example.org/fig/a.png?x=1', init, pinned);
    expect(await lookUp(options, 'cdn.example.org', {})).toEqual(['93.184.216.34', 4]);
    expect(await lookUp(options, 'cdn.example.org', { all: true })).toEqual([
      [{ address: '93.184.216.34', family: 4 }],
    ]);
    // Whatever name the socket layer asks about, the answer does not move.
    expect(await lookUp(options, 'rebound.example.org', {})).toEqual(['93.184.216.34', 4]);
  });

  it('still checks the certificate against the name, and shares no connection', () => {
    const options = pinnedRequestOptions(
      'https://cdn.example.org:8443/fig/a.png?x=1',
      init,
      pinned
    );
    expect(options).toMatchObject({
      protocol: 'https:',
      hostname: 'cdn.example.org',
      servername: 'cdn.example.org',
      port: 8443,
      path: '/fig/a.png?x=1',
      method: 'GET',
      agent: false,
      rejectUnauthorized: true,
    });
    expect(options.headers).toEqual({ accept: 'image/*' });
  });

  it('sends no server name for an address written as an IP', () => {
    const options = pinnedRequestOptions('https://[2606:2800:220:1::1]/a.png', init, {
      address: '2606:2800:220:1::1',
      family: 6,
    });
    expect(options.hostname).toBe('2606:2800:220:1::1');
    expect(options.servername).toBeUndefined();
    expect(options.port).toBe(443);
  });

  it('refuses anything but https', () => {
    expect(() => pinnedRequestOptions('http://cdn.example.org/a.png', init, pinned)).toThrow();
  });
});
