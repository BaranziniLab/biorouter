/**
 * Puts `downloadWithResume` under electron-updater's update download.
 *
 * electron-updater fetches the macOS update zip with `httpExecutor.download`,
 * which makes one request and fails the update on the first dropped
 * connection. This replaces that one method on the updater's own executor, so
 * the rest of electron-updater (the feed check, the cache, the sha512 it
 * passes, the Squirrel.Mac hand-off) is untouched.
 *
 * ⚠ `httpExecutor` is not public API. The replacement is installed only when
 * the executor has the expected shape, and `resumableUpdaterDownloads.test.ts`
 * reads the installed electron-updater to check that MacUpdater still
 * downloads through `this.httpExecutor.download(url, destination, options)`.
 * If a future electron-updater changes that, the test fails rather than the
 * update quietly losing its resume.
 */
import { downloadWithResume, type DownloadProgress, type OpenPart } from './resumableDownload';

/** builder-util-runtime's CancellationToken, as much of it as this needs. */
interface CancellationTokenLike {
  createPromise<T>(
    callback: (
      resolve: (value: T) => void,
      reject: (error: Error) => void,
      onCancel: (handler: () => void) => void
    ) => void
  ): Promise<T>;
}

/** The options AppUpdater.executeDownload passes to `httpExecutor.download`. */
export interface UpdaterDownloadOptions {
  headers?: Record<string, unknown> | null;
  cancellationToken?: CancellationTokenLike;
  sha512?: string | null;
  sha2?: string | null;
  onProgress?: (progress: DownloadProgress) => void;
}

type ExecutorDownload = (
  url: URL | string,
  destination: string,
  options: UpdaterDownloadOptions
) => Promise<string>;

function stringHeaders(
  headers: Record<string, unknown> | null | undefined
): Record<string, string> {
  const result: Record<string, string> = {};
  for (const [name, value] of Object.entries(headers ?? {})) {
    if (value === undefined || value === null) continue;
    result[name] = Array.isArray(value) ? value.join(', ') : String(value);
  }
  return result;
}

/**
 * Make `updater`'s update downloads resumable. Returns false, and changes
 * nothing, when the updater does not look the way this expects.
 */
export function installResumableUpdaterDownloads(
  updater: unknown,
  openPart: OpenPart,
  log: (message: string) => void
): boolean {
  const executor = (updater as { httpExecutor?: { download?: unknown } } | null)?.httpExecutor;
  if (!executor || typeof executor.download !== 'function') {
    log('electron-updater has no httpExecutor.download; update downloads will not resume');
    return false;
  }

  const download: ExecutorDownload = (url, destination, options) => {
    const run = (signal: AbortSignal) =>
      downloadWithResume({
        url: typeof url === 'string' ? url : url.href,
        destination,
        openPart,
        headers: stringHeaders(options?.headers),
        sha512: options?.sha512,
        sha2: options?.sha2,
        onProgress: options?.onProgress,
        signal,
        log,
      }).then(() => destination);

    const token = options?.cancellationToken;
    if (!token) return run(new AbortController().signal);
    return token.createPromise<string>((resolve, reject, onCancel) => {
      const controller = new AbortController();
      onCancel(() => controller.abort());
      run(controller.signal).then(resolve, reject);
    });
  };
  executor.download = download;
  return true;
}

/**
 * What the update dialog says when a download fails. The raw error stays in
 * it, because "net::ERR_CONNECTION_RESET" is what someone searching for help,
 * or reading a bug report, needs.
 */
export function describeUpdateDownloadError(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  if (/checksum mismatch/i.test(raw)) {
    return `The downloaded update did not match its checksum, so it was not installed. Try again. (${raw})`;
  }
  if (/net::ERR_|ECONNRESET|ETIMEDOUT|EPIPE|ENOTFOUND|EAI_AGAIN|socket hang up/i.test(raw)) {
    return `The download kept failing (${raw}). Check your connection, then try again.`;
  }
  return raw;
}
