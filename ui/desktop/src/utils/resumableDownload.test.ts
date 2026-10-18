// @vitest-environment node

/**
 * The macOS update failed outright on one `net::ERR_CONNECTION_RESET`
 * (1.93.0 → 1.93.1, 2026-10-18). These pin what replaced that: a dropped
 * connection asks for the rest of the file, a server that will not resume
 * starts the file over, a dead link gives up instead of looping, and the
 * finished file is checked against the manifest's digest whichever way it
 * arrived.
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { createHash } from 'crypto';
import * as fs from 'fs/promises';
import * as os from 'os';
import * as path from 'path';
import {
  ChecksumMismatchError,
  DownloadAbortedError,
  DownloadHttpError,
  downloadWithResume,
  isPermanentDownloadError,
  parseContentRange,
  type DownloadProgress,
  type OpenPart,
  type PartResponse,
} from './resumableDownload';

const payload = Buffer.from(Array.from({ length: 10_000 }, (_, i) => i % 251));
const sha512 = createHash('sha512').update(payload).digest('base64');

let dir: string;
let destination: string;

beforeEach(async () => {
  dir = await fs.mkdtemp(path.join(os.tmpdir(), 'br-resume-'));
  destination = path.join(dir, 'update.zip');
});

afterEach(async () => {
  await fs.rm(dir, { recursive: true, force: true });
});

interface Step {
  status?: number;
  /** Bytes of the payload this response sends, from where the request starts. */
  bytes?: number;
  /** Fail after sending `bytes`. */
  failWith?: Error;
  /** Ignore the Range header and send from byte 0 with a 200. */
  ignoreRange?: boolean;
  /** Send a Content-Range that starts here instead of where we asked. */
  rangeStart?: number;
  /** Reject the open itself. */
  openError?: Error;
}

/** A fake server that plays one step per request and records what it was asked. */
function server(steps: Step[]) {
  const requests: Array<Record<string, string>> = [];
  const openPart: OpenPart = async (_url, headers) => {
    requests.push(headers);
    const step = steps.shift() ?? {};
    if (step.openError) throw step.openError;
    const asked = /bytes=(\d+)-/.exec(headers.Range ?? '');
    const resuming = asked !== null && !step.ignoreRange;
    const start = resuming ? (step.rangeStart ?? Number(asked![1])) : 0;
    const status = step.status ?? (resuming ? 206 : 200);
    const end = Math.min(payload.length, start + (step.bytes ?? payload.length));
    const headerMap: Record<string, string> =
      status === 206
        ? {
            'content-range': `bytes ${start}-${payload.length - 1}/${payload.length}`,
            'content-length': String(payload.length - start),
          }
        : { 'content-length': String(payload.length) };
    const response: PartResponse = {
      status,
      header: (name) => headerMap[name],
      body: (async function* () {
        for (let at = start; at < end; at += 1000) {
          yield payload.subarray(at, Math.min(at + 1000, end));
        }
        if (step.failWith) throw step.failWith;
      })(),
    };
    return response;
  };
  return { openPart, requests };
}

const reset = () => new Error('net::ERR_CONNECTION_RESET');
const noSleep = async () => {};

