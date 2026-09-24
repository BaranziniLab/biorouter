/**
 * The main-process half of D-DROP: sharing a file a person dropped or pasted into Crew.
 *
 * The trust boundary is the native dialog. The renderer can send any path it likes (a
 * compromised one can skip the preload's `webUtils.getPathForFile` and speak IPC directly), so
 * nothing here believes the path names what the person meant. Instead:
 *
 * 1. The path is inspected here, in the main process, and refused with one plain sentence when
 *    it is not a regular file: a folder, a socket, a pipe, a device, a shortcut (symbolic link)
 *    whose target lies outside the folder the shortcut sits in, a file over the 1 GiB attachment
 *    limit, or a name that is not a file at all.
 * 2. The resolved real path, the name, the size and the destination go into a native
 *    `dialog.showMessageBox` parented to the window, with Cancel as the default button. The
 *    renderer can neither draw nor answer that dialog.
 * 3. Only after Share does the file reach the daemon, through the same `POST /crew/files`
 *    registration the Attach picker uses, and the renderer gets back only the daemon's opaque
 *    capability. The file is inspected again after the click, and the daemon's own reading of
 *    it (name and size) must match what the dialog showed, so a file swapped while the dialog
 *    was open is refused rather than shared.
 *
 * So a compromised renderer can at most make the dialog appear; it cannot upload anything a
 * person has not read the path of and accepted.
 *
 * The one exception is the development auto-confirm for automated QA
 * ({@link resolveDevAutoConfirmShare}): the gating of the development approval stdin
 * (`developmentApprovalInput.ts`) plus an explicit `BIOROUTER_DEV_AUTO_CONFIRM_SHARE=1`. It still
 * refuses everything step 1 refuses and logs every path it confirms.
 */
import nodeFs from 'node:fs/promises';
import { constants as fsConstants } from 'node:fs';
import path from 'node:path';
import type { MessageBoxOptions } from 'electron';
import { formatBytes } from '../components/crew/files/formatBytes';
import { sanitizeUntrustedLabel, stripHiddenCharacters } from './untrustedText';
import type { CrewShareDestination, CrewShareDroppedFileResult } from './crewSharePathBridge';

/** The daemon's attachment limit, `local_files::MAX_SIZE`: 1 GiB. */
export const CREW_SHARE_SIZE_LIMIT = 1024 * 1024 * 1024;

/** The environment switch for the development auto-confirm. Only the exact value `1` counts. */
export const DEV_AUTO_CONFIRM_SHARE_ENV = 'BIOROUTER_DEV_AUTO_CONFIRM_SHARE';

/** Destination names are display text; longer ones are shortened with an ellipsis. */
export const CREW_SHARE_LABEL_MAX_CHARS = 80;

/** A path longer than this is not something a drop produces. */
const MAX_PATH_CHARS = 4096;

/** Connection, channel and capability ids, as the Attach picker validates them. */
const ID = /^[a-zA-Z0-9_-]{1,128}$/;

/** The index of Share in {@link crewShareDialogOptions}'s buttons. Cancel is the other one. */
export const CREW_SHARE_BUTTON_SHARE = 0;

/** The daemon's refusal when a connection's privacy moved under the renderer. */
const PRIVACY_CHANGED_ERROR =
  'Crew connection privacy changed; refresh the verified workspace before selecting a file';

