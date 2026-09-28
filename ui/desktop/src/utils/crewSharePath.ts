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
 *    renderer can neither draw nor answer that dialog. The destination's names are the one
 *    thing in it the renderer wrote, so they are flattened to a single line and placed below
 *    the true path, never above it ({@link crewShareDialogOptions}).
 * 3. Only after Share does the file reach the daemon, through the same `POST /crew/files`
 *    registration the Attach picker uses, and the renderer gets back only the daemon's opaque
 *    capability. The file is inspected again after the click, and the daemon's own reading of
 *    it (name and size) must match what the dialog showed, so a file swapped while the dialog
 *    was open is refused rather than shared. The daemon refuses a credential file here too, and
 *    the person is told so in one sentence ({@link crewShareCopy.credential}).
 *
 * So through this channel a compromised renderer can at most make the dialog appear, and pick
 * the destination names it prints below the true path; it can neither name an arbitrary file
 * nor skip the confirmation. This channel is not the only way to the daemon, though: the
 * renderer still holds the daemon secret and the user-action key, so it can call
 * `POST /crew/files` itself and skip both the Attach picker and this dialog. D-DROP does not
 * widen that. What holds on that route whichever door a path came through (the Attach picker,
 * this flow, the CLI, or page script) is the daemon's credential floor (Q3-01): a path the BR-23
 * secret guard names — a private key, a cloud credential file, a `secrets.*` or `.env` store,
 * judged after links are resolved and with case folded — or a file whose first 64 KiB hold
 * credential material is refused with the code {@link CREW_FILE_IS_CREDENTIAL}, and so is a
 * download into such a location. A main-only registration door, which would close the route to
 * the renderer altogether, is a deferred design decision (implementation plan §16, D-DROP).
 *
 * The one exception is the development auto-confirm for automated QA
 * ({@link resolveDevAutoConfirmShare}): the gating of the development approval stdin
 * (`developmentApprovalInput.ts`) plus an explicit `BIOROUTER_DEV_AUTO_CONFIRM_SHARE=1`. It still
 * refuses everything step 1 refuses and logs every path it confirms.
 */
import nodeFs from 'node:fs/promises';
import { constants as fsConstants } from 'node:fs';
import path from 'node:path';
import type { MessageBoxOptions, OpenDialogOptions, SaveDialogOptions } from 'electron';
import { formatBytes } from '../components/crew/files/formatBytes';
import { sanitizeDisplayText } from '../components/crew/identity/displayText';
import { stripHiddenCharacters } from './untrustedText';
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

/**
 * The code `POST /crew/files` answers when the BR-23 credential floor refuses a path
 * (`local_files::CREDENTIAL_REFUSAL_CODE`, Q3-01).
 */
export const CREW_FILE_IS_CREDENTIAL = 'crew_file_is_credential';
/** A download whose file name starts with a dot, which the daemon never saves into the home. */
export const CREW_FILE_NAME_HIDDEN = 'crew_file_name_hidden';
/** A download into a folder another account owns or can change (`/tmp`, a shared folder). */
export const CREW_FOLDER_SHARED = 'crew_folder_shared';
/**
 * The connection's privacy mode is not the one the renderer verified and sent as
 * `expected_mode`. The daemon names both (`actual_mode`, `expected_mode`). Keyed on the code, never
 * on the sentence beside it (DW-12): a reworded sentence broke a match on its text silently.
 */
export const CREW_MODE_MISMATCH = 'crew_mode_mismatch';

