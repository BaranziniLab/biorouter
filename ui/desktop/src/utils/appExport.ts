/**
 * Applications ▸ Export, in the main process (`apps:export`).
 *
 * The order is the point of this module:
 *
 * 1. The folder is picked FIRST. Cancelling sends nothing: the old flow built the export (and
 *    could rebuild and re-stamp the app's bundle) before it asked where to put it.
 * 2. A folder already at `<picked>/<app id>` with anything in it is replaced only on a yes,
 *    asked before the daemon is.
 * 3. The window's daemon builds the export (`POST /apps/{id}/export`) with the secret and the
 *    proof of a person, so a private knowledge base can travel when the person asked for it,
 *    and answers a zip. Harness errors come back as `lint`, with a token that lets the
 *    renderer's "Export anyway" reuse this folder.
 * 4. The zip is unpacked beside the target, entry by entry, refusing a path that climbs out
 *    (`safeZip`), skipping a link, and giving each file the mode the daemon recorded, so
 *    `run.command` is double-clickable. Then it is swapped into place, and a folder it replaced
 *    is moved to the Trash rather than deleted.
 *
 * A daemon older than `POST` answers it 405; such a daemon still answers the legacy `GET` with
 * text files only, which are written under the same rules (launchers executable), with a note
 * that a full export needs an updated background service.
 */
import AdmZip from 'adm-zip';
import crypto from 'node:crypto';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { safeZipEntryTarget } from './safeZip';
import {
  APP_ID_PATTERN,
  EXPORT_REPORT_HEADER,
  decodeExportReport,
  exportRequestBody,
  lintRefusal,
  parseExportRequest,
  refusalText,
  type AppExportRequest,
  type AppExportResult,
  type AppExportSummary,
} from './appExportRequest';

/**
 * Files the launcher runs, which must be executable whatever the zip recorded: a daemon on
 * Windows has no file modes to record. Mirrors `render::EXECUTABLE_EXPORT_FILES` plus a bundled
 * daemon (`export::is_executable_export_path`).
 */
export const EXECUTABLE_EXPORT_PATHS: ReadonlySet<string> = new Set([
  'run.command',
  'run.sh',
  'biorouter-launch.sh',
  'payload/bin/biorouterd',
]);

/** More entries than any export has; a zip past it is refused rather than half-written. */
const MAX_ENTRIES = 60_000;
/** How long a lint answer's folder is remembered for "Export anyway". */
const RETRY_TTL_MS = 10 * 60 * 1000;

export interface AppExportDeps {
  /** The window's daemon, as its client talks to it (the proxy, for a shared daemon). */
  baseUrl: string;
  secretKey: string;
  /** The proof of a person; the proxy translates it for the daemon. */
  userActionKey?: string;
  /** The native folder picker. `undefined` is a cancel. */
  chooseDirectory: () => Promise<string | undefined>;
  /** Ask before replacing a folder with something in it. */
  confirmReplace: (target: string) => Promise<boolean>;
  /** Move a replaced folder to the Trash (`shell.trashItem`). */
  trash: (target: string) => Promise<void>;
  fetch?: typeof fetch;
  now?: () => number;
}

interface PendingRetry {
  appId: string;
  directory: string;
  replaceConfirmed: boolean;
  expires: number;
}

const pendingRetries = new Map<string, PendingRetry>();

function rememberRetry(entry: Omit<PendingRetry, 'expires'>, now: number): string {
  for (const [token, pending] of pendingRetries) {
    if (pending.expires <= now) pendingRetries.delete(token);
  }
  const token = crypto.randomBytes(16).toString('hex');
  pendingRetries.set(token, { ...entry, expires: now + RETRY_TTL_MS });
  return token;
}

/** A remembered lint answer for this app, once: a token is spent by the export it unlocks. */
function takeRetry(token: string, appId: string, now: number): PendingRetry | undefined {
  const pending = pendingRetries.get(token);
  pendingRetries.delete(token);
  if (!pending || pending.appId !== appId || pending.expires <= now) return undefined;
  return pending;
}