/** Every sentence this flow can show a person. `name` is already made visible. */
export const crewShareCopy = {
  notAFile:
    "This item isn't a saved file, so it can't be shared. Save it as a file first, then drop it again.",
  missing: (name: string) => `"${name}" is no longer there. It may have been moved or deleted.`,
  unreadable: (name: string) =>
    `Biorouter can't read "${name}". Check that you're allowed to open it, then try again.`,
  folder: (name: string) =>
    `"${name}" is a folder. Crew shares one file at a time, so zip the folder or drop the files inside it.`,
  shortcutOutside: (name: string) =>
    `"${name}" is a shortcut to a file in another folder. Drop the original file instead.`,
  shortcutBroken: (name: string) => `"${name}" is a shortcut to something that no longer exists.`,
  special: (name: string) =>
    `"${name}" isn't a regular file (it's a device, a socket or a pipe), so it can't be shared.`,
  // The limit, not the file's size: a file just over 1 GiB also reads "1 GB".
  tooLarge: (name: string, limit: number = CREW_SHARE_SIZE_LIMIT) =>
    `"${name}" is larger than ${formatBytes(limit)}, the most Crew can attach.`,
  changed: (name: string) =>
    `"${name}" changed while you were deciding. Drop it again to share the current version.`,
  busy: 'Finish the open share confirmation first.',
  privacyChanged: 'Connection privacy changed. Refresh Crew and drop the file again.',
  daemonRefused: (name: string) =>
    `Crew couldn't take "${name}". Check that the file is readable, then drop it again.`,
} as const;

/** What the main process accepts over `crew:share-dropped-file`, after validation. */
export interface CrewShareRequest extends CrewShareDestination {
  /** Whatever the preload resolved; possibly `''`. Never trusted to name what was meant. */
  path: string;
}

/**
 * Renders text for a native dialog so nothing in it is invisible: every control and format
 * character (a newline, a bidi override, a zero-width space) becomes U+FFFD. A filename that
 * carries one then looks odd instead of looking like a different name.
 *
 * Built on {@link stripHiddenCharacters} one code point at a time, so the drop set keeps its
 * single definition in `untrustedText.ts`.
 */
export function visibleText(value: string): string {
  return Array.from(value, (character) =>
    stripHiddenCharacters(character) === '' ? '�' : character
  ).join('');
}

function destinationLabel(value: unknown, channel: boolean): string {
  if (typeof value !== 'string') return '';
  let text = sanitizeUntrustedLabel(value, 1024);
  if (channel) text = text.replace(/^#+/, '').trim();
  const characters = Array.from(text);
  return characters.length > CREW_SHARE_LABEL_MAX_CHARS
    ? `${characters.slice(0, CREW_SHARE_LABEL_MAX_CHARS - 1).join('')}…`
    : text;
}

/**
 * Validates the IPC payload. A malformed request is a renderer bug, not a person's mistake, so
 * it throws; everything about the file itself is answered later with a plain sentence.
 */
export function parseCrewShareRequest(raw: unknown): CrewShareRequest {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw))
    throw new Error('Invalid share request.');
  const request = raw as Record<string, unknown>;
  if (typeof request.path !== 'string') throw new Error('Invalid share request.');
  if (
    request.expectedMode !== undefined &&
    request.expectedMode !== 'private' &&
    request.expectedMode !== 'public'
  )
    throw new Error(
      'Invalid expected transfer privacy. Refresh the workspace before sharing a file.'
    );
  if (
    typeof request.connectionId !== 'string' ||
    typeof request.channelId !== 'string' ||
    !ID.test(request.connectionId) ||
    !ID.test(request.channelId)
  )
    throw new Error('Invalid transfer destination.');
  const channelName = destinationLabel(request.channelName, true);
  const workspaceName = destinationLabel(request.workspaceName, false);
  if (!channelName || !workspaceName) throw new Error('Invalid share destination name.');
  return {
    path: request.path,
    connectionId: request.connectionId,
    channelId: request.channelId,
    channelName,
    workspaceName,
    ...(request.expectedMode === undefined ? {} : { expectedMode: request.expectedMode }),
  };
}

interface StatLike {
  isFile(): boolean;
  isDirectory(): boolean;
  isSymbolicLink(): boolean;
  size: number;
  dev: number;
  ino: number;
  mtimeMs: number;
}

/** The three filesystem calls the inspection makes, injectable for tests. */
export interface ShareFs {
  lstat(target: string): Promise<StatLike>;
  realpath(target: string): Promise<string>;
  /** Resolves when the file may be read, rejects otherwise. */
  access(target: string): Promise<void>;
}

