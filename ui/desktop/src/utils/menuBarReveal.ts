/**
 * Bringing Biorouter's windows forward from the menu bar icon: a click on the icon (macOS and
 * Windows) and the Home, New Chat, Settings, Extensions and Skills items of its menu. Free of
 * Electron so the rules can be tested; `main.ts` and `autoUpdater.ts` pass real windows.
 *
 * - Chat windows only. The hidden dictation capture window and a dismissed dictation review card
 *   are BrowserWindows too, and revealing every window brought them back on screen, or showed
 *   one of them in place of opening a chat when no chat window was left.
 * - On macOS, `show()` every time. `focus()` alone left a window that was already visible behind
 *   the app in front (#550); `show()` activates Biorouter as well. Elsewhere only a hidden window
 *   is shown, as before.
 * - A minimized window is restored first, on every platform.
 * - Nothing is moved. The icon's click used to shift window N by 30 × N px on every click.
 */
export interface MenuBarWindow {
  isDestroyed(): boolean;
  isMinimized(): boolean;
  isVisible(): boolean;
  restore(): void;
  show(): void;
  focus(): void;
}

export function menuBarWindows<W extends MenuBarWindow>(
  windows: readonly W[],
  isChatWindow: (win: W) => boolean
): W[] {
  return windows.filter((win) => !win.isDestroyed() && isChatWindow(win));
}

export function revealFromMenuBar(win: MenuBarWindow, platform: string): void {
  if (win.isMinimized()) win.restore();
  if (platform === 'darwin' || !win.isVisible()) win.show();
  win.focus();
}