/** Test seam: forget every remembered lint answer. */
export function resetExportRetriesForTests(): void {
  pendingRetries.clear();
}

type TargetState = 'absent' | 'empty' | 'occupied';

/** What is at `target`, or the sentence saying why nothing can be written there. */
async function inspectTarget(target: string): Promise<TargetState | { refusal: string }> {
  let stat: fs.Stats;
  try {
    stat = await fsp.lstat(target);
  } catch (error) {
    if ((error as { code?: string }).code === 'ENOENT') return 'absent';
    return { refusal: `Could not look at ${target}: ${(error as Error).message}` };
  }
  if (stat.isSymbolicLink()) {
    return { refusal: `${target} is a link. Pick another folder.` };
  }
  if (!stat.isDirectory()) {
    return { refusal: `A file named ${path.basename(target)} is already in that folder.` };
  }
  return (await fsp.readdir(target)).length === 0 ? 'empty' : 'occupied';
}

/**
 * Unpack the daemon's zip into `staging`. Every entry must sit under `<app id>/`; a path that
 * would climb out is refused (the whole export, not the entry), a link is skipped, and each file
 * gets the mode the daemon recorded, with the launchers executable whatever it recorded.
 *
 * Returns the number of files written and the links it skipped.
 */
export function extractExportZip(
  zip: Buffer,
  appId: string,
  staging: string
): { files: number; skipped: string[] } {
  const archive = new AdmZip(zip);
  const entries = archive.getEntries();
  if (entries.length > MAX_ENTRIES) {
    throw new Error(`The export holds more than ${MAX_ENTRIES} files.`);
  }
  const prefix = `${appId}/`;
  let files = 0;
  const skipped: string[] = [];
  for (const entry of entries) {
    const name = entry.entryName.replace(/\\/g, '/');
    if (!name.startsWith(prefix)) {
      throw new Error(`The export holds an entry outside its folder (${entry.entryName}).`);
    }
    const rel = name.slice(prefix.length);
    if (!rel || rel === '/') continue;
    // Throws on an absolute path or a `..` segment: the zip-slip check every unpacker here uses.
    const destination = safeZipEntryTarget(staging, rel);
    const unixMode = (entry.attr >>> 16) & 0xffff;
    const kind = unixMode & 0o170000;
    if (kind === 0o120000) {
      skipped.push(`${rel}: a link, never written`);
      continue;
    }
    if (entry.isDirectory || kind === 0o040000) {
      fs.mkdirSync(destination, { recursive: true });
      continue;
    }
    const cleanRel = rel.replace(/\/+$/, '');
    const recorded = unixMode & 0o777;
    const mode = EXECUTABLE_EXPORT_PATHS.has(cleanRel) ? 0o755 : recorded || 0o644;
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    // `wx`: a second entry for one path is a broken zip, never an overwrite.
    fs.writeFileSync(destination, entry.getData(), { mode, flag: 'wx' });
    // The process umask narrowed `mode`; the recorded one is what the launcher needs.
    fs.chmodSync(destination, mode);
    files += 1;
  }
  return { files, skipped };
}

/**
 * Write the legacy `GET`'s text map into `staging`, under the same path checks as a zip, the
 * launchers executable.
 */
export function writeLegacyFiles(files: Record<string, unknown>, staging: string): number {
  let written = 0;
  for (const [rel, content] of Object.entries(files)) {
    if (typeof content !== 'string') continue;
    const destination = safeZipEntryTarget(staging, rel);
    const mode = EXECUTABLE_EXPORT_PATHS.has(rel.replace(/\\/g, '/')) ? 0o755 : 0o644;
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.writeFileSync(destination, content, { encoding: 'utf8', mode, flag: 'wx' });
    fs.chmodSync(destination, mode);
    written += 1;
  }
  return written;
}

/**
 * Build the export in a hidden folder beside the target and swap it in. A folder already at the
 * target is replaced only when `replaceConfirmed`, and is moved to the Trash afterwards; one
 * that appeared while the export was being made is left alone and the export is refused.
 */
