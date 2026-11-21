import { app, type DownloadItem } from 'electron';
import { execFile } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import log from 'electron-log';

/**
 * Downloads the person starts in the Biorouter Browser (BB-05).
 *
 * The browser used to cancel every download, so a link to a data file, a supplement or anything
 * served as an attachment did nothing at all. A download the person causes now goes through
 * Electron's own save dialog, which opens in the Downloads folder, and its progress shows in the
 * browser bar with **Show in Finder** once it is done. (Downloads the agent causes arrive with the
 * agent tools and follow the act grant; they are not handled here.)
 *
 * ⚠ Electron does not quarantine a download on macOS (measured on 39.8.10: no
 * `com.apple.quarantine` attribute on a completed file), so Gatekeeper would never check an
 * application downloaded here. The attribute is written after completion, the way Safari and
 * Chrome leave it, without the "user approved" flag.
 */

export type BrowserDownloadState = {
  id: string;
  filename: string;
  receivedBytes: number;
  totalBytes: number;
  state: 'progressing' | 'completed' | 'cancelled' | 'interrupted';
};

let testDownloadDir: string | null = null;

/**
 * Tests only: save without a dialog into `dir`. Main passes a folder only when the test hooks are
 * enabled (`!app.isPackaged && BIOROUTER_E2E === '1'`).
 */
export function configureBrowserTestDownloads(dir: string | null): void {
  testDownloadDir = dir;
}

let sequence = 0;
const completedPaths = new Map<string, string>();
const MAX_REMEMBERED = 64;

/** The path a completed download was saved at, for Show in Finder. Never sent to the renderer. */
export function completedDownloadPath(id: string): string | null {
  return completedPaths.get(id) ?? null;
}

export function quarantineValue(nowMs: number): string {
  // flags;seconds since the epoch in hex;agent;event id. 0x0001 marks a download and 0x0080 a
  // download from a web browser; 0x0040 ("user approved") is deliberately absent.
  return `0081;${Math.floor(nowMs / 1000).toString(16)};Biorouter;`;
}

function quarantine(file: string): void {
  if (process.platform !== 'darwin') return;
  execFile(
    '/usr/bin/xattr',
    ['-w', 'com.apple.quarantine', quarantineValue(Date.now()), file],
    // macOS only, but the console-window census reads every spawn site platform-blind.
    { windowsHide: true },
    (error) => {
      if (error) log.warn('[BiorouterBrowser] could not quarantine a download');
    }
  );
}

/**
 * Take over a download the person started. `onUpdate` receives the progress, at most four times
 * a second and once more when it ends.
 */
export function handlePersonDownload(
  item: DownloadItem,
  onUpdate: (state: BrowserDownloadState) => void
): void {
  sequence += 1;
  const id = `download-${sequence}`;
  const filename = path.basename(item.getFilename() || 'download');
  if (testDownloadDir) {
    item.setSavePath(path.join(testDownloadDir, filename));
  } else {
    item.setSaveDialogOptions({
      title: 'Save download',
      defaultPath: path.join(app.getPath('downloads'), filename),
    });
  }

  let lastSent = 0;
  const snapshot = (state: BrowserDownloadState['state']): BrowserDownloadState => ({
    id,
    filename: path.basename(item.getSavePath() || filename),
    receivedBytes: item.getReceivedBytes(),
    totalBytes: item.getTotalBytes(),
    state,
  });
  item.on('updated', (_event, state) => {
    const now = Date.now();
    if (now - lastSent < 250) return;
    lastSent = now;
    onUpdate(snapshot(state === 'interrupted' ? 'interrupted' : 'progressing'));
  });
  item.once('done', (_event, state) => {
    if (state === 'completed') {
      const saved = item.getSavePath();
      if (saved) {
        quarantine(saved);
        completedPaths.set(id, saved);
        if (completedPaths.size > MAX_REMEMBERED) {
          const oldest = completedPaths.keys().next().value;
          if (oldest) completedPaths.delete(oldest);
        }
      }
    }
    onUpdate(snapshot(state));
  });
  onUpdate(snapshot('progressing'));
}

/** The most an agent download may be (BB-05); a larger one is cancelled. */
export const AGENT_DOWNLOAD_MAX_BYTES = 500 * 1024 * 1024;

export type AgentDownloadResult = {
  state: 'completed' | 'cancelled' | 'interrupted' | 'progressing';
  filename: string;
  bytes: number;
  mime: string;
  origin: string;
  path?: string;
  reason?: string;
};

/** `name`, or `name (2).ext` when `name` is taken in `dir`. */
export function uniqueDownloadPath(
  dir: string,
  filename: string,
  exists: (candidate: string) => boolean
): string {
  const safe =
    [...path.basename(filename)]
      .filter((char) => char.charCodeAt(0) >= 0x20)
      .join('')
      .trim() || 'download';
  const extension = path.extname(safe);
  const stem = safe.slice(0, safe.length - extension.length) || 'download';
  let candidate = path.join(dir, safe);
  for (let n = 2; exists(candidate) && n < 1000; n += 1) {
    candidate = path.join(dir, `${stem} (${n})${extension}`);
  }
  return candidate;
}

/**
 * A download the agent caused (BB-05). With the act grant (`dir` set) it is saved without a
 * dialog into the chat's `downloads/` folder, never opened or executed, capped at 500 MB and
 * quarantined like the person's. With only the observe grant (`dir` null) it is cancelled, and
 * the result says the download needs the act grant.
 */
export function handleAgentDownload(
  item: DownloadItem,
  dir: string | null,
  onDone: (result: AgentDownloadResult) => void
): void {
  const filename = path.basename(item.getFilename() || 'download');
  let origin = 'unknown';
  try {
    origin = new URL(item.getURL()).origin;
  } catch {
    origin = 'unknown';
  }
  const base = (): AgentDownloadResult => ({
    state: 'cancelled',
    filename,
    bytes: item.getReceivedBytes(),
    mime: item.getMimeType() || 'application/octet-stream',
    origin,
  });
  if (!dir) {
    item.cancel();
    onDone({
      ...base(),
      reason:
        'downloading needs the permission to act on websites, which this request does not have. Ask the person, or have them download it.',
    });
    return;
  }
  try {
    fs.mkdirSync(dir, { recursive: true });
  } catch {
    item.cancel();
    onDone({ ...base(), reason: `the folder ${dir} could not be created.` });
    return;
  }
  const target = uniqueDownloadPath(dir, filename, (candidate) => fs.existsSync(candidate));
  item.setSavePath(target);
  if (item.getTotalBytes() > AGENT_DOWNLOAD_MAX_BYTES) {
    item.cancel();
    onDone({ ...base(), reason: 'it is larger than 500 MB.' });
    return;
  }
  item.on('updated', () => {
    if (item.getReceivedBytes() > AGENT_DOWNLOAD_MAX_BYTES) item.cancel();
  });
  item.once('done', (_event, state) => {
    if (state === 'completed') {
      quarantine(target);
      onDone({
        ...base(),
        state: 'completed',
        bytes: item.getReceivedBytes(),
        path: target,
        filename: path.basename(target),
      });
      return;
    }
    onDone({
      ...base(),
      state: state === 'interrupted' ? 'interrupted' : 'cancelled',
      reason:
        item.getReceivedBytes() > AGENT_DOWNLOAD_MAX_BYTES
          ? 'it is larger than 500 MB.'
          : `the download was ${state}.`,
    });
  });
}
