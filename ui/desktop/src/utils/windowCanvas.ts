/**
 * The colour a chat window paints wherever the renderer has not.
 *
 * WHY THIS EXISTS. A window's frame and the page inside it are drawn by different
 * processes. When the window changes size faster than the compositor hands over a
 * frame at the new size — a live resize on a loaded machine, a GPU process that
 * is descheduled for a moment, a shrink that lands one step ahead of its frame —
 * the part of the window the last frame does not cover shows whatever the NATIVE
 * window paints behind the page. On macOS that used to be the `vibrancy: 'window'`
 * material over Electron's default `#FFF`: a flat band with no app pixels in it,
 * glaring on a dark app, and a blank hole where panes, tabs and borders belong on
 * a light one. Measured with the band detector in
 * docs/desktop-ui/window-scaling-regressions.md ("Unpainted window area").
 *
 * The fix makes that fallback the app's own canvas, so a late frame reads as the
 * app still settling instead of as a hole. It does not make the frame any less late:
 * the uncovered part is still there for as long as the frame is, in the canvas
 * colour — which in light mode is the same white it always was. A chat window is created with the
 * canvas of the theme it last showed (remembered in settings) and the renderer
 * reports every resolved-theme change over `set-window-canvas`.
 *
 * ⚠ The values are `--background-app` in `styles/main.css` — the body's
 * background, so the colour the page itself paints edge to edge. They are the
 * same in every theme family (one neutral set), and `windowCanvas.test.ts` reads
 * every family block to keep them that way: a canvas moved in main.css without
 * this file brings the band back in exactly the theme that moved.
 *
 * No Electron and no DOM here, so the main process and the renderer both import it.
 */
export type WindowCanvasMode = 'light' | 'dark';

export const WINDOW_CANVAS: Readonly<Record<WindowCanvasMode, string>> = Object.freeze({
  light: '#ffffff',
  dark: '#131312',
});

/** The IPC payload is a renderer's word: accept exactly the two modes, never a colour. */
export function isWindowCanvasMode(value: unknown): value is WindowCanvasMode {
  return value === 'light' || value === 'dark';
}

/**
 * The canvas a window is CREATED with, before its renderer has said anything.
 *
 * The remembered mode wins: it is the theme the app last showed, which is the one
 * the page is about to paint — including for someone whose app theme differs from
 * the OS. With nothing remembered (a first launch) the page follows the OS,
 * because `loadThemePreference` resolves an unset preference to `system`, so the
 * window does too.
 */
export function initialWindowCanvas(remembered: unknown, systemPrefersDark: boolean): string {
  if (isWindowCanvasMode(remembered)) return WINDOW_CANVAS[remembered];
  return WINDOW_CANVAS[systemPrefersDark ? 'dark' : 'light'];
}
