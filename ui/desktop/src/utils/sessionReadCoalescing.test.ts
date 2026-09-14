import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  coalesceSessionReads,
  isCoalescableSessionRead,
  SESSION_READ_HOLD_MS,
} from './sessionReadCoalescing';

/**
 * The rule under test: a read may share only a request that has NOT yet been
 * handed to the network. Everything else here — the key, the per-caller
 * Response, the abort isolation — exists so that sharing is invisible to each
 * caller apart from the request count.
 */

const DAEMON = 'http://127.0.0.1:4711';
const PROOF = { 'X-Secret-Key': 'secret', 'X-User-Action': 'proof' };

/** A fake daemon whose store the test can change between reads. */
function fakeDaemon() {
  const store = { name: 'before' };
  const sent: Request[] = [];
  const pending: Array<() => void> = [];
  let holdResponses = false;
  const base = vi.fn(async (request: Request) => {
    sent.push(request);
    // The daemon reads its store when the request ARRIVES — which is the moment
    // the renderer hands it to the network.
    const snapshot = { ...store, url: request.url, proof: request.headers.get('X-User-Action') };
    if (holdResponses) await new Promise<void>((resolve) => pending.push(resolve));
    return new Response(JSON.stringify(snapshot), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  });
  return {
    store,
    sent,
    base,
    holdResponses: () => {
      holdResponses = true;
    },
    releaseResponses: () => pending.splice(0).forEach((resolve) => resolve()),
  };
}

const read = (path: string, headers: Record<string, string> = PROOF, init: RequestInit = {}) =>
  new Request(`${DAEMON}${path}`, { method: 'GET', headers, ...init });

const visible = { isHidden: () => false, subscribeHidden: () => {} };

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe('isCoalescableSessionRead', () => {
  it.each([
    ['GET', '/sessions/20260905_8', true],
    ['GET', '/sessions/20260905_8?metadata_only=true', true],
    ['GET', '/sessions/20260905_8/extensions', true],
    ['GET', '/sessions/20260905_8/events', false],
    ['GET', '/sessions/20260905_8/usage', true],
    ['GET', '/sessions/20260905_8/usage/extra', false],
    ['GET', '/sessions/changes?since=0&ids=20260905_8', false],
    ['GET', '/sessions/sidebar?limit=10', false],
    ['GET', '/sessions/running', false],
    ['GET', '/sessions/activity', false],
    ['GET', '/sessions/insights', false],
    ['GET', '/sessions', false],
    ['DELETE', '/sessions/20260905_8', false],
    ['PUT', '/sessions/20260905_8/name', false],
    ['POST', '/agent/resume', false],
  ])('%s %s -> %s', (method, path, expected) => {
    expect(isCoalescableSessionRead(new Request(`${DAEMON}${path}`, { method }))).toBe(expected);
  });
});

describe('coalesceSessionReads', () => {
  it('sends identical reads issued together as ONE request, and each caller gets its own body', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const answers = Array.from({ length: 10 }, () => fetch(read('/sessions/s1')));
    expect(daemon.base).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);

    const responses = await Promise.all(answers);
    expect(daemon.base).toHaveBeenCalledTimes(1);
    // Ten independent bodies: each one can be consumed, and consuming one does
    // not consume another.
    const bodies = await Promise.all(responses.map((r) => r.json()));
    expect(bodies.every((b) => b.name === 'before')).toBe(true);
    expect(responses.every((r) => r.status === 200)).toBe(true);
  });

  it('never lets a read issued after the request left join it — it asks again and sees the change', async () => {
    const daemon = fakeDaemon();
    daemon.holdResponses();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const before = fetch(read('/sessions/s1'));
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    expect(daemon.base).toHaveBeenCalledTimes(1);

    // The first request is IN FLIGHT (its response is held). Something changes
    // the chat — a rename, a turn, a declassification — and the caller that
    // waited for that change reads.
    daemon.store.name = 'after';
    const after = fetch(read('/sessions/s1'));
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);

    expect(daemon.base).toHaveBeenCalledTimes(2);
    daemon.releaseResponses();
    expect((await (await before).json()).name).toBe('before');
    expect((await (await after).json()).name).toBe('after');
  });

  it('keeps nothing once a response lands', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const first = fetch(read('/sessions/s1'));
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    expect((await (await first).json()).name).toBe('before');

    daemon.store.name = 'after';
    const second = fetch(read('/sessions/s1'));
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    expect(daemon.base).toHaveBeenCalledTimes(2);
    expect((await (await second).json()).name).toBe('after');
  });

  it('never shares a read carrying the proof with one that does not, in either direction', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const proven = fetch(read('/sessions/s1', PROOF));
    const unproven = fetch(read('/sessions/s1', { 'X-Secret-Key': 'secret' }));
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);

    expect(daemon.base).toHaveBeenCalledTimes(2);
    // Each request that reached the daemon still carries exactly what its caller
    // put on it.
    expect(daemon.sent.map((r) => r.headers.get('X-User-Action')).sort()).toEqual(
      [null, 'proof'].sort()
    );
    expect((await (await proven).json()).proof).toBe('proof');
    expect((await (await unproven).json()).proof).toBeNull();
  });

  it('keeps the proof on a shared request', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const answers = [fetch(read('/sessions/s1')), fetch(read('/sessions/s1'))];
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    await Promise.all(answers);

    expect(daemon.base).toHaveBeenCalledTimes(1);
    expect(daemon.sent[0].headers.get('X-User-Action')).toBe('proof');
    expect(daemon.sent[0].headers.get('X-Secret-Key')).toBe('secret');
  });

  it('does not share a metadata read with a full read, or one chat with another', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const answers = [
      fetch(read('/sessions/s1')),
      fetch(read('/sessions/s1?metadata_only=true')),
      fetch(read('/sessions/s2?metadata_only=true')),
      fetch(read('/sessions/s1/extensions')),
      fetch(read('/sessions/s1?metadata_only=true')),
    ];
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    await Promise.all(answers);

    expect(daemon.sent.map((r) => r.url.replace(DAEMON, '')).sort()).toEqual(
      [
        '/sessions/s1',
        '/sessions/s1/extensions',
        '/sessions/s1?metadata_only=true',
        '/sessions/s2?metadata_only=true',
      ].sort()
    );
  });

  it('passes every other request straight through, with no hold', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const paths = [
      '/sessions/s1/events',
      '/sessions/changes?since=0',
      '/sessions/sidebar?limit=10',
      '/sessions/s1/events',
    ];
    const answers = paths.map((p) => fetch(read(p)));
    // No timer advanced: a pass-through request is sent synchronously.
    expect(daemon.base).toHaveBeenCalledTimes(paths.length);
    const rename = fetch(
      new Request(`${DAEMON}/sessions/s1/name`, { method: 'PUT', headers: PROOF, body: '{}' })
    );
    expect(daemon.base).toHaveBeenCalledTimes(paths.length + 1);
    await Promise.all([...answers, rename]);
  });

  it('hands a lone read the network response itself', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const answer = fetch(read('/sessions/s1'));
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    const response = await answer;
    expect(response).toBe(await daemon.base.mock.results[0].value);
  });

  it('rejects every joined caller when the request fails', async () => {
    const base = vi.fn(async () => {
      throw new TypeError('Failed to fetch');
    });
    const fetch = coalesceSessionReads(base, visible);

    const answers = [fetch(read('/sessions/s1')), fetch(read('/sessions/s1'))];
    const settled = Promise.allSettled(answers);
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    const results = await settled;
    expect(results.map((r) => r.status)).toEqual(['rejected', 'rejected']);
    expect(base).toHaveBeenCalledTimes(1);
  });

  it('gives every caller the error status and body of a refused read', async () => {
    const base = vi.fn(async () => new Response('refused: this chat is private', { status: 403 }));
    const fetch = coalesceSessionReads(base, visible);

    const answers = [fetch(read('/sessions/s1')), fetch(read('/sessions/s1'))];
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    const responses = await Promise.all(answers);
    expect(responses.map((r) => r.status)).toEqual([403, 403]);
    expect(await Promise.all(responses.map((r) => r.text()))).toEqual([
      'refused: this chat is private',
      'refused: this chat is private',
    ]);
  });

  it('shares a null-body status without constructing an invalid Response', async () => {
    const base = vi.fn(async () => new Response(null, { status: 204 }));
    const fetch = coalesceSessionReads(base, visible);

    const answers = [fetch(read('/sessions/s1')), fetch(read('/sessions/s1'))];
    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    expect((await Promise.all(answers)).map((r) => r.status)).toEqual([204, 204]);
  });

  it('lets one caller abort without cancelling the request the others wait on', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const controller = new AbortController();
    const aborted = fetch(read('/sessions/s1', PROOF, { signal: controller.signal }));
    const kept = fetch(read('/sessions/s1'));
    const abortedSettled = aborted.then(
      () => 'resolved',
      () => 'rejected'
    );

    controller.abort();
    expect(await abortedSettled).toBe('rejected');

    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS);
    expect(daemon.base).toHaveBeenCalledTimes(1);
    expect(daemon.sent[0].signal.aborted).toBe(false);
    expect((await (await kept).json()).name).toBe('before');
  });

  it('sends nothing for a batch every caller abandoned', async () => {
    const daemon = fakeDaemon();
    const fetch = coalesceSessionReads(daemon.base, visible);

    const controller = new AbortController();
    const answer = fetch(read('/sessions/s1', PROOF, { signal: controller.signal })).catch(
      () => 'rejected'
    );
    controller.abort();
    expect(await answer).toBe('rejected');

    await vi.advanceTimersByTimeAsync(SESSION_READ_HOLD_MS * 4);
    expect(daemon.base).not.toHaveBeenCalled();
  });

  it('does not hold reads while the page is hidden, and sends an open batch when it hides', async () => {
    const daemon = fakeDaemon();
    let hidden = false;
    let onHidden: () => void = () => {};
    const fetch = coalesceSessionReads(daemon.base, {
      isHidden: () => hidden,
      subscribeHidden: (callback) => {
        onHidden = callback;
      },
    });

    const held = [fetch(read('/sessions/s1')), fetch(read('/sessions/s1'))];
    expect(daemon.base).not.toHaveBeenCalled();
    hidden = true;
    onHidden();
    // Sent at once, still as one request, without waiting for a throttled timer.
    expect(daemon.base).toHaveBeenCalledTimes(1);
    await Promise.all(held);

    const passThrough = fetch(read('/sessions/s1'));
    expect(daemon.base).toHaveBeenCalledTimes(2);
    await passThrough;
  });
});
