import { exportSession } from '../api';
import { toastError, toastSuccess } from '../toasts';
import { userActionHeaders } from './userAction';

/** The title of the toast a failed export shows; its message says why. */
export const EXPORT_FAILED_TOAST_TITLE = "Couldn't export this chat";

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
  return 'The chat could not be read for export. Nothing was downloaded.';
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
 * Export one chat as JSON and hand it to the browser as a download named after
 * the chat. The one export path, shared by History and the sidebar row menu.
 *
 * The read carries the user's proof, like every read of a chat's transcript:
 * the export route refuses a private chat to a caller without it. A refusal is
 * shown as the daemon's own sentence.
 *
 * @returns true when the file was handed over, false when the export failed
 *   (a toast has already said why).
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

  const blob = new Blob([json], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = `${name}.json`;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
  toastSuccess({
    title: 'Chat exported',
    msg: `"${name}" was downloaded.`,
  });
  return true;
}
