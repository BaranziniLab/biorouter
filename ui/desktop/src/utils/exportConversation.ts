import { exportSession } from '../api';
import { chatRowCopy } from '../components/chats/copy';
import { toastError, toastSuccess } from '../toasts';
import { isBrowserSurface } from './surface';
import { userActionHeaders } from './userAction';

const copy = chatRowCopy.export;

/** The title of the toast a failed export shows; its message says why. */
export const EXPORT_FAILED_TOAST_TITLE = copy.failed;

/**
 * What to tell the person when an export fails.
 *
 * A refusal from the daemon arrives as its plain-text body (the generated
 * client throws the parsed body under `throwOnError`), and that sentence is
 * written for a person: it says why and what to do instead. Anything else (an
 * empty 404 body, a network error) gets a sentence of its own rather than a
 * raw status or `[object Object]`.
 */
export function exportFailureMessage(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error.trim();
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  return copy.readFailed;
}

/**
 * The file name an export suggests: the chat's name, made safe as one file
 * name on every system (no folder separators, nothing Windows refuses, no
 * leading dot that would hide it), then `.json`.
 */
export function chatExportFileName(name: string): string {
  const base = name
    // eslint-disable-next-line no-control-regex
    .replace(/[\u0000-\u001f\u007f]/g, '')
    .replace(/[/\\:*?"<>|]/g, '-')
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/^\.+/, '')
    .slice(0, 120)
    .trim();
  return `${base || copy.untitled}.json`;
}

/** The saved file's name and the name of the folder it went to, for the toast. */
export function savedFileParts(filePath: string): { file: string; folder: string } {
  const parts = filePath.split(/[\\/]+/).filter(Boolean);
  const file = parts.pop() ?? filePath;
  return { file, folder: parts.pop() ?? filePath };
}

export interface ExportConversationOptions {
  /**
   * Which toasts go when the person leaves the screen. History's export is
   * about that screen's row, so it passes `'screen'` (T3-SH-11); the sidebar is
   * on every screen, so its toasts are app-scoped.
   */
  scope?: 'screen' | 'app';
}

/**
 * Export one chat as JSON, named after the chat. The one export path, shared by
 * History and the sidebar row menu.
 *
 * The read carries the user's proof, like every read of a chat's transcript:
 * the export route refuses a private chat to a caller without it. A refusal is
 * shown as the daemon's own sentence.
 *
 * In the app the file is saved through the main process: its Save window, then
 * a write of the file, and only a written file is announced, naming the file
 * and its folder (vision V4-R5-2). A Save window the person cancels says
 * nothing. It used to be a browser download, which in Electron opens the same
 * Save window but tells the page nothing, so "was downloaded" showed while the
 * window was still open and stayed after a cancel.
 *
 * In a browser (`biorouter serve`) the app has no Save window of its own (the
 * shim's would write on the server), so the file is handed to the browser as a
 * download, and the browser's own download list says whether and where it was
 * saved; the page cannot know, so it claims nothing.
 *
 * @returns true when the file was written (or handed to the browser), false
 *   when it was not: the read failed or the write failed (a toast has said
 *   why), or the person cancelled the Save window.
 */
export async function exportConversation(
  sessionId: string,
  name: string,
  options: ExportConversationOptions = {}
): Promise<boolean> {
  const scope = options.scope === 'app' ? undefined : options.scope;
  let json: string;
  try {
    const response = await exportSession({
      path: { session_id: sessionId },
      headers: await userActionHeaders(),
      throwOnError: true,
    });
    json = response.data;
  } catch (error) {
    console.error('Failed to export session:', error);
    toastError({
      title: EXPORT_FAILED_TOAST_TITLE,
      msg: exportFailureMessage(error),
      ...(scope ? { scope } : {}),
    });
    return false;
  }

  const fileName = chatExportFileName(name);
  const electron = window.electron;
  if (isBrowserSurface() || !electron?.showSaveDialog || !electron?.writeFile) {
    downloadInBrowser(json, fileName);
    return true;
  }

  const choice = await electron.showSaveDialog({
    title: copy.dialogTitle,
    defaultPath: fileName,
    filters: [{ name: copy.fileType, extensions: ['json'] }],
  });
  if (choice.canceled || !choice.filePath) return false;

  const { file, folder } = savedFileParts(choice.filePath);
  let written = false;
  try {
    written = await electron.writeFile(choice.filePath, json);
  } catch (error) {
    console.error('Failed to write the exported chat:', error);
  }
  if (!written) {
    toastError({
      title: EXPORT_FAILED_TOAST_TITLE,
      msg: copy.writeFailed(folder),
      ...(scope ? { scope } : {}),
    });
    return false;
  }
  toastSuccess({ title: copy.saved, msg: copy.savedTo(file, folder) });
  return true;
}

function downloadInBrowser(json: string, fileName: string) {
  const blob = new Blob([json], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = fileName;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
}
