/**
 * A download that survives a dropped connection by asking for the rest of the
 * file, rather than starting over or giving up.
 *
 * ⚠ Why this exists: electron-updater downloads the ~250 MB macOS update in ONE
 * request and has no retry. A single `net::ERR_CONNECTION_RESET` anywhere in it
 * ended the update with "Update download failed", and the next attempt was the
 * three-hourly background check, which starts again from byte 0 (measured on
 * 1.93.0 → 1.93.1, 2026-10-18). On a network that drops long transfers that is
 * an update that never arrives.
 *
 * Here a failure mid-body is followed by a `Range: bytes=<received>-` request
 * for the remainder. The attempt budget counts only attempts that received
 * nothing, so a slow, flaky link that keeps making progress keeps going, while a
 * dead one gives up within about a minute. The checksum is computed over the
 * finished FILE, not the streams, so a resumed download is verified exactly like
 * an uninterrupted one.
 *
 * The transport is injected: production passes Electron's `net` (the system
 * proxy, the system certificate store) and tests pass a fake that can fail on
 * cue. Nothing here imports Electron.
 */
import { createHash } from 'crypto';
import { createReadStream } from 'fs';
import * as fs from 'fs/promises';

/** One HTTP response for one part of the file, redirects already followed. */
export interface PartResponse {
  status: number;
  /** Header value by lower-case name. */
  header(name: string): string | undefined;
  /** The body. Throws, or rejects, when the connection fails mid-body. */
  body: AsyncIterable<Uint8Array>;
}

export type OpenPart = (
  url: string,
  headers: Record<string, string>,
  signal: AbortSignal
) => Promise<PartResponse>;

/** The shape electron-updater's `download-progress` event carries. */
export interface DownloadProgress {
  total: number;
  delta: number;
  transferred: number;
  percent: number;
  bytesPerSecond: number;
}

export interface ResumableDownloadOptions {
  url: string;
  destination: string;
  openPart: OpenPart;
  headers?: Record<string, string>;
  /** base64 (or 128-char hex) SHA-512 the finished file must match. */
  sha512?: string | null;
  /** hex SHA-256, used when no SHA-512 is given. */
  sha2?: string | null;
  onProgress?: (progress: DownloadProgress) => void;
  signal?: AbortSignal;
  /** Consecutive attempts that may receive nothing before the download fails. */
  maxAttemptsWithoutProgress?: number;
  /** Wait before attempt n+1; the last value repeats. */
  retryDelaysMs?: readonly number[];
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
  now?: () => number;
  log?: (message: string) => void;
}

export const DEFAULT_RETRY_DELAYS_MS: readonly number[] = [1_000, 3_000, 8_000, 15_000, 30_000];
export const DEFAULT_MAX_ATTEMPTS_WITHOUT_PROGRESS = 6;

/** An HTTP status that ended a part. */
export class DownloadHttpError extends Error {
  constructor(
    readonly status: number,
    url: string
  ) {
    super(`Cannot download "${url}", status ${status}`);
    this.name = 'DownloadHttpError';
  }
}

/** The finished file does not match the digest the release manifest promised. */
export class ChecksumMismatchError extends Error {
  readonly code = 'ERR_CHECKSUM_MISMATCH';
  constructor(algorithm: string, expected: string, actual: string) {
    // The wording matches electron-updater's own DigestTransform error, which
    // the live end-to-end test and anyone reading an old log look for.
    super(`${algorithm} checksum mismatch, expected ${expected}, got ${actual}`);
    this.name = 'ChecksumMismatchError';
  }
}

export class DownloadAbortedError extends Error {
  constructor() {
    super('Download cancelled');
    this.name = 'DownloadAbortedError';
  }
}

/**
 * Statuses a retry cannot fix. 408, 425 and 429 are about timing, and every 5xx
 * is the server's problem, so those are retried.
 */
export function isPermanentHttpStatus(status: number): boolean {
  return status >= 400 && status < 500 && status !== 408 && status !== 425 && status !== 429;
}