const defaultFs: ShareFs = {
  lstat: (target) => nodeFs.lstat(target),
  realpath: (target) => nodeFs.realpath(target),
  access: (target) => nodeFs.access(target, fsConstants.R_OK),
};

export type InspectedFile =
  | {
      ok: true;
      /** Every link resolved: the file the daemon will open, and the path the dialog shows. */
      realPath: string;
      /** The real file's name, as the daemon will report it. */
      name: string;
      size: number;
      /** Device, inode, size and modification time: changes if the file is swapped or edited. */
      identity: string;
    }
  | { ok: false; message: string };

export interface InspectOptions {
  fs?: ShareFs;
  limit?: number;
}

function errorCode(error: unknown): string | undefined {
  return error && typeof error === 'object' && 'code' in error
    ? String((error as { code: unknown }).code)
    : undefined;
}

function isInside(folder: string, target: string): boolean {
  const relative = path.relative(folder, target);
  return (
    relative === '' ||
    (relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))
  );
}

/**
 * Decides whether a dropped path may be offered for sharing, and what the dialog must show.
 *
 * - An empty, relative or NUL-carrying path is not a file on this computer (pasted image data,
 *   a synthetic `File`), and so is anything longer than a drop produces.
 * - A folder is refused.
 * - A shortcut (symbolic link, or a Windows junction, which Node reports the same way) is
 *   followed only when its real target lies in the shortcut's own folder or below it; one that
 *   points anywhere else is refused, so a harmless-looking name cannot stand in for a file
 *   elsewhere. Folders above the file that are themselves links (macOS's `/tmp`, a synced
 *   folder) are resolved, and the dialog shows the real path.
 * - Anything that is not a regular file once resolved (a socket, a pipe, a device) is refused.
 * - A file over the attachment limit is refused, and so is one this user cannot read.
 */
export async function inspectDroppedFile(
  droppedPath: string,
  options: InspectOptions = {}
): Promise<InspectedFile> {
  const fs = options.fs ?? defaultFs;
  const limit = options.limit ?? CREW_SHARE_SIZE_LIMIT;
  if (
    typeof droppedPath !== 'string' ||
    droppedPath.length === 0 ||
    droppedPath.length > MAX_PATH_CHARS ||
    droppedPath.includes('\0') ||
    !path.isAbsolute(droppedPath)
  )
    return { ok: false, message: crewShareCopy.notAFile };
  const droppedName = visibleText(path.basename(droppedPath)) || visibleText(droppedPath);

  let dropped: StatLike;
  try {
    dropped = await fs.lstat(droppedPath);
  } catch (error) {
    const code = errorCode(error);
    return {
      ok: false,
      message:
        code === 'ENOENT' || code === 'ENOTDIR'
          ? crewShareCopy.missing(droppedName)
          : crewShareCopy.unreadable(droppedName),
    };
  }
  if (dropped.isDirectory()) return { ok: false, message: crewShareCopy.folder(droppedName) };

  let realPath: string;
  try {
    realPath = await fs.realpath(droppedPath);
  } catch (error) {
    const code = errorCode(error);
    if (dropped.isSymbolicLink())
      return { ok: false, message: crewShareCopy.shortcutBroken(droppedName) };
    return {
      ok: false,
      message:
        code === 'ENOENT' || code === 'ENOTDIR'
          ? crewShareCopy.missing(droppedName)
          : crewShareCopy.unreadable(droppedName),
    };
  }
  if (!path.isAbsolute(realPath)) return { ok: false, message: crewShareCopy.notAFile };

  if (dropped.isSymbolicLink()) {
    let folder: string;
    try {
      folder = await fs.realpath(path.dirname(droppedPath));
    } catch {
      return { ok: false, message: crewShareCopy.unreadable(droppedName) };
    }
    if (!isInside(folder, realPath))
      return { ok: false, message: crewShareCopy.shortcutOutside(droppedName) };
  }

  const name = visibleText(path.basename(realPath)) || droppedName;
  let stat: StatLike;
  try {
    stat = await fs.lstat(realPath);
  } catch (error) {
    const code = errorCode(error);
    return {
      ok: false,
      message:
        code === 'ENOENT' || code === 'ENOTDIR'
          ? crewShareCopy.missing(name)
          : crewShareCopy.unreadable(name),
    };
  }
  // `realpath` resolved every link, so a link here was put in place since: refuse it.
  if (stat.isSymbolicLink()) return { ok: false, message: crewShareCopy.changed(name) };
  if (stat.isDirectory()) return { ok: false, message: crewShareCopy.folder(name) };
  if (!stat.isFile()) return { ok: false, message: crewShareCopy.special(name) };
  if (stat.size > limit) return { ok: false, message: crewShareCopy.tooLarge(name, limit) };
  try {
    await fs.access(realPath);
  } catch {
    return { ok: false, message: crewShareCopy.unreadable(name) };
  }
  return {
    ok: true,
    realPath,
    name: path.basename(realPath),
    size: stat.size,
    identity: `${stat.dev}:${stat.ino}:${stat.size}:${stat.mtimeMs}`,
  };
}