describe('downloadWithResume', () => {
  it('downloads in one request when nothing goes wrong', async () => {
    const { openPart, requests } = server([{}]);
    await downloadWithResume({ url: 'https://x/u.zip', destination, openPart, sha512 });
    expect(await fs.readFile(destination)).toEqual(payload);
    expect(requests).toHaveLength(1);
    expect(requests[0].Range).toBeUndefined();
  });

  it('asks for the rest of the file after a reset and appends it', async () => {
    const { openPart, requests } = server([{ bytes: 4000, failWith: reset() }, {}]);
    const progress: DownloadProgress[] = [];
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512,
      sleep: noSleep,
      onProgress: (p) => progress.push(p),
    });
    expect(await fs.readFile(destination)).toEqual(payload);
    expect(requests.map((r) => r.Range)).toEqual([undefined, 'bytes=4000-']);
    const final = progress[progress.length - 1];
    expect(final).toMatchObject({
      transferred: payload.length,
      total: payload.length,
      percent: 100,
    });
  });

  it('keeps the caller’s headers on every request', async () => {
    const { openPart, requests } = server([{ bytes: 2000, failWith: reset() }, {}]);
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      headers: { 'Cache-Control': 'no-cache' },
      sleep: noSleep,
    });
    expect(requests.every((r) => r['Cache-Control'] === 'no-cache')).toBe(true);
  });

  it('starts the file over when the server ignores the Range', async () => {
    const { openPart, requests } = server([
      { bytes: 3000, failWith: reset() },
      { ignoreRange: true },
    ]);
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512,
      sleep: noSleep,
    });
    expect(await fs.readFile(destination)).toEqual(payload);
    expect(requests[1].Range).toBe('bytes=3000-');
  });

  it('starts the file over when the resumed range does not begin where it stopped', async () => {
    const { openPart } = server([
      { bytes: 3000, failWith: reset() },
      { rangeStart: 2000, bytes: 0, failWith: reset() },
      {},
    ]);
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512,
      sleep: noSleep,
    });
    expect(await fs.readFile(destination)).toEqual(payload);
  });

  it('treats a body that ends short of Content-Length as a dropped connection', async () => {
    const { openPart, requests } = server([{ bytes: 6000 }, {}]);
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512,
      sleep: noSleep,
    });
    expect(await fs.readFile(destination)).toEqual(payload);
    expect(requests[1].Range).toBe('bytes=6000-');
  });

  it('finishes when the server says the remaining range is empty', async () => {
    const { openPart } = server([{ failWith: reset() }, { status: 416, bytes: 0 }]);
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512,
      sleep: noSleep,
    });
    expect(await fs.readFile(destination)).toEqual(payload);
  });

  it('gives up after repeated attempts that receive nothing, with the last error', async () => {
    const steps: Step[] = Array.from({ length: 10 }, () => ({ openError: reset() }));
    const { openPart, requests } = server(steps);
    const waits: number[] = [];
    await expect(
      downloadWithResume({
        url: 'https://x/u.zip',
        destination,
        openPart,
        maxAttemptsWithoutProgress: 4,
        retryDelaysMs: [10, 20, 30],
        sleep: async (ms) => {
          waits.push(ms);
        },
      })
    ).rejects.toThrow('net::ERR_CONNECTION_RESET');
    expect(requests).toHaveLength(4);
    expect(waits).toEqual([10, 20, 30]);
  });

  it('keeps going on a flaky link as long as each attempt makes progress', async () => {
    // Nine drops, more than the attempt budget, but every one moved the file on.
    const steps: Step[] = Array.from({ length: 9 }, () => ({ bytes: 1000, failWith: reset() }));
    const { openPart, requests } = server([...steps, {}]);
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512,
      maxAttemptsWithoutProgress: 3,
      sleep: noSleep,
    });
    expect(await fs.readFile(destination)).toEqual(payload);
    expect(requests).toHaveLength(10);
  });

  it('does not retry a status a retry cannot fix', async () => {
    const { openPart, requests } = server([{ status: 404, bytes: 0 }, {}]);
    await expect(
      downloadWithResume({ url: 'https://x/u.zip', destination, openPart, sleep: noSleep })
    ).rejects.toBeInstanceOf(DownloadHttpError);
    expect(requests).toHaveLength(1);
  });

  it('retries a server error', async () => {
    const { openPart } = server([{ status: 503, bytes: 0 }, {}]);
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512,
      sleep: noSleep,
    });
    expect(await fs.readFile(destination)).toEqual(payload);
  });

  it('does not retry a certificate failure', async () => {
    const { openPart, requests } = server([
      { openError: new Error('net::ERR_CERT_AUTHORITY_INVALID') },
      {},
    ]);
    await expect(
      downloadWithResume({ url: 'https://x/u.zip', destination, openPart, sleep: noSleep })
    ).rejects.toThrow('ERR_CERT_AUTHORITY_INVALID');
    expect(requests).toHaveLength(1);
  });

  it('rejects a file that does not match the digest, after a resume too', async () => {
    const wrong = createHash('sha512').update('something else').digest('base64');
    const { openPart } = server([{ bytes: 5000, failWith: reset() }, {}]);
    const failure = downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sha512: wrong,
      sleep: noSleep,
    });
    await expect(failure).rejects.toBeInstanceOf(ChecksumMismatchError);
    await expect(failure).rejects.toThrow(/^sha512 checksum mismatch, expected /);
  });

  it('accepts a hex SHA-512 the way electron-updater does', async () => {
    const hex = createHash('sha512').update(payload).digest('hex');
    const { openPart } = server([{}]);
    await downloadWithResume({ url: 'https://x/u.zip', destination, openPart, sha512: hex });
  });

  it('stops at once when cancelled, without another attempt', async () => {
    const controller = new AbortController();
    const { openPart, requests } = server([{ bytes: 2000, failWith: reset() }, {}]);
    const failure = downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      signal: controller.signal,
      sleep: async () => {
        controller.abort();
        throw new DownloadAbortedError();
      },
    });
    await expect(failure).rejects.toBeInstanceOf(DownloadAbortedError);
    expect(requests).toHaveLength(1);
  });

  it('reports progress that never goes backwards', async () => {
    let clock = 0;
    const { openPart } = server([{ bytes: 4000, failWith: reset() }, {}]);
    const seen: number[] = [];
    await downloadWithResume({
      url: 'https://x/u.zip',
      destination,
      openPart,
      sleep: noSleep,
      now: () => (clock += 600),
      onProgress: (p) => seen.push(p.transferred),
    });
    expect(seen.length).toBeGreaterThan(1);
    expect([...seen].sort((a, b) => a - b)).toEqual(seen);
    expect(seen[seen.length - 1]).toBe(payload.length);
  });
});

describe('isPermanentDownloadError', () => {
  it.each([
    ['net::ERR_CONNECTION_RESET', false],
    ['net::ERR_NETWORK_CHANGED', false],
    ['net::ERR_TIMED_OUT (no data for 60 s)', false],
    ['net::ERR_NAME_NOT_RESOLVED', false],
    ['net::ERR_CERT_DATE_INVALID', true],
    ['net::ERR_SSL_PROTOCOL_ERROR', true],
    ['Too many redirects (> 10)', true],
    ['ENOSPC: no space left on device, write', true],
  ])('%s → %s', (message, permanent) => {
    expect(isPermanentDownloadError(new Error(message))).toBe(permanent);
  });

  it('judges HTTP statuses by whether waiting could help', () => {
    expect(isPermanentDownloadError(new DownloadHttpError(403, 'u'))).toBe(true);
    expect(isPermanentDownloadError(new DownloadHttpError(429, 'u'))).toBe(false);
    expect(isPermanentDownloadError(new DownloadHttpError(500, 'u'))).toBe(false);
  });
});

describe('parseContentRange', () => {
  it('reads a complete range', () => {
    expect(parseContentRange('bytes 100-199/1000')).toEqual({ start: 100, end: 199, total: 1000 });
  });
  it('reads an unknown total', () => {
    expect(parseContentRange('bytes 0-9/*')).toEqual({ start: 0, end: 9, total: null });
  });
  it('rejects anything else', () => {
    expect(parseContentRange(undefined)).toBeNull();
    expect(parseContentRange('items 1-2/3')).toBeNull();
  });
});