/**
 * Network failures that will fail again the same way: a certificate or TLS
 * problem, a blocked or malformed URL, a redirect loop, or the local disk.
 * Everything else (resets, timeouts, a changed network, DNS while waking up) is
 * worth another attempt.
 */
const PERMANENT_FAILURES = [
  /ERR_CERT_/,
  /ERR_SSL_/,
  /ERR_BAD_SSL/,
  /ERR_BLOCKED/,
  /ERR_ACCESS_DENIED/,
  /ERR_INVALID_URL/,
  /ERR_UNSAFE_REDIRECT/,
  /ERR_DISALLOWED_URL_SCHEME/,
  /ERR_UNKNOWN_URL_SCHEME/,
  /ERR_TOO_MANY_REDIRECTS/,
  /Too many redirects/,
  /\bENOSPC\b/,
  /\bEACCES\b/,
  /\bEPERM\b/,
  /\bEROFS\b/,
  /\bEDQUOT\b/,
];

export function isPermanentDownloadError(error: unknown): boolean {
  if (error instanceof DownloadHttpError) return isPermanentHttpStatus(error.status);
  if (error instanceof ChecksumMismatchError || error instanceof DownloadAbortedError) return true;
  const text = errorText(error);
  return PERMANENT_FAILURES.some((pattern) => pattern.test(text));
}

function errorText(error: unknown): string {
  if (error instanceof Error) {
    const code = (error as Error & { code?: unknown }).code;
    return typeof code === 'string' ? `${error.message} ${code}` : error.message;
  }
  return String(error);
}

/** `bytes 100-199/1000` → its numbers. `*` totals and malformed values are null. */
export function parseContentRange(
  value: string | undefined
): { start: number; end: number; total: number | null } | null {
  const match = /^bytes (\d+)-(\d+)\/(\d+|\*)$/i.exec(value?.trim() ?? '');
  if (!match) return null;
  return {
    start: Number(match[1]),
    end: Number(match[2]),
    total: match[3] === '*' ? null : Number(match[3]),
  };
}

function parseLength(value: string | undefined): number | null {
  if (value === undefined || !/^\d+$/.test(value.trim())) return null;
  return Number(value.trim());
}

const defaultSleep = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    if (signal?.aborted) {
      reject(new DownloadAbortedError());
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(new DownloadAbortedError());
    };
    signal?.addEventListener('abort', onAbort, { once: true });
  });

/** The digest electron-updater expects, in its encoding rule. */
function expectedDigest(
  sha512: string | null | undefined,
  sha2: string | null | undefined
): { algorithm: 'sha512' | 'sha256'; encoding: 'base64' | 'hex'; value: string } | null {
  if (sha512) {
    // Same rule as electron-updater's configurePipes: 128 hex characters is hex.
    const hex = sha512.length === 128 && /^[0-9a-f]+$/i.test(sha512);
    return { algorithm: 'sha512', encoding: hex ? 'hex' : 'base64', value: sha512 };
  }
  if (sha2) return { algorithm: 'sha256', encoding: 'hex', value: sha2 };
  return null;
}

async function digestOfFile(
  path: string,
  algorithm: 'sha512' | 'sha256',
  encoding: 'base64' | 'hex'
): Promise<string> {
  const hash = createHash(algorithm);
  for await (const chunk of createReadStream(path)) hash.update(chunk as Buffer);
  return hash.digest(encoding);
}

/**
 * Download `url` to `destination`, resuming after a failed connection. Resolves
 * once the whole file is on disk and matches the digest, if one was given.
 */