/**
 * The native confirmation: `Share "<name>" (<size>) to #<channel> in <workspace>?`, the full
 * real path as the detail, and Share / Cancel with Cancel the default and the Escape answer.
 */
export function crewShareDialogOptions(
  file: { name: string; size: number; realPath: string },
  destination: Pick<CrewShareDestination, 'channelName' | 'workspaceName'>
): MessageBoxOptions {
  return {
    type: 'question',
    title: 'Share file to Crew',
    message: `Share "${visibleText(file.name)}" (${formatBytes(file.size)}) to #${destination.channelName} in ${destination.workspaceName}?`,
    detail: `Full path: ${visibleText(file.realPath)}`,
    buttons: ['Share', 'Cancel'],
    defaultId: 1,
    cancelId: 1,
    noLink: true,
  };
}

/** The body of `POST /crew/files`: the Attach picker's upload registration, field for field. */
export function crewShareRegistrationBody(request: CrewShareRequest, realPath: string) {
  return {
    direction: 'upload' as const,
    purpose: 'transfer' as const,
    path: realPath,
    overwrite: false,
    approval_pending: false,
    connection_id: request.connectionId,
    channel_id: request.channelId,
    ...(request.expectedMode === undefined ? {} : { expected_mode: request.expectedMode }),
  };
}

export interface ShareDroppedFileDeps extends InspectOptions {
  /** True only when {@link resolveDevAutoConfirmShare} said so at startup. */
  autoConfirm: boolean;
  /** Shows the native dialog parented to the window and answers the chosen button's index. */
  confirm(options: MessageBoxOptions): Promise<number>;
  /** `POST /crew/files` with the daemon secret and the user-action proof. */
  register(
    body: ReturnType<typeof crewShareRegistrationBody>
  ): Promise<{ ok: boolean; body: unknown }>;
  /** `DELETE /crew/files/{id}`: gives back a capability that will not be used. */
  discard(capabilityId: string): Promise<void>;
  /** True once the window or its document is gone. */
  isClosed(): boolean;
  log(message: string): void;
}

const refused = (message: string): CrewShareDroppedFileResult => ({ outcome: 'refused', message });

/**
 * The whole flow behind `crew:share-dropped-file`: inspect, confirm in native UI (or the gated
 * development auto-confirm), inspect again, register with the daemon, and check the daemon
 * opened the file the person accepted.
 */