/** Every sentence this flow can show a person. `name` is already made visible. */
export const crewShareCopy = {
  notAFile:
    "This item isn't a saved file, so it can't be shared. Save it as a file first, then share it again.",
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
  /** A Save or Open request while this window's Crew file window is still open (FILES-F2). */
  pickerBusy: 'Finish the open Save or Open window first.',
  /** A Crew file window asked for while another dialog is attached to the window (FILES-F2). */
  sheetOpen: 'Close the open dialog in this window first.',
  /**
   * `crew_mode_mismatch`: the connection is `actual`, the file was checked for `expected`. `again`
   * is how to retry ("try again", "drop the file again").
   */
  modeChanged: (actual: CrewMode, expected: CrewMode, again: string) =>
    `Your connection is now ${modeName(actual)}; this file was checked for ${modeName(expected)}. Refresh Crew and ${again}.`,
  /** The same, from a daemon that named neither mode and a request that sent none. */
  modeChangedUnknown: (again: string) =>
    `Your connection's privacy changed since Crew checked it. Refresh Crew and ${again}.`,
  /** `crew_file_name_hidden`, the daemon's sentence (W2-HRD-4) rebuilt from the name chosen here. */
  nameHidden: (name: string) =>
    `“${name}” starts with a dot, which Crew doesn't save into your home. Choose a name without the leading dot.`,
  /** `crew_folder_shared`, the daemon's sentence (W2-HRD-4). */
  folderShared: "Choose a folder owned by your account that other accounts can't change.",
  daemonRefused: (name: string) =>
    `Crew couldn't take "${name}". Check that the file is readable, then drop it again.`,
  /**
   * The daemon's own sentences for {@link CREW_FILE_IS_CREDENTIAL}, word for word
   * (`CredentialRefusal`'s `Display` in `crew/local_files.rs`), rebuilt here from the name this
   * process read rather than relayed, so no daemon text reaches a dialog unchecked.
   */
  credential: (name: string) =>
    `“${name}” looks like a credential file (a password, key or token store), so Crew won't share it.`,
  credentialLocation:
    "Crew won't save into a credential or settings location. Choose another folder.",
} as const;

/** A connection's privacy mode, as the daemon and the renderer spell it. */
export type CrewMode = 'private' | 'public';

/** The mode as the manual names it. */
function modeName(mode: CrewMode): string {
  return mode === 'private' ? 'Private' : 'Public';
}

function crewMode(value: unknown): CrewMode | undefined {
  return value === 'private' || value === 'public' ? value : undefined;
}

/** What else a refusal's sentence may need from the request that was refused. */
export interface CrewFileRefusalContext {
  /** The mode the request sent as `expected_mode`, for a daemon that names none. */
  expectedMode?: CrewMode;
  /** How to retry, ending the privacy sentence: "try again" (the default), "drop the file again". */
  again?: string;
}

/**
 * The `crew_mode_mismatch` sentence. The daemon names both modes; when it does not, the one the
 * request expected is enough, since a mismatch between two modes names the other one too.
 */
function modeMismatchSentence(body: Record<string, unknown>, context: CrewFileRefusalContext) {
  const again = context.again ?? 'try again';
  const details =
    body.details && typeof body.details === 'object'
      ? (body.details as Record<string, unknown>)
      : {};
  const expected =
    crewMode(body.expected_mode) ?? crewMode(details.expected_mode) ?? context.expectedMode;
  const actual =
    crewMode(body.actual_mode) ??
    crewMode(details.actual_mode) ??
    (expected ? (expected === 'private' ? 'public' : 'private') : undefined);
  return actual && expected && actual !== expected
    ? crewShareCopy.modeChanged(actual, expected, again)
    : crewShareCopy.modeChangedUnknown(again);
}

/**
 * The sentence for a `POST /crew/files` refusal whose code has one of its own, or `undefined` for
 * the caller's wording. Keyed on the daemon's code, never on the words beside it:
 *
 * - the credential floor (Q3-01) and, for a download, the settings locations beside it (Q4-55):
 *   an upload names the file (made visible), a download names no file, because the refusal is
 *   about the folder;
 * - a download named with a leading dot, or into a folder other accounts can change
 *   (W2-HRD-4), each naming what to change (the daemon's other download refusals, a folder given
 *   as the file, an existing file, a program, name the path they refuse, and the picker shows
 *   them in the daemon's words);
 * - a connection whose privacy mode is not the one the request expected (DW-12), naming both.
 *
 * `name` is the file this process chose or read, never the daemon's text.
 */
