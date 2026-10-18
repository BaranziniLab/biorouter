// @vitest-environment node

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { readFileSync } from 'fs';
import * as fs from 'fs/promises';
import * as os from 'os';
import * as path from 'path';
import {
  describeUpdateDownloadError,
  installResumableUpdaterDownloads,
  type UpdaterDownloadOptions,
} from './resumableUpdaterDownloads';
import type { OpenPart } from './resumableDownload';

const DESK = path.resolve(__dirname, '..', '..');
const updaterSource = (file: string) =>
  readFileSync(path.join(DESK, 'node_modules', 'electron-updater', 'out', file), 'utf8');

let dir: string;
beforeEach(async () => {
  dir = await fs.mkdtemp(path.join(os.tmpdir(), 'br-updater-dl-'));
});
afterEach(async () => {
  await fs.rm(dir, { recursive: true, force: true });
});

/** A minimal CancellationToken: createPromise plus a cancel the test can fire. */
function token() {
  const handlers: Array<() => void> = [];
  return {
    cancel: () => handlers.forEach((h) => h()),
    createPromise<T>(
      callback: (
        resolve: (value: T) => void,
        reject: (error: Error) => void,
        onCancel: (handler: () => void) => void
      ) => void
    ): Promise<T> {
      return new Promise<T>((resolve, reject) =>
        callback(resolve, reject, (h) => handlers.push(h))
      );
    },
  };
}

describe('installResumableUpdaterDownloads', () => {
  it('replaces the executor download and resolves with the destination', async () => {
    const seen: Array<{ url: string; headers: Record<string, string> }> = [];
    const openPart: OpenPart = async (url, headers) => {
      seen.push({ url, headers });
      return {
        status: 200,
        header: (name) => (name === 'content-length' ? '3' : undefined),
        body: (async function* () {
          yield Buffer.from('abc');
        })(),
      };
    };
    const updater = { httpExecutor: { download: () => Promise.reject(new Error('original')) } };
    expect(installResumableUpdaterDownloads(updater, openPart, () => {})).toBe(true);

    const destination = path.join(dir, 'u.zip');
    const options: UpdaterDownloadOptions = {
      headers: { 'Cache-Control': 'no-cache', 'X-Many': ['a', 'b'], Skip: undefined },
      cancellationToken: token(),
    };
    const result = await (
      updater.httpExecutor.download as unknown as (
        u: URL,
        d: string,
        o: UpdaterDownloadOptions
      ) => Promise<string>
    )(new URL('https://github.com/o/r/releases/download/v1/u.zip'), destination, options);

    expect(result).toBe(destination);
    expect(await fs.readFile(destination, 'utf8')).toBe('abc');
    expect(seen[0].url).toBe('https://github.com/o/r/releases/download/v1/u.zip');
    expect(seen[0].headers).toEqual({ 'Cache-Control': 'no-cache', 'X-Many': 'a, b' });
  });

  it('aborts the transfer when electron-updater cancels', async () => {
    let signalSeen: AbortSignal | null = null;
    const openPart: OpenPart = async (_url, _headers, signal) => {
      signalSeen = signal;
      return new Promise(() => {}); // never answers
    };
    const updater = { httpExecutor: { download: () => Promise.resolve('') } };
    installResumableUpdaterDownloads(updater, openPart, () => {});
    const cancellation = token();
    const pending = (
      updater.httpExecutor.download as unknown as (
        u: string,
        d: string,
        o: UpdaterDownloadOptions
      ) => Promise<string>
    )('https://x/u.zip', path.join(dir, 'u.zip'), { cancellationToken: cancellation });
    await new Promise((r) => setTimeout(r, 10));
    cancellation.cancel();
    expect(signalSeen!.aborted).toBe(true);
    void pending.catch(() => {});
  });

  it('changes nothing on an updater without the expected executor', () => {
    expect(
      installResumableUpdaterDownloads(
        {},
        async () => ({}) as never,
        () => {}
      )
    ).toBe(false);
    expect(
      installResumableUpdaterDownloads(
        null,
        async () => ({}) as never,
        () => {}
      )
    ).toBe(false);
  });
});

/**
 * ⚠ The replacement rides on electron-updater internals. These read the
 * INSTALLED electron-updater so an upgrade that moves them fails here, instead
 * of shipping an update download that silently stopped resuming.
 */
describe('the electron-updater internals this relies on', () => {
  it('MacUpdater downloads the zip through httpExecutor.download', () => {
    expect(updaterSource('MacUpdater.js')).toContain(
      'this.httpExecutor.download(zipFileInfo.url, destinationFile, downloadOptions)'
    );
  });

  it('executeDownload passes the digest, the cancellation token and progress', () => {
    const source = updaterSource('AppUpdater.js');
    expect(source).toMatch(
      /cancellationToken: taskOptions\.downloadUpdateOptions\.cancellationToken/
    );
    expect(source).toMatch(/sha512: fileInfo\.info\.sha512/);
    expect(source).toMatch(
      /downloadOptions\.onProgress = it => this\.emit\(types_1\.DOWNLOAD_PROGRESS, it\)/
    );
  });

  it('the transport uses electron-updater’s own session partition', () => {
    const partition = /NET_SESSION_NAME = "([^"]+)"/.exec(
      updaterSource('electronHttpExecutor.js')
    )?.[1];
    const ours = readFileSync(path.join(__dirname, 'electronNetTransport.ts'), 'utf8');
    expect(ours).toContain(`UPDATER_SESSION_PARTITION = '${partition}'`);
  });
});

describe('describeUpdateDownloadError', () => {
  it('keeps the network code and says what to do', () => {
    expect(describeUpdateDownloadError(new Error('net::ERR_CONNECTION_RESET'))).toBe(
      'The download kept failing (net::ERR_CONNECTION_RESET). Check your connection, then try again.'
    );
  });
  it('names a checksum failure', () => {
    expect(
      describeUpdateDownloadError(new Error('sha512 checksum mismatch, expected a, got b'))
    ).toMatch(/^The downloaded update did not match its checksum/);
  });
  it('passes anything else through', () => {
    expect(describeUpdateDownloadError(new Error('Something odd'))).toBe('Something odd');
  });
});
