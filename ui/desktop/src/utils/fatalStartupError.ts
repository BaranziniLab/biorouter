/**
 * What the app does when startup fails: log it, tell the person, and quit.
 *
 * A fatal startup error must never leave a process that ignores SIGTERM. On Linux the
 * synchronous `dialog.showErrorBox` runs a nested GTK loop on the main thread, and while it is
 * open Electron's own SIGTERM and SIGINT handling cannot run: the 1.92.0 RPM smoke test sat there
 * until SIGKILL. The Promise `showMessageBox` leaves the main loop running, so a quit request
 * (a signal, a logout) still ends the process, and the dialog's own button quits as before.
 *
 * macOS and Windows keep the synchronous box. Parentless macOS dialogs run a native modal loop
 * even with the Promise API, so changing it there buys nothing.
 */

export interface FatalStartupDeps {
  platform: typeof process.platform;
  /** Synchronous: the log line must be on disk before any dialog opens. */
  logStartupFailure: (error: unknown) => void;
  showErrorBox: (title: string, content: string) => void;
  showMessageBox: (options: {
    type: 'error';
    title: string;
    message: string;
    buttons: string[];
  }) => Promise<unknown>;
  quit: () => void;
  /** True once the app has started quitting, for example because it was sent SIGTERM. */
  isQuitting: () => boolean;
}

export const FATAL_STARTUP_TITLE = 'Biorouter Error';

export function reportFatalStartupError(error: unknown, deps: FatalStartupDeps): Promise<void> {
  deps.logStartupFailure(error);
  // Quitting already (a signal closed the secure prompt, which is what failed): the person asked
  // for the app to stop, so do not put a dialog in the way of that.
  if (deps.isQuitting()) {
    deps.quit();
    return Promise.resolve();
  }
  const content = `Failed to create main window: ${error}`;
  if (deps.platform === 'darwin' || deps.platform === 'win32') {
    deps.showErrorBox(FATAL_STARTUP_TITLE, content);
    deps.quit();
    return Promise.resolve();
  }
  return deps
    .showMessageBox({
      type: 'error',
      title: FATAL_STARTUP_TITLE,
      message: content,
      buttons: ['Quit'],
    })
    .then(
      () => undefined,
      () => undefined
    )
    .finally(() => deps.quit());
}