export async function shareDroppedFile(
  request: CrewShareRequest,
  deps: ShareDroppedFileDeps
): Promise<CrewShareDroppedFileResult> {
  const inspected = await inspectDroppedFile(request.path, deps);
  if (!inspected.ok) return refused(inspected.message);
  const shownName = visibleText(inspected.name);

  if (deps.autoConfirm) {
    deps.log(
      `[crew-share] ${DEV_AUTO_CONFIRM_SHARE_ENV}=1 confirmed without a dialog: ${inspected.realPath} (${inspected.size} bytes) to connection ${request.connectionId}, channel ${request.channelId}`
    );
  } else {
    if (deps.isClosed()) return { outcome: 'cancelled' };
    let response: number;
    try {
      response = await deps.confirm(crewShareDialogOptions(inspected, request));
    } catch {
      return { outcome: 'cancelled' };
    }
    if (response !== CREW_SHARE_BUTTON_SHARE) return { outcome: 'cancelled' };
  }
  if (deps.isClosed()) return { outcome: 'cancelled' };

  const again = await inspectDroppedFile(request.path, deps);
  if (!again.ok || again.realPath !== inspected.realPath || again.identity !== inspected.identity)
    return refused(crewShareCopy.changed(shownName));

  let answer: { ok: boolean; body: unknown };
  try {
    answer = await deps.register(crewShareRegistrationBody(request, inspected.realPath));
  } catch {
    return refused(crewShareCopy.daemonRefused(shownName));
  }
  if (!answer.ok) {
    const failure = answer.body as { error?: unknown } | null;
    return refused(
      failure?.error === PRIVACY_CHANGED_ERROR
        ? crewShareCopy.privacyChanged
        : crewShareCopy.daemonRefused(shownName)
    );
  }
  const result = answer.body as Record<string, unknown> | null;
  const capabilityId = result?.capability_id;
  if (typeof capabilityId !== 'string' || !ID.test(capabilityId))
    return refused(crewShareCopy.daemonRefused(shownName));
  const giveBack = () => deps.discard(capabilityId).catch(() => undefined);
  if (result?.name !== inspected.name || result?.size !== inspected.size) {
    await giveBack();
    return refused(crewShareCopy.changed(shownName));
  }
  if (deps.isClosed()) {
    await giveBack();
    return { outcome: 'cancelled' };
  }
  return {
    outcome: 'shared',
    capability_id: capabilityId,
    name: inspected.name,
    size: inspected.size,
  };
}

/**
 * The development auto-confirm's gate: exactly the development approval stdin's conditions
 * (`createDevelopmentApprovalReader`): an unpackaged app, a validated development profile,
 * `ENABLE_PLAYWRIGHT` and shared daemon mode, plus `BIOROUTER_DEV_AUTO_CONFIRM_SHARE=1` exactly.
 *
 * Unlike the approval stdin it never throws: a stray variable in an installed build must not stop
 * the app from starting. It fails closed instead, leaving the native dialog on, and says why in
 * `notice` so the refusal is not silent.
 */
export function resolveDevAutoConfirmShare(options: {
  value: string | undefined;
  isPackaged: boolean;
  developmentProfileRoot: string | undefined;
  testDriverEnabled: boolean;
  sharedDaemonEnabled: boolean;
}): { enabled: boolean; notice?: string } {
  if (options.value === undefined || options.value === '') return { enabled: false };
  if (options.value !== '1')
    return {
      enabled: false,
      notice: `${DEV_AUTO_CONFIRM_SHARE_ENV} must be exactly 1; the native share confirmation stays on.`,
    };
  if (
    options.isPackaged ||
    !options.developmentProfileRoot ||
    !options.testDriverEnabled ||
    !options.sharedDaemonEnabled
  )
    return {
      enabled: false,
      notice: `${DEV_AUTO_CONFIRM_SHARE_ENV} requires an unpackaged app, a validated development profile, ENABLE_PLAYWRIGHT, and shared daemon mode; the native share confirmation stays on.`,
    };
  return {
    enabled: true,
    notice: `${DEV_AUTO_CONFIRM_SHARE_ENV}=1: Crew files dropped or pasted in this development profile are shared without the native confirmation. Every confirmed path is logged.`,
  };
}

/** One open share confirmation per window, so a renderer cannot stack dialogs. */
export class CrewSharePending {
  private readonly owners = new Set<number>();
  enter(ownerId: number): boolean {
    if (this.owners.has(ownerId)) return false;
    this.owners.add(ownerId);
    return true;
  }
  leave(ownerId: number): void {
    this.owners.delete(ownerId);
  }
}
