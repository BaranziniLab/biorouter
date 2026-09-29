/**
 * The sentence inside an error the main process threw from an `ipcMain.handle` handler.
 *
 * Electron does not hand the renderer the main process's `Error`. `ipcRenderer.invoke` rejects
 * with a new one whose message wraps the original: `Error invoking remote method
 * 'crew:select-transfer-file': Error: Finish the open Save or Open window first.` The wrapper is
 * machinery, never words for a person, and it reached the screen as written wherever a caller
 * showed `error.message` (FILES-F6: the file picker's refusals read that way while the drop path's
 * read cleanly).
 *
 * Unwrapped once, where the IPC is called, so every surface after it shows the sentence the main
 * process wrote. `components/crew/dialogs/KeysDialog.tsx` keeps a private copy of the same pattern.
 */
const IPC_WRAPPER = /^Error invoking remote method '[^']*':\s*(?:[A-Za-z]*Error:\s*)?/;

/** The main process's own sentence, without Electron's wrapper. `''` when nothing is left. */
export function ipcErrorMessage(message: string): string {
  return message.replace(IPC_WRAPPER, '').trim();
}

/**
 * `error` with Electron's invoke wrapper taken off its message, as a plain `Error`. Anything that
 * is not an `Error`, or whose message carries no wrapper, comes back unchanged. A wrapper with
 * nothing inside it becomes `fallback`.
 */
export function unwrapIpcError(error: unknown, fallback = 'The request failed.'): unknown {
  if (!(error instanceof Error) || !IPC_WRAPPER.test(error.message)) return error;
  return Object.assign(new Error(ipcErrorMessage(error.message) || fallback), { cause: error });
}

/** What to show for a failure: the unwrapped sentence of an `Error`, else `fallback`. */
export function failureSentence(error: unknown, fallback: string): string {
  if (!(error instanceof Error)) return fallback;
  return ipcErrorMessage(error.message) || fallback;
}