export async function downloadWithResume(options: ResumableDownloadOptions): Promise<void> {
  const {
    url,
    destination,
    openPart,
    headers = {},
    onProgress,
    signal,
    maxAttemptsWithoutProgress = DEFAULT_MAX_ATTEMPTS_WITHOUT_PROGRESS,
    retryDelaysMs = DEFAULT_RETRY_DELAYS_MS,
    sleep = defaultSleep,
    now = Date.now,
    log = () => {},
  } = options;

  const startedAt = now();
  let received = 0;
  let total: number | null = null;
  let attemptsWithoutProgress = 0;
  let lastReportAt = startedAt;
  let deltaSinceReport = 0;

  const report = (final: boolean) => {
    if (!onProgress || total === null || total <= 0) return;
    const at = now();
    if (!final && at - lastReportAt < 1000) return;
    const elapsedSeconds = Math.max((at - startedAt) / 1000, 0.001);
    onProgress({
      total,
      delta: deltaSinceReport,
      transferred: received,
      percent: Math.min(100, (received / total) * 100),
      bytesPerSecond: Math.round(received / elapsedSeconds),
    });
    lastReportAt = at;
    deltaSinceReport = 0;
  };

  // Start from an empty file: whatever a caller left at this path is not ours.
  await fs.writeFile(destination, new Uint8Array(0));

  for (;;) {
    if (signal?.aborted) throw new DownloadAbortedError();

    const partHeaders: Record<string, string> = { ...headers };
    if (received > 0) partHeaders.Range = `bytes=${received}-`;

    let receivedThisAttempt = 0;
    let file: fs.FileHandle | null = null;
    try {
      const part = await openPart(url, partHeaders, signal ?? new AbortController().signal);

      if (received > 0 && part.status === 416 && total !== null && received >= total) {
        break; // The server says there is nothing left to send.
      }
      if (part.status < 200 || part.status >= 300) {
        throw new DownloadHttpError(part.status, url);
      }

      if (received > 0) {
        const range = part.status === 206 ? parseContentRange(part.header('content-range')) : null;
        if (range && range.start === received) {
          total = range.total ?? total;
        } else {
          // A 200 ignores the Range, and a range that does not start where we
          // stopped cannot be appended. Either way the body is the file from
          // byte 0, so the bytes already written go.
          log(
            `Server answered the resume with ${part.status}` +
              `${range ? ` from byte ${range.start}` : ''}; starting the file again`
          );
          received = 0;
          await fs.writeFile(destination, new Uint8Array(0));
          total = range?.total ?? parseLength(part.header('content-length'));
        }
      } else {
        total = parseLength(part.header('content-length'));
      }

      file = await fs.open(destination, 'a');
      for await (const chunk of part.body) {
        if (signal?.aborted) throw new DownloadAbortedError();
        await file.write(chunk);
        received += chunk.length;
        receivedThisAttempt += chunk.length;
        deltaSinceReport += chunk.length;
        report(false);
      }
      await file.close();
      file = null;

      if (total !== null && received < total) {
        throw new Error(`net::ERR_CONNECTION_CLOSED after ${received} of ${total} bytes`);
      }
      break;
    } catch (error) {
      await file?.close().catch(() => {});
      if (signal?.aborted) throw new DownloadAbortedError();
      if (isPermanentDownloadError(error)) throw error;

      attemptsWithoutProgress = receivedThisAttempt > 0 ? 1 : attemptsWithoutProgress + 1;
      if (attemptsWithoutProgress >= maxAttemptsWithoutProgress) {
        log(`Download failed ${attemptsWithoutProgress} times without progress; giving up`);
        throw error;
      }
      const delay =
        retryDelaysMs[Math.min(attemptsWithoutProgress - 1, retryDelaysMs.length - 1)] ?? 0;
      log(
        `Download interrupted at ${received}${total !== null ? ` of ${total}` : ''} bytes ` +
          `(${errorText(error)}); resuming in ${delay} ms`
      );
      await sleep(delay, signal);
    }
  }

  if (total === null) total = received;
  report(true);

  const digest = expectedDigest(options.sha512, options.sha2);
  if (digest) {
    const actual = await digestOfFile(destination, digest.algorithm, digest.encoding);
    if (actual !== digest.value) {
      throw new ChecksumMismatchError(digest.algorithm, digest.value, actual);
    }
  }
}