export function crewFileRefusal(
  failure: unknown,
  direction: 'upload' | 'download',
  name: string,
  context: CrewFileRefusalContext = {}
): string | undefined {
  if (!failure || typeof failure !== 'object') return undefined;
  const body = failure as Record<string, unknown>;
  switch (body.code) {
    case CREW_FILE_IS_CREDENTIAL:
      return direction === 'upload'
        ? crewShareCopy.credential(visibleText(name) || 'This file')
        : crewShareCopy.credentialLocation;
    case CREW_FILE_NAME_HIDDEN:
      return crewShareCopy.nameHidden(visibleText(name) || 'This name');
    case CREW_FOLDER_SHARED:
      return crewShareCopy.folderShared;
    case CREW_MODE_MISMATCH:
      return modeMismatchSentence(body, context);
    default:
      return undefined;
  }
}

/** The longest daemon sentence shown as it came; a longer one is cut with an ellipsis. */
export const DAEMON_SENTENCE_MAX_CHARS = 300;

/**
 * The daemon's own sentence for a refusal that has no code of its own, made visible (hidden
 * characters shown, one line) and bounded, or `undefined` when it gave none. Shown instead of a
 * vaguer sentence of ours (FILES-F1): the daemon's reason is the one that says what to change.
 */
export function daemonRefusalSentence(failure: unknown): string | undefined {
  if (!failure || typeof failure !== 'object') return undefined;
  const text = (failure as { error?: unknown }).error;
  if (typeof text !== 'string') return undefined;
  const visible = visibleText(text).trim();
  if (!visible) return undefined;
  const characters = Array.from(visible);
  return characters.length > DAEMON_SENTENCE_MAX_CHARS
    ? `${characters.slice(0, DAEMON_SENTENCE_MAX_CHARS - 1).join('')}…`
    : visible;
}

/** What the main process accepts over `crew:share-dropped-file`, after validation. */
export interface CrewShareRequest extends CrewShareDestination {
  /** Whatever the preload resolved; possibly `''`. Never trusted to name what was meant. */
  path: string;
}

/** U+2028 LINE SEPARATOR and U+2029 PARAGRAPH SEPARATOR: neither a control nor a format character. */
const LINE_OR_PARAGRAPH_SEPARATOR = /^[\p{Zl}\p{Zp}]$/u;
/** What is left of `White_Space` once controls and separators are gone: the space separators. */
const WHITE_SPACE_RUN = /\p{White_Space}{2,}/gu;

/**
 * Renders text for a native dialog so nothing in it is invisible and it stays on one line.
 *
 * - Every control and format character (a newline, a bidi override, a zero-width space) and
 *   every line or paragraph separator (U+2028, U+2029) becomes U+FFFD. The separators are the
 *   ones that slip past a control-character rule: they are mandatory line breaks in the macOS
 *   alert, GTK and every layout that follows Unicode line breaking, so left alone they write a
 *   line of their own into the dialog. A filename that carries any of these then looks odd
 *   instead of looking like a different name, or like two lines.
 * - A run of spaces (of any width) becomes one space, which takes away the cheap way to steer
 *   where a long line wraps. A single space, including the narrow no-break space in macOS
 *   screenshot names, is kept as it is. Wrapping itself cannot be prevented, which is why
 *   {@link crewShareDialogOptions} puts the renderer's text last.
 *
 * Built on {@link stripHiddenCharacters} one code point at a time, so the drop set keeps its
 * single definition in `untrustedText.ts`.
 */
export function visibleText(value: string): string {
  return Array.from(value, (character) =>
    stripHiddenCharacters(character) === '' || LINE_OR_PARAGRAPH_SEPARATOR.test(character)
      ? '\uFFFD'
      : character
  )
    .join('')
    .replace(WHITE_SPACE_RUN, ' ');
}