async function placeFolder(
  directory: string,
  appId: string,
  replaceConfirmed: boolean,
  fill: (staging: string) => Promise<string[]> | string[],
  trash: (target: string) => Promise<void>
): Promise<string[]> {
  const nonce = crypto.randomBytes(6).toString('hex');
  const target = path.join(directory, appId);
  const staging = path.join(directory, `.${appId}.partial-${nonce}`);
  await fsp.mkdir(staging);
  let notes: string[];
  try {
    notes = await fill(staging);
  } catch (error) {
    await fsp.rm(staging, { recursive: true, force: true });
    throw error;
  }
  const state = await inspectTarget(target);
  try {
    if (typeof state === 'object') throw new Error(state.refusal);
    if (state === 'occupied' && !replaceConfirmed) {
      throw new Error(
        `A folder appeared at ${target} while the export was made; it was left alone.`
      );
    }
    if (state === 'absent') {
      await fsp.rename(staging, target);
      return notes;
    }
    if (state === 'empty') {
      await fsp.rmdir(target);
      await fsp.rename(staging, target);
      return notes;
    }
  } catch (error) {
    await fsp.rm(staging, { recursive: true, force: true });
    throw error;
  }
  const aside = path.join(directory, `.${appId}.replaced-${nonce}`);
  try {
    await fsp.rename(target, aside);
  } catch (error) {
    await fsp.rm(staging, { recursive: true, force: true });
    throw error;
  }
  try {
    await fsp.rename(staging, target);
  } catch (error) {
    await fsp.rename(aside, target).catch(() => undefined);
    await fsp.rm(staging, { recursive: true, force: true });
    throw error;
  }
  try {
    await trash(aside);
  } catch {
    notes.push(`The folder it replaced could not be moved to the Trash; it is at ${aside}.`);
  }
  return notes;
}

const failed = (message: string): AppExportResult => ({ status: 'failed', message });

/**
 * Export app `appId` into a folder the person picks, as `options` asks (`parseExportRequest`).
 * Never throws: every outcome is an `AppExportResult` the renderer can show.
 */
export async function exportAppToFolder(
  appId: unknown,
  options: unknown,
  deps: AppExportDeps
): Promise<AppExportResult> {
  if (typeof appId !== 'string' || !APP_ID_PATTERN.test(appId)) {
    return failed('That is not an app name.');
  }
  const request = parseExportRequest(options);
  if (typeof request === 'string') return failed(request);
  const now = deps.now ?? Date.now;
  const fetchImpl = deps.fetch ?? fetch;

  const retry = request.retry ? takeRetry(request.retry, appId, now()) : undefined;
  const directory = retry?.directory ?? (await deps.chooseDirectory());
  if (!directory) return { status: 'canceled' };
  const target = path.join(directory, appId);

  const state = await inspectTarget(target);
  if (typeof state === 'object') return failed(state.refusal);
  let replaceConfirmed = retry?.replaceConfirmed ?? false;
  if (state === 'occupied' && !replaceConfirmed) {
    if (!(await deps.confirmReplace(target))) return { status: 'canceled' };
    replaceConfirmed = true;
  }

  const base = `${deps.baseUrl.replace(/\/+$/, '')}/apps/${encodeURIComponent(appId)}/export`;
  const headers: Record<string, string> = { 'X-Secret-Key': deps.secretKey };
  if (deps.userActionKey) headers['X-User-Action'] = deps.userActionKey;

  let response: Response;
  try {
    response = await fetchImpl(base, {
      method: 'POST',
      headers: { ...headers, 'Content-Type': 'application/json' },
      body: exportRequestBody(request),
    });
  } catch (error) {
    return failed(`Could not reach the background service: ${(error as Error).message}`);
  }

  if (response.status === 422) {
    const { message, findings } = await lintRefusal(response);
    const token = rememberRetry({ appId, directory, replaceConfirmed }, now());
    return { status: 'lint', message, findings, retry: token };
  }

  // A daemon from before `POST /apps/{id}/export` answers 405 (the path exists for GET only). A
  // current one answers 404 for an app it does not have, and its GET says so in the same words.
  if (response.status === 405 || response.status === 404) {
    return legacyExport(appId, request, directory, replaceConfirmed, base, headers, deps, target);
  }
  if (!response.ok) return failed(await refusalText(response));

  let zip: Buffer;
  try {
    zip = Buffer.from(await response.arrayBuffer());
  } catch (error) {
    return failed(`The export did not arrive whole: ${(error as Error).message}`);
  }
  const summary = decodeExportReport(response.headers.get(EXPORT_REPORT_HEADER), request.mode);
  try {
    const notes = await placeFolder(
      directory,
      appId,
      replaceConfirmed,
      (staging) => {
        const { files, skipped } = extractExportZip(zip, appId, staging);
        if (!summary.files) summary.files = files;
        summary.skipped.push(...skipped);
        return [];
      },
      deps.trash
    );
    summary.notes.push(...notes);
  } catch (error) {
    return failed(`Could not write the export into ${target}: ${(error as Error).message}`);
  }
  return { status: 'exported', target, summary, legacy: false };
}