/**
 * A destination name as the dialog shows it. The renderer wrote it, so it gets the Crew display
 * name rule (`sanitizeDisplayText`: every `White_Space` run, line separators included, to one
 * space; hidden, private-use, unassigned and default-ignorable characters removed) and then
 * {@link visibleText}.
 */
function destinationLabel(value: unknown, channel: boolean): string {
  if (typeof value !== 'string') return '';
  let text = visibleText(sanitizeDisplayText(value)).trim();
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
 * The native confirmation, and Share / Cancel with Cancel the default and the Escape answer:
 *
 * ```text
 * Share "<name>" (<size>) to Crew?          (the message, shown bold)
 * Full path: <real path>                    (the detail's first line)
 * Destination: #<channel> in <workspace>    (its second)
 * It uploads now and appears in #<channel> when you send your message.   (its third and last)
 * ```
 *
 * The last line says what Share does (Q3-14): the bytes leave for the host at once, and the file
 * reaches the channel with the message it is attached to.
 *
 * ⚠ The order is the security property, not a matter of taste. The channel and workspace names
 * are the only text here the renderer writes, and they are not checked against the ids the
 * capability is bound to. Everything above the true path is therefore the main process's own
 * words or what it read from the file system, and the renderer's names come only below it:
 * whatever a name spells, even `Full path: …`, can only appear after the real one, never above
 * it. Every value also passes through {@link visibleText}, so none of them can start a line.
 */
export function crewShareDialogOptions(
  file: { name: string; size: number; realPath: string },
  destination: Pick<CrewShareDestination, 'channelName' | 'workspaceName'>
): MessageBoxOptions {
  const channel = visibleText(destination.channelName);
  return {
    type: 'question',
    title: 'Share file to Crew',
    message: `Share "${visibleText(file.name)}" (${formatBytes(file.size)}) to Crew?`,
    detail: [
      `Full path: ${visibleText(file.realPath)}`,
      `Destination: #${channel} in ${visibleText(destination.workspaceName)}`,
      `It uploads now and appears in #${channel} when you send your message.`,
    ].join('\n'),
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
    // A refusal with a code of its own says what to change; any other keeps this flow's own
    // sentence rather than the daemon's text, which here is usually the operating system's.
    return refused(
      crewFileRefusal(answer.body, 'upload', inspected.name, {
        expectedMode: request.expectedMode,
        again: 'drop the file again',
      }) ?? crewShareCopy.daemonRefused(shownName)
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

/** Which Crew flow holds a window's native dialogs. */
export type CrewSheetKind = 'share' | 'picker';

/**
 * One Crew native flow per window at a time, and none while another dialog is attached to it
 * (FILES-F2).
 *
 * On macOS a parented dialog is a sheet, and AppKit neither shows nor queues a Save or Open panel
 * asked for while the window already has a sheet: the panel never appears and Electron's promise
 * never settles, so the card that asked kept Save disabled until the window reloaded. The picker
 * (its Save or Open panel and its own replace alert) and the drop confirmation therefore hold the
 * window for their whole flow, including the daemon round trip between the panel and the alert,
 * and a second request is refused with a sentence instead of being lost.
 *
 * `sheetBegan`/`sheetEnded` follow the window's `sheet-begin`/`sheet-end` events, so a dialog
 * another part of the app attached (the "Open this with…" alert, the diagnostics Save) refuses a
 * Crew panel too, rather than swallowing it. Those events fire on macOS only, the one platform
 * whose parented dialogs are sheets.
 */
export class CrewSheetGate {
  private readonly flows = new Map<number, CrewSheetKind>();
  private readonly sheets = new Map<number, number>();

  /** Hold `windowId` for a flow; the sentence to refuse with when it is already held. */
  enter(windowId: number, kind: CrewSheetKind): string | undefined {
    const held = this.flows.get(windowId);
    if (held) return held === 'share' ? crewShareCopy.busy : crewShareCopy.pickerBusy;
    if (this.hasSheet(windowId)) return crewShareCopy.sheetOpen;
    this.flows.set(windowId, kind);
    return undefined;
  }

  leave(windowId: number): void {
    this.flows.delete(windowId);
  }

  /** Whether a native sheet is attached to the window now. */
  hasSheet(windowId: number): boolean {
    return (this.sheets.get(windowId) ?? 0) > 0;
  }

  sheetBegan(windowId: number): void {
    this.sheets.set(windowId, (this.sheets.get(windowId) ?? 0) + 1);
  }

  sheetEnded(windowId: number): void {
    const open = (this.sheets.get(windowId) ?? 0) - 1;
    if (open > 0) this.sheets.set(windowId, open);
    else this.sheets.delete(windowId);
  }

  /** The window closed: nothing of it is held any more. */
  forget(windowId: number): void {
    this.flows.delete(windowId);
    this.sheets.delete(windowId);
  }
}

/**
 * Run `flow` holding `windowId` in `gate`, or refuse at once, with the gate's sentence, when the
 * window is already held or has a sheet. The hold ends however the flow ends.
 */
export async function holdCrewSheet<T>(
  gate: CrewSheetGate,
  windowId: number,
  kind: CrewSheetKind,
  flow: () => Promise<T>
): Promise<T> {
  const busy = gate.enter(windowId, kind);
  if (busy) throw new Error(busy);
  try {
    return await flow();
  } finally {
    gate.leave(windowId);
  }
}

// ---------------------------------------------------------------------------------------------
// The Attach picker, the Save window and the cleanup window (`crew:select-transfer-file`)
// ---------------------------------------------------------------------------------------------

/** The Save window's default name when nothing usable is left of the file's own. */
export const CREW_DEFAULT_SAVE_NAME = 'crew-download';

/** A character the Save window's default name leaves out: a control, format, separator or private-use one. */
const UNSAVED_CHARACTER = /^[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\p{Co}]$/u;

/**
 * The default name the Save window proposes for a file another member named, so it is one the
 * daemon accepts (FILES-F3): the last path segment only; every control, format, line or paragraph
 * separator and private-use character left out (a U+202E in `q3_\u202Efdp.exe` would otherwise
 * make the proposed name read as a PDF); and no leading dot, since the daemon never saves a dot
 * name into the home. `fileName.ts`'s `saveNameFor` applies the same rule in the renderer, and
 * `peerFileNames.test.tsx` holds the two together.
 */
export function crewSaveName(raw: unknown): string {
  if (typeof raw !== 'string') return CREW_DEFAULT_SAVE_NAME;
  const name = Array.from(path.basename(raw))
    .filter((character) => !UNSAVED_CHARACTER.test(character))
    .join('')
    .trim()
    .replace(/^\.+/, '')
    .trim();
  return name || CREW_DEFAULT_SAVE_NAME;
}

/** What the main process accepts over `crew:select-transfer-file`, after validation. */
export interface CrewPickerRequest {
  expectedMode?: CrewMode;
  purpose: 'transfer' | 'cleanup';
  direction: 'upload' | 'download';
  connectionId: string;
  channelId: string;
  blobId?: string;
  transferId?: string;
  suggestedName?: unknown;
}

/**
 * Validates the picker's IPC payload. A malformed request is a renderer bug, so it throws; the
 * words are the ones this handler has always used.
 */
export function parseCrewPickerRequest(raw: unknown): CrewPickerRequest {
  if (!raw || typeof raw !== 'object') throw new Error('Invalid transfer request.');
  const options = raw as Record<string, unknown>;
  if (
    options.expectedMode !== undefined &&
    options.expectedMode !== 'private' &&
    options.expectedMode !== 'public'
  )
    throw new Error(
      'Invalid expected transfer privacy. Refresh the workspace before choosing a file.'
    );
  if (
    !['upload', 'download'].includes(String(options.direction)) ||
    typeof options.connectionId !== 'string' ||
    typeof options.channelId !== 'string' ||
    !ID.test(options.connectionId) ||
    !ID.test(options.channelId)
  )
    throw new Error('Invalid transfer destination.');
  for (const field of ['blobId', 'transferId']) {
    const value = options[field];
    if (value !== undefined && (typeof value !== 'string' || !ID.test(value)))
      throw new Error('Invalid transfer capability binding.');
  }
  const purpose = options.purpose ?? 'transfer';
  if (purpose !== 'transfer' && purpose !== 'cleanup')
    throw new Error('Invalid transfer selection purpose.');
  if (purpose === 'cleanup' && (options.direction !== 'download' || !options.transferId))
    throw new Error('Temporary download cleanup needs its original transfer.');
  return {
    ...(options.expectedMode === undefined ? {} : { expectedMode: options.expectedMode }),
    purpose,
    direction: options.direction as 'upload' | 'download',
    connectionId: options.connectionId,
    channelId: options.channelId,
    ...(options.blobId === undefined ? {} : { blobId: options.blobId as string }),
    ...(options.transferId === undefined ? {} : { transferId: options.transferId as string }),
    suggestedName: options.suggestedName,
  };
}

/** The Electron and daemon calls the picker makes, injectable for tests. */
export interface CrewPickerDeps {
  /** `dialog.showOpenDialog` parented to the window. */
  showOpenDialog(options: OpenDialogOptions): Promise<{ canceled: boolean; filePaths: string[] }>;
  /** `dialog.showSaveDialog` parented to the window. */
  showSaveDialog(options: SaveDialogOptions): Promise<{ canceled: boolean; filePath?: string }>;
  /** `dialog.showMessageBox` parented to the window. */
  showMessageBox(options: MessageBoxOptions): Promise<{ response: number }>;
  /** `/crew/files{endpoint}` with the daemon secret and the user-action proof. */
  crewFiles(
    endpoint: string,
    method: 'POST' | 'DELETE',
    body: unknown
  ): Promise<{ ok: boolean; body: unknown }>;
  /** True once the window or its document is gone. */
  isClosed(): boolean;
}

/** The opaque file capability the renderer gets back. */
export interface CrewPickedFile {
  capability_id: string;
  name: string;
  size?: number;
}

/**
 * The whole flow behind `crew:select-transfer-file`: the secure native window (Open for an
 * upload, Save for a download, a folder for a temporary download's cleanup), the daemon's
 * registration of what was chosen, and for a download onto an existing file the replace
 * confirmation. Only the daemon's opaque capability goes back to the renderer.
 *
 * A refusal throws one sentence: the code's own ({@link crewFileRefusal}), else the daemon's
 * sentence ({@link daemonRefusalSentence}), else a general one. The caller holds the window in a
 * {@link CrewSheetGate} for the whole flow.
 */
export async function selectCrewTransferFile(
  request: CrewPickerRequest,
  deps: CrewPickerDeps
): Promise<CrewPickedFile | null> {
  let selected: string | undefined;
  if (request.purpose === 'cleanup') {
    const original = request.suggestedName;
    if (
      typeof original !== 'string' ||
      !original ||
      path.basename(original) !== original ||
      ['.', '..'].includes(original)
    )
      throw new Error('The original download filename is unavailable.');
    // The name is the file's own, used to find it; the dialog shows it made visible.
    const shown = visibleText(original);
    const folder = await deps.showOpenDialog({
      title: `Locate the original folder for ${shown}`,
      properties: ['openDirectory'],
    });
    if (folder.canceled || !folder.filePaths[0]) return null;
    selected = path.join(folder.filePaths[0], original);
    const cleanup = await deps.showMessageBox({
      type: 'question',
      title: 'Remove incomplete Crew download',
      message: `Remove the temporary download for ${shown}?`,
      detail:
        'Only this transfer’s verified temporary file will be removed. The destination file and the attachment in Crew are kept.',
      buttons: ['Cancel', 'Remove temporary file'],
      defaultId: 0,
      cancelId: 0,
      noLink: true,
    });
    if (cleanup.response !== 1) return null;
  } else if (request.direction === 'upload') {
    const result = await deps.showOpenDialog({
      title: 'Choose a file for Crew',
      properties: ['openFile'],
    });
    if (!result.canceled) selected = result.filePaths[0];
  } else {
    const result = await deps.showSaveDialog({
      title: 'Save Crew file',
      defaultPath: crewSaveName(request.suggestedName),
    });
    if (!result.canceled) selected = result.filePath;
  }
  if (!selected) return null;
  const chosen = selected;
  const pendingDownload = request.direction === 'download' && request.purpose !== 'cleanup';

  const postSelection = async (
    endpoint: string,
    body: Record<string, unknown>,
    method: 'POST' | 'DELETE' = 'POST'
  ): Promise<Record<string, unknown> | null> => {
    if (deps.isClosed()) throw new Error('The file selection window closed.');
    const answer = await deps.crewFiles(endpoint, method, body);
    if (!answer.ok) {
      const coded = crewFileRefusal(answer.body, request.direction, path.basename(chosen), {
        expectedMode: request.expectedMode,
      });
      if (coded) throw new Error(coded);
      if (endpoint)
        throw new Error(
          'The selected destination could not be confirmed. Choose the destination again and review any replacement request.'
        );
      throw new Error(
        daemonRefusalSentence(answer.body) ??
          'The daemon refused this file selection. Choose an accessible file or a new destination filename.'
      );
    }
    if (method === 'DELETE') return null;
    return answer.body && typeof answer.body === 'object'
      ? (answer.body as Record<string, unknown>)
      : null;
  };

  let result = await postSelection('', {
    direction: request.direction,
    purpose: request.purpose,
    path: chosen,
    overwrite: pendingDownload,
    approval_pending: pendingDownload,
    connection_id: request.connectionId,
    channel_id: request.channelId,
    blob_id: request.blobId,
    transfer_id: request.transferId,
    expected_mode: request.expectedMode,
  });
  if (
    typeof result?.capability_id !== 'string' ||
    !ID.test(result.capability_id) ||
    typeof result.name !== 'string'
  )
    throw new Error('Invalid daemon file capability.');
  if (pendingDownload) {
    if (typeof result.target_exists !== 'boolean')
      throw new Error(
        'The daemon did not verify this destination. Update the daemon before downloading.'
      );
    if (deps.isClosed()) throw new Error('The file selection window closed.');
    const capabilityId = result.capability_id;
    if (result.target_exists) {
      const replacement = await deps.showMessageBox({
        type: 'warning',
        title: 'Replace Crew download destination',
        message: `Replace ${visibleText(path.basename(chosen))} after the download is verified?`,
        detail:
          'The daemon has checked the existing file. It remains in place until the download passes verification; any destination change requires a new selection.',
        buttons: ['Cancel', 'Replace file'],
        defaultId: 0,
        cancelId: 0,
        noLink: true,
      });
      if (replacement.response !== 1) {
        await postSelection(`/${encodeURIComponent(capabilityId)}`, {}, 'DELETE').catch(
          () => undefined
        );
        return null;
      }
    }
    result = await postSelection(`/${encodeURIComponent(capabilityId)}/confirm`, {});
    if (result?.capability_id !== capabilityId || typeof result.name !== 'string')
      throw new Error('The daemon did not confirm the selected destination. Choose it again.');
  }
  return {
    capability_id: result.capability_id as string,
    name: result.name as string,
    ...(typeof result.size === 'number' ? { size: result.size } : {}),
  };
}