async function legacyExport(
  appId: string,
  request: AppExportRequest,
  directory: string,
  replaceConfirmed: boolean,
  url: string,
  headers: Record<string, string>,
  deps: AppExportDeps,
  target: string
): Promise<AppExportResult> {
  let response: Response;
  try {
    response = await (deps.fetch ?? fetch)(url, { headers });
  } catch (error) {
    return failed(`Could not reach the background service: ${(error as Error).message}`);
  }
  if (!response.ok) return failed(await refusalText(response));
  let payload: Record<string, unknown>;
  try {
    payload = (await response.json()) as Record<string, unknown>;
  } catch {
    return failed('The background service answered with something that is not an export.');
  }
  const files =
    typeof payload.files === 'object' && payload.files !== null && !Array.isArray(payload.files)
      ? (payload.files as Record<string, unknown>)
      : {};
  const summary: AppExportSummary = {
    mode: 'launcher',
    files: 0,
    kbs: [],
    skills: [],
    extensions: [],
    notes: [],
    lintWarnings: [],
    skipped: [],
  };
  const skippedBinary = Array.isArray(payload.skipped_binary)
    ? payload.skipped_binary.filter((item): item is string => typeof item === 'string')
    : [];
  summary.notes.push(
    request.mode === 'full' || request.bundleDaemon
      ? 'The background service is older than this app, so it made a launcher export without knowledge bases, skills or a bundled daemon. Update Biorouter and export again for a full export.'
      : 'The background service is older than this app, so the export carries its text files only.'
  );
  if (skippedBinary.length > 0) {
    summary.notes.push(
      `Left out ${skippedBinary.length} binary file(s): ${skippedBinary.join(', ')}.`
    );
  }
  try {
    const notes = await placeFolder(
      directory,
      appId,
      replaceConfirmed,
      (staging) => {
        summary.files = writeLegacyFiles(files, staging);
        return [];
      },
      deps.trash
    );
    summary.notes.push(...notes);
  } catch (error) {
    return failed(`Could not write the export into ${target}: ${(error as Error).message}`);
  }
  if (summary.files === 0) return failed('The background service sent nothing to export.');
  return { status: 'exported', target, summary, legacy: true };
}

/** The native picker's options: a folder, which it may create. */
export const EXPORT_PICKER_OPTIONS = {
  title: 'Export to a folder',
  buttonLabel: 'Export Here',
  properties: ['openDirectory', 'createDirectory'] as Array<'openDirectory' | 'createDirectory'>,
};

/** The question asked before a folder with something in it is replaced. */
export function replacePrompt(target: string) {
  return {
    type: 'question' as const,
    buttons: ['Replace', 'Cancel'],
    defaultId: 1,
    cancelId: 1,
    message: `Replace the existing folder “${path.basename(target)}”?`,
    detail: `${target} already has files in it. The export takes its place, and the folder that was there is moved to the Trash.`,
  };
}
